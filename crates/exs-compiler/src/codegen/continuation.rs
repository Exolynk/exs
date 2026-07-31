//! Continuation-graph lowering for functions that may suspend through the Host ABI.

use std::collections::HashMap;

use exs_abi::{HOST_CALL_PENDING, HOST_CALL_READY, STATUS_COMPLETE, STATUS_PENDING, STATUS_READY};
use exs_value::is_valid_int;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::ast::{
    AssignmentTarget, BinaryOperator, Expression, FunctionDeclaration, Statement, UnaryOperator,
};
use crate::codegen::diagnostics;
use crate::codegen::function::{FunctionSignature, InstanceMethod, MethodRegistry};
use crate::codegen::source_map::SourceMap;
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan, module_span};

/// One lowered resumable function and the durable frame capacity it requires.
pub(super) struct CompiledContinuation {
    /// The generated one-argument frame step function.
    pub(super) function: Function,
    /// The number of initialized-or-reserved durable frame slots.
    pub(super) slot_count: u32,
}

/// Compiler-known durable capacity for one suspendable frame.
#[derive(Clone, Copy)]
pub(super) struct FrameLayout {
    /// Compiler-assigned function identifier stored in the runtime frame.
    pub(super) function_id: u32,
    /// Number of durable slots allocated when this function is invoked.
    pub(super) slot_count: u32,
}

/// Returns a conservative durable-frame capacity for one source declaration.
pub(super) fn frame_slot_capacity<'a>(
    declaration: &FunctionDeclaration<'a>,
) -> Result<u32, CompileDiagnostics<'a>> {
    let source_bytes = declaration
        .span
        .end_byte
        .checked_sub(declaration.span.start_byte)
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                declaration.span,
                "invalid function source span",
            ))
        })?;
    let parameter_count = u32::try_from(declaration.parameters.len()).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            declaration.span,
            "too many continuation function parameters",
        ))
    })?;
    let capacity = source_bytes
        .checked_add(parameter_count)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many continuation frame slots",
            ))
        })?;
    Ok(capacity)
}

/// Lowers a sequential resumable function into a frame-driven Wasm step function.
#[allow(clippy::too_many_arguments)] // The linked compiler dependencies are intentionally explicit.
pub(super) fn compile_function<'source>(
    declaration: &FunctionDeclaration<'source>,
    key: &str,
    signatures: &HashMap<String, FunctionSignature>,
    runtime: &HashMap<String, u32>,
    literals: &HashMap<String, u32>,
    source_map: &SourceMap<'source>,
    frame_layouts: &HashMap<String, FrameLayout>,
    methods: &MethodRegistry,
    types: &TypeRegistry,
) -> Result<CompiledContinuation, CompileDiagnostics<'source>> {
    let signature = signatures.get(key).ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0999",
            declaration.name.span,
            "missing resumable function signature",
        ))
    })?;
    let graph = ContinuationGraph::build(
        declaration,
        signature,
        signatures,
        frame_layouts,
        methods,
        types,
    )?;
    let mut compiler = StepCompiler {
        runtime,
        literals,
        source_map,
        frame_layouts,
        return_contract: &signature.return_type,
        function: Function::new([(2, ValType::I32)]),
        scratch_local: 1,
    };
    for (state, operation) in graph.operations.iter().enumerate() {
        let state = u32::try_from(state).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many continuation states for one function",
            ))
        })?;
        compiler.emit_state(state, operation)?;
    }
    compiler.function.instruction(&Instruction::Unreachable);
    compiler.function.instruction(&Instruction::End);
    Ok(CompiledContinuation {
        function: compiler.function,
        slot_count: graph.slot_count,
    })
}

/// Generates the root entry wrapper for a resumable `main` function.
pub(super) fn compile_start<'a>(
    module: &crate::ast::Module<'a>,
    main: &FunctionSignature,
    frame_slot_count: u32,
    dispatcher: u32,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(5, ValType::I32)]);
    let frame = 2_u32;
    let arguments = 3_u32;
    let count = 4_u32;
    let result = 5_u32;
    let status = 6_u32;
    let parameter_count = i32::try_from(main.arity).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many main parameters for the Wasm i32 ABI",
        ))
    })?;
    let frame_slot_count = i32::try_from(frame_slot_count).map_err(|_| {
        diagnostics(CompileDiagnostic::new(
            "E0212",
            module_span(module),
            "too many continuation frame slots for the Wasm i32 ABI",
        ))
    })?;

    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_decode_input",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(arguments));
    function.instruction(&Instruction::LocalGet(arguments));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_input_argument_count",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(count));
    function.instruction(&Instruction::LocalGet(count));
    function.instruction(&Instruction::I32Const(parameter_count));
    function.instruction(&Instruction::I32GtU);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::LocalGet(arguments));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_input_arity_error",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(result));
    function.instruction(&Instruction::LocalGet(result));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_set_result",
        module_span(module),
    )?;
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);

    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_execution_start",
        module_span(module),
    )?;
    function.instruction(&Instruction::I32Const(main.function_id.cast_signed()));
    function.instruction(&Instruction::I32Const(frame_slot_count));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_async_frame_new",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(frame));
    function.instruction(&Instruction::I32Const(main.function_id.cast_signed()));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_frame_push",
        module_span(module),
    )?;
    for index in 0..parameter_count {
        function.instruction(&Instruction::LocalGet(frame));
        function.instruction(&Instruction::I32Const(index));
        function.instruction(&Instruction::LocalGet(arguments));
        function.instruction(&Instruction::I32Const(index));
        call_runtime(
            &mut function,
            runtime,
            "__exs_rt_input_argument",
            module_span(module),
        )?;
        call_runtime(
            &mut function,
            runtime,
            "__exs_rt_async_frame_set_slot",
            module_span(module),
        )?;
    }
    function.instruction(&Instruction::LocalGet(frame));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_async_frame_set_current",
        module_span(module),
    )?;
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_scheduler_checkpoint",
        module_span(module),
    )?;
    emit_dispatch(
        &mut function,
        dispatcher,
        status,
        result,
        runtime,
        module_span(module),
    )
}

/// Generates the runner-facing export that resumes the active host-call continuation.
pub(super) fn compile_resume<'a>(
    module: &crate::ast::Module<'a>,
    dispatcher: u32,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(3, ValType::I32)]);
    let status = 4_u32;
    let result = 5_u32;
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::LocalGet(2));
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_host_call_resume",
        module_span(module),
    )?;
    function.instruction(&Instruction::LocalSet(status));
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I32Const(exs_abi::STATUS_CANCELLED));
    function.instruction(&Instruction::Return);
    function.instruction(&Instruction::End);
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_scheduler_checkpoint",
        module_span(module),
    )?;
    emit_dispatch(
        &mut function,
        dispatcher,
        status,
        result,
        runtime,
        module_span(module),
    )
}

/// Generates the runner-facing export that cancels a suspended root execution.
pub(super) fn compile_cancel<'a>(
    module: &crate::ast::Module<'a>,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([]);
    call_runtime(
        &mut function,
        runtime,
        "__exs_rt_execution_cancel",
        module_span(module),
    )?;
    function.instruction(&Instruction::End);
    Ok(function)
}

