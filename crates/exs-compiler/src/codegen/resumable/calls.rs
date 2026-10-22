//! Calls in resumable code: direct calls of synchronous functions run inline, calls of
//! resumable units link a child frame and return to the dispatcher.

use exs_layout::{BuiltinMethod, STATUS_READY};
use wasm_encoder::{BlockType, Instruction};

use super::layouts::StepInfo;
use super::step::StepEmitter;
use crate::ast::{BinaryOperator, Expression, Identifier};
use crate::codegen::closures::accepts;
use crate::codegen::continuation::{ComparisonFallback, Operation};
use crate::codegen::declarations::InstanceMethod;
use crate::codegen::function::error;
use crate::codegen::trait_registry::TraitOperator;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Returns the source operator `operator` dispatches.
const fn binary_operator(operator: TraitOperator) -> BinaryOperator {
    match operator {
        TraitOperator::Add => BinaryOperator::Add,
        TraitOperator::Subtract => BinaryOperator::Subtract,
        TraitOperator::Multiply => BinaryOperator::Multiply,
        TraitOperator::Divide => BinaryOperator::Divide,
        TraitOperator::Equal => BinaryOperator::Equal,
        TraitOperator::NotEqual => BinaryOperator::NotEqual,
        TraitOperator::LessThan => BinaryOperator::LessThan,
        TraitOperator::LessOrEqual => BinaryOperator::LessOrEqual,
        TraitOperator::GreaterThan => BinaryOperator::GreaterThan,
        TraitOperator::GreaterOrEqual => BinaryOperator::GreaterOrEqual,
    }
}

/// Where the result of an operator or method call goes.
#[derive(Clone, Copy)]
struct Destination {
    /// Durable slot receiving the result.
    slot: u32,
    /// State continuing after the call.
    next: u32,
}

