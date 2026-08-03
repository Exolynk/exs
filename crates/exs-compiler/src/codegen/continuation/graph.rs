//! Continuation graph data structures and graph-wide helper functions.

use std::collections::{HashMap, HashSet};

use crate::ast::{BinaryOperator, Expression, FunctionDeclaration, UnaryOperator};
use crate::codegen::diagnostics;
use crate::codegen::function::{FunctionSignature, InstanceMethod, LiftedFunction, MethodRegistry};
use crate::codegen::trait_registry::TraitOperator;
use crate::codegen::types;
use crate::codegen::types::{TypeContract, TypeRegistry};
use crate::codegen::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::FrameLayout;
pub(super) struct ContinuationGraph<'source, 'function> {
    /// One continuation operation per generated state.
    pub(super) operations: Vec<Operation<'source, 'function>>,
    /// Number of slots required by parameters, lexical bindings, and temporaries.
    pub(super) slot_count: u32,
}

/// One non-suspending operation or host-call boundary in a continuation graph.
pub(super) enum Operation<'source, 'function> {
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
    /// Allocates shared Cell storage for one captured lexical binding.
    CellNew {
        value: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Reads the current value from one shared captured lexical binding.
    CellGet {
        cell: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Replaces the current value in one shared captured lexical binding.
    CellSet {
        cell: u32,
        value: u32,
        span: SourceSpan<'source>,
    },
    /// Creates one runtime closure retaining shared captured Cells.
    Closure {
        layout: FrameLayout,
        arity: usize,
        captures: Vec<u32>,
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
    /// Dispatches one standard operator trait before using its runtime fallback.
    Operator {
        /// Source operator selecting the trait implementation.
        operator: TraitOperator,
        left: u32,
        right: u32,
        targets: Vec<InstanceMethod>,
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
    /// Constructs one tagged enum variant after all payload values are evaluated.
    Enum {
        /// Compiler-owned nominal enum tag.
        type_id: u32,
        /// Stable host-boundary enum identity.
        type_identity: String,
        /// Selected source-visible variant name.
        variant: String,
        /// Ordered payload frame slots.
        fields: Vec<u32>,
        /// Frame slot retaining the type identity string across further allocations.
        type_identity_slot: u32,
        /// Frame slot retaining the variant string across further allocations.
        variant_slot: u32,
        /// Frame slot receiving the constructed enum value.
        destination: u32,
        /// Source location for this constructor invocation.
        span: SourceSpan<'source>,
    },
    /// Tests one value against a nominal enum identity and variant name.
    EnumMatches {
        /// Frame slot holding the value under test.
        value: u32,
        /// Stable host-boundary enum identity.
        type_identity: String,
        /// Source-visible variant name.
        variant: String,
        /// Frame slot retaining the identity across literal allocation.
        type_identity_slot: u32,
        /// Frame slot retaining the variant across literal allocation.
        variant_slot: u32,
        /// Frame slot receiving the Boolean test result.
        destination: u32,
        /// Source location for this match pattern.
        span: SourceSpan<'source>,
    },
    /// Reads one payload field from an enum value selected by a preceding match test.
    EnumField {
        value: u32,
        index: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Creates the Error returned when no match arm accepts a runtime value.
    MatchError {
        value: u32,
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
    /// Invokes a runtime closure through its generated continuation frame.
    ClosureCall {
        closure: u32,
        arguments: Vec<u32>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Spawns zero-argument closure values as parallel child tasks and suspends their parent.
    ParallelStart {
        tasks: Vec<u32>,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Spawns every closure stored in a runtime List as a parallel child task.
    ParallelDynamicStart {
        functions: u32,
        destination: u32,
        span: SourceSpan<'source>,
    },
    /// Replaces a completed compiler-internal parallel group with its ordered result List.
    ParallelTake {
        group: u32,
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
        offset: u32,
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
pub(super) struct GraphBuilder<'source, 'function> {
    /// Durable lexical scopes, mapping names to frame slots.
    pub(super) scopes: Vec<HashMap<String, BindingSlot>>,
    /// The next durable temporary slot.
    pub(super) next_slot: u32,
    /// Graph operations emitted in source evaluation order.
    pub(super) operations: Vec<Operation<'source, 'function>>,
    /// Active loop back-edges and break exits while lowering nested blocks.
    pub(super) loops: Vec<LoopBuilderContext>,
    /// Whether this function may return Error through `?`.
    pub(super) permits_error: bool,
    /// Linked source function signatures used while lowering direct calls.
    pub(super) signatures: &'function HashMap<String, FunctionSignature>,
    /// Durable layouts of every suspendable call target.
    pub(super) frame_layouts: &'function HashMap<String, FrameLayout>,
    /// Instance and static implementation-method metadata.
    pub(super) methods: &'function MethodRegistry,
    /// Nominal Object declarations and field contracts.
    pub(super) types: &'function TypeRegistry,
    /// Source spellings that require shared Cell storage when declared.
    pub(super) captured_names: HashSet<String>,
    /// Compiler-private functions lifted from closure expressions.
    pub(super) lifted: &'function [LiftedFunction<'source>],
}

/// One durable lexical binding slot and its storage representation.
#[derive(Clone, Copy)]
pub(super) struct BindingSlot {
    /// The durable frame slot holding a value or Cell reference.
    pub(super) slot: u32,
    /// Whether the slot holds a Cell rather than its source-visible value.
    pub(super) cell: bool,
}

/// Unresolved loop branches collected until a loop's exit state is known.
pub(super) struct LoopBuilderContext {
    /// Explicit `continue` branches whose target is emitted after the loop body.
    pub(super) continues: Vec<usize>,
    /// Explicit `break` branches whose target is emitted after the loop body.
    pub(super) breaks: Vec<usize>,
}

impl<'source, 'function> ContinuationGraph<'source, 'function> {
    /// Lowers the sequential subset of one resumable function.
    pub(super) fn build(
        declaration: &'function FunctionDeclaration<'source>,
        signature: &FunctionSignature,
        signatures: &'function HashMap<String, FunctionSignature>,
        frame_layouts: &'function HashMap<String, FrameLayout>,
        lifted: &'function [LiftedFunction<'source>],
        methods: &'function MethodRegistry,
        types: &'function TypeRegistry,
    ) -> Result<Self, CompileDiagnostics<'source>> {
        let capture_count = signature.capture_count;
        let parameter_count = declaration
            .parameters
            .len()
            .checked_add(capture_count)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many function parameters",
                ))
            })?;
        let captured_names = lifted
            .iter()
            .flat_map(|closure| closure.captures.iter().cloned())
            .collect::<HashSet<_>>();
        let mut parameters = HashMap::new();
        let lifted_current = lifted
            .iter()
            .find(|closure| closure.key == declaration.name.name);
        if let Some(closure) = lifted_current {
            for (index, capture) in closure.captures.iter().enumerate() {
                parameters.insert(
                    capture.clone(),
                    BindingSlot {
                        slot: u32::try_from(index).map_err(|_| {
                            diagnostics(CompileDiagnostic::new(
                                "E0212",
                                declaration.span,
                                "too many closure captures",
                            ))
                        })?,
                        cell: true,
                    },
                );
            }
        }
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            let index = capture_count
                .checked_add(index)
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(|| {
                    diagnostics(CompileDiagnostic::new(
                        "E0212",
                        declaration.span,
                        "too many function parameters",
                    ))
                })?;
            parameters.insert(
                parameter.name.name.clone(),
                BindingSlot {
                    slot: index,
                    cell: captured_names.contains(&parameter.name.name),
                },
            );
        }
        let cell_parameters = parameters
            .values()
            .filter(|parameter| parameter.cell && parameter.slot >= capture_count as u32)
            .copied()
            .collect::<Vec<_>>();
        let mut builder = GraphBuilder {
            scopes: vec![parameters],
            next_slot: parameter_count,
            operations: Vec::new(),
            loops: Vec::new(),
            permits_error: types::permits_error(&signature.return_type),
            signatures,
            frame_layouts,
            methods,
            types,
            captured_names,
            lifted,
        };
        builder.operations.push(Operation::ValidateParameters {
            contracts: signature.parameter_types.clone(),
            offset: u32::try_from(capture_count).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    declaration.span,
                    "too many closure captures",
                ))
            })?,
            span: declaration.span,
        });
        for parameter in cell_parameters {
            builder.operations.push(Operation::CellNew {
                value: parameter.slot,
                destination: parameter.slot,
                span: declaration.span,
            });
        }
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