/// Generates one module-wide step dispatcher selected by the active frame function id.
pub(super) fn compile_dispatch<'a>(
    module: &crate::ast::Module<'a>,
    signatures: &HashMap<String, FunctionSignature>,
    suspendable: &HashMap<String, FrameLayout>,
    runtime: &HashMap<String, u32>,
) -> Result<Function, CompileDiagnostics<'a>> {
    let mut function = Function::new([(2, ValType::I32)]);
    function.instruction(&Instruction::Call(runtime_index(
        runtime,
        "__exs_rt_async_frame_current",
        module_span(module),
    )?));
    function.instruction(&Instruction::LocalSet(0));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Call(runtime_index(
        runtime,
        "__exs_rt_async_frame_function",
        module_span(module),
    )?));
    function.instruction(&Instruction::LocalSet(1));
    let mut targets = suspendable
        .iter()
        .filter_map(|(key, layout)| {
            signatures
                .get(key)
                .map(|signature| (layout.function_id, signature.index))
        })
        .collect::<Vec<_>>();
    targets.sort_unstable_by_key(|(function_id, _)| *function_id);
    emit_dispatch_target(&mut function, &targets, 0);
    function.instruction(&Instruction::End);
    Ok(function)
}

/// Emits nested Wasm conditionals selecting a generated suspendable step function.
fn emit_dispatch_target(function: &mut Function, targets: &[(u32, u32)], index: usize) {
    let Some((function_id, step)) = targets.get(index) else {
        function.instruction(&Instruction::Unreachable);
        return;
    };
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I32Const(function_id.cast_signed()));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::Call(*step));
    function.instruction(&Instruction::Else);
    emit_dispatch_target(function, targets, index + 1);
    function.instruction(&Instruction::End);
}

/// A sequence of operations executed through durable frame slots.
struct ContinuationGraph<'source, 'function> {
    /// One continuation operation per generated state.
    operations: Vec<Operation<'source, 'function>>,
    /// Number of slots required by parameters, lexical bindings, and temporaries.
    slot_count: u32,
}

