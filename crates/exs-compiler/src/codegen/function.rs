//! Direct lowering of ExS function bodies to WebAssembly functions.

use std::collections::HashMap;

use exs_value::is_valid_int;
use wasm_encoder::{BlockType, Function, Instruction, TypeSection, ValType};

use crate::ast::{
    AssignmentTarget, BinaryOperator, Block, Expression, FunctionDeclaration, Module,
    ObjectProperty, Statement, UnaryOperator,
};
use crate::codegen::{diagnostics, module_span};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// Extra compiler locals reserved for root-frame return and operand-spill bookkeeping.
const ROOT_FRAME_RESERVED_LOCALS: u32 = 8;

/// The linked Wasm function index and source arity of one ExS function.
#[derive(Debug, Clone, Copy)]
pub(super) struct FunctionSignature {
    pub(super) index: u32,
    pub(super) arity: usize,
}

/// Structured Wasm targets and lexical cleanup data for one active source loop.
#[derive(Clone, Copy)]
struct LoopContext {
    /// Control-stack depth of the enclosing block exited by break.
    break_depth: u32,
    /// Control-stack depth reached by continue.
    continue_depth: u32,
    /// First lexical scope whose roots must be cleared before a loop branch.
    cleanup_scope_start: usize,
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
    loops: Vec<LoopContext>,
    next_local: u32,
    root_frame_local: u32,
    control_depth: u32,
}

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Prepares direct function lowering with enough ValueRef local slots.
    pub(super) fn new(
        declaration: &'module FunctionDeclaration<'a>,
        signatures: &'module HashMap<String, FunctionSignature>,
        runtime: &'module HashMap<String, u32>,
        literals: &'module HashMap<String, u32>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let expression_locals = count_expressions_block(&declaration.body)
            .checked_mul(3)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many expression temporaries for one function",
                ))
            })?;
        let local_count = count_lets(&declaration.body)
            .checked_add(expression_locals)
            .and_then(|count| count.checked_add(ROOT_FRAME_RESERVED_LOCALS))
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many locals for one function",
                ))
            })?;
        let root_slot_count = u32::try_from(declaration.parameters.len())
            .ok()
            .and_then(|parameters| parameters.checked_add(local_count))
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many root slots for one function",
                ))
            })?;
        let root_frame_local = root_slot_count;
        let mut parameters = HashMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.insert(parameter.name.clone(), index as u32);
        }
        let mut compiler = Self {
            declaration,
            signatures,
            runtime,
            literals,
            function: Function::new([(local_count + 1, ValType::I32)]),
            scopes: vec![parameters],
            loops: Vec::new(),
            next_local: declaration.parameters.len() as u32,
            root_frame_local,
            control_depth: 0,
        };
        compiler.initialize_root_frame(root_slot_count)?;
        Ok(compiler)
    }

    /// Compiles this function body, including the implicit null return path.
    pub(super) fn compile(&mut self) -> Result<Function, CompileDiagnostics<'a>> {
        self.compile_block(&self.declaration.body, false)?;
        self.runtime_call("__exs_rt_none_new", self.declaration.span)?;
        self.return_stack_value()?;
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
        if new_scope && let Some(scope) = self.scopes.pop() {
            for local in scope.into_values() {
                self.clear_root_slot(local)?;
            }
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
                self.store_stack_value_in(local)?;
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.name.clone(), local);
                }
            }
            Statement::Assign { target, value, .. } => match target {
                AssignmentTarget::Variable(name) => {
                    let local = self.lookup(&name.name).ok_or_else(|| {
                        diagnostics(CompileDiagnostic::new(
                            "E0205",
                            name.span,
                            format!("unknown binding `{}`", name.name),
                        ))
                    })?;
                    self.compile_expression(value)?;
                    self.store_stack_value_in(local)?;
                }
                AssignmentTarget::Index {
                    receiver,
                    index,
                    span,
                } => {
                    self.compile_expression(receiver)?;
                    let receiver = self.store_stack_value()?;
                    self.compile_expression(index)?;
                    let index = self.store_stack_value()?;
                    self.compile_expression(value)?;
                    let value = self.store_stack_value()?;
                    self.function.instruction(&Instruction::LocalGet(receiver));
                    self.function.instruction(&Instruction::LocalGet(index));
                    self.function.instruction(&Instruction::LocalGet(value));
                    self.runtime_value_call("__exs_rt_index_set", 3, *span)?;
                    self.function.instruction(&Instruction::Drop);
                    self.clear_root_slot(receiver)?;
                    self.clear_root_slot(index)?;
                    self.clear_root_slot(value)?;
                }
                AssignmentTarget::Property {
                    receiver,
                    property,
                    span,
                } => {
                    self.compile_expression(receiver)?;
                    let receiver = self.store_stack_value()?;
                    self.compile_string(&property.name, property.span)?;
                    let property = self.store_stack_value()?;
                    self.compile_expression(value)?;
                    let value = self.store_stack_value()?;
                    self.function.instruction(&Instruction::LocalGet(receiver));
                    self.function.instruction(&Instruction::LocalGet(property));
                    self.function.instruction(&Instruction::LocalGet(value));
                    self.runtime_value_call("__exs_rt_index_set", 3, *span)?;
                    self.function.instruction(&Instruction::Drop);
                    self.clear_root_slot(receiver)?;
                    self.clear_root_slot(property)?;
                    self.clear_root_slot(value)?;
                }
            },
            Statement::Return { value, span } => {
                if let Some(value) = value {
                    self.compile_expression(value)?;
                } else {
                    self.runtime_call("__exs_rt_none_new", *span)?;
                }
                self.return_stack_value()?;
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
                self.runtime_value_call("__exs_rt_condition", 1, condition_span(condition))?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.enter_control()?;
                self.compile_block(then_block, true)?;
                if let Some(else_block) = else_block {
                    self.function.instruction(&Instruction::Else);
                    self.compile_block(else_block, true)?;
                }
                self.function.instruction(&Instruction::End);
                self.exit_control()?;
            }
            Statement::While {
                condition,
                body,
                span,
            } => self.compile_while(condition, body, *span)?,
            Statement::For {
                binding,
                iterable,
                body,
                span,
            } => self.compile_for(binding, iterable, body, *span)?,
            Statement::Break { span } => self.compile_loop_branch(*span, true)?,
            Statement::Continue { span } => self.compile_loop_branch(*span, false)?,
        }
        Ok(())
    }

    /// Lowers a while loop to one break block and one condition-checking Wasm loop.
    fn compile_while(
        &mut self,
        condition: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let break_depth = self.control_depth;
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.enter_control()?;
        let continue_depth = self.control_depth;

        self.compile_expression(condition)?;
        self.runtime_value_call("__exs_rt_condition", 1, condition_span(condition))?;
        self.function.instruction(&Instruction::I32Eqz);
        self.branch_if_to(break_depth, span)?;

        self.loops.push(LoopContext {
            break_depth,
            continue_depth,
            cleanup_scope_start: self.scopes.len(),
        });
        self.compile_block(body, true)?;
        let _loop = self.loops.pop();
        self.branch_to(continue_depth, span)?;

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Lowers a for loop through a runtime iterable snapshot and indexed Wasm loop.
    fn compile_for(
        &mut self,
        binding: &crate::ast::Identifier<'a>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(iterable)?;
        let iterable = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(iterable));
        self.runtime_value_call("__exs_rt_iter_snapshot", 1, span)?;
        let snapshot = self.store_stack_value()?;
        self.clear_root_slot(iterable)?;

        self.function.instruction(&Instruction::I64Const(0));
        self.runtime_call("__exs_rt_int_new", span)?;
        let index = self.store_stack_value()?;
        let item = self.allocate_local();
        let mut iteration_scope = HashMap::new();
        iteration_scope.insert(binding.name.clone(), item);
        self.scopes.push(iteration_scope);

        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let break_depth = self.control_depth;
        self.function
            .instruction(&Instruction::Loop(BlockType::Empty));
        self.enter_control()?;
        let loop_depth = self.control_depth;

        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.runtime_value_call("__exs_rt_length", 1, span)?;
        let length = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::LocalGet(length));
        self.runtime_value_call("__exs_rt_lt", 2, span)?;
        let condition = self.allocate_local();
        self.function.instruction(&Instruction::LocalSet(condition));
        self.clear_root_slot(length)?;
        self.function.instruction(&Instruction::LocalGet(condition));
        self.runtime_value_call("__exs_rt_condition", 1, span)?;
        self.function.instruction(&Instruction::I32Eqz);
        self.branch_if_to(break_depth, span)?;

        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let continue_depth = self.control_depth;
        self.loops.push(LoopContext {
            break_depth,
            continue_depth,
            cleanup_scope_start: self.scopes.len(),
        });

        self.function.instruction(&Instruction::LocalGet(snapshot));
        self.function.instruction(&Instruction::LocalGet(index));
        self.runtime_value_call("__exs_rt_index_get", 2, span)?;
        self.store_stack_value_in(item)?;
        self.compile_block(body, true)?;
        let _loop = self.loops.pop();

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::LocalGet(index));
        self.function.instruction(&Instruction::I64Const(1));
        self.runtime_call("__exs_rt_int_new", span)?;
        self.runtime_value_call("__exs_rt_add", 2, span)?;
        self.store_stack_value_in(index)?;
        self.branch_to(loop_depth, span)?;

        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        let Some(iteration_scope) = self.scopes.pop() else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing for-loop iteration scope",
            )));
        };
        for local in iteration_scope.into_values() {
            self.clear_root_slot(local)?;
        }
        self.clear_root_slot(snapshot)?;
        self.clear_root_slot(index)
    }

    /// Emits break or continue after releasing roots local to the current loop body.
    fn compile_loop_branch(
        &mut self,
        span: SourceSpan<'a>,
        is_break: bool,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(context) = self.loops.last().copied() else {
            let keyword = if is_break { "break" } else { "continue" };
            return Err(diagnostics(CompileDiagnostic::new(
                "E0213",
                span,
                format!("{keyword} is only valid inside a loop"),
            )));
        };
        self.clear_roots_from_scope(context.cleanup_scope_start)?;
        if is_break {
            self.branch_to(context.break_depth, span)
        } else {
            self.branch_to(context.continue_depth, span)
        }
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
                self.compile_string(value, *span)?;
            }
            Expression::Bool(value, span) => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.runtime_call("__exs_rt_bool_new", *span)?;
            }
            Expression::None(span) => {
                self.runtime_call("__exs_rt_none_new", *span)?;
            }
            Expression::Ok { value, span } => {
                self.compile_expression(value)?;
                self.runtime_value_call("__exs_rt_ok_new", 1, *span)?;
            }
            Expression::IsError { value, span } => {
                self.compile_expression(value)?;
                self.runtime_value_call("__exs_rt_is_error", 1, *span)?;
            }
            Expression::Propagate { value, span } => self.compile_propagate(value, *span)?,
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
            Expression::List { elements, span } => {
                self.compile_list(elements, *span)?;
            }
            Expression::Object { properties, span } => {
                self.compile_object(properties, *span)?;
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
                self.runtime_value_call(
                    match operator {
                        UnaryOperator::Negate => "__exs_rt_neg",
                        UnaryOperator::Not => "__exs_rt_not",
                    },
                    1,
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
                    let left = self.store_stack_value()?;
                    self.compile_expression(right)?;
                    let right = self.store_stack_value()?;
                    self.function.instruction(&Instruction::LocalGet(left));
                    self.function.instruction(&Instruction::LocalGet(right));
                    self.runtime_value_call(runtime_operation(*operator), 2, *span)?;
                    self.clear_root_slot(left)?;
                    self.clear_root_slot(right)?;
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
                let mut argument_locals = Vec::new();
                for argument in arguments {
                    self.compile_expression(argument)?;
                    argument_locals.push(self.store_stack_value()?);
                }
                for local in &argument_locals {
                    self.function.instruction(&Instruction::LocalGet(*local));
                }
                self.function
                    .instruction(&Instruction::Call(signature.index));
                for local in argument_locals {
                    self.clear_root_slot(local)?;
                }
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => {
                self.compile_expression(receiver)?;
                let receiver_local = self.store_stack_value()?;
                self.compile_string(&method.name, method.span)?;
                let method_local = self.store_stack_value()?;
                self.compile_list(arguments, *span)?;
                let arguments_local = self.store_stack_value()?;
                self.function
                    .instruction(&Instruction::LocalGet(receiver_local));
                self.function
                    .instruction(&Instruction::LocalGet(method_local));
                self.function
                    .instruction(&Instruction::LocalGet(arguments_local));
                self.runtime_value_call("__exs_rt_call_method", 3, *span)?;
                self.clear_root_slot(receiver_local)?;
                self.clear_root_slot(method_local)?;
                self.clear_root_slot(arguments_local)?;
            }
            Expression::Index {
                receiver,
                index,
                span,
            } => {
                self.compile_expression(receiver)?;
                let receiver = self.store_stack_value()?;
                self.compile_expression(index)?;
                let index = self.store_stack_value()?;
                self.function.instruction(&Instruction::LocalGet(receiver));
                self.function.instruction(&Instruction::LocalGet(index));
                self.runtime_value_call("__exs_rt_index_get", 2, *span)?;
                self.clear_root_slot(receiver)?;
                self.clear_root_slot(index)?;
            }
            Expression::Property {
                receiver,
                property,
                span,
            } => {
                self.compile_expression(receiver)?;
                let receiver = self.store_stack_value()?;
                self.compile_string(&property.name, property.span)?;
                let property = self.store_stack_value()?;
                self.function.instruction(&Instruction::LocalGet(receiver));
                self.function.instruction(&Instruction::LocalGet(property));
                self.runtime_value_call("__exs_rt_index_get", 2, *span)?;
                self.clear_root_slot(receiver)?;
                self.clear_root_slot(property)?;
            }
        }
        Ok(())
    }

    /// Compiles one static source string through the compiler-owned literal pool.
    fn compile_string(
        &mut self,
        value: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let data_index = self.literals.get(value).copied().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "missing compiler string literal data segment",
            ))
        })?;
        let length = i32::try_from(value.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0211",
                span,
                "string literal is too large for Wasm linear memory",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(length));
        self.runtime_call("__exs_rt_literal_buffer_alloc", span)?;
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
        self.runtime_call("__exs_rt_string_new", span)
    }

    /// Constructs a runtime list while evaluating every element in source order.
    fn compile_list(
        &mut self,
        elements: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_list_new", span)?;
        let list_local = self.store_stack_value()?;
        for element in elements {
            self.compile_expression(element)?;
            let element = self.store_stack_value()?;
            self.function
                .instruction(&Instruction::LocalGet(list_local));
            self.function.instruction(&Instruction::LocalGet(element));
            self.runtime_value_call("__exs_rt_append", 2, span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(element)?;
        }
        self.function
            .instruction(&Instruction::LocalGet(list_local));
        Ok(())
    }

    /// Constructs a runtime object while evaluating property values in source order.
    fn compile_object(
        &mut self,
        properties: &[ObjectProperty<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.runtime_call("__exs_rt_object_new", span)?;
        let object_local = self.store_stack_value()?;
        for property in properties {
            self.compile_string(&property.key, property.key_span)?;
            let key = self.store_stack_value()?;
            self.compile_expression(&property.value)?;
            let value = self.store_stack_value()?;
            self.function
                .instruction(&Instruction::LocalGet(object_local));
            self.function.instruction(&Instruction::LocalGet(key));
            self.function.instruction(&Instruction::LocalGet(value));
            self.runtime_value_call("__exs_rt_index_set", 3, property.span)?;
            self.function.instruction(&Instruction::Drop);
            self.clear_root_slot(key)?;
            self.clear_root_slot(value)?;
        }
        self.function
            .instruction(&Instruction::LocalGet(object_local));
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
        self.runtime_value_call("__exs_rt_condition", 1, span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
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
        self.exit_control()
    }

    /// Lowers the postfix propagation operator for Option and Result values.
    fn compile_propagate(
        &mut self,
        value: &Expression<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(value)?;
        self.runtime_value_call("__exs_rt_propagate", 1, span)?;
        let outcome = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.runtime_value_call("__exs_rt_is_error", 1, span)?;
        self.runtime_value_call("__exs_rt_condition", 1, span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.enter_control()?;
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.return_stack_value()?;
        self.function.instruction(&Instruction::Else);
        self.function.instruction(&Instruction::LocalGet(outcome));
        self.runtime_value_call("__exs_rt_unwrap", 1, span)?;
        let extracted = self.store_stack_value()?;
        self.clear_root_slot(outcome)?;
        self.function.instruction(&Instruction::LocalGet(extracted));
        self.function.instruction(&Instruction::End);
        self.exit_control()
    }

    /// Compiles an expression and verifies it is a boolean without consuming it.
    fn checked_boolean_expression(
        &mut self,
        expression: &Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(expression)?;
        let temporary = self.store_stack_value()?;
        self.function.instruction(&Instruction::LocalGet(temporary));
        self.runtime_value_call("__exs_rt_condition", 1, condition_span(expression))?;
        self.function.instruction(&Instruction::Drop);
        self.function.instruction(&Instruction::LocalGet(temporary));
        self.clear_root_slot(temporary)?;
        Ok(())
    }

    /// Emits one named runtime ABI call after resolving its template function index.
    fn runtime_call(
        &mut self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if name != "__exs_rt_set_source_position" {
            self.set_runtime_source_position(span)?;
        }
        self.runtime_call_unpositioned(name, span)
    }

    /// Emits the runtime source position used by a subsequent fallible ABI operation.
    fn set_runtime_source_position(
        &mut self,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let position = i32::try_from(span.start_byte).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0214",
                span,
                "source position exceeds the Wasm i32 ABI",
            ))
        })?;
        self.function.instruction(&Instruction::I32Const(position));
        self.runtime_call_unpositioned("__exs_rt_set_source_position", span)
    }

    /// Emits one runtime ABI call without updating the active source position.
    fn runtime_call_unpositioned(
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

    /// Calls a runtime operation after spilling every ValueRef argument into rooted locals.
    fn runtime_value_call(
        &mut self,
        name: &str,
        argument_count: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mut arguments = Vec::new();
        for _ in 0..argument_count {
            arguments.push(self.store_stack_value()?);
        }
        arguments.reverse();
        for argument in &arguments {
            self.function.instruction(&Instruction::LocalGet(*argument));
        }
        self.runtime_call(name, span)?;
        for argument in arguments {
            self.clear_root_slot(argument)?;
        }
        Ok(())
    }

    /// Increases the tracked structured-control nesting depth.
    fn enter_control(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        self.control_depth = self.control_depth.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "too many nested control-flow blocks in one function",
            ))
        })?;
        Ok(())
    }

    /// Decreases the tracked structured-control nesting depth.
    fn exit_control(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        self.control_depth = self.control_depth.checked_sub(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                self.declaration.span,
                "unbalanced compiler control-flow bookkeeping",
            ))
        })?;
        Ok(())
    }

    /// Branches to one active structured-control depth.
    fn branch_to(
        &mut self,
        target_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let depth = self
            .control_depth
            .checked_sub(target_depth)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "invalid compiler loop branch target",
                ))
            })?;
        self.function.instruction(&Instruction::Br(depth));
        Ok(())
    }

    /// Conditionally branches to one active structured-control depth.
    fn branch_if_to(
        &mut self,
        target_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let depth = self
            .control_depth
            .checked_sub(target_depth)
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0999",
                    span,
                    "invalid compiler loop branch target",
                ))
            })?;
        self.function.instruction(&Instruction::BrIf(depth));
        Ok(())
    }

    /// Clears roots introduced in lexical scopes at or below one scope-stack position.
    fn clear_roots_from_scope(&mut self, scope_start: usize) -> Result<(), CompileDiagnostics<'a>> {
        let locals = self
            .scopes
            .iter()
            .skip(scope_start)
            .flat_map(|scope| scope.values().copied())
            .collect::<Vec<_>>();
        for local in locals {
            self.clear_root_slot(local)?;
        }
        Ok(())
    }

    /// Creates this invocation's root frame and registers its parameters.
    fn initialize_root_frame(
        &mut self,
        root_slot_count: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let slot_count = i32::try_from(root_slot_count).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "too many root slots for the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(slot_count));
        self.runtime_call("__exs_rt_root_push", self.declaration.span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.root_frame_local));
        for parameter in 0..self.declaration.parameters.len() {
            self.set_root_slot(u32::try_from(parameter).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    self.declaration.span,
                    "too many parameters for one function",
                ))
            })?)?;
        }
        Ok(())
    }

    /// Stores the stack's ValueRef in a fresh compiler local and roots it.
    fn store_stack_value(&mut self) -> Result<u32, CompileDiagnostics<'a>> {
        let local = self.allocate_local();
        self.store_stack_value_in(local)?;
        Ok(local)
    }

    /// Stores the stack's ValueRef in one compiler local and updates its root slot.
    fn store_stack_value_in(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        self.function.instruction(&Instruction::LocalSet(local));
        self.set_root_slot(local)
    }

    /// Registers one compiler local in the active root frame.
    fn set_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let slot = i32::try_from(local).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "root slot exceeds the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::I32Const(slot));
        self.function.instruction(&Instruction::LocalGet(local));
        self.runtime_call("__exs_rt_root_set", self.declaration.span)
    }

    /// Removes one compiler local from the active root frame.
    fn clear_root_slot(&mut self, local: u32) -> Result<(), CompileDiagnostics<'a>> {
        let slot = i32::try_from(local).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                self.declaration.span,
                "root slot exceeds the Wasm i32 ABI",
            ))
        })?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.function.instruction(&Instruction::I32Const(slot));
        self.runtime_call("__exs_rt_root_clear", self.declaration.span)
    }

    /// Returns the ValueRef on the stack after safely removing this function's root frame.
    fn return_stack_value(&mut self) -> Result<(), CompileDiagnostics<'a>> {
        let result = self.store_stack_value()?;
        self.function
            .instruction(&Instruction::LocalGet(self.root_frame_local));
        self.runtime_call("__exs_rt_root_pop", self.declaration.span)?;
        self.function.instruction(&Instruction::LocalGet(result));
        self.function.instruction(&Instruction::Return);
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
            Statement::While { body, .. } => count_lets(body),
            Statement::For { body, .. } => 1 + count_lets(body),
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
        | Statement::Expression {
            expression: value, ..
        } => count_expressions(value),
        Statement::Assign { target, value, .. } => {
            count_assignment_target_expressions(target) + count_expressions(value)
        }
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
        Statement::While {
            condition, body, ..
        } => count_expressions(condition) + count_expressions_block(body),
        Statement::For { iterable, body, .. } => {
            6 + count_expressions(iterable) + count_expressions_block(body)
        }
        Statement::Break { .. } | Statement::Continue { .. } => 0,
    }
}

