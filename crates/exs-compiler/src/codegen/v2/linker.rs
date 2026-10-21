//! Linking of a v2 module: the core template plus the compiled program.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, INPUT_ALLOC_EXPORT, MODULE_METADATA_SECTION,
    RESULT_LENGTH_EXPORT, RESULT_POINTER_EXPORT, START_EXPORT_PREFIX,
};
use exs_abi::{TYPE_BYTES, TYPE_STRING};
use exs_layout::{GC_STATIC, ObjectKind, VmState};
use exs_value::layout::type_kind;
use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, Module as WasmModule, TypeSection, ValType,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef};

use super::entry::EntryLayout;
use super::function::{Context, FunctionCompiler, FunctionInfo, error};
use super::protocols::Protocols;
use super::reachability::{bytes_literals, object_key_lists, reachable_functions, string_literals};
use super::static_keys::StaticKeys;
use super::type_table::TypeTable;
use crate::ast::{FunctionVisibility, Module};
use crate::codegen::prune::{PruneHints, prune_unreachable};
use crate::codegen::source_map::{SOURCE_MAP_SECTION, SourceMap};
use crate::codegen::trait_registry::TraitRegistry;
use crate::codegen::types::TypeRegistry;
use crate::codegen::{VALUE_TYPE, module_span};
use crate::diagnostic::CompileDiagnostics;

/// Facts read from the core template before re-encoding it.
struct TemplateFacts {
    /// Number of types in the template.
    types: u32,
    /// Number of imported functions.
    imported_functions: u32,
    /// Number of defined functions.
    defined_functions: u32,
    /// Indices of every exported function by name.
    exports: HashMap<String, u32>,
    /// Address just past the template's static data and stack.
    heap_base: u32,
    /// Number of globals in the template.
    globals: u32,
}

