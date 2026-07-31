//! Runtime-template reencoding and final executable module linking.

use std::collections::{HashMap, HashSet};

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, MODULE_METADATA_SECTION, RESUME_HOST_EXPORT, START_EXPORT,
};
use exs_runtime::WASM_TEMPLATE;
use wasm_encoder::{
    CodeSection, CustomSection, DataCountSection, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, Instruction, Module as WasmModule, SectionId, TypeSection,
    ValType,
    reencode::{self, Reencode},
};
use wasmparser::{ExternalKind, Parser as WasmParser, TypeRef};

use super::continuation;
use super::entry::compile_start;
use super::function::{
    FunctionCompiler, FunctionCompilerContext, FunctionSignature, MethodRegistry,
    add_program_types, build_signatures,
};
use super::literals::{LiteralPool, TemplateDataLayout, template_data_layout};
use super::source_map::{SOURCE_MAP_SECTION, SOURCES_SECTION, SourceMap};
use super::types::TypeRegistry;
use super::{diagnostics, module_span};
use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// Links one source module against the committed runtime template.
pub(super) fn link<'a>(
    module: &Module<'a>,
    source: &'a str,
    options: CompileOptions,
    suspendable_functions: &HashSet<String>,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let literal_pool = LiteralPool::collect(module);
    let template_data = template_data_layout(module)?;
    let source_map = SourceMap::collect(module);
    let mut linker = TemplateLinker::new(
        module,
        literal_pool,
        template_data,
        source_map,
        suspendable_functions.clone(),
    )?;
    let mut wasm = WasmModule::new();
    reencode::utils::parse_core_module(&mut linker, &mut wasm, WasmParser::new(0), WASM_TEMPLATE)
        .map_err(|error| match error {
        reencode::Error::UserError(diagnostics) => diagnostics,
        error => diagnostics(CompileDiagnostic::new(
            "E1001",
            module_span(module),
            format!("could not link runtime template: {error}"),
        )),
    })?;
    wasm.section(&CustomSection {
        name: MODULE_METADATA_SECTION.into(),
        data: format!("abi={ABI_VERSION}\nentry=main\n")
            .into_bytes()
            .into(),
    });
    wasm.section(&CustomSection {
        name: SOURCE_MAP_SECTION.into(),
        data: linker.source_map.encode().into(),
    });
    if options.embed_sources {
        wasm.section(&CustomSection {
            name: SOURCES_SECTION.into(),
            data: linker.source_map.encode_source(source).into(),
        });
    }
    Ok(wasm.finish())
}

/// Reencodes the runtime template while appending generated program sections.
struct TemplateLinker<'source, 'module> {
    module: &'module Module<'source>,
    program_types: Vec<u32>,
    signatures: Option<HashMap<String, FunctionSignature>>,
    methods: Option<MethodRegistry>,
    runtime_functions: HashMap<String, u32>,
    type_registry: TypeRegistry,
    start_type: Option<u32>,
    abi_version_type: Option<u32>,
    resume_type: Option<u32>,
    dispatch_type: Option<u32>,
    start_index: Option<u32>,
    abi_version_index: Option<u32>,
    resume_index: Option<u32>,
    dispatch_index: Option<u32>,
    literals: LiteralPool,
    source_map: SourceMap<'source>,
    template_has_data_count: bool,
    template_has_data_section: bool,
    template_function_import_count: u32,
    suspendable_functions: HashSet<String>,
    frame_layouts: HashMap<String, continuation::FrameLayout>,
}