/// One non-suspending operation or host-call boundary in a continuation graph.
enum Operation<'source, 'function> {
    /// Constructs a scalar literal in a destination slot.
    Literal {
        expression: &'function Expression<'source>,
        destination: u32,
    },
    /// Constructs an integer literal used by continuation control bookkeeping.
    Integer {
        value: i64,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Constructs the singular None value in a destination slot.
    None {
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Constructs a Boolean value in a destination slot.
    Boolean {
        value: bool,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Copies an already-rooted durable slot.
    Copy {
        source: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Applies a runtime unary operation.
    Unary {
        operator: UnaryOperator,
        operand: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Applies a non-short-circuiting runtime binary operation.
    Binary {
        operator: BinaryOperator,
        left: u32,
        right: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Constructs a List after all elements have been evaluated.
    List {
        elements: Vec<u32>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Constructs an Object after all property values have been evaluated.
    Object {
        properties: Vec<(&'function str, SourceSpan<'source>, u32)>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Constructs an empty nominal Object with a compiler-resolved type tag.
    TypedObject {
        type_id: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Tests one value for the Error variant.
    IsError {
        value: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Converts None to Error and returns early when the result is Error.
    Propagate {
        value: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Reads one dynamic index.
    Index {
        receiver: u32,
        index: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Reads one statically named property.
    Property {
        receiver: u32,
        property: String,
        property_span: SourceSpan<'source>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Mutates a dynamic index and discards its runtime result.
    IndexSet {
        receiver: u32,
        index: u32,
        value: u32,
        span: SourceSpan<'source>,
    },
    /// Mutates a statically named property and discards its runtime result.
    PropertySet {
        receiver: u32,
        property: String,
        property_span: SourceSpan<'source>,
        value: u32,
        span: SourceSpan<'source>,
    },
    /// Starts a host call and either takes its immediate result or suspends.
    HostCall {
        name: u32,
        arguments: Vec<u32>,
        argument_list: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Consumes the host result delivered after a pending host call.
    HostResume {
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Invokes a direct non-suspendable Wasm function with frame-backed arguments.
    DirectCall {
        signature: FunctionSignature,
        arguments: Vec<u32>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Creates a linked child frame for a suspendable direct function invocation.
    ChildCall {
        layout: FrameLayout,
        arguments: Vec<u32>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Dispatches an instance or trait method through nominal targets or the runtime fallback.
    InstanceCall {
        receiver: u32,
        method: &'function str,
        method_span: SourceSpan<'source>,
        arguments: Vec<u32>,
        targets: Vec<InstanceMethod>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Validates frame-backed parameter slots against their function contracts.
    ValidateParameters {
        contracts: Vec<TypeContract>,
        span: SourceSpan<'source>,
    },
    /// Validates one durable slot against a source type contract.
    ValidateSlot {
        slot: u32,
        contract: TypeContract,
        span: SourceSpan<'source>,
    },
    /// Branches after checking a source Boolean condition.
    Branch {
        condition: u32,
        checked: u32,
        when_true: u32,
        when_false: u32,
        span: SourceSpan<'source>,
    },
    /// Transfers execution to an explicit continuation state.
    Goto {
        target: u32,
        checkpoint: bool,
        span: SourceSpan<'source>,
    },
    /// Creates a runtime-owned iterable snapshot and returns an Error early on failure.
    IterSnapshot {
        iterable: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Evaluates one for-loop iteration condition.
    ForBranch {
        snapshot: u32,
        index: u32,
        length: u32,
        checked: u32,
        when_true: u32,
        when_false: u32,
        span: SourceSpan<'source>,
    },
    /// Increments one durable loop index.
    Increment {
        slot: u32,
        span: SourceSpan<'source>,
    },
    /// Completes the resumable frame with a source result value.
    Return {
        value: u32,
        span: SourceSpan<'source>,
    },
}

/// Builds a flat continuation graph for sequential source statements.
struct GraphBuilder<'source, 'function> {
    /// Durable lexical scopes, mapping names to frame slots.
    scopes: Vec<HashMap<String, u32>>,
    /// The next durable temporary slot.
    next_slot: u32,
    /// Graph operations emitted in source evaluation order.
    operations: Vec<Operation<'source, 'function>>,
    /// Active loop back-edges and break exits while lowering nested blocks.
    loops: Vec<LoopBuilderContext>,
    /// Whether this function may return Error through `?`.
    permits_error: bool,
    /// Linked source function signatures used while lowering direct calls.
    signatures: &'function HashMap<String, FunctionSignature>,
    /// Durable layouts of every suspendable call target.
    frame_layouts: &'function HashMap<String, FrameLayout>,
    /// Instance and static implementation-method metadata.
    methods: &'function MethodRegistry,
    /// Nominal Object declarations and field contracts.
    types: &'function TypeRegistry,
}

/// Unresolved loop branches collected until a loop's exit state is known.
struct LoopBuilderContext {
    /// Explicit `continue` branches whose target is emitted after the loop body.
    continues: Vec<usize>,
    /// Explicit `break` branches whose target is emitted after the loop body.
    breaks: Vec<usize>,
}

impl<'source, 'function> ContinuationGraph<'source, 'function> {
    /// Lowers the sequential subset of one resumable function.
    fn build(
        declaration: &'function FunctionDeclaration<'source>,
        signature: &FunctionSignature,
        signatures: &'function HashMap<String, FunctionSignature>,
        frame_layouts: &'function HashMap<String, FrameLayout>,
        methods: &'function MethodRegistry,
        types: &'function TypeRegistry,
    ) -> Result<Self, CompileDiagnostics<'source>> {
        let parameter_count = u32::try_from(declaration.parameters.len()).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                declaration.span,
                "too many function parameters",
            ))
        })?;
        let mut parameters = HashMap::new();
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            parameters.insert(parameter.name.name.clone(), index as u32);
        }
        let mut builder = GraphBuilder {
            scopes: vec![parameters],
            next_slot: parameter_count,
            operations: Vec::new(),
            loops: Vec::new(),
            permits_error: super::types::permits_error(&signature.return_type),
            signatures,
            frame_layouts,
            methods,
            types,
        };
        builder.operations.push(Operation::ValidateParameters {
            contracts: signature.parameter_types.clone(),
            span: declaration.span,
        });
        for statement in &declaration.body.statements {
            builder.lower_statement(statement)?;
        }
        let none = builder.temporary(declaration.span)?;
        builder.operations.push(Operation::None {
            destination: none,
            span: declaration.span,
        });
        builder.operations.push(Operation::Return {
            value: none,
            span: declaration.span,
        });
        Ok(Self {
            operations: builder.operations,
            slot_count: builder.next_slot,
        })
    }
}

impl<'source, 'function> GraphBuilder<'source, 'function> {
    /// Lowers one source statement into contiguous graph states and explicit branch edges.
    fn lower_statement(
        &mut self,
        statement: &'function Statement<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
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
                let value = self.lower_expression(value)?;
                let binding = self.temporary(name.span)?;
                self.operations.push(Operation::Copy {
                    source: value,
                    destination: binding,
                    span: name.span,
                });
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.name.clone(), binding);
                }
            }
            Statement::Assign { target, value, .. } => match target {
                AssignmentTarget::Variable(name) => {
                    let destination = self.lookup(&name.name, name.span)?;
                    let value = self.lower_expression(value)?;
                    self.operations.push(Operation::Copy {
                        source: value,
                        destination,
                        span: name.span,
                    });
                }
                AssignmentTarget::Index {
                    receiver,
                    index,
                    span,
                } => {
                    let receiver = self.lower_expression(receiver)?;
                    let index = self.lower_expression(index)?;
                    let value = self.lower_expression(value)?;
                    self.operations.push(Operation::IndexSet {
                        receiver,
                        index,
                        value,
                        span: *span,
                    });
                }
                AssignmentTarget::Property {
                    receiver,
                    property,
                    span,
                } => {
                    let receiver = self.lower_expression(receiver)?;
                    let value = self.lower_expression(value)?;
                    self.operations.push(Operation::PropertySet {
                        receiver,
                        property: property.name.clone(),
                        property_span: property.span,
                        value,
                        span: *span,
                    });
                }
            },
            Statement::Return { value, span } => {
                let value = match value {
                    Some(value) => self.lower_expression(value)?,
                    None => {
                        let destination = self.temporary(*span)?;
                        self.operations.push(Operation::None {
                            destination,
                            span: *span,
                        });
                        destination
                    }
                };
                self.operations
                    .push(Operation::Return { value, span: *span });
            }
            Statement::Expression { expression, .. } => {
                let _value = self.lower_expression(expression)?;
            }
            Statement::If {
                condition,
                then_block,
                else_block,
                span,
            } => self.lower_if(condition, then_block, else_block.as_ref(), *span)?,
            Statement::While {
                condition,
                body,
                span,
            } => self.lower_while(condition, body, *span)?,
            Statement::For {
                binding,
                iterable,
                body,
                span,
            } => self.lower_for(binding, iterable, body, *span)?,
            Statement::Break { span } => self.lower_loop_branch(*span, true)?,
            Statement::Continue { span } => self.lower_loop_branch(*span, false)?,
        }
        Ok(())
    }

    /// Lowers a conditional statement using true and false state targets.
    fn lower_if(
        &mut self,
        condition: &'function Expression<'source>,
        then_block: &'function crate::ast::Block<'source>,
        else_block: Option<&'function crate::ast::Block<'source>>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let condition = self.lower_expression(condition)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let then_start = self.operations.len();
        self.lower_block(then_block)?;
        let skip_else = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let else_start = self.operations.len();
        if let Some(else_block) = else_block {
            self.lower_block(else_block)?;
        }
        let after = self.operations.len();
        self.set_branch_targets(branch, then_start, else_start, span)?;
        self.set_goto_target(skip_else, after, span)
    }

    /// Lowers a while loop with explicit break and continue branch targets.
    fn lower_while(
        &mut self,
        condition: &'function Expression<'source>,
        body: &'function crate::ast::Block<'source>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let condition_start = self.operations.len();
        let condition = self.lower_expression(condition)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let body_start = self.operations.len();
        self.loops.push(LoopBuilderContext {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        self.lower_block(body)?;
        let back_edge = self.push(Operation::Goto {
            target: self.state_id(condition_start, span)?,
            checkpoint: true,
            span,
        })?;
        let exit = self.operations.len();
        self.set_branch_targets(branch, body_start, exit, span)?;
        let context = self.loops.pop().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            ))
        })?;
        for branch in context.breaks {
            self.set_goto_target(branch, exit, span)?;
        }
        for branch in context.continues {
            self.set_goto_target(branch, condition_start, span)?;
        }
        let _back_edge = back_edge;
        Ok(())
    }

    /// Lowers a for loop through a durable iterable snapshot and index states.
    fn lower_for(
        &mut self,
        binding: &'function crate::ast::Identifier<'source>,
        iterable: &'function Expression<'source>,
        body: &'function crate::ast::Block<'source>,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let iterable = self.lower_expression(iterable)?;
        let snapshot = self.temporary(span)?;
        self.operations.push(Operation::IterSnapshot {
            iterable,
            destination: snapshot,
            span,
        });
        let index = self.temporary(span)?;
        self.operations.push(Operation::Integer {
            value: 0,
            destination: index,
            span,
        });
        let condition_start = self.operations.len();
        let length = self.temporary(span)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::ForBranch {
            snapshot,
            index,
            length,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let body_start = self.operations.len();
        let item = self.temporary(binding.span)?;
        self.operations.push(Operation::Index {
            receiver: snapshot,
            index,
            destination: item,
            span,
        });
        self.scopes
            .push(HashMap::from([(binding.name.clone(), item)]));
        self.loops.push(LoopBuilderContext {
            continues: Vec::new(),
            breaks: Vec::new(),
        });
        for statement in &body.statements {
            self.lower_statement(statement)?;
        }
        let _scope = self.scopes.pop();
        let increment = self.operations.len();
        self.operations
            .push(Operation::Increment { slot: index, span });
        self.operations.push(Operation::Goto {
            target: self.state_id(condition_start, span)?,
            checkpoint: true,
            span,
        });
        let exit = self.operations.len();
        self.set_for_targets(branch, body_start, exit, span)?;
        let context = self.loops.pop().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            ))
        })?;
        for branch in context.breaks {
            self.set_goto_target(branch, exit, span)?;
        }
        for branch in context.continues {
            self.set_goto_target(branch, increment, span)?;
        }
        Ok(())
    }

    /// Lowers break or continue into a branch patched by the enclosing loop.
    fn lower_loop_branch(
        &mut self,
        span: SourceSpan<'source>,
        is_break: bool,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if self.loops.is_empty() {
            let keyword = if is_break { "break" } else { "continue" };
            return Err(diagnostics(CompileDiagnostic::new(
                "E0213",
                span,
                format!("{keyword} is only valid inside a loop"),
            )));
        }
        let branch = self.push(Operation::Goto {
            target: 0,
            checkpoint: !is_break,
            span,
        })?;
        let Some(target) = self.loops.last_mut() else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing active continuation loop",
            )));
        };
        if is_break {
            target.breaks.push(branch);
        } else {
            target.continues.push(branch);
        }
        Ok(())
    }