/// Reads the template's sizes, exports and heap base.
fn template_facts(template: &[u8]) -> Result<TemplateFacts, String> {
    let error = |error: wasmparser::BinaryReaderError| error.to_string();
    let mut facts = TemplateFacts {
        types: 0,
        imported_functions: 0,
        defined_functions: 0,
        exports: HashMap::new(),
        heap_base: 0,
        globals: 0,
    };
    let mut globals = Vec::new();
    let mut heap_base_global = None;
    for payload in Parser::new(0).parse_all(template) {
        match payload.map_err(error)? {
            Payload::TypeSection(section) => facts.types = section.count(),
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(
                        import.map_err(error)?.ty,
                        TypeRef::Func(_) | TypeRef::FuncExact(_)
                    ) {
                        facts.imported_functions += 1;
                    }
                }
            }
            Payload::FunctionSection(section) => facts.defined_functions = section.count(),
            Payload::GlobalSection(section) => {
                for global in section {
                    let mut reader = global.map_err(error)?.init_expr.get_operators_reader();
                    let value = match reader.read().map_err(error)? {
                        Operator::I32Const { value } => Some(value.cast_unsigned()),
                        _ => None,
                    };
                    globals.push(value);
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.map_err(error)?;
                    match export.kind {
                        ExternalKind::Func => {
                            facts.exports.insert(export.name.to_owned(), export.index);
                        }
                        ExternalKind::Global if export.name == "__heap_base" => {
                            heap_base_global = Some(export.index as usize);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    facts.globals = globals.len() as u32;
    facts.heap_base = heap_base_global
        .and_then(|index| globals.get(index).copied().flatten())
        .ok_or_else(|| "the v2 core template exports no constant `__heap_base`".to_owned())?;
    Ok(facts)
}

/// Program additions appended to the template's sections.
struct Program {
    /// Function types, appended after the template's types.
    types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// Type index of every appended function.
    function_types: Vec<u32>,
    /// Bodies of every appended function, in index order.
    bodies: Vec<Function>,
    /// Exports of appended functions.
    exports: Vec<(String, u32)>,
    /// Number of template types, the base of `types`.
    type_base: u32,
}

impl Program {
    /// Returns the index of a function type, appending it on first use.
    fn type_index(&mut self, parameters: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let key = (parameters, results);
        let position = self.types.iter().position(|existing| *existing == key);
        let position = position.unwrap_or_else(|| {
            self.types.push(key);
            self.types.len() - 1
        });
        self.type_base + position as u32
    }
}

/// Compiles `module` with the v2 backend into a linked, pruned Wasm module.
pub(in crate::codegen) fn compile<'a>(
    module: &Module<'a>,
    traits: &TraitRegistry<'a>,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let span = module_span(module);
    let template = exs_core_template::WASM_TEMPLATE;
    let facts = template_facts(template).map_err(|message| error("E1001", span, message))?;
    let types = TypeRegistry::build(module, traits)?;
    let vm = facts.heap_base.next_multiple_of(8);
    let functions = reachable_functions(module);
    let statics = StaticStrings::build(
        &string_literals(&functions, TypeTable::field_names(&types)),
        &bytes_literals(&functions),
        vm + core::mem::size_of::<VmState>() as u32,
    )
    .ok_or_else(|| {
        error(
            "E0210",
            span,
            "too many string literals for linear memory".to_owned(),
        )
    })?;
    let type_table = TypeTable::build(&types, &statics.references, statics.end())
        .ok_or_else(|| error("E0210", span, "too many types for linear memory".to_owned()))?;
    let static_keys = StaticKeys::build(
        object_key_lists(&functions),
        &statics.references,
        type_table.end(),
    )
    .ok_or_else(|| {
        error(
            "E0210",
            span,
            "too many Object literals for linear memory".to_owned(),
        )
    })?;
    let source_map = SourceMap::collect(module);
    let protocols = Protocols::build(module, traits, &types);
    let positions_base = static_keys.end();
    let base = facts.imported_functions + facts.defined_functions;
    let mut program = Program {
        types: Vec::new(),
        function_types: Vec::new(),
        bodies: Vec::new(),
        exports: Vec::new(),
        type_base: facts.types,
    };
    let mut infos = HashMap::new();
    // Like v1, a function's trace id is its position among the module's functions.
    let function_ids: HashMap<&str, u32> = (0_u32..)
        .zip(&module.functions)
        .map(|(id, function)| (function.name.name.as_str(), id))
        .collect();
    for (index, function) in (base..).zip(&functions) {
        let mut parameters = Vec::with_capacity(function.parameters.len());
        for parameter in &function.parameters {
            if parameter.variadic {
                return Err(super::function::unsupported(
                    function.span,
                    "variadic parameters",
                ));
            }
            parameters.push(types.resolve(parameter.type_annotation.as_ref(), function.span)?);
        }
        let return_contract = types.resolve(function.return_type.as_ref(), function.span)?;
        infos.insert(
            function.name.name.clone(),
            FunctionInfo {
                index,
                function_id: function_ids
                    .get(function.name.name.as_str())
                    .copied()
                    .unwrap_or_default(),
                arity: parameters.len() as u32,
                parameters,
                return_contract,
            },
        );
    }
    let int_new = base + functions.len() as u32;
    let float_new = int_new + 1;
    let context = Context {
        core: &facts.exports,
        functions: &infos,
        int_new,
        float_new,
        strings: &statics.references,
        bytes: &statics.bytes_references,
        vm,
        types: &types,
        object_keys: &static_keys.addresses,
        next_global: Cell::new(facts.globals),
        source_map: &source_map,
        positions_base,
        positions: RefCell::new(Vec::new()),
        protocols: &protocols,
    };
    for function in &functions {
        let arity = function.parameters.len();
        let type_index = program.type_index(vec![VALUE_TYPE; arity], vec![VALUE_TYPE]);
        program.function_types.push(type_index);
        program
            .bodies
            .push(FunctionCompiler::new(&context, function).compile()?);
    }
    let positions: Vec<u8> = context
        .positions
        .borrow()
        .iter()
        .flat_map(|position| position.to_le_bytes())
        .collect();
    let heap_start = positions_base + positions.len() as u32;
    let layout = EntryLayout {
        core: &facts.exports,
        vm,
        heap_start,
        types: (type_table.base, type_table.count),
    };
    let int_type = program.type_index(vec![VALUE_TYPE], vec![VALUE_TYPE]);
    program.function_types.push(int_type);
    program
        .bodies
        .push(layout.boxed_number(ObjectKind::BigInt, span)?);
    program.function_types.push(int_type);
    program
        .bodies
        .push(layout.boxed_number(ObjectKind::Float, span)?);
    let mut next = float_new + 1;
    let start_type = program.type_index(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
    let mut entries = String::new();
    for function in &functions {
        if function.visibility != FunctionVisibility::Public {
            continue;
        }
        let info = infos
            .get(&function.name.name)
            .ok_or_else(|| error("E0999", span, "missing v2 entry signature".to_owned()))?;
        program.function_types.push(start_type);
        program.bodies.push(layout.start(info, function.span)?);
        program
            .exports
            .push((format!("{START_EXPORT_PREFIX}{}", function.name.name), next));
        entries.push_str(&format!("entry={}\n", function.name.name));
        next += 1;
    }
    let version_type = program.type_index(Vec::new(), vec![ValType::I32]);
    let allocate_type = program.type_index(vec![ValType::I32], vec![ValType::I32]);
    for (name, type_index, body) in [
        (ABI_VERSION_EXPORT, version_type, EntryLayout::abi_version()),
        (INPUT_ALLOC_EXPORT, allocate_type, layout.input_alloc(span)?),
        (
            RESULT_POINTER_EXPORT,
            version_type,
            layout.vm_accessor("__exs_core_result_ptr", span)?,
        ),
        (
            RESULT_LENGTH_EXPORT,
            version_type,
            layout.vm_accessor("__exs_core_result_len", span)?,
        ),
    ] {
        program.function_types.push(type_index);
        program.bodies.push(body);
        program.exports.push((name.to_owned(), next));
        next += 1;
    }
    let mut linker = Linker {
        program,
        segments: vec![
            (statics.base, statics.bytes),
            (type_table.base, type_table.bytes),
            (static_keys.base, static_keys.bytes),
            (positions_base, positions),
        ],
        globals: context.next_global.get() - facts.globals,
    };
    let mut wasm = WasmModule::new();
    reencode::utils::parse_core_module(&mut linker, &mut wasm, Parser::new(0), template).map_err(
        |failure| {
            error(
                "E1001",
                span,
                format!("could not link the v2 template: {failure:?}"),
            )
        },
    )?;
    wasm.section(&CustomSection {
        name: MODULE_METADATA_SECTION.into(),
        data: format!("abi={ABI_VERSION}\n{entries}").into_bytes().into(),
    });
    wasm.section(&CustomSection {
        name: SOURCE_MAP_SECTION.into(),
        data: source_map.encode().into(),
    });
    let hints = PruneHints {
        weak_caller: None,
        weak_targets: Vec::new(),
        roots: Vec::new(),
    };
    prune_unreachable(&wasm.finish(), &hints).map_err(|message| error("E1001", span, message))
}

/// Static String and Bytes objects placed between the VM state block and the heap.
struct StaticStrings {
    /// Address of the first object.
    base: u32,
    /// Encoded objects, 8-byte aligned.
    bytes: Vec<u8>,
    /// Raw `ValueRef` of every String literal text.
    references: HashMap<String, u64>,
    /// Raw `ValueRef` of every Bytes literal, keyed by its source text.
    bytes_references: HashMap<String, u64>,
}

impl StaticStrings {
    /// Lays out one static String object per String literal and one static Bytes object per
    /// Bytes literal starting at `base`.
    fn build(literals: &[String], bytes_literals: &[String], base: u32) -> Option<Self> {
        let base = base.next_multiple_of(8);
        let mut statics = Self {
            base,
            bytes: Vec::new(),
            references: HashMap::new(),
            bytes_references: HashMap::new(),
        };
        for literal in literals {
            let reference = statics.push(literal, ObjectKind::String, TYPE_STRING)?;
            statics.references.insert(literal.clone(), reference);
        }
        for literal in bytes_literals {
            let reference = statics.push(literal, ObjectKind::Bytes, TYPE_BYTES)?;
            statics.bytes_references.insert(literal.clone(), reference);
        }
        Some(statics)
    }

    /// Appends one static object of `kind` holding the octets of `literal` and returns its
    /// reference; Strings record their character count, Bytes none.
    fn push(&mut self, literal: &str, kind: ObjectKind, type_bit: u32) -> Option<u64> {
        let address = self
            .base
            .checked_add(u32::try_from(self.bytes.len()).ok()?)?;
        let len = u32::try_from(literal.len()).ok()?;
        let chars = match kind {
            ObjectKind::String => u32::try_from(literal.chars().count()).ok()?,
            _ => 0,
        };
        self.bytes.extend([kind as u8, GC_STATIC, 0, 0]);
        self.bytes.extend(chars.to_le_bytes());
        self.bytes.extend(len.to_le_bytes());
        self.bytes.extend(0_u32.to_le_bytes());
        self.bytes.extend(literal.as_bytes());
        self.bytes.resize(self.bytes.len().next_multiple_of(8), 0);
        Some(exs_value::ValueRef::from_parts(type_kind(type_bit), 0, address)?.raw())
    }

    /// Returns the first address after the static objects.
    fn end(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }
}

/// Re-encoder that appends the program to the template's sections.
struct Linker {
    /// The compiled program.
    program: Program,
    /// Address and bytes of every static data segment: the String objects, the type table and
    /// the Object literal keys.
    segments: Vec<(u32, Vec<u8>)>,
    /// Number of mutable `i32` globals the program appends, all initialized to 0.
    globals: u32,
}

impl Reencode for Linker {
    type Error = std::convert::Infallible;

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_type_section(self, types, section)?;
        for (parameters, results) in &self.program.types {
            types
                .ty()
                .function(parameters.iter().copied(), results.iter().copied());
        }
        Ok(())
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_function_section(self, functions, section)?;
        for type_index in &self.program.function_types {
            functions.function(*type_index);
        }
        Ok(())
    }

    fn parse_export_section(
        &mut self,
        exports: &mut ExportSection,
        section: wasmparser::ExportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_export_section(self, exports, section)?;
        for (name, index) in &self.program.exports {
            exports.export(name, ExportKind::Func, *index);
        }
        Ok(())
    }

    fn parse_global_section(
        &mut self,
        globals: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_global_section(self, globals, section)?;
        for _ in 0..self.globals {
            globals.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(0),
            );
        }
        Ok(())
    }

    fn parse_data_section(
        &mut self,
        data: &mut DataSection,
        section: wasmparser::DataSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_data_section(self, data, section)?;
        for (base, bytes) in &self.segments {
            if !bytes.is_empty() {
                data.active(
                    0,
                    &ConstExpr::i32_const(base.cast_signed()),
                    bytes.iter().copied(),
                );
            }
        }
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_code_section(self, code, section)?;
        for body in &self.program.bodies {
            code.function(body);
        }
        Ok(())
    }
}