impl<'source, 'module> TemplateLinker<'source, 'module> {
    /// Creates a linker for one parsed source module.
    fn new(
        module: &'module Module<'source>,
        literals: LiteralPool,
        template_data: TemplateDataLayout,
        source_map: SourceMap<'source>,
        suspendable_functions: HashSet<String>,
    ) -> Result<Self, CompileDiagnostics<'source>> {
        let literal_count = u32::try_from(literals.bytes.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0210",
                module_span(module),
                "too many string literals for the Wasm data index space",
            ))
        })?;
        let _total_data_count =
            template_data
                .count
                .checked_add(literal_count)
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0210",
                        module_span(module),
                        "too many data segments for the Wasm data index space",
                    ))
                })?;
        let type_registry = TypeRegistry::build(module)?;
        Ok(Self {
            module,
            program_types: Vec::new(),
            signatures: None,
            methods: None,
            runtime_functions: HashMap::new(),
            type_registry,
            start_type: None,
            abi_version_type: None,
            resume_type: None,
            dispatch_type: None,
            start_index: None,
            abi_version_index: None,
            resume_index: None,
            dispatch_index: None,
            literals: literals.with_data_index_base(template_data.count),
            source_map,
            template_has_data_count: template_data.has_data_count,
            template_has_data_section: template_data.has_data_section,
            template_function_import_count: 0,
            suspendable_functions,
            frame_layouts: HashMap::new(),
        })
    }

    /// Converts an internal linker-state failure into a compiler diagnostic.
    fn state_error(&self, message: &str) -> reencode::Error<CompileDiagnostics<'source>> {
        reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
            "E0999",
            module_span(self.module),
            message,
        )))
    }

    /// Returns the total number of template and compiler literal data segments.
    fn total_data_count(&self) -> Result<u32, reencode::Error<CompileDiagnostics<'source>>> {
        let literal_count = u32::try_from(self.literals.bytes.len())
            .map_err(|_| self.state_error("too many compiler string literals"))?;
        self.literals
            .data_index_base
            .checked_add(literal_count)
            .ok_or_else(|| self.state_error("too many Wasm data segments"))
    }

    /// Returns the number of direct and implementation methods linked into this module.
    fn program_function_count(&self) -> Result<u32, reencode::Error<CompileDiagnostics<'source>>> {
        let count = self.module.functions.len().checked_add(
            self.module
                .implementations
                .iter()
                .map(|implementation| implementation.methods.len())
                .sum::<usize>(),
        );
        count
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| self.state_error("too many source functions"))
    }
}