impl<'a> StepEmitter<'a, '_> {
    /// Emits `operation` when it is a call, ending in a state transition. Returns whether it
    /// was one.
    pub(super) fn call_operation(
        &mut self,
        next: u32,
        operation: &Operation<'a, '_>,
    ) -> Result<bool, CompileDiagnostics<'a>> {
        match operation {
            Operation::DirectCall {
                signature,
                arguments,
                destination,
                span,
            } => {
                let key = self.key_of(signature.function_id, *span)?;
                self.compiler.check_static_access(key, *span)?;
                if let Some(callee) = self.step_of(signature.function_id) {
                    self.child_call(next, callee, arguments, *destination, *span)?;
                    return Ok(true);
                }
                let index = self.sync_index(key, *span)?;
                for argument in arguments {
                    self.get_slot(*argument);
                }
                self.compiler.direct_call(index, *span)?;
                self.set_slot(*destination);
                self.ready(next, *span)?;
            }
            Operation::ChildCall {
                layout,
                arguments,
                destination,
                span,
                ..
            } => {
                let key = self.key_of(layout.function_id, *span)?;
                self.compiler.check_static_access(key, *span)?;
                let callee = self
                    .step_of(layout.function_id)
                    .ok_or_else(|| error("E0999", *span, "missing resumable callee".to_owned()))?;
                self.child_call(next, callee, arguments, *destination, *span)?;
            }
            Operation::ClosureCall {
                closure,
                arguments,
                destination,
                span,
            } => self.closure_call(next, *closure, arguments, *destination, *span)?,
            Operation::Operator {
                operator,
                left,
                right,
                targets,
                destination,
                comparison,
                span,
            } => self.operator(
                *operator,
                [*left, *right],
                targets,
                Destination {
                    slot: *destination,
                    next,
                },
                *comparison,
                *span,
            )?,
            Operation::InstanceCall {
                receiver,
                method,
                method_span,
                arguments,
                targets,
                destination,
                span,
                ..
            } => self.instance_call(
                *receiver,
                method,
                *method_span,
                arguments,
                targets,
                Destination {
                    slot: *destination,
                    next,
                },
                *span,
            )?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Returns the Wasm index of the synchronously compiled function `key`.
    fn sync_index(&self, key: &str, span: SourceSpan<'a>) -> Result<u32, CompileDiagnostics<'a>> {
        self.compiler
            .context
            .functions
            .get(key)
            .map(|info| info.index)
            .ok_or_else(|| error("E0999", span, format!("missing function `{key}`")))
    }

    /// Returns whether any of `targets` is a resumable unit.
    fn any_resumable(&self, targets: &[InstanceMethod]) -> bool {
        targets
            .iter()
            .any(|target| self.step_of(target.signature.function_id).is_some())
    }

    /// Emits the operator `operator` on the durable `operands`: synchronous code when no
    /// implementation is resumable, else the dispatch chain with child calls.
    fn operator(
        &mut self,
        operator: TraitOperator,
        operands: [u32; 2],
        targets: &[InstanceMethod],
        destination: Destination,
        comparison: Option<ComparisonFallback>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let binary = binary_operator(operator);
        // Built-in comparisons produce the final Bool and skip the following OrderingTest.
        let builtin = comparison.map_or(destination, |comparison| Destination {
            slot: comparison.destination,
            next: comparison.next,
        });
        if !self.any_resumable(targets) {
            self.synthesize(&operands, span, |mut operands| {
                let right = operands.remove(1);
                Expression::Binary {
                    operator: binary,
                    left: Box::new(operands.remove(0)),
                    right: Box::new(right),
                    span,
                }
            })?;
            self.set_slot(builtin.slot);
            return self.ready(builtin.next, span);
        }
        let left = self.load(operands[0]);
        let right = self.load(operands[1]);
        let context = self.compiler.context;
        let chain = context.dispatch.operator(operator);
        for target in chain {
            self.instruction(Instruction::LocalGet(left));
            self.compiler.type_tag_test(target.type_id, span)?;
            self.instruction(Instruction::If(BlockType::Empty));
            self.method_target(
                &target.key,
                operands[0],
                left,
                &[operands[1]],
                &[right],
                destination,
                span,
            )?;
            self.instruction(Instruction::End);
        }
        self.compiler
            .compile_builtin_binary(binary, left, right, span)?;
        self.set_slot(builtin.slot);
        self.ready(builtin.next, span)
    }

    /// Calls the implementation method `key` on the receiver in durable slot `receiver` and
    /// its temporary `receiver_local` with the durable `arguments`, loaded in `locals`: a
    /// child frame when it is resumable, else inline.
    #[allow(clippy::too_many_arguments)] // Durable slots and their loaded temporaries.
    fn method_target(
        &mut self,
        key: &str,
        receiver: u32,
        receiver_local: u32,
        arguments: &[u32],
        locals: &[u32],
        destination: Destination,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if let Some(callee) = self.resumable.steps.get(key).copied() {
            return self.resumable_method(
                key,
                callee,
                receiver,
                receiver_local,
                arguments,
                destination,
                span,
            );
        }
        self.compiler
            .call_method(key, receiver_local, locals, span)?;
        self.set_slot(destination.slot);
        self.ready(destination.next, span)
    }

    /// Emits `receiver.method(arguments)`: synchronous code when no implementation is
    /// resumable, else the dispatch chain with child calls before the built-in behavior.
    #[allow(clippy::too_many_arguments)] // Mirrors the graph operation's fields.
    fn instance_call(
        &mut self,
        receiver: u32,
        method: &str,
        method_span: SourceSpan<'a>,
        arguments: &[u32],
        targets: &[InstanceMethod],
        destination: Destination,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if self.iterator_next_states.contains(&self.current_state()) {
            return self.iterator_next(receiver, destination, span);
        }
        let helper = crate::prelude::iterator_helper(method);
        let helper_resumable =
            helper.is_some_and(|helper| self.resumable.steps.contains_key(helper));
        if !self.any_resumable(targets) && !helper_resumable {
            let slots: Vec<u32> = core::iter::once(receiver)
                .chain(arguments.iter().copied())
                .collect();
            self.synthesize(&slots, span, |mut operands| {
                let receiver = operands.remove(0);
                Expression::MethodCall {
                    receiver: Box::new(receiver),
                    method: Identifier {
                        name: method.to_owned(),
                        span: method_span,
                    },
                    arguments: operands,
                    span,
                }
            })?;
            self.set_slot(destination.slot);
            return self.ready(destination.next, span);
        }
        let receiver_local = self.load(receiver);
        let count = u32::try_from(arguments.len())
            .map_err(|_| error("E0212", span, "too many method arguments".to_owned()))?;
        let block = self.compiler.consecutive_locals(count.max(1));
        for (offset, argument) in (0..count).zip(arguments) {
            self.get_slot(*argument);
            self.compiler.store(block + offset);
        }
        let locals: Vec<u32> = (0..count).map(|offset| block + offset).collect();
        let context = self.compiler.context;
        let package = &self.compiler.declaration.package;
        for target in context.dispatch.instance(method, package) {
            self.instruction(Instruction::LocalGet(receiver_local));
            self.compiler.type_tag_test(target.type_id, span)?;
            self.instruction(Instruction::If(BlockType::Empty));
            self.method_target(
                &target.key,
                receiver,
                receiver_local,
                arguments,
                &locals,
                destination,
                span,
            )?;
            self.instruction(Instruction::End);
        }
        // The prelude's eager iterator helpers come before built-in methods.
        if let Some(helper) = helper {
            return self.method_target(
                helper,
                receiver,
                receiver_local,
                arguments,
                &locals,
                destination,
                span,
            );
        }
        self.compiler.builtin_method_call(
            BuiltinMethod::from_name(method),
            receiver_local,
            block,
            count,
            span,
        )?;
        self.set_slot(destination.slot);
        self.ready(destination.next, span)
    }

    /// Calls the resumable method `key` with the receiver in durable slot `receiver` and the
    /// durable `arguments` as a child frame, packing variadic arguments, or stores the
    /// ArityError when it takes a different number of arguments.
    #[allow(clippy::too_many_arguments)] // Durable slots and their loaded receiver.
    fn resumable_method(
        &mut self,
        key: &str,
        callee: StepInfo,
        receiver: u32,
        receiver_local: u32,
        arguments: &[u32],
        destination: Destination,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let (arity, variadic) = self
            .compiler
            .context
            .functions
            .get(key)
            .map(|info| (info.arity, info.variadic.is_some()))
            .ok_or_else(|| error("E0999", span, format!("missing method `{key}`")))?;
        if !accepts(arity, variadic, arguments.len() + 1) {
            self.compiler.set_position(span)?;
            self.compiler.push_vm();
            self.instruction(Instruction::LocalGet(receiver_local));
            self.compiler
                .core_call("__exs_core_nominal_arity_error", span)?;
            self.set_slot(destination.slot);
            return self.ready(destination.next, span);
        }
        let fixed = arity.saturating_sub(1) as usize;
        let rest = self.rest_list(&arguments[fixed.min(arguments.len())..], variadic, span)?;
        let child = self.link_child(destination.next, callee, destination.slot, span)?;
        self.child_slot(child, 0, |emitter| emitter.get_slot(receiver));
        for (slot, argument) in (1_u32..).zip(&arguments[..fixed]) {
            self.child_slot(child, slot, |emitter| emitter.get_slot(*argument));
        }
        if let Some(rest) = rest {
            self.child_slot(child, arity, |emitter| {
                emitter.instruction(Instruction::LocalGet(rest));
            });
        }
        self.leave(STATUS_READY);
        Ok(())
    }

    /// Advances the Object in durable slot `receiver` of a `for` loop through the
    /// `Iterator::next` dispatch like synchronous code: an Object without an Iterator
    /// implementation yields the NotIterable Error.
    fn iterator_next(
        &mut self,
        receiver: u32,
        destination: Destination,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let receiver_local = self.load(receiver);
        let context = self.compiler.context;
        for target in context.dispatch.iterator_next() {
            if !context.functions.contains_key(&target.key) {
                continue;
            }
            self.instruction(Instruction::LocalGet(receiver_local));
            self.compiler.type_tag_test(target.type_id, span)?;
            self.instruction(Instruction::If(BlockType::Empty));
            self.method_target(
                &target.key,
                receiver,
                receiver_local,
                &[],
                &[],
                destination,
                span,
            )?;
            self.instruction(Instruction::End);
        }
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(receiver_local));
        self.compiler
            .core_call("__exs_core_not_an_iterator", span)?;
        self.set_slot(destination.slot);
        self.ready(destination.next, span)
    }
}
