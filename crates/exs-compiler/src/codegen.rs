//! Validation and WebAssembly code generation for the Phase-1 language subset.

use std::collections::HashMap;

use exs_abi::{
    ABI_VERSION, ABI_VERSION_EXPORT, MODULE_METADATA_SECTION, START_EXPORT, STATUS_COMPLETE,
};
use exs_runtime::WASM_TEMPLATE;
use exs_value::is_valid_int;
use wasm_encoder::{
    BlockType, CodeSection, CustomSection, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, Instruction, Module as WasmModule, TypeSection, ValType,
    reencode::{self, Reencode},
};
use wasmparser::{ExternalKind, Parser as WasmParser};

use crate::ast::{
    BinaryOperator, Block, Expression, FunctionDeclaration, Module, Statement, UnaryOperator,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Compiles a parsed module into a complete linked Wasm module.
pub fn compile_module<'a>(module: &Module<'a>) -> Result<Vec<u8>, CompileDiagnostics<'a>> {
    let mut linker = TemplateLinker::new(module);
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
}

impl<'source, 'module> TemplateLinker<'source, 'module> {
    /// Creates a linker for one parsed source module.
    fn new(module: &'module Module<'source>) -> Self {
        Self {
            module,
            program_types: Vec::new(),
            signatures: None,
            runtime_functions: HashMap::new(),
            start_type: None,
            abi_version_type: None,
            start_index: None,
            abi_version_index: None,
        }
    }

    /// Converts an internal linker-state failure into a compiler diagnostic.
    fn state_error(&self, message: &str) -> reencode::Error<CompileDiagnostics<'source>> {
        reencode::Error::UserError(diagnostics(CompileDiagnostic::new(
            "E0999",
            module_span(self.module),
            message,
        )))
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
            let mut compiler = FunctionCompiler::new(function, signatures, &self.runtime_functions)
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
        let decode_input = self
            .runtime_functions
            .get("__exs_rt_decode_input")
            .copied()
            .ok_or_else(|| {
                self.state_error("runtime template does not export __exs_rt_decode_input")
            })?;
        let mut start = Function::new([]);
        start.instruction(&Instruction::LocalGet(0));
        start.instruction(&Instruction::LocalGet(1));
        start.instruction(&Instruction::Call(decode_input));
        start.instruction(&Instruction::Call(main.index));
        let set_result = self
            .runtime_functions
            .get("__exs_rt_set_result")
            .copied()
            .ok_or_else(|| {
                self.state_error("runtime template does not export __exs_rt_set_result")
            })?;
        start.instruction(&Instruction::Call(set_result));
        start.instruction(&Instruction::I32Const(STATUS_COMPLETE));
        start.instruction(&Instruction::End);
        codes.function(&start);

        let mut abi_version = Function::new([]);
        abi_version.instruction(&Instruction::I32Const(ABI_VERSION.cast_signed()));
        abi_version.instruction(&Instruction::End);
        codes.function(&abi_version);

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct FunctionSignature {
    index: u32,
    arity: usize,
}

fn build_signatures<'a>(
    module: &Module<'a>,
    program_base: u32,
) -> Result<HashMap<String, FunctionSignature>, CompileDiagnostics<'a>> {
    let mut signatures = HashMap::new();
    for (offset, function) in module.functions.iter().enumerate() {
        if signatures.contains_key(&function.name.name) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0201",
                function.name.span,
                format!("duplicate function `{}`", function.name.name),
            )));
        }
        let mut parameters = HashMap::new();
        for parameter in &function.parameters {
            if parameters.insert(&parameter.name, ()).is_some() {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0202",
                    parameter.span,
                    format!("duplicate parameter `{}`", parameter.name),
                )));
            }
        }
        signatures.insert(
            function.name.name.clone(),
            FunctionSignature {
                index: program_base + offset as u32,
                arity: function.parameters.len(),
            },
        );
    }
    match signatures.get("main") {
        Some(signature) if signature.arity == 1 => Ok(signatures),
        Some(_) => Err(diagnostics(CompileDiagnostic::new(
            "E0203",
            module_span(module),
            "Phase 1 requires fn main(input) with exactly one parameter",
        ))),
        None => Err(diagnostics(CompileDiagnostic::new(
            "E0200",
            module_span(module),
            "missing fn main()",
        ))),
    }
}

fn add_program_types(module: &Module<'_>, types: &mut TypeSection) -> Vec<u32> {
    module
        .functions
        .iter()
        .map(|function| {
            let index = types.len();
            types.ty().function(
                std::iter::repeat_n(ValType::I32, function.parameters.len()),
                [ValType::I32],
            );
            index
        })
        .collect()
}

