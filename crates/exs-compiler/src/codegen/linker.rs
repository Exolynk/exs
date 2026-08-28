//! Runtime-template reencoding and final executable module linking.

use std::collections::{HashMap, HashSet};

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, CANCEL_EXPORT, MODULE_METADATA_SECTION, RESUME_HOST_EXPORT,
    START_EXPORT_PREFIX,
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
    FunctionCompiler, FunctionCompilerContext, FunctionSignature, LiftedFunction, MethodRegistry,
    add_program_types, build_signatures,
};
use super::literals::{LiteralPool, TemplateDataLayout, template_data_layout};
use super::source_map::{SOURCE_MAP_SECTION, SOURCES_SECTION, SourceMap};
use super::trait_registry::TraitRegistry;
use super::types::TypeRegistry;
use super::{diagnostics, module_span};
use crate::CompileOptions;
use crate::ast::{
    AssignmentTarget, Block, Expression, FunctionDeclaration, FunctionVisibility, Identifier,
    Module, Parameter, Statement,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};
use crate::hir::HirModule;

/// Builds the stable Wasm export name for one runner-callable source function.
fn entry_export_name(name: &str) -> String {
    format!("{START_EXPORT_PREFIX}{name}")
}

/// Links one source module against the committed runtime template.
pub(super) fn link<'source>(
    module: &Module<'source>,
    sources: &[crate::SourceInput<'source>],
    options: CompileOptions,
    lifted: Vec<LiftedFunction<'source>>,
    suspendable_functions: &HashSet<String>,
    traits: &TraitRegistry<'source>,
) -> Result<Vec<u8>, CompileDiagnostics<'source>> {
    let literal_pool = LiteralPool::collect(module);
    let template_data = template_data_layout(module)?;
    let source_map = SourceMap::collect(module);
    let mut linker = TemplateLinker::new(
        module,
        lifted,
        literal_pool,
        template_data,
        source_map,
        suspendable_functions.clone(),
        traits,
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
    let entries = module
        .functions
        .iter()
        .filter(|function| function.visibility == FunctionVisibility::Public)
        .map(|function| format!("entry={}\n", function.name.name))
        .collect::<String>();
    wasm.section(&CustomSection {
        name: MODULE_METADATA_SECTION.into(),
        data: format!("abi={ABI_VERSION}\n{entries}").into_bytes().into(),
    });
    wasm.section(&CustomSection {
        name: SOURCE_MAP_SECTION.into(),
        data: linker.source_map.encode().into(),
    });
    if options.embed_sources {
        wasm.section(&CustomSection {
            name: SOURCES_SECTION.into(),
            data: linker.source_map.encode_source(sources).into(),
        });
    }
    Ok(wasm.finish())
}

/// Copies closure source bodies and HIR capture data into linker-owned lifted declarations.
pub(super) fn lifted_functions<'source>(
    module: &Module<'source>,
    hir: &HirModule<'_>,
) -> Vec<LiftedFunction<'source>> {
    let mut sources = Vec::new();
    for function in &module.functions {
        collect_closures_block(&function.body, &mut sources);
    }
    for implementation in &module.implementations {
        for method in &implementation.methods {
            collect_closures_block(&method.body, &mut sources);
        }
    }
    let metadata = hir.closures().collect::<Vec<_>>();
    debug_assert_eq!(sources.len(), metadata.len());
    sources
        .into_iter()
        .zip(metadata)
        .map(|((parameters, body, span), closure)| LiftedFunction {
            key: format!("$closure:{}", closure.id().0),
            declaration: FunctionDeclaration {
                visibility: FunctionVisibility::Private,
                name: Identifier {
                    name: format!("$closure:{}", closure.id().0),
                    span,
                },
                parameters: parameters.to_vec(),
                return_type: None,
                body: body.clone(),
                span,
            },
            captures: closure
                .captures()
                .iter()
                .map(|capture| capture.name.to_owned())
                .collect(),
        })
        .collect()
}

/// Collects closure expressions in the same pre-order traversal used by HIR lowering.
fn collect_closures_block<'source, 'ast>(
    block: &'ast Block<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    for statement in &block.statements {
        collect_closures_statement(statement, closures);
    }
}