    /// Lowers one lexical block and drops its name bindings after its final state.
    fn lower_block(
        &mut self,
        block: &'function crate::ast::Block<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.scopes.push(HashMap::new());
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        let _scope = self.scopes.pop();
        Ok(())
    }

    /// Appends one operation and returns its zero-based graph state index.
    fn push(
        &mut self,
        operation: Operation<'source, 'function>,
    ) -> Result<usize, CompileDiagnostics<'source>> {
        let state = self.operations.len();
        let _state = self.state_id(state, operation_span(&operation))?;
        self.operations.push(operation);
        Ok(state)
    }

    /// Converts a graph index to the Wasm i32 state domain.
    fn state_id(
        &self,
        state: usize,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        u32::try_from(state).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states for one function",
            ))
        })
    }

    /// Patches one conditional branch after both block starts are known.
    fn set_branch_targets(
        &mut self,
        state: usize,
        when_true: usize,
        when_false: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let when_true = self.state_id(when_true, span)?;
        let when_false = self.state_id(when_false, span)?;
        let Some(Operation::Branch {
            when_true: target_true,
            when_false: target_false,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation conditional branch",
            )));
        };
        *target_true = when_true;
        *target_false = when_false;
        Ok(())
    }

    /// Patches one for-loop branch after its body and exit starts are known.
    fn set_for_targets(
        &mut self,
        state: usize,
        when_true: usize,
        when_false: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let when_true = self.state_id(when_true, span)?;
        let when_false = self.state_id(when_false, span)?;
        let Some(Operation::ForBranch {
            when_true: target_true,
            when_false: target_false,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation for-loop branch",
            )));
        };
        *target_true = when_true;
        *target_false = when_false;
        Ok(())
    }

    /// Patches one explicit graph jump target.
    fn set_goto_target(
        &mut self,
        state: usize,
        target: usize,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let target = self.state_id(target, span)?;
        let Some(Operation::Goto {
            target: destination,
            ..
        }) = self.operations.get_mut(state)
        else {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                span,
                "missing continuation graph jump",
            )));
        };
        *destination = target;
        Ok(())
    }

    /// Lowers one source expression into a new durable destination slot.
    fn lower_expression(
        &mut self,
        expression: &'function Expression<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        match expression {
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {
                let destination = self.temporary(expression_span(expression))?;
                self.operations.push(Operation::Literal {
                    expression,
                    destination,
                });
                Ok(destination)
            }
            Expression::Variable(identifier) => Ok(self.lookup(&identifier.name, identifier.span)?),
            Expression::Unary {
                operator,
                operand,
                span,
            } => {
                let operand = self.lower_expression(operand)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Unary {
                    operator: *operator,
                    operand,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => {
                if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    return self.lower_logical(
                        left,
                        right,
                        matches!(operator, BinaryOperator::Or),
                        *span,
                    );
                }
                let left = self.lower_expression(left)?;
                let right = self.lower_expression(right)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Binary {
                    operator: *operator,
                    left,
                    right,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::List { elements, span } => {
                let mut slots = Vec::with_capacity(elements.len());
                for element in elements {
                    slots.push(self.lower_expression(element)?);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::List {
                    elements: slots,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Object { properties, span } => {
                let mut slots = Vec::with_capacity(properties.len());
                for property in properties {
                    slots.push((
                        property.key.as_str(),
                        property.key_span,
                        self.lower_expression(&property.value)?,
                    ));
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Object {
                    properties: slots,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::IsError { value, span } => {
                let value = self.lower_expression(value)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::IsError {
                    value,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Propagate { value, span } => {
                if !self.permits_error {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0218",
                        *span,
                        "? requires the current function return type to include Error or Any",
                    )));
                }
                let value = self.lower_expression(value)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Propagate {
                    value,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::HostCall {
                name,
                arguments,
                span,
            } => {
                let name = self.lower_expression(name)?;
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let argument_list = self.temporary(*span)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::HostCall {
                    name,
                    arguments: slots,
                    argument_list,
                    destination,
                    span: *span,
                });
                self.operations.push(Operation::HostResume {
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Index {
                receiver,
                index,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let index = self.lower_expression(index)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Index {
                    receiver,
                    index,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Property {
                receiver,
                property,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::Property {
                    receiver,
                    property: property.name.clone(),
                    property_span: property.span,
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                let signature = self.signatures.get(&callee.name).cloned().ok_or_else(|| {
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
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                if let Some(layout) = self.frame_layouts.get(&callee.name).copied() {
                    self.operations.push(Operation::ChildCall {
                        layout,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                } else {
                    self.operations.push(Operation::DirectCall {
                        signature,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                }
                Ok(destination)
            }
            Expression::StaticMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => {
                let key = format!("{}::{}", type_name.name, method.name);
                let signature = self.signatures.get(&key).cloned().ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0225",
                        method.span,
                        format!(
                            "type `{}` has no static method `{}`",
                            type_name.name, method.name
                        ),
                    ))
                })?;
                if !self.methods.is_static(&key) {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0225",
                        method.span,
                        format!(
                            "type `{}` method `{}` requires a receiver",
                            type_name.name, method.name
                        ),
                    )));
                }
                if signature.arity != arguments.len() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0208",
                        *span,
                        format!(
                            "static method `{}::{}` expects {} arguments but received {}",
                            type_name.name,
                            method.name,
                            signature.arity,
                            arguments.len()
                        ),
                    )));
                }
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                if let Some(layout) = self.frame_layouts.get(&key).copied() {
                    self.operations.push(Operation::ChildCall {
                        layout,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                } else {
                    self.operations.push(Operation::DirectCall {
                        signature,
                        arguments: slots,
                        destination,
                        span: *span,
                    });
                }
                Ok(destination)
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => {
                let receiver = self.lower_expression(receiver)?;
                let mut slots = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    slots.push(self.lower_expression(argument)?);
                }
                let destination = self.temporary(*span)?;
                self.operations.push(Operation::InstanceCall {
                    receiver,
                    method: &method.name,
                    method_span: method.span,
                    arguments: slots,
                    targets: self
                        .methods
                        .instance(&method.name)
                        .unwrap_or_default()
                        .to_vec(),
                    destination,
                    span: *span,
                });
                Ok(destination)
            }
            Expression::TypedObject {
                type_name,
                properties,
                span,
            } => self.lower_typed_object(type_name, properties, *span),
        }
    }

    /// Lowers one short-circuiting Boolean expression into explicit continuation branches.
    fn lower_logical(
        &mut self,
        left: &'function Expression<'source>,
        right: &'function Expression<'source>,
        is_or: bool,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let left = self.lower_expression(left)?;
        let destination = self.temporary(span)?;
        let checked = self.temporary(span)?;
        let branch = self.push(Operation::Branch {
            condition: left,
            checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;

        let right_start = self.operations.len();
        let right = self.lower_expression(right)?;
        let right_checked = self.temporary(span)?;
        let right_branch = self.push(Operation::Branch {
            condition: right,
            checked: right_checked,
            when_true: 0,
            when_false: 0,
            span,
        })?;
        let right_true = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: true,
            destination,
            span,
        });
        let right_true_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let right_false = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: false,
            destination,
            span,
        });
        let right_false_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;

        let short_start = self.operations.len();
        self.operations.push(Operation::Boolean {
            value: is_or,
            destination,
            span,
        });
        let short_exit = self.push(Operation::Goto {
            target: 0,
            checkpoint: false,
            span,
        })?;
        let after = self.operations.len();

        if is_or {
            self.set_branch_targets(branch, short_start, right_start, span)?;
        } else {
            self.set_branch_targets(branch, right_start, short_start, span)?;
        }
        self.set_branch_targets(right_branch, right_true, right_false, span)?;
        self.set_goto_target(right_true_exit, after, span)?;
        self.set_goto_target(right_false_exit, after, span)?;
        self.set_goto_target(short_exit, after, span)?;
        Ok(destination)
    }

    /// Lowers nominal construction while preserving direct-lowering field validation order.
    fn lower_typed_object(
        &mut self,
        type_name: &'function crate::ast::Identifier<'source>,
        properties: &'function [crate::ast::ObjectProperty<'source>],
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        let nominal = self.types.get(&type_name.name).cloned().ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0216",
                type_name.span,
                format!("unknown type `{}`", type_name.name),
            ))
        })?;
        for property in properties {
            if !nominal
                .fields
                .iter()
                .any(|field| field.name == property.key)
                || properties
                    .iter()
                    .filter(|other| other.key == property.key)
                    .count()
                    > 1
            {
                let value = self.temporary(property.key_span)?;
                self.operations.push(Operation::None {
                    destination: value,
                    span: property.key_span,
                });
                self.operations.push(Operation::ValidateSlot {
                    slot: value,
                    contract: TypeContract {
                        builtin_mask: 0,
                        nominal_type_ids: Vec::new(),
                    },
                    span: property.key_span,
                });
                return Ok(value);
            }
        }

        let object = self.temporary(span)?;
        self.operations.push(Operation::TypedObject {
            type_id: nominal.id,
            destination: object,
            span,
        });
        for property in properties {
            let field = nominal
                .fields
                .iter()
                .find(|field| field.name == property.key)
                .cloned()
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0999",
                        property.key_span,
                        "missing resolved nominal field",
                    ))
                })?;
            let value = self.lower_expression(&property.value)?;
            self.operations.push(Operation::ValidateSlot {
                slot: value,
                contract: field.contract,
                span: property.span,
            });
            self.operations.push(Operation::PropertySet {
                receiver: object,
                property: property.key.clone(),
                property_span: property.key_span,
                value,
                span: property.span,
            });
        }
        for field in &nominal.fields {
            if properties.iter().any(|property| property.key == field.name) {
                continue;
            }
            let value = self.temporary(span)?;
            self.operations.push(Operation::None {
                destination: value,
                span,
            });
            self.operations.push(Operation::ValidateSlot {
                slot: value,
                contract: field.contract.clone(),
                span,
            });
            self.operations.push(Operation::PropertySet {
                receiver: object,
                property: field.name.clone(),
                property_span: span,
                value,
                span,
            });
        }
        Ok(object)
    }

    /// Allocates one durable frame slot.
    fn temporary(&mut self, span: SourceSpan<'source>) -> Result<u32, CompileDiagnostics<'source>> {
        let slot = self.next_slot;
        self.next_slot = self.next_slot.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation frame slots",
            ))
        })?;
        Ok(slot)
    }

    /// Resolves one lexical binding to its durable slot.
    fn lookup(
        &self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0205",
                    span,
                    format!("unknown binding `{name}`"),
                ))
            })
    }
}

