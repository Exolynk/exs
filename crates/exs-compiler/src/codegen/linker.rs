//! Runtime-template reencoding and final executable module linking.

use std::collections::HashMap;

use exs_abi::{ABI_VERSION, ABI_VERSION_EXPORT, MODULE_METADATA_SECTION, START_EXPORT};
use exs_runtime::WASM_TEMPLATE;
use wasm_encoder::{
    CodeSection, CustomSection, DataCountSection, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, Instruction, Module as WasmModule, SectionId, TypeSection,
    ValType,
    reencode::{self, Reencode},
};
use wasmparser::{ExternalKind, Parser as WasmParser};

use super::entry::compile_start;
use super::function::{FunctionCompiler, FunctionSignature, add_program_types, build_signatures};
use super::literals::{LiteralPool, TemplateDataLayout, template_data_layout};
use super::source_map::{SOURCE_MAP_SECTION, SOURCES_SECTION, SourceMap};
use super::{diagnostics, module_span};
use crate::CompileOptions;
use crate::ast::Module;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

/// Links one source module against the committed runtime template.
pub(super) fn link<'a>(
    module: &Module<'a>,
    source: &'a str,
    options: CompileOptions,
) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let literal_pool = LiteralPool::collect(module);
    let template_data = template_data_layout(module)?;
    let source_map = SourceMap::collect(module);
    let mut linker = TemplateLinker::new(module, literal_pool, template_data, source_map)?;
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
    runtime_functions: HashMap<String, u32>,
    start_type: Option<u32>,
    abi_version_type: Option<u32>,
    start_index: Option<u32>,
    abi_version_index: Option<u32>,
    literals: LiteralPool,
    source_map: SourceMap<'source>,
    template_has_data_count: bool,
    template_has_data_section: bool,
}

impl<'source, 'module> TemplateLinker<'source, 'module> {
    /// Creates a linker for one parsed source module.
    fn new(
        module: &'module Module<'source>,
        literals: LiteralPool,
        template_data: TemplateDataLayout,
        source_map: SourceMap<'source>,
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
        Ok(Self {
            module,
            program_types: Vec::new(),
            signatures: None,
            runtime_functions: HashMap::new(),
            start_type: None,
            abi_version_type: None,
            start_index: None,
            abi_version_index: None,
            literals: literals.with_data_index_base(template_data.count),
            source_map,
            template_has_data_count: template_data.has_data_count,
            template_has_data_section: template_data.has_data_section,
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
}

impl<'source> Reencode for TemplateLinker<'source, '_> {
    type Error = CompileDiagnostics<'source>;

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_type_section(self, types, section)?;
        self.program_types = add_program_types(self.module, types);
        let start_type = types.len();
        types
            .ty()
            .function([ValType::I32, ValType::I32], [ValType::I32]);
        let abi_version_type = types.len();
        types.ty().function([], [ValType::I32]);
        self.start_type = Some(start_type);
        self.abi_version_type = Some(abi_version_type);
        Ok(())
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_function_section(self, functions, section)?;
        let program_base = functions.len();
        self.signatures =
            Some(build_signatures(self.module, program_base).map_err(reencode::Error::UserError)?);
        for type_index in &self.program_types {
            functions.function(*type_index);
        }
        let start_index = program_base + self.module.functions.len() as u32;
        self.start_index = Some(start_index);
        self.abi_version_index = Some(start_index + 1);
        functions.function(
            self.start_type
                .ok_or_else(|| self.state_error("missing start type"))?,
        );
        functions.function(
            self.abi_version_type
                .ok_or_else(|| self.state_error("missing ABI version type"))?,
        );
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
        for function in &self.module.functions {
            let mut compiler = FunctionCompiler::new(
                function,
                signatures,
                &self.runtime_functions,
                &self.literals.indices,
                &self.source_map,
            )
            .map_err(reencode::Error::UserError)?;
            codes.function(&compiler.compile().map_err(reencode::Error::UserError)?);
        }
        let main = signatures.get("main").ok_or_else(|| {
            reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                "E0200",
                module_span(self.module),
                "missing fn main()",
            )))
        })?;
        let start = compile_start(self.module, main, &self.runtime_functions)
            .map_err(reencode::Error::UserError)?;
        codes.function(&start);

        let mut abi_version = Function::new([]);
        abi_version.instruction(&Instruction::I32Const(ABI_VERSION.cast_signed()));
        abi_version.instruction(&Instruction::End);
        codes.function(&abi_version);
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