/// Counts scratch-local requirements needed to evaluate one assignment target.
fn count_assignment_target_expressions(target: &AssignmentTarget<'_>) -> u32 {
    match target {
        AssignmentTarget::Variable(_) => 0,
        AssignmentTarget::Index {
            receiver, index, ..
        } => count_expressions(receiver) + count_expressions(index),
        AssignmentTarget::Property { receiver, .. } => 1 + count_expressions(receiver),
    }
}

/// Counts expression scratch-local requirements recursively.
fn count_expressions(expression: &Expression<'_>) -> u32 {
    match expression {
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => 1,
        Expression::Ok { value, .. }
        | Expression::IsError { value, .. }
        | Expression::Propagate { value, .. } => 1 + count_expressions(value),
        Expression::Unary { operand, .. } => 1 + count_expressions(operand),
        Expression::Binary { left, right, .. } => {
            1 + count_expressions(left) + count_expressions(right)
        }
        Expression::Call { arguments, .. } => {
            1 + arguments.iter().map(count_expressions).sum::<u32>()
        }
        Expression::List { elements, .. } => {
            1 + elements.iter().map(count_expressions).sum::<u32>()
        }
        Expression::Object { properties, .. } => {
            1 + properties
                .iter()
                .map(|property| 1 + count_expressions(&property.value))
                .sum::<u32>()
        }
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => 5 + count_expressions(receiver) + arguments.iter().map(count_expressions).sum::<u32>(),
        Expression::Index {
            receiver, index, ..
        } => 1 + count_expressions(receiver) + count_expressions(index),
        Expression::Property { receiver, .. } => 1 + count_expressions(receiver),
    }
}

/// Returns the source span used for a runtime condition check.
fn condition_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::List { span, .. } => *span,
        Expression::Object { span, .. } => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Unary { span, .. }
        | Expression::Ok { span, .. }
        | Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
    }
}
