//! Linking of a module: the core template plus the compiled program.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, INPUT_ALLOC_EXPORT, MODULE_METADATA_SECTION,
    RESULT_VALUE_EXPORT, START_EXPORT_PREFIX, VM_EXPORT,
};
use exs_abi::{CANCEL_EXPORT, RESUME_HOST_EXPORT};
use exs_layout::value::layout::type_kind;
use exs_layout::{GC_STATIC, ObjectKind, VmState};
use exs_layout::{TYPE_BYTES, TYPE_STRING};
use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, Module as WasmModule, TypeSection, ValType,
};
use wasmparser::{ExternalKind, Operator, Parser, Payload, TypeRef};

use super::closures::{
    ClosureInfo, DispatchCase, closure_dispatcher as closure_dispatcher_body, closure_span_key,
};
use super::dispatch::Dispatch;
use super::entry::EntryLayout;
use super::function::{Context, FunctionCompiler, FunctionInfo, error};
use super::reachability::{
    bytes_literals, changes_case, object_key_lists, reachable_units, string_literals,
};
use super::resumable::{ClosureStep, Resumable, StepEmitter, parallel_units, resumable_units};
use super::static_keys::StaticKeys;
use super::type_table::{CustomBehavior, TypeTable};
use super::units::{Unit, units};
use super::used_types::{declared_types, used_types};
use super::validation::unreached_diagnostics;
use crate::ast::Module;
use crate::codegen::declarations::lifted_functions;
use crate::codegen::prune::{PruneHints, prune_unreachable};
use crate::codegen::source_map::{SOURCE_MAP_SECTION, SOURCES_SECTION, SourceMap};
use crate::codegen::template_calls::template_calls;
use crate::codegen::trait_registry::TraitOperator;
use crate::codegen::trait_registry::TraitRegistry;
use crate::codegen::types::TypeRegistry;
use crate::codegen::{VALUE_TYPE, module_span};
use crate::diagnostic::CompileDiagnostics;
use crate::hir::HirModule;

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
        .ok_or_else(|| "the core template exports no constant `__heap_base`".to_owned())?;
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