struct FunctionCompiler<'a, 'module> {
    declaration: &'module FunctionDeclaration<'a>,
    signatures: &'module HashMap<String, FunctionSignature>,
    runtime: &'module HashMap<String, u32>,
    function: Function,
    scopes: Vec<HashMap<String, u32>>,
    next_local: u32,
}

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    fn new(
        declaration: &'module FunctionDeclaration<'a>,
        signatures: &'module HashMap<String, FunctionSignature>,
        runtime: &'module HashMap<String, u32>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let local_count =
            count_lets(&declaration.body) + count_expressions_block(&declaration.body);
        let mut parameters = HashMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.insert(parameter.name.clone(), index as u32);
        }
        Ok(Self {
            declaration,
            signatures,
            runtime,
            function: Function::new([(local_count, ValType::I32)]),
            scopes: vec![parameters],
            next_local: declaration.parameters.len() as u32,
        })
    }

    fn compile(&mut self) -> Result<Function, CompileDiagnostics<'a>> {
        self.compile_block(&self.declaration.body, false)?;
        self.runtime_call("__exs_rt_null_new", self.declaration.span)?;
        self.function.instruction(&Instruction::End);
        let placeholder = Function::new([]);
        Ok(std::mem::replace(&mut self.function, placeholder))
    }

    fn compile_block(
        &mut self,
        block: &Block<'a>,
        new_scope: bool,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if new_scope {
            self.scopes.push(HashMap::new());
        }
        for statement in &block.statements {
            self.compile_statement(statement)?;
        }
        if new_scope {
            let _removed = self.scopes.pop();
        }
        Ok(())
    }

    fn compile_statement(
        &mut self,
        statement: &Statement<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match statement {
            Statement::Let { name, value, .. } => {
                if self
                    .scopes
                    .last()
                    .is_some_and(|scope| scope.contains_key(&name.name))
                {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0204",
                        name.span,
                        format!("duplicate binding `{}`", name.name),
                    )));
                }
                self.compile_expression(value)?;
                let local = self.allocate_local();
                self.function.instruction(&Instruction::LocalSet(local));
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.name.clone(), local);
                }
            }
            Statement::Assign { name, value, .. } => {
                let local = self.lookup(&name.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0205",
                        name.span,
                        format!("unknown binding `{}`", name.name),
                    ))
                })?;
                self.compile_expression(value)?;
                self.function.instruction(&Instruction::LocalSet(local));
            }
            Statement::Return { value, span } => {
                if let Some(value) = value {
                    self.compile_expression(value)?;
                } else {
                    self.runtime_call("__exs_rt_null_new", *span)?;
                }
                self.function.instruction(&Instruction::Return);
            }
            Statement::Expression { expression, .. } => {
                self.compile_expression(expression)?;
                self.function.instruction(&Instruction::Drop);
            }
            Statement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.compile_expression(condition)?;
                self.runtime_call("__exs_rt_condition", condition_span(condition))?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.compile_block(then_block, true)?;
                if let Some(else_block) = else_block {
                    self.function.instruction(&Instruction::Else);
                    self.compile_block(else_block, true)?;
                }
                self.function.instruction(&Instruction::End);
            }
        }
        Ok(())
    }

    fn compile_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match expression {
            Expression::Integer(value, span) => {
                if !is_valid_int(*value) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0206",
                        *span,
                        "integer literal is outside the ExS 56-bit range",
                    )));
                }
                self.function.instruction(&Instruction::I64Const(*value));
                self.runtime_call("__exs_rt_int_new", *span)?;
            }
            Expression::Float(value, span) => {
                self.function
                    .instruction(&Instruction::F64Const((*value).into()));
                self.runtime_call("__exs_rt_float_new", *span)?;
            }
            Expression::Bool(value, span) => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.runtime_call("__exs_rt_bool_new", *span)?;
            }
            Expression::Variable(identifier) => {
                let local = self.lookup(&identifier.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0205",
                        identifier.span,
                        format!("unknown binding `{}`", identifier.name),
                    ))
                })?;
                self.function.instruction(&Instruction::LocalGet(local));
            }
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                if matches!(operator, UnaryOperator::Negate)
                    && let Expression::Integer(value, operand_span) = operand.as_ref()
                {
                    let negative = value
                        .checked_neg()
                        .filter(|value| is_valid_int(*value))
                        .ok_or_else(|| {
                            diagnostics(CompileDiagnostic::new(
                                "E0206",
                                *operand_span,
                                "integer literal is outside the ExS 56-bit range",
                            ))
                        })?;
                    self.function.instruction(&Instruction::I64Const(negative));
                    self.runtime_call("__exs_rt_int_new", *operand_span)?;
                    return Ok(());
                }
                self.compile_expression(operand)?;
                self.runtime_call(
                    match operator {
                        UnaryOperator::Negate => "__exs_rt_neg",
                        UnaryOperator::Not => "__exs_rt_not",
                    },
                    *span,
                )?;
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => match operator {
                BinaryOperator::And => self.compile_logical(left, right, false, *span)?,
                BinaryOperator::Or => self.compile_logical(left, right, true, *span)?,
                _ => {
                    self.compile_expression(left)?;
                    self.compile_expression(right)?;
                    self.runtime_call(runtime_operation(*operator), *span)?;
                }
            },
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                let signature = self.signatures.get(&callee.name).ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0207",
                        callee.span,
                        format!("unknown function `{}`", callee.name),
                    ))
                })?;
                if signature.arity != arguments.len() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "function `{}` expects {} arguments but received {}",
                            callee.name,
                            signature.arity,
                            arguments.len()
                        ),
                    )));
                }
                for argument in arguments {
                    self.compile_expression(argument)?;
                }
                self.function
                    .instruction(&Instruction::Call(signature.index));
            }
        }
        Ok(())
    }

    fn compile_logical(
        &mut self,
        left: &Expression<'a>,
        right: &Expression<'a>,
        is_or: bool,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(left)?;
        self.runtime_call("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        if is_or {
            self.function.instruction(&Instruction::I32Const(1));
            self.runtime_call("__exs_rt_bool_new", span)?;
        } else {
            self.checked_boolean_expression(right)?;
        }
        self.function.instruction(&Instruction::Else);
        if is_or {
            self.checked_boolean_expression(right)?;
        } else {
            self.function.instruction(&Instruction::I32Const(0));
            self.runtime_call("__exs_rt_bool_new", span)?;
        }
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    fn checked_boolean_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let temporary = self.allocate_local();
        self.function.instruction(&Instruction::LocalTee(temporary));
        self.runtime_call("__exs_rt_condition", condition_span(expression))?;
        self.function.instruction(&Instruction::Drop);
        self.function.instruction(&Instruction::LocalGet(temporary));
        Ok(())
    }

    fn runtime_call(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let index = self.runtime.get(name).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0209",
                span,
                format!("runtime template does not export `{name}`"),
            ))
        })?;
        self.function.instruction(&Instruction::Call(index));
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn allocate_local(&mut self) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        local
    }
}