/// Emits one graph operation as a Wasm step-function state.
struct StepCompiler<'source, 'context> {
    /// Runtime ABI export indexes.
    runtime: &'context HashMap<String, u32>,
    /// Passive data indexes for compiler string literals.
    literals: &'context HashMap<String, u32>,
    /// Compiler-assigned source positions.
    source_map: &'context SourceMap<'source>,
    /// Durable child-frame layouts for nominal suspendable method targets.
    frame_layouts: &'context HashMap<String, FrameLayout>,
    /// The declared return contract checked before a resumable frame completes.
    return_contract: &'context TypeContract,
    /// Wasm function body under construction.
    function: Function,
    /// Reused Wasm local for host status and literal-buffer pointers.
    scratch_local: u32,
}

impl<'source, 'context> StepCompiler<'source, 'context> {
    /// Emits a state guard and its operation body.
    fn emit_state(
        &mut self,
        state: u32,
        operation: &Operation<'source, '_>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.call_runtime("__exs_rt_async_frame_state", operation_span(operation))?;
        self.function
            .instruction(&Instruction::I32Const(state.cast_signed()));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.emit_operation(state, operation)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Emits one operation body, which always returns a dispatcher status.
    fn emit_operation(
        &mut self,
        state: u32,
        operation: &Operation<'source, '_>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let next = state.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                operation_span(operation),
                "too many continuation states",
            ))
        })?;
        match operation {
            Operation::Literal {
                expression,
                destination,
            } => {
                self.literal(expression)?;
                self.set_slot(*destination, operation_span(operation))?;
                self.ready(next, operation_span(operation))?;
            }
            Operation::Integer {
                value,
                destination,
                span,
            } => {
                self.integer(*value, *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::None { destination, span } => {
                self.call_runtime("__exs_rt_none_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Boolean {
                value,
                destination,
                span,
            } => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.call_runtime("__exs_rt_bool_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Copy {
                source,
                destination,
                span,
            } => {
                self.get_slot(*source, *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Unary {
                operator,
                operand,
                destination,
                span,
            } => {
                self.get_slot(*operand, *span)?;
                self.call_runtime(
                    match operator {
                        UnaryOperator::Negate => "__exs_rt_neg",
                        UnaryOperator::Not => "__exs_rt_not",
                    },
                    *span,
                )?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Binary {
                operator,
                left,
                right,
                destination,
                span,
            } => {
                self.get_slot(*left, *span)?;
                self.get_slot(*right, *span)?;
                self.call_runtime(binary_operation(*operator), *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::List {
                elements,
                destination,
                span,
            } => {
                self.call_runtime("__exs_rt_list_new", *span)?;
                self.set_slot(*destination, *span)?;
                for element in elements {
                    self.get_slot(*destination, *span)?;
                    self.get_slot(*element, *span)?;
                    self.call_runtime("__exs_rt_append", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.ready(next, *span)?;
            }
            Operation::Object {
                properties,
                destination,
                span,
            } => {
                self.call_runtime("__exs_rt_object_new", *span)?;
                self.set_slot(*destination, *span)?;
                for (key, key_span, value) in properties {
                    self.get_slot(*destination, *span)?;
                    self.string(key, *key_span)?;
                    self.get_slot(*value, *span)?;
                    self.call_runtime("__exs_rt_index_set", *span)?;
                    self.function.instruction(&Instruction::Drop);
                }
                self.ready(next, *span)?;
            }
            Operation::TypedObject {
                type_id,
                destination,
                span,
            } => {
                self.function
                    .instruction(&Instruction::I32Const(type_id.cast_signed()));
                self.call_runtime("__exs_rt_object_typed_new", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::IsError {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_is_error", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Propagate {
                value,
                destination,
                span,
            } => {
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_propagate", *span)?;
                self.set_slot(*destination, *span)?;
                self.get_slot(*destination, *span)?;
                self.call_runtime("__exs_rt_is_error", *span)?;
                self.call_runtime("__exs_rt_condition", *span)?;
                self.function
                    .instruction(&Instruction::If(BlockType::Empty));
                self.complete(*destination, *span)?;
                self.function.instruction(&Instruction::End);
                self.ready(next, *span)?;
            }
            Operation::Index {
                receiver,
                index,
                destination,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.get_slot(*index, *span)?;
                self.call_runtime("__exs_rt_index_get", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Property {
                receiver,
                property,
                property_span,
                destination,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.string(property, *property_span)?;
                self.call_runtime("__exs_rt_index_get", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::IndexSet {
                receiver,
                index,
                value,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.get_slot(*index, *span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_index_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.ready(next, *span)?;
            }
            Operation::PropertySet {
                receiver,
                property,
                property_span,
                value,
                span,
            } => {
                self.get_slot(*receiver, *span)?;
                self.string(property, *property_span)?;
                self.get_slot(*value, *span)?;
                self.call_runtime("__exs_rt_index_set", *span)?;
                self.function.instruction(&Instruction::Drop);
                self.ready(next, *span)?;
            }
            Operation::HostCall {
                name,
                arguments,
                argument_list,
                destination,
                span,
            } => self.host_call(state, *name, arguments, *argument_list, *destination, *span)?,
            Operation::HostResume { destination, span } => {
                self.call_runtime("__exs_rt_host_call_take_ready", *span)?;
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::DirectCall {
                signature,
                arguments,
                destination,
                span,
            } => {
                for argument in arguments {
                    self.get_slot(*argument, *span)?;
                }
                self.set_call_site(*span)?;
                self.function
                    .instruction(&Instruction::Call(signature.index));
                self.set_slot(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::ChildCall {
                layout,
                arguments,
                destination,
                span,
            } => self.child_call(next, *layout, arguments, *destination, *span)?,
            Operation::InstanceCall {
                receiver,
                method,
                method_span,
                arguments,
                targets,
                destination,
                span,
            } => self.instance_call(
                next,
                *receiver,
                method,
                *method_span,
                arguments,
                targets,
                *destination,
                *span,
            )?,
            Operation::ValidateParameters { contracts, span } => {
                for (slot, contract) in contracts.iter().enumerate() {
                    let slot = u32::try_from(slot).map_err(|_| {
                        diagnostics(CompileDiagnostic::new(
                            "E0212",
                            *span,
                            "too many continuation parameter slots",
                        ))
                    })?;
                    self.validate_slot_or_complete(slot, contract, *span)?;
                }
                self.ready(next, *span)?;
            }
            Operation::ValidateSlot {
                slot,
                contract,
                span,
            } => {
                self.validate_slot_or_complete(*slot, contract, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Branch {
                condition,
                checked,
                when_true,
                when_false,
                span,
            } => self.branch_on_value(*condition, *checked, *when_true, *when_false, *span)?,
            Operation::Goto {
                target,
                checkpoint,
                span,
            } => {
                if *checkpoint {
                    self.call_runtime("__exs_rt_scheduler_checkpoint", *span)?;
                }
                self.ready(*target, *span)?;
            }
            Operation::IterSnapshot {
                iterable,
                destination,
                span,
            } => {
                self.get_slot(*iterable, *span)?;
                self.call_runtime("__exs_rt_iter_snapshot", *span)?;
                self.set_slot(*destination, *span)?;
                self.complete_if_error(*destination, *span)?;
                self.ready(next, *span)?;
            }
            Operation::ForBranch {
                snapshot,
                index,
                length,
                checked,
                when_true,
                when_false,
                span,
            } => {
                self.get_slot(*snapshot, *span)?;
                self.call_runtime("__exs_rt_length", *span)?;
                self.set_slot(*length, *span)?;
                self.get_slot(*index, *span)?;
                self.get_slot(*length, *span)?;
                self.call_runtime("__exs_rt_lt", *span)?;
                self.set_slot(*checked, *span)?;
                self.branch_on_value(*checked, *length, *when_true, *when_false, *span)?;
            }
            Operation::Increment { slot, span } => {
                self.get_slot(*slot, *span)?;
                self.integer(1, *span)?;
                self.call_runtime("__exs_rt_add", *span)?;
                self.set_slot(*slot, *span)?;
                self.ready(next, *span)?;
            }
            Operation::Return { value, span } => self.complete(*value, *span)?,
        }
        Ok(())
    }

    /// Emits a host-call boundary and its synchronous fast path.
    fn host_call(
        &mut self,
        state: u32,
        name: u32,
        arguments: &[u32],
        argument_list: u32,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let resume = state.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states",
            ))
        })?;
        let after_resume = resume.checked_add(1).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many continuation states",
            ))
        })?;
        self.call_runtime("__exs_rt_list_new", span)?;
        self.set_slot(argument_list, span)?;
        for argument in arguments {
            self.get_slot(argument_list, span)?;
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.get_slot(name, span)?;
        self.get_slot(argument_list, span)?;
        self.call_runtime("__exs_rt_host_call_start", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::I32Const(HOST_CALL_READY));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.call_runtime("__exs_rt_host_call_take_ready", span)?;
        self.set_slot(destination, span)?;
        self.ready(after_resume, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::I32Const(HOST_CALL_PENDING));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.ready_pending(resume, span)?;
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Unreachable);
        Ok(())
    }

    /// Starts one suspendable child frame and transfers dispatch to it.
    fn child_call(
        &mut self,
        next: u32,
        layout: FrameLayout,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let slot_count = i32::try_from(layout.slot_count).map_err(|_| {
            diagnostics(CompileDiagnostic::new(
                "E0212",
                span,
                "too many child continuation frame slots",
            ))
        })?;
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.set_call_site(span)?;
        self.function
            .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
        self.function
            .instruction(&Instruction::I32Const(slot_count));
        self.call_runtime("__exs_rt_async_frame_new", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.function
            .instruction(&Instruction::I32Const(layout.function_id.cast_signed()));
        self.call_runtime("__exs_rt_frame_push", span)?;
        for (slot, argument) in arguments.iter().enumerate() {
            let slot = i32::try_from(slot).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    span,
                    "too many child function arguments",
                ))
            })?;
            self.function.instruction(&Instruction::LocalGet(2));
            self.function.instruction(&Instruction::I32Const(slot));
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_async_frame_set_slot", span)?;
        }
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(destination.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_caller", span)?;
        self.call_runtime("__exs_rt_scheduler_checkpoint", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Emits nominal instance dispatch, including suspendable child targets and runtime fallback.
    #[allow(clippy::too_many_arguments)] // This directly mirrors the already-evaluated source call.
    fn instance_call(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        targets: &[InstanceMethod],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.instance_call_target(
            next,
            receiver,
            method,
            method_span,
            arguments,
            targets,
            0,
            destination,
            span,
        )
    }

    /// Emits one branch of the static nominal method-target chain.
    #[allow(clippy::too_many_arguments)] // Recursion keeps the generated Wasm branch chain local.
    fn instance_call_target(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        targets: &[InstanceMethod],
        index: usize,
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let Some(target) = targets.get(index) else {
            return self.runtime_method_call(
                next,
                receiver,
                method,
                method_span,
                arguments,
                destination,
                span,
            );
        };
        self.get_slot(receiver, span)?;
        self.function
            .instruction(&Instruction::I32Const(target.type_id.cast_signed()));
        self.call_runtime("__exs_rt_object_is_type", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        if target.signature.arity != arguments.len() + 1 {
            self.get_slot(receiver, span)?;
            self.call_runtime("__exs_rt_method_arity_error", span)?;
            self.set_slot(destination, span)?;
            self.ready(next, span)?;
        } else if let Some(layout) = self
            .frame_layouts
            .values()
            .find(|layout| layout.function_id == target.signature.function_id)
            .copied()
        {
            let mut child_arguments = Vec::with_capacity(arguments.len() + 1);
            child_arguments.push(receiver);
            child_arguments.extend_from_slice(arguments);
            self.child_call(next, layout, &child_arguments, destination, span)?;
        } else {
            self.get_slot(receiver, span)?;
            for argument in arguments {
                self.get_slot(*argument, span)?;
            }
            self.set_call_site(span)?;
            self.function
                .instruction(&Instruction::Call(target.signature.index));
            self.set_slot(destination, span)?;
            self.ready(next, span)?;
        }
        self.function.instruction(&Instruction::Else);
        self.instance_call_target(
            next,
            receiver,
            method,
            method_span,
            arguments,
            targets,
            index + 1,
            destination,
            span,
        )?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Calls the runtime built-in method dispatcher after all source operands were evaluated.
    #[allow(clippy::too_many_arguments)] // Runtime fallback receives the same complete call context.
    fn runtime_method_call(
        &mut self,
        next: u32,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'source>,
        arguments: &[u32],
        destination: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.call_runtime("__exs_rt_list_new", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        for argument in arguments {
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.get_slot(*argument, span)?;
            self.call_runtime("__exs_rt_append", span)?;
            self.function.instruction(&Instruction::Drop);
        }
        self.string(method, method_span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        self.get_slot(receiver, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.call_runtime("__exs_rt_call_method", span)?;
        self.set_slot(destination, span)?;
        self.ready(next, span)
    }

    /// Stores a value in a durable frame slot.
    fn set_slot(
        &mut self,
        slot: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(slot.cast_signed()));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::Call(
            self.runtime_index("__exs_rt_async_frame_set_slot", span)?,
        ));
        Ok(())
    }

    /// Emits the source call-site position consumed by a child frame-stack push.
    fn set_call_site(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let position = self.source_map.id(span).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0214",
                span,
                "missing source-map position for generated function call",
            ))
        })?;
        self.function
            .instruction(&Instruction::I32Const(position.cast_signed()));
        self.function.instruction(&Instruction::Call(
            self.runtime_index("__exs_rt_set_call_site", span)?,
        ));
        Ok(())
    }

    /// Loads a durable frame slot onto the Wasm stack.
    fn get_slot(
        &mut self,
        slot: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(slot.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_get_slot", span)
    }

    /// Advances to the next state and returns runnable status.
    fn ready(
        &mut self,
        next: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(next.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Stores the host-resume state and returns pending status.
    fn ready_pending(
        &mut self,
        resume: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::I32Const(resume.cast_signed()));
        self.call_runtime("__exs_rt_async_frame_set_state", span)?;
        self.function
            .instruction(&Instruction::I32Const(STATUS_PENDING));
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Completes this async frame and returns root-complete or caller-runnable status.
    fn complete(
        &mut self,
        value: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.validate_local_return(span)?;
        self.complete_local(span)
    }

    /// Completes this async frame with the value held in the scratch local.
    fn complete_local(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.call_runtime("__exs_rt_frame_pop", span)?;
        self.function.instruction(&Instruction::LocalGet(0));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.call_runtime("__exs_rt_async_frame_complete", span)?;
        self.function.instruction(&Instruction::I32Const(1));
        self.function.instruction(&Instruction::I32Eq);
        self.function
            .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        self.function
            .instruction(&Instruction::I32Const(STATUS_COMPLETE));
        self.function.instruction(&Instruction::Else);
        self.function
            .instruction(&Instruction::I32Const(STATUS_READY));
        self.function.instruction(&Instruction::End);
        self.function.instruction(&Instruction::Return);
        Ok(())
    }

    /// Completes early when one durable slot contains a recoverable language Error.
    fn complete_if_error(
        &mut self,
        value: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_is_error", span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.complete(value, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Branches after validating a source Boolean and returns Error values early.
    fn branch_on_value(
        &mut self,
        value: u32,
        checked: u32,
        when_true: u32,
        when_false: u32,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(value, span)?;
        self.call_runtime("__exs_rt_condition_value", span)?;
        self.set_slot(checked, span)?;
        self.complete_if_error(checked, span)?;
        self.get_slot(checked, span)?;
        self.call_runtime("__exs_rt_condition", span)?;
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.ready(when_true, span)?;
        self.function.instruction(&Instruction::Else);
        self.ready(when_false, span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Checks one frame slot against a parameter contract or completes with TypeError.
    fn validate_slot_or_complete(
        &mut self,
        slot: u32,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.validate_slot_matches(slot, contract, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.get_slot(slot, span)?;
        self.function.instruction(&Instruction::I32Const(i32::from(
            super::types::permits_error(self.return_contract),
        )));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.complete_local(span)?;
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Replaces the scratch value with a TypeError when it violates the return contract.
    fn validate_local_return(
        &mut self,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.validate_scratch_matches(self.return_contract, span)?;
        self.function.instruction(&Instruction::LocalGet(2));
        self.function.instruction(&Instruction::I32Eqz);
        self.function
            .instruction(&Instruction::If(BlockType::Empty));
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::I32Const(i32::from(
            super::types::permits_error(self.return_contract),
        )));
        self.call_runtime("__exs_rt_type_mismatch", span)?;
        self.function
            .instruction(&Instruction::LocalSet(self.scratch_local));
        self.function.instruction(&Instruction::End);
        Ok(())
    }

    /// Writes the contract match result for one frame slot into local two.
    fn validate_slot_matches(
        &mut self,
        slot: u32,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.get_slot(slot, span)?;
        self.function
            .instruction(&Instruction::I32Const(contract.builtin_mask.cast_signed()));
        self.call_runtime("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.get_slot(slot, span)?;
            self.function
                .instruction(&Instruction::I32Const(type_id.cast_signed()));
            self.call_runtime("__exs_rt_object_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        Ok(())
    }

    /// Writes the contract match result for the scratch local into local two.
    fn validate_scratch_matches(
        &mut self,
        contract: &TypeContract,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function
            .instruction(&Instruction::I32Const(contract.builtin_mask.cast_signed()));
        self.call_runtime("__exs_rt_type_matches", span)?;
        self.function.instruction(&Instruction::LocalSet(2));
        for type_id in &contract.nominal_type_ids {
            self.function.instruction(&Instruction::LocalGet(2));
            self.function
                .instruction(&Instruction::LocalGet(self.scratch_local));
            self.function
                .instruction(&Instruction::I32Const(type_id.cast_signed()));
            self.call_runtime("__exs_rt_object_is_type", span)?;
            self.function.instruction(&Instruction::I32Or);
            self.function.instruction(&Instruction::LocalSet(2));
        }
        Ok(())
    }

    /// Emits a scalar literal construction.
    fn literal(
        &mut self,
        expression: &Expression<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
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
                self.call_runtime("__exs_rt_int_new", *span)
            }
            Expression::Float(value, span) => {
                self.function
                    .instruction(&Instruction::F64Const((*value).into()));
                self.call_runtime("__exs_rt_float_new", *span)
            }
            Expression::String(value, span) => self.string(value, *span),
            Expression::Bool(value, span) => {
                self.function
                    .instruction(&Instruction::I32Const(i32::from(*value)));
                self.call_runtime("__exs_rt_bool_new", *span)
            }
            Expression::None(span) => self.call_runtime("__exs_rt_none_new", *span),
            _ => Err(diagnostics(CompileDiagnostic::new(
                "E0999",
                expression_span(expression),
                "invalid scalar continuation literal",
            ))),
        }
    }

    /// Emits one checked ExS integer construction.
    fn integer(
        &mut self,
        value: i64,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if !is_valid_int(value) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0206",
                span,
                "integer literal is outside the ExS 56-bit range",
            )));
        }
        self.function.instruction(&Instruction::I64Const(value));
        self.call_runtime("__exs_rt_int_new", span)
    }

    /// Emits one compiler-owned passive-data string construction.
    fn string(
        &mut self,
        value: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
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
        self.call_runtime("__exs_rt_literal_buffer_alloc", span)?;
        self.function
            .instruction(&Instruction::LocalTee(self.scratch_local));
        self.function.instruction(&Instruction::I32Const(0));
        self.function.instruction(&Instruction::I32Const(length));
        self.function
            .instruction(&Instruction::MemoryInit { mem: 0, data_index });
        self.function
            .instruction(&Instruction::LocalGet(self.scratch_local));
        self.function.instruction(&Instruction::I32Const(length));
        self.call_runtime("__exs_rt_string_new", span)
    }

    /// Emits a named runtime ABI call after setting its source position.
    fn call_runtime(
        &mut self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        if name != "__exs_rt_set_source_position" {
            let position = self.source_map.id(span).ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0214",
                    span,
                    "missing source-map position for generated runtime call",
                ))
            })?;
            self.function
                .instruction(&Instruction::I32Const(position.cast_signed()));
            self.function.instruction(&Instruction::Call(
                self.runtime_index("__exs_rt_set_source_position", span)?,
            ));
        }
        self.function
            .instruction(&Instruction::Call(self.runtime_index(name, span)?));
        Ok(())
    }

    /// Resolves one stable runtime ABI export index.
    fn runtime_index(
        &self,
        name: &str,
        span: SourceSpan<'source>,
    ) -> Result<u32, CompileDiagnostics<'source>> {
        runtime_index(self.runtime, name, span)
    }
}

/// Runs step functions until the root completes or a Host ABI future suspends it.
fn emit_dispatch<'a>(
    function: &mut Function,
    dispatcher: u32,
    status: u32,
    result: u32,
    runtime: &HashMap<String, u32>,
    span: SourceSpan<'a>,
) -> Result<Function, CompileDiagnostics<'a>> {
    function.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
    function.instruction(&Instruction::Loop(BlockType::Empty));
    function.instruction(&Instruction::Call(dispatcher));
    function.instruction(&Instruction::LocalSet(status));
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_READY));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_PENDING));
    function.instruction(&Instruction::I32Eq);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::I32Const(STATUS_PENDING));
    function.instruction(&Instruction::Br(2));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::LocalGet(status));
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::I32Ne);
    function.instruction(&Instruction::If(BlockType::Empty));
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    call_runtime(
        function,
        runtime,
        "__exs_rt_async_frame_take_completed",
        span,
    )?;
    function.instruction(&Instruction::LocalSet(result));
    function.instruction(&Instruction::LocalGet(result));
    call_runtime(function, runtime, "__exs_rt_set_result", span)?;
    function.instruction(&Instruction::I32Const(STATUS_COMPLETE));
    function.instruction(&Instruction::Br(1));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::Unreachable);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    Ok(std::mem::replace(function, Function::new([])))
}