/// Collects closure expressions nested inside one source statement.
fn collect_closures_statement<'source, 'ast>(
    statement: &'ast Statement<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    match statement {
        Statement::Let { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => collect_closures_expression(value, closures),
        Statement::Assign { target, value, .. } => {
            match target {
                AssignmentTarget::Variable(_) => {}
                AssignmentTarget::Index {
                    receiver, index, ..
                } => {
                    collect_closures_expression(receiver, closures);
                    collect_closures_expression(index, closures);
                }
                AssignmentTarget::Property { receiver, .. } => {
                    collect_closures_expression(receiver, closures);
                }
            }
            collect_closures_expression(value, closures);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                collect_closures_expression(value, closures);
            }
        }
        Statement::Block { block, .. } => collect_closures_block(block, closures),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            collect_closures_expression(condition, closures);
            collect_closures_block(then_block, closures);
            if let Some(else_branch) = else_branch {
                match else_branch {
                    crate::ast::ElseBranch::Block(block) => collect_closures_block(block, closures),
                    crate::ast::ElseBranch::If(statement) => {
                        collect_closures_statement(statement, closures)
                    }
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            collect_closures_expression(condition, closures);
            collect_closures_block(body, closures);
        }
        Statement::For { iterable, body, .. } => {
            collect_closures_expression(iterable, closures);
            collect_closures_block(body, closures);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Collects closure expressions nested inside one source expression.
fn collect_closures_expression<'source, 'ast>(
    expression: &'ast Expression<'source>,
    closures: &mut Vec<(
        &'ast [Parameter<'source>],
        &'ast Block<'source>,
        crate::SourceSpan<'source>,
    )>,
) {
    match expression {
        Expression::Closure {
            parameters,
            body,
            span,
        } => {
            closures.push((parameters, body, *span));
            collect_closures_block(body, closures);
        }
        Expression::FormattedString { parts, .. } => {
            for part in parts {
                if let crate::ast::FormattedStringPart::Expression(expression) = part {
                    collect_closures_expression(expression, closures);
                }
            }
        }
        Expression::ParallelStatic { tasks, .. } => {
            for task in tasks {
                collect_closures_expression(task, closures);
            }
        }
        Expression::ParallelDynamic { functions, .. } => {
            collect_closures_expression(functions, closures);
        }
        Expression::IsError { value, .. }
        | Expression::Propagate { value, .. }
        | Expression::Unary { operand: value, .. }
        | Expression::Property {
            receiver: value, ..
        } => collect_closures_expression(value, closures),
        Expression::Binary { left, right, .. }
        | Expression::Index {
            receiver: left,
            index: right,
            ..
        } => {
            collect_closures_expression(left, closures);
            collect_closures_expression(right, closures);
        }
        Expression::Call { arguments, .. } | Expression::StaticMethodCall { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            collect_closures_expression(name, closures);
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostStream { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::HostTime { arguments, .. } => {
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            collect_closures_expression(receiver, closures);
            for argument in arguments {
                collect_closures_expression(argument, closures);
            }
        }
        Expression::List { elements, .. } => {
            for element in elements {
                collect_closures_expression(element, closures);
            }
        }
        Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
            for property in properties {
                collect_closures_expression(&property.value, closures);
            }
        }
        Expression::Match { value, arms, .. } => {
            collect_closures_expression(value, closures);
            for arm in arms {
                match &arm.body {
                    crate::ast::MatchArmBody::Expression(value) => {
                        collect_closures_expression(value, closures);
                    }
                    crate::ast::MatchArmBody::Block(block) => {
                        collect_closures_block(block, closures);
                    }
                }
            }
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
    }
}

/// Reencodes the runtime template while appending generated program sections.
struct TemplateLinker<'source, 'module> {
    module: &'module Module<'source>,
    traits: &'module TraitRegistry<'source>,
    lifted: Vec<LiftedFunction<'source>>,
    program_types: Vec<u32>,
    signatures: Option<HashMap<String, FunctionSignature>>,
    methods: Option<MethodRegistry>,
    runtime_functions: HashMap<String, u32>,
    type_registry: TypeRegistry,
    entry_names: Vec<String>,
    start_type: Option<u32>,
    abi_version_type: Option<u32>,
    resume_type: Option<u32>,
    cancel_type: Option<u32>,
    dispatch_type: Option<u32>,
    entry_indices: Vec<u32>,
    abi_version_index: Option<u32>,
    resume_index: Option<u32>,
    cancel_index: Option<u32>,
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
        lifted: Vec<LiftedFunction<'source>>,
        literals: LiteralPool,
        template_data: TemplateDataLayout,
        source_map: SourceMap<'source>,
        suspendable_functions: HashSet<String>,
        traits: &'module TraitRegistry<'source>,
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
        let type_registry = TypeRegistry::build(module, traits)?;
        let entry_names = module
            .functions
            .iter()
            .filter(|function| function.visibility == FunctionVisibility::Public)
            .map(|function| function.name.name.clone())
            .collect();
        Ok(Self {
            module,
            traits,
            lifted,
            program_types: Vec::new(),
            signatures: None,
            methods: None,
            runtime_functions: HashMap::new(),
            type_registry,
            entry_names,
            start_type: None,
            abi_version_type: None,
            resume_type: None,
            cancel_type: None,
            dispatch_type: None,
            entry_indices: Vec::new(),
            abi_version_index: None,
            resume_index: None,
            cancel_index: None,
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
            .and_then(|count| count.checked_add(self.lifted.len()))
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
        self.program_types = add_program_types(
            self.module,
            &self.lifted,
            types,
            &self.suspendable_functions,
        );
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
        let cancel_type = types.len();
        types.ty().function([], []);
        self.start_type = Some(start_type);
        self.abi_version_type = Some(abi_version_type);
        self.resume_type = Some(resume_type);
        self.dispatch_type = Some(dispatch_type);
        self.cancel_type = Some(cancel_type);
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
        let signatures =
            build_signatures(self.module, &self.lifted, program_base, &self.type_registry)
                .map_err(reencode::Error::UserError)?;
        self.methods = Some(
            MethodRegistry::build(self.module, self.traits, &self.type_registry, &signatures)
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
                    slot_count: continuation::frame_slot_capacity(function, 0)
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
                        slot_count: continuation::frame_slot_capacity(method, 0)
                            .map_err(reencode::Error::UserError)?,
                    },
                );
            }
        }
        for closure in &self.lifted {
            let signature = signatures
                .get(&closure.key)
                .ok_or_else(|| self.state_error("missing lifted closure function signature"))?;
            self.frame_layouts.insert(
                closure.key.clone(),
                continuation::FrameLayout {
                    function_id: signature.function_id,
                    slot_count: continuation::frame_slot_capacity(
                        &closure.declaration,
                        closure.captures.len(),
                    )
                    .map_err(reencode::Error::UserError)?,
                },
            );
        }
        for type_index in &self.program_types {
            functions.function(*type_index);
        }
        let dispatch_index = program_base + self.program_function_count()?;
        let has_suspendable_entry = self
            .entry_names
            .iter()
            .any(|name| self.suspendable_functions.contains(name));
        self.dispatch_index = has_suspendable_entry.then_some(dispatch_index);
        if has_suspendable_entry {
            functions.function(
                self.dispatch_type
                    .ok_or_else(|| self.state_error("missing dispatcher type"))?,
            );
        }
        let first_entry_index = dispatch_index + u32::from(has_suspendable_entry);
        let entry_count = u32::try_from(self.entry_names.len())
            .map_err(|_| self.state_error("too many root functions"))?;
        self.entry_indices = (0..entry_count)
            .map(|offset| first_entry_index + offset)
            .collect();
        self.abi_version_index = Some(first_entry_index + entry_count);
        if has_suspendable_entry {
            self.resume_index = Some(first_entry_index + entry_count + 1);
            self.cancel_index = Some(first_entry_index + entry_count + 2);
        }
        for _ in &self.entry_names {
            functions.function(
                self.start_type
                    .ok_or_else(|| self.state_error("missing start type"))?,
            );
        }
        functions.function(
            self.abi_version_type
                .ok_or_else(|| self.state_error("missing ABI version type"))?,
        );
        if has_suspendable_entry {
            functions.function(
                self.resume_type
                    .ok_or_else(|| self.state_error("missing resume type"))?,
            );
            functions.function(
                self.cancel_type
                    .ok_or_else(|| self.state_error("missing cancellation type"))?,
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
        for (name, index) in self.entry_names.iter().zip(&self.entry_indices) {
            exports.export(&entry_export_name(name), ExportKind::Func, *index);
        }
        if let Some(resume_index) = self.resume_index {
            exports.export(RESUME_HOST_EXPORT, ExportKind::Func, resume_index);
        }
        if let Some(cancel_index) = self.cancel_index {
            exports.export(CANCEL_EXPORT, ExportKind::Func, cancel_index);
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
                    &self.lifted,
                    methods,
                    &self.type_registry,
                )
                .map(|compiled| compiled.function)
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
                        &self.lifted,
                        methods,
                        &self.type_registry,
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
        for closure in &self.lifted {
            match continuation::compile_function(
                &closure.declaration,
                &closure.key,
                signatures,
                &self.runtime_functions,
                &self.literals.indices,
                &self.source_map,
                &self.frame_layouts,
                &self.lifted,
                methods,
                &self.type_registry,
            ) {
                Ok(compiled) => {
                    codes.function(&compiled.function);
                }
                Err(diagnostics) => body_diagnostics.extend(diagnostics),
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
        for name in &self.entry_names {
            let entry = signatures.get(name).ok_or_else(|| {
                reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                    "E0999",
                    module_span(self.module),
                    format!("missing root function signature for `{name}`"),
                )))
            })?;
            let start = if self.suspendable_functions.contains(name) {
                let frame_layout = self.frame_layouts.get(name).ok_or_else(|| {
                    reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
                        "E0999",
                        module_span(self.module),
                        format!("missing continuation frame layout for `{name}`"),
                    )))
                })?;
                continuation::compile_start(
                    self.module,
                    entry,
                    frame_layout.slot_count,
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
                compile_start(self.module, entry, &self.runtime_functions)
            }
            .map_err(reencode::Error::UserError)?;
            codes.function(&start);
        }

        let mut abi_version = Function::new([]);
        abi_version.instruction(&Instruction::I32Const(ABI_VERSION.cast_signed()));
        abi_version.instruction(&Instruction::End);
        codes.function(&abi_version);
        if dispatcher_index.is_some() {
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
            let cancel = continuation::compile_cancel(self.module, &self.runtime_functions)
                .map_err(reencode::Error::UserError)?;
            codes.function(&cancel);
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
