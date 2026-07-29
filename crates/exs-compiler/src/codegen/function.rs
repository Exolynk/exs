//! Direct lowering of ExS function bodies to WebAssembly functions.

use std::collections::HashMap;

use exs_value::is_valid_int;
use wasm_encoder::{BlockType, Function, Instruction, TypeSection, ValType};

use crate::ast::{
    BinaryOperator, Block, Expression, FunctionDeclaration, Module, Statement, UnaryOperator,
};
use crate::codegen::{diagnostics, module_span};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// The linked Wasm function index and source arity of one ExS function.
#[derive(Debug, Clone, Copy)]
pub(super) struct FunctionSignature {
    pub(super) index: u32,
    pub(super) arity: usize,
}

/// Validates declarations and assigns their final linked Wasm function indexes.
pub(super) fn build_signatures<'a>(
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

/// Adds one ValueRef-based Wasm signature for every source function.
pub(super) fn add_program_types(module: &Module<'_>, types: &mut TypeSection) -> Vec<u32> {
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

/// Lowers one direct ExS function to a Wasm function.
pub(super) struct FunctionCompiler<'a, 'module> {
    declaration: &'module FunctionDeclaration<'a>,
    signatures: &'module HashMap<String, FunctionSignature>,
    runtime: &'module HashMap<String, u32>,
    literals: &'module HashMap<String, u32>,
    function: Function,
    scopes: Vec<HashMap<String, u32>>,
    next_local: u32,
}

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Prepares direct function lowering with enough ValueRef local slots.
    pub(super) fn new(
        declaration: &'module FunctionDeclaration<'a>,
        signatures: &'module HashMap<String, FunctionSignature>,
        runtime: &'module HashMap<String, u32>,
        literals: &'module HashMap<String, u32>,
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
            literals,
            function: Function::new([(local_count, ValType::I32)]),
            scopes: vec![parameters],
            next_local: declaration.parameters.len() as u32,
        })
    }

    /// Compiles this function body, including the implicit null return path.
    pub(super) fn compile(&mut self) -> Result<Function, CompileDiagnostics<'a>> {
        self.compile_block(&self.declaration.body, false)?;
        self.runtime_call("__exs_rt_null_new", self.declaration.span)?;
        self.function.instruction(&Instruction::End);
        let placeholder = Function::new([]);
        Ok(std::mem::replace(&mut self.function, placeholder))
    }

    /// Compiles statements in one lexical block.
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

    /// Compiles one source statement.
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

    /// Compiles one source expression into a ValueRef on the Wasm stack.
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
            Expression::String(value, span) => {
                let data_index = self.literals.get(value).copied().ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0211",
                        *span,
                        "missing compiler string literal data segment",
                    ))
                })?;
                let length = i32::try_from(value.len()).map_err(|_| {
                    diagnostics(CompileDiagnostic::new(
                        "E0211",
                        *span,
                        "string literal is too large for Wasm linear memory",
                    ))
                })?;
                self.function.instruction(&Instruction::I32Const(length));
                self.runtime_call("__exs_rt_literal_buffer_alloc", *span)?;
                let buffer_pointer = self.allocate_local();
                self.function
                    .instruction(&Instruction::LocalTee(buffer_pointer));
                self.function.instruction(&Instruction::I32Const(0));
                self.function.instruction(&Instruction::I32Const(length));
                self.function
                    .instruction(&Instruction::MemoryInit { mem: 0, data_index });
                self.function
                    .instruction(&Instruction::LocalGet(buffer_pointer));
                self.function.instruction(&Instruction::I32Const(length));
                self.runtime_call("__exs_rt_string_new", *span)?;
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

    /// Compiles short-circuiting boolean conjunction or disjunction.
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

    /// Compiles an expression and verifies it is a boolean without consuming it.
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

    /// Emits one named runtime ABI call after resolving its template function index.
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

    /// Looks up one lexical binding's Wasm local index.
    fn lookup(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Reserves the next preallocated ValueRef local slot.
    fn allocate_local(&mut self) -> u32 {
        let local = self.next_local;
        self.next_local += 1;
        local
    }
}

/// Maps a source binary operator to its runtime ABI operation name.
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

/// Counts local declarations in one block and nested blocks.
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

/// Counts expression scratch-local requirements in one block.
fn count_expressions_block(block: &Block<'_>) -> u32 {
    block
        .statements
        .iter()
        .map(count_expressions_statement)
        .sum()
}

/// Counts expression scratch-local requirements in one statement.
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

/// Counts expression scratch-local requirements recursively.
fn count_expressions(expression: &Expression<'_>) -> u32 {
    match expression {
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
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

/// Returns the source span used for a runtime condition check.
fn condition_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bool(_, span) => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. } => *span,
    }
}