fn runtime_operation(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "__exs_rt_add",
        BinaryOperator::Subtract => "__exs_rt_sub",
        BinaryOperator::Multiply => "__exs_rt_mul",
        BinaryOperator::Equal => "__exs_rt_eq",
        BinaryOperator::NotEqual => "__exs_rt_ne",
        BinaryOperator::LessThan => "__exs_rt_lt",
        BinaryOperator::LessOrEqual => "__exs_rt_le",
        BinaryOperator::GreaterThan => "__exs_rt_gt",
        BinaryOperator::GreaterOrEqual => "__exs_rt_ge",
        BinaryOperator::And | BinaryOperator::Or => unreachable!(),
    }
}

fn count_lets(block: &Block<'_>) -> u32 {
    block
        .statements
        .iter()
        .map(|statement| match statement {
            Statement::Let { .. } => 1,
            Statement::If {
                then_block,
                else_block,
                ..
            } => count_lets(then_block) + else_block.as_ref().map_or(0, count_lets),
            _ => 0,
        })
        .sum()
}

fn count_expressions_block(block: &Block<'_>) -> u32 {
    block
        .statements
        .iter()
        .map(count_expressions_statement)
        .sum()
}

fn count_expressions_statement(statement: &Statement<'_>) -> u32 {
    match statement {
        Statement::Let { value, .. }
        | Statement::Assign { value, .. }
        | Statement::Expression {
            expression: value, ..
        } => count_expressions(value),
        Statement::Return { value, .. } => value.as_ref().map_or(0, count_expressions),
        Statement::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            count_expressions(condition)
                + count_expressions_block(then_block)
                + else_block.as_ref().map_or(0, count_expressions_block)
        }
    }
}

fn count_expressions(expression: &Expression<'_>) -> u32 {
    match expression {
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::Bool(_, _)
        | Expression::Variable(_) => 1,
        Expression::Unary { operand, .. } => 1 + count_expressions(operand),
        Expression::Binary { left, right, .. } => {
            1 + count_expressions(left) + count_expressions(right)
        }
        Expression::Call { arguments, .. } => {
            1 + arguments.iter().map(count_expressions).sum::<u32>()
        }
    }
}

fn condition_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span) | Expression::Float(_, span) | Expression::Bool(_, span) => {
            *span
        }
        Expression::Variable(identifier) => identifier.span,
        Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. } => *span,
    }
}

fn module_span<'a>(module: &Module<'a>) -> SourceSpan<'a> {
    module
        .functions
        .first()
        .map_or_else(|| SourceSpan::empty("<unknown>"), |function| function.span)
}

fn diagnostics(diagnostic: CompileDiagnostic<'_>) -> CompileDiagnostics<'_> {
    CompileDiagnostics::from(diagnostic)
}