/// Emits one runtime call used by entry and resume wrappers.
fn call_runtime<'a>(
    function: &mut Function,
    runtime: &HashMap<String, u32>,
    name: &str,
    span: SourceSpan<'a>,
) -> Result<(), CompileDiagnostics<'a>> {
    function.instruction(&Instruction::Call(runtime_index(runtime, name, span)?));
    Ok(())
}

/// Resolves one stable runtime ABI export index.
fn runtime_index<'a>(
    runtime: &HashMap<String, u32>,
    name: &str,
    span: SourceSpan<'a>,
) -> Result<u32, CompileDiagnostics<'a>> {
    runtime.get(name).copied().ok_or_else(|| {
        diagnostics(CompileDiagnostic::new(
            "E0209",
            span,
            format!("runtime template does not export `{name}`"),
        ))
    })
}

/// Returns the runtime ABI operation implementing one binary source operator.
fn binary_operation(operator: BinaryOperator) -> &'static str {
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

/// Returns the source span owned by one continuation operation.
fn operation_span<'source>(operation: &Operation<'source, '_>) -> SourceSpan<'source> {
    match operation {
        Operation::Literal { expression, .. } => expression_span(expression),
        Operation::Integer { span, .. }
        | Operation::None { span, .. }
        | Operation::Boolean { span, .. }
        | Operation::Copy { span, .. }
        | Operation::Unary { span, .. }
        | Operation::Binary { span, .. }
        | Operation::List { span, .. }
        | Operation::Object { span, .. }
        | Operation::TypedObject { span, .. }
        | Operation::IsError { span, .. }
        | Operation::Propagate { span, .. }
        | Operation::Index { span, .. }
        | Operation::Property { span, .. }
        | Operation::IndexSet { span, .. }
        | Operation::PropertySet { span, .. }
        | Operation::HostCall { span, .. }
        | Operation::HostResume { span, .. }
        | Operation::DirectCall { span, .. }
        | Operation::ChildCall { span, .. }
        | Operation::InstanceCall { span, .. }
        | Operation::ValidateParameters { span, .. }
        | Operation::ValidateSlot { span, .. }
        | Operation::Branch { span, .. }
        | Operation::Goto { span, .. }
        | Operation::IterSnapshot { span, .. }
        | Operation::ForBranch { span, .. }
        | Operation::Increment { span, .. }
        | Operation::Return { span, .. } => *span,
    }
}

/// Returns the source span owned by one AST expression.
fn expression_span<'source>(expression: &Expression<'source>) -> SourceSpan<'source> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::List { span, .. }
        | Expression::Object { span, .. }
        | Expression::TypedObject { span, .. }
        | Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
    }
}