pub(super) fn operation_span<'source>(operation: &Operation<'source, '_>) -> SourceSpan<'source> {
    match operation {
        Operation::Literal { expression, .. } => expression_span(expression),
        Operation::Integer { span, .. }
        | Operation::None { span, .. }
        | Operation::Boolean { span, .. }
        | Operation::Copy { span, .. }
        | Operation::CellNew { span, .. }
        | Operation::CellGet { span, .. }
        | Operation::CellSet { span, .. }
        | Operation::Closure { span, .. }
        | Operation::Unary { span, .. }
        | Operation::Binary { span, .. }
        | Operation::Operator { span, .. }
        | Operation::List { span, .. }
        | Operation::Object { span, .. }
        | Operation::TypedObject { span, .. }
        | Operation::Enum { span, .. }
        | Operation::EnumMatches { span, .. }
        | Operation::EnumField { span, .. }
        | Operation::MatchError { span, .. }
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
        | Operation::ClosureCall { span, .. }
        | Operation::ParallelStart { span, .. }
        | Operation::ParallelDynamicStart { span, .. }
        | Operation::ParallelTake { span, .. }
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
pub(super) fn expression_span<'source>(expression: &Expression<'source>) -> SourceSpan<'source> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Closure { span, .. } => *span,
        Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::List { span, .. }
        | Expression::Object { span, .. }
        | Expression::TypedObject { span, .. }
        | Expression::Match { span, .. }
        | Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
        Expression::ParallelStatic { span, .. } | Expression::ParallelDynamic { span, .. } => *span,
    }
}