/// Compiles `module` into a linked, pruned Wasm module, embedding
/// `sources` when `options` ask for it.
pub(in crate::codegen) fn compile<'a>(
    module: &Module<'a>,
    sources: &[crate::SourceInput<'a>],
    options: crate::CompileOptions,
    traits: &TraitRegistry<'a>,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let span = module_span(module);
    let template = exs_core_template::WASM_TEMPLATE;
    let facts = template_facts(template).map_err(|message| error("E1001", span, message))?;
    let types = TypeRegistry::build(module, traits)?;
    let vm = facts.heap_base.next_multiple_of(8);
    let dispatch = Dispatch::build(module, traits, &types);
    let hir = HirModule::lower(module, traits);
    let lifted = lifted_functions(module, &hir);
    let captured: HashSet<String> = lifted
        .iter()
        .flat_map(|closure| closure.captures.iter().cloned())
        .collect();
    let closure_keys: HashMap<(String, u32, u32), String> = lifted
        .iter()
        .map(|closure| {
            (
                closure_span_key(closure.declaration.span),
                closure.key.clone(),
            )
        })
        .collect();
    let all_units = units(module, &lifted)?;
    let declared = declared_types(module, &types);
    let reached = reachable_units(&all_units, &dispatch, &closure_keys, &types, &declared);
    let reached_keys: HashSet<&str> = reached.iter().map(|unit| unit.key.as_str()).collect();
    let mut diagnostics =
        unreached_diagnostics(module, traits, &types, &lifted, &all_units, &reached_keys);
    let parallel = parallel_units(&reached, &closure_keys);
    let resumable_keys = resumable_units(&hir, &dispatch, &reached_keys, &parallel);
    drop(hir);
    let functions: Vec<_> = reached.iter().map(|unit| unit.declaration).collect();
    let used = used_types(&types, &reached);
    let statics = StaticStrings::build(
        &string_literals(&functions, TypeTable::field_names(&types, &used)),
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
    let custom = CustomBehavior {
        equality: dispatch
            .operator(TraitOperator::Equal)
            .iter()
            .map(|target| target.type_id)
            .collect(),
        rendering: dispatch
            .debug()
            .iter()
            .chain(dispatch.to_string())
            .map(|target| target.type_id)
            .collect(),
    };
    let type_table =
        TypeTable::build(&types, &used, &custom, &statics.references, statics.end())
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
    // The case tables cost data and therefore fuel, so only programs that change the case of
    // Strings carry them.
    let case_tables = changes_case(&functions).then(|| {
        let base = static_keys.end().next_multiple_of(8);
        (base, exs_layout::CASE_TABLES.to_vec())
    });
    let positions_base = case_tables
        .as_ref()
        .map_or(static_keys.end(), |(base, bytes)| base + bytes.len() as u32)
        .next_multiple_of(8);
    let base = facts.imported_functions + facts.defined_functions;
    let mut program = Program {
        types: Vec::new(),
        function_types: Vec::new(),
        bodies: Vec::new(),
        exports: Vec::new(),
        type_base: facts.types,
    };
    let mut infos = HashMap::new();
    for (index, unit) in (base..).zip(&reached) {
        let function = unit.declaration;
        let mut parameters = Vec::with_capacity(function.parameters.len());
        let mut variadic = None;
        for parameter in &function.parameters {
            let contract = types.resolve(parameter.type_annotation.as_ref(), function.span)?;
            if parameter.variadic {
                variadic = Some(contract);
            } else {
                parameters.push(contract);
            }
        }
        let return_contract = types.resolve(function.return_type.as_ref(), function.span)?;
        infos.insert(
            unit.key.clone(),
            FunctionInfo {
                index,
                function_id: unit.function_id,
                arity: parameters.len() as u32,
                parameters,
                variadic,
                captures: unit.captures.len() as u32,
                return_contract,
            },
        );
    }
    let mut resumable = (!resumable_keys.is_empty())
        .then(|| {
            Resumable::build(
                module,
                traits,
                &types,
                &lifted,
                &reached,
                &resumable_keys,
                !parallel.is_empty(),
                |key| infos.get(key).map(|info| info.index),
            )
        })
        .transpose()?;
    let reached_closures: Vec<&&Unit<'_, '_>> = reached
        .iter()
        .filter(|unit| unit.key.starts_with("$closure:"))
        .collect();
    let closures: HashMap<(String, u32, u32), ClosureInfo> = reached_closures
        .iter()
        .zip(0_u32..)
        .map(|(unit, index)| {
            (
                closure_span_key(unit.declaration.span),
                ClosureInfo {
                    index,
                    captures: unit.captures.to_vec(),
                },
            )
        })
        .collect();
    if let Some(resumable) = &mut resumable {
        for (unit, index) in reached_closures.iter().zip(0_u32..) {
            let (Some(step), Some(info)) = (resumable.steps.get(&unit.key), infos.get(&unit.key))
            else {
                continue;
            };
            let closure = ClosureStep {
                step: *step,
                captures: info.captures,
                arity: info.arity,
                variadic: info.variadic.is_some(),
            };
            resumable.closures.insert(index, closure);
        }
    }
    let int_new = base + functions.len() as u32;
    let float_new = int_new + 1;
    let closure_dispatcher = (!reached_closures.is_empty()).then_some(float_new + 1);
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
        static_arrays: RefCell::new(HashMap::new()),
        dispatch: &dispatch,
        captured: &captured,
        closures: &closures,
        closure_dispatcher,
        used_types: &used,
    };
    for unit in &reached {
        if let Some(resumable) = &resumable
            && resumable.steps.contains_key(&unit.key)
        {
            let type_index = program.type_index(vec![ValType::I32], vec![ValType::I32]);
            program.function_types.push(type_index);
            match StepEmitter::compile(&context, resumable, &unit.key, unit.declaration) {
                Ok(body) => program.bodies.push(body),
                Err(error) => diagnostics.extend(error),
            }
            continue;
        }
        let arity = unit.captures.len() + unit.declaration.parameters.len();
        let type_index = program.type_index(vec![VALUE_TYPE; arity], vec![VALUE_TYPE]);
        program.function_types.push(type_index);
        // Every function body reports its errors before the module is rejected.
        match FunctionCompiler::new(&context, &unit.key, unit.declaration, unit.captures).compile()
        {
            Ok(body) => program.bodies.push(body),
            Err(error) => diagnostics.extend(error),
        }
    }
    if !diagnostics.is_empty() {
        diagnostics.sort_by_span();
        return Err(diagnostics);
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
        case_tables: case_tables.as_ref().map_or(0, |(base, _)| *base),
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
    if closure_dispatcher.is_some() {
        let cases = reached_closures
            .iter()
            .map(|unit| {
                infos
                    .get(&unit.key)
                    .map(|info| DispatchCase {
                        function: info.index,
                        captures: info.captures,
                        arity: info.arity,
                        variadic: info.variadic.is_some(),
                        suspends: resumable_keys.contains(&unit.key),
                    })
                    .ok_or_else(|| error("E0999", span, "missing closure".to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dispatcher_type = program.type_index(
            vec![VALUE_TYPE, ValType::I32, ValType::I32],
            vec![VALUE_TYPE],
        );
        program.function_types.push(dispatcher_type);
        program
            .bodies
            .push(closure_dispatcher_body(&layout, &cases, span)?);
        next += 1;
    }
    // The run loop of a module with resumable units, which its entries and resume call.
    let run = match &resumable {
        Some(resumable) => {
            let mut steps: Vec<_> = resumable.steps.values().copied().collect();
            steps.sort_by_key(|step| step.step);
            let indices: Vec<u32> = steps.iter().map(|step| step.index).collect();
            let run_type = program.type_index(Vec::new(), vec![ValType::I32]);
            program.function_types.push(run_type);
            program.bodies.push(layout.run(&indices, span)?);
            next += 1;
            Some(next - 1)
        }
        None => None,
    };
    let start_type = program.type_index(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
    let mut entries = String::new();
    for unit in &reached {
        let function = unit.declaration;
        if !function.entry || unit.key.contains("::") {
            continue;
        }
        let info = infos
            .get(&function.name.name)
            .ok_or_else(|| error("E0999", span, "missing entry signature".to_owned()))?;
        program.function_types.push(start_type);
        let step = resumable
            .as_ref()
            .and_then(|resumable| resumable.steps.get(&unit.key).copied());
        match (step, run) {
            (Some(step), Some(run)) => {
                program
                    .bodies
                    .push(layout.resumable_start(info, step, run, function.span)?)
            }
            _ => program.bodies.push(layout.start(info, function.span)?),
        }
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
        (VM_EXPORT, version_type, layout.vm_address()),
        (
            RESULT_VALUE_EXPORT,
            program.type_index(Vec::new(), vec![VALUE_TYPE]),
            layout.result_value(),
        ),
    ] {
        program.function_types.push(type_index);
        program.bodies.push(body);
        program.exports.push((name.to_owned(), next));
        next += 1;
    }
    if let Some(run) = run {
        let resume_type = program.type_index(
            vec![ValType::I64, ValType::I32, ValType::I32],
            vec![ValType::I32],
        );
        let cancel_type = program.type_index(Vec::new(), Vec::new());
        for (name, type_index, body) in [
            (
                RESUME_HOST_EXPORT,
                resume_type,
                layout.resume_host(run, span)?,
            ),
            (CANCEL_EXPORT, cancel_type, layout.cancel(span)?),
        ] {
            program.function_types.push(type_index);
            program.bodies.push(body);
            program.exports.push((name.to_owned(), next));
            next += 1;
        }
    }
    let mut linker = Linker {
        program,
        segments: vec![
            (statics.base, statics.bytes),
            (type_table.base, type_table.bytes),
            (static_keys.base, static_keys.bytes),
            case_tables.unwrap_or_default(),
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
                format!("could not link the core template: {failure:?}"),
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
    if options.embed_sources {
        wasm.section(&CustomSection {
            name: SOURCES_SECTION.into(),
            data: source_map.encode_source(sources).into(),
        });
    }
    let hints = PruneHints {
        weak_caller: None,
        weak_targets: Vec::new(),
        roots: Vec::new(),
        template: Some(template_calls().map_err(|message| error("E1001", span, message))?),
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
        Some(exs_layout::ValueRef::from_parts(type_kind(type_bit), 0, address)?.raw())
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

    /// Copies the template's bodies unchanged and appends the program's bodies.
    ///
    /// The program only appends types, functions and globals, so no index inside a template
    /// body changes and its bytes need no decoding or re-encoding.
    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for body in section {
            code.raw(body?.as_bytes());
        }
        for body in &self.program.bodies {
            code.function(body);
        }
        Ok(())
    }
}