impl<'source> Reencode for TemplateLinker<'source, '_> {
    type Error = CompileDiagnostics<'source>;

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_type_section(self, types, section)?;
        self.program_types = add_program_types(self.module, types, &self.suspendable_functions);
        let start_type = types.len();
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]);
        let abi_version_type = types.len();
        types.ty().function([], [ValType::I32]);
        let resume_type = types.len();
        types
            .ty()
            .function([ValType::I64, ValType::I32, ValType::I32], [ValType::I32]);
        let dispatch_type = types.len();
        types.ty().function([], [ValType::I32]);
        self.start_type = Some(start_type);
        self.abi_version_type = Some(abi_version_type);
        self.resume_type = Some(resume_type);
        self.dispatch_type = Some(dispatch_type);
        Ok(())
    }

    fn parse_import_section(
        &mut self,
        imports: &mut wasm_encoder::ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        let mut function_import_count = 0_u32;
        for imports in section.clone() {
            let imports = imports.map_err(|error| {
                reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                    "E1001",
                    module_span(self.module),
                    error.to_string(),
                )))
            })?;
            for import in imports {
                let (_, import) = import.map_err(|error| {
                    reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                        "E1001",
                        module_span(self.module),
                        error.to_string(),
                    )))
                })?;
                if matches!(import.ty, TypeRef::Func(_) | TypeRef::FuncExact(_)) {
                    function_import_count = function_import_count
                        .checked_add(1)
                        .ok_or_else(|| self.state_error("too many imported runtime functions"))?;
                }
            }
        }
        self.template_function_import_count = function_import_count;
        reencode::utils::parse_import_section(self, imports, section)
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_function_section(self, functions, section)?;
        let program_base = self
            .template_function_import_count
            .checked_add(functions.len())
            .ok_or_else(|| self.state_error("too many runtime functions"))?;
        let signatures = build_signatures(self.module, program_base, &self.type_registry)
            .map_err(reencode::Error::UserError)?;
        self.methods = Some(
            MethodRegistry::build(self.module, &self.type_registry, &signatures)
                .map_err(reencode::Error::UserError)?,
        );
        self.signatures = Some(signatures);
        let signatures = self
            .signatures
            .as_ref()
            .ok_or_else(|| self.state_error("missing program signatures"))?
            .clone();
        for function in &self.module.functions {
            if !self.suspendable_functions.contains(&function.name.name) {
                continue;
            }
            let signature = signatures
                .get(&function.name.name)
                .ok_or_else(|| self.state_error("missing suspendable function signature"))?;
            self.frame_layouts.insert(
                function.name.name.clone(),
                continuation::FrameLayout {
                    function_id: signature.function_id,
                    slot_count: continuation::frame_slot_capacity(function)
                        .map_err(reencode::Error::UserError)?,
                },
            );
        }
        for implementation in &self.module.implementations {
            for method in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, method.name.name);
                if !self.suspendable_functions.contains(&key) {
                    continue;
                }
                let signature = signatures.get(&key).ok_or_else(|| {
                    self.state_error("missing suspendable implementation signature")
                })?;
                self.frame_layouts.insert(
                    key,
                    continuation::FrameLayout {
                        function_id: signature.function_id,
                        slot_count: continuation::frame_slot_capacity(method)
                            .map_err(reencode::Error::UserError)?,
                    },
                );
            }
        }
        for type_index in &self.program_types {
            functions.function(*type_index);
        }
        let dispatch_index = program_base + self.program_function_count()?;
        self.dispatch_index = self
            .suspendable_functions
            .contains("main")
            .then_some(dispatch_index);
        if self.suspendable_functions.contains("main") {
            functions.function(
                self.dispatch_type
                    .ok_or_else(|| self.state_error("missing dispatcher type"))?,
            );
        }
        let start_index = dispatch_index + u32::from(self.suspendable_functions.contains("main"));
        self.start_index = Some(start_index);
        self.abi_version_index = Some(start_index + 1);
        if self.suspendable_functions.contains("main") {
            self.resume_index = Some(start_index + 2);
        }
        functions.function(
            self.start_type
                .ok_or_else(|| self.state_error("missing start type"))?,
        );
        functions.function(
            self.abi_version_type
                .ok_or_else(|| self.state_error("missing ABI version type"))?,
        );
        if self.suspendable_functions.contains("main") {
            functions.function(
                self.resume_type
                    .ok_or_else(|| self.state_error("missing resume type"))?,
            );
        }
        Ok(())
    }

    fn parse_global_section(
        &mut self,
        globals: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_global_section(self, globals, section)
    }

    fn parse_export_section(
        &mut self,
        exports: &mut ExportSection,
        section: wasmparser::ExportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for export in section.clone() {
            let export = export.map_err(|error| {
                reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                    "E1001",
                    module_span(self.module),
                    error.to_string(),
                )))
            })?;
            if export.kind == ExternalKind::Func {
                self.runtime_functions
                    .insert(export.name.to_owned(), export.index);
            }
        }
        reencode::utils::parse_export_section(self, exports, section)?;
        exports.export(
            START_EXPORT,
            ExportKind::Func,
            self.start_index
                .ok_or_else(|| self.state_error("missing start function index"))?,
        );
        if let Some(resume_index) = self.resume_index {
            exports.export(RESUME_HOST_EXPORT, ExportKind::Func, resume_index);
        }
        exports.export(
            ABI_VERSION_EXPORT,
            ExportKind::Func,
            self.abi_version_index
                .ok_or_else(|| self.state_error("missing ABI version function index"))?,
        );
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        codes: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_code_section(self, codes, section)?;
        let signatures = self
            .signatures
            .as_ref()
            .ok_or_else(|| self.state_error("missing program signatures"))?;
        let methods = self
            .methods
            .as_ref()
            .ok_or_else(|| self.state_error("missing implementation methods"))?;
        let mut body_diagnostics = CompileDiagnostics::new();
        let mut main_frame_slot_count = None;
        for function in &self.module.functions {
            let result = if self.suspendable_functions.contains(&function.name.name) {
                continuation::compile_function(
                    function,
                    &function.name.name,
                    signatures,
                    &self.runtime_functions,
                    &self.literals.indices,
                    &self.source_map,
                    &self.frame_layouts,
                    methods,
                )
                .map(|compiled| {
                    if function.name.name == "main" {
                        main_frame_slot_count = Some(compiled.slot_count);
                    }
                    compiled.function
                })
            } else {
                FunctionCompiler::new(
                    function,
                    &function.name.name,
                    FunctionCompilerContext {
                        signatures,
                        runtime: &self.runtime_functions,
                        literals: &self.literals.indices,
                        source_map: &self.source_map,
                        types: &self.type_registry,
                        methods,
                    },
                )
                .and_then(|mut compiler| compiler.compile())
            };
            match result {
                Ok(function) => {
                    codes.function(&function);
                }
                Err(function_diagnostics) => body_diagnostics.extend(function_diagnostics),
            }
        }
        for implementation in &self.module.implementations {
            for method in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, method.name.name);
                let result = if self.suspendable_functions.contains(&key) {
                    continuation::compile_function(
                        method,
                        &key,
                        signatures,
                        &self.runtime_functions,
                        &self.literals.indices,
                        &self.source_map,
                        &self.frame_layouts,
                        methods,
                    )
                    .map(|compiled| compiled.function)
                } else {
                    FunctionCompiler::new(
                        method,
                        &key,
                        FunctionCompilerContext {
                            signatures,
                            runtime: &self.runtime_functions,
                            literals: &self.literals.indices,
                            source_map: &self.source_map,
                            types: &self.type_registry,
                            methods,
                        },
                    )
                    .and_then(|mut compiler| compiler.compile())
                };
                match result {
                    Ok(function) => {
                        codes.function(&function);
                    }
                    Err(function_diagnostics) => body_diagnostics.extend(function_diagnostics),
                }
            }
        }
        if !body_diagnostics.is_empty() {
            body_diagnostics.sort_by_span();
            return Err(reencode::Error::UserError(body_diagnostics));
        }
        let dispatcher_index = self.dispatch_index;
        if dispatcher_index.is_some() {
            let dispatcher = continuation::compile_dispatch(
                self.module,
                signatures,
                &self.frame_layouts,
                &self.runtime_functions,
            )
            .map_err(reencode::Error::UserError)?;
            codes.function(&dispatcher);
        }
        let main = signatures.get("main").ok_or_else(|| {
            reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                "E0200",
                module_span(self.module),
                "missing fn main()",
            )))
        })?;
        let start = if self.suspendable_functions.contains("main") {
            continuation::compile_start(
                self.module,
                main,
                main_frame_slot_count.ok_or_else(|| {
                    reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                        "E0999",
                        module_span(self.module),
                        "missing main continuation frame layout",
                    )))
                })?,
                dispatcher_index.ok_or_else(|| {
                    reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                        "E0999",
                        module_span(self.module),
                        "missing continuation dispatcher index",
                    )))
                })?,
                &self.runtime_functions,
            )
        } else {
            compile_start(self.module, main, &self.runtime_functions)
        }
        .map_err(reencode::Error::UserError)?;
        codes.function(&start);

        let mut abi_version = Function::new([]);
        abi_version.instruction(&Instruction::I32Const(ABI_VERSION.cast_signed()));
        abi_version.instruction(&Instruction::End);
        codes.function(&abi_version);
        if self.suspendable_functions.contains("main") {
            let resume = continuation::compile_resume(
                self.module,
                dispatcher_index.ok_or_else(|| {
                    reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                        "E0999",
                        module_span(self.module),
                        "missing continuation dispatcher index",
                    )))
                })?,
                &self.runtime_functions,
            )
            .map_err(reencode::Error::UserError)?;
            codes.function(&resume);
        }
        Ok(())
    }

    fn parse_data_section(
        &mut self,
        data: &mut DataSection,
        section: wasmparser::DataSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_data_section(self, data, section)?;
        for literal in &self.literals.bytes {
            data.passive(literal.iter().copied());
        }
        Ok(())
    }

    fn data_count(&mut self, count: u32) -> Result<u32, reencode::Error<Self::Error>> {
        count
            .checked_add(
                u32::try_from(self.literals.bytes.len())
                    .map_err(|_| self.state_error("too many compiler string literals"))?,
            )
            .ok_or_else(|| self.state_error("too many Wasm data segments"))
    }

    fn intersperse_section_hook(
        &mut self,
        module: &mut WasmModule,
        after: Option<SectionId>,
        before: Option<SectionId>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        if self.literals.bytes.is_empty() {
            return Ok(());
        }
        if !self.template_has_data_count && before == Some(SectionId::Code) {
            module.section(&DataCountSection {
                count: self.total_data_count()?,
            });
        }
        if !self.template_has_data_section
            && after == Some(SectionId::Code)
            && before != Some(SectionId::Data)
        {
            let mut data = DataSection::new();
            for literal in &self.literals.bytes {
                data.passive(literal.iter().copied());
            }
            module.section(&data);
        }
        Ok(())
    }
}
