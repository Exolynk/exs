//! `for` loops.
//!
//! A loop over a Range literal of immediate Ints counts in locals; a loop over any other value
//! iterates the core's snapshot, a List copy for Lists, Strings and Bytes. Both have one inline
//! fast path. When it is exhausted, a loop over an Object, or over a Range literal whose bounds
//! are no immediate Ints, advances through the `Iterator::next` dispatch and its
//! `IteratorStep`, like the resumable loops. The body is compiled once.

use std::collections::HashMap;

use core::mem::{offset_of, size_of};

use exs_layout::{BufferObject, EnumObject, ListObject};
use exs_layout::{RANGE_BEGIN_FALLBACK, RANGE_STATE_FINISHED, RANGE_STATE_LAST, TYPE_LIST};
use wasm_encoder::{BlockType, Instruction, ValType};

use super::collections::{u32_field, value_field};
use super::errors::ForBinding;
use super::function::{FunctionCompiler, error};
use crate::ast::{Block, Expression};
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the element count of a List.
const LEN: u64 = offset_of!(ListObject, len) as u64;
/// Byte offset of the element Buffer of a List.
const ITEMS: u64 = offset_of!(ListObject, items) as u64;
/// Byte offset of the first slot of a Buffer.
const SLOTS: u64 = size_of::<BufferObject>() as u64;
/// Byte offset of the variant index of an enum.
const VARIANT: u64 = offset_of!(EnumObject, variant) as u64;
/// Byte offset of the payload field count of an enum.
const FIELD_COUNT: u64 = offset_of!(EnumObject, field_count) as u64;
/// Byte offset of the first payload field of an enum.
const FIELDS: u64 = size_of::<EnumObject>() as u64;

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles a `for` loop over the value of `iterable`.
    pub(super) fn compile_iterable_for(
        &mut self,
        binding: ForBinding<'a, '_>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(iterable)?;
        let source = self.store_new();
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(source));
        self.core_call("__exs_core_for_snapshot", span)?;
        let snapshot = self.store_new();
        self.return_if_error(snapshot, span)?;
        // A List snapshot is a private copy, so its length and Buffer stay fixed.
        let (limit, items, position) = (
            self.scalar_local(),
            self.scalar_local(),
            self.scalar_local(),
        );
        for (local, offset) in [(limit, LEN), (items, ITEMS)] {
            self.instruction(Instruction::LocalGet(snapshot));
            immediate::type_mask_test(&mut self.function, TYPE_LIST);
            self.instruction(Instruction::If(BlockType::Result(ValType::I64)));
            self.push_address(snapshot);
            self.instruction(Instruction::I32Load(u32_field(offset)));
            self.instruction(Instruction::I64ExtendI32U);
            self.instruction(Instruction::Else);
            self.instruction(Instruction::I64Const(0));
            self.instruction(Instruction::End);
            self.instruction(Instruction::LocalSet(local));
        }
        self.instruction(Instruction::I64Const(0));
        self.instruction(Instruction::LocalSet(position));
        let item = self.value_local();
        self.instruction(Instruction::Block(BlockType::Empty));
        let break_depth = self.depth;
        self.instruction(Instruction::Loop(BlockType::Empty));
        let loop_depth = self.depth;
        self.instruction(Instruction::LocalGet(position));
        self.instruction(Instruction::LocalGet(limit));
        self.instruction(Instruction::I64GeU);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        // Exhausted List, or an Object that iterates through `Iterator::next`.
        self.instruction(Instruction::LocalGet(snapshot));
        immediate::type_mask_test(&mut self.function, TYPE_LIST);
        self.instruction(Instruction::BrIf(self.depth - break_depth));
        self.iterator_next(snapshot, break_depth, span)?;
        self.instruction(Instruction::Else);
        self.instruction(Instruction::LocalGet(items));
        self.instruction(Instruction::LocalGet(position));
        self.instruction(Instruction::I64Const(3));
        self.instruction(Instruction::I64Shl);
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I64Load(value_field(SLOTS)));
        self.instruction(Instruction::LocalGet(position));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::LocalSet(position));
        self.instruction(Instruction::End);
        self.loop_body(binding, item, body, break_depth, loop_depth)
    }

    /// Compiles a `for` loop over a literal `a..b` or `a..=b` Range.
    ///
    /// Immediate Int bounds count in locals like the prelude `Range::next`: the cursor moves
    /// toward the end by one and an inclusive Range yields its end last. Other bounds create
    /// the prelude Range, with its contract checks, and iterate it.
    pub(super) fn compile_range_for(
        &mut self,
        binding: ForBinding<'a, '_>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Expression::StaticMethodCall {
            type_name,
            method,
            arguments,
            span: call_span,
        } = iterable
        else {
            return Err(error("E0999", span, "missing Range literal".to_owned()));
        };
        let [start_expression, end_expression] = arguments.as_slice() else {
            return Err(error("E0999", span, "missing Range bounds".to_owned()));
        };
        self.compile_expression(start_expression)?;
        let start = self.store_new();
        self.compile_expression(end_expression)?;
        let end = self.store_new();
        let counter = Counter::new(self, method.name == "through", false);
        immediate::both_small_ints_test(
            &mut self.function,
            immediate::Operand::Local(start),
            immediate::Operand::Local(end),
        );
        self.instruction(Instruction::If(BlockType::Empty));
        immediate::push_payload(&mut self.function, start);
        self.instruction(Instruction::LocalSet(counter.cursor));
        immediate::push_payload(&mut self.function, end);
        self.instruction(Instruction::LocalSet(counter.last));
        counter.start_counting(self);
        self.instruction(Instruction::Else);
        let key = format!("{}::{}", type_name.name, method.name);
        let index = self
            .context
            .functions
            .get(&key)
            .map(|info| info.index)
            .ok_or_else(|| error("E0999", span, format!("missing function `{key}`")))?;
        self.instruction(Instruction::LocalGet(start));
        self.instruction(Instruction::LocalGet(end));
        self.direct_call(index, *call_span)?;
        self.store(counter.range);
        self.return_if_error(counter.range, span)?;
        counter.iterate_object(self);
        self.instruction(Instruction::End);
        self.counting_loop(&counter, binding, body, span)
    }

    /// Compiles a `for` loop over a Range variable no source code observes while it runs.
    ///
    /// A well-formed unit-step Range counts in locals and gets its `current` and
    /// `done` fields written back once the loop exits; any other value iterates itself.
    pub(super) fn compile_range_variable_for(
        &mut self,
        binding: ForBinding<'a, '_>,
        iterable: &Expression<'a>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(range_type) = self.context.types.get("Range").map(|range| range.id) else {
            return self.compile_iterable_for(binding, iterable, body, span);
        };
        self.compile_expression(iterable)?;
        let counter = Counter::new(self, true, true);
        self.store(counter.range);
        self.instruction(Instruction::LocalGet(counter.range));
        self.instruction(Instruction::I32Const(range_type.cast_signed()));
        self.core_call("__exs_core_range_begin", span)?;
        self.instruction(Instruction::LocalTee(counter.cursor));
        self.instruction(Instruction::I64Const(RANGE_BEGIN_FALLBACK));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        // The fallback iterates the value itself, which must be iterable.
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(counter.range));
        self.core_call("__exs_core_for_snapshot", span)?;
        self.store(counter.range);
        self.return_if_error(counter.range, span)?;
        counter.iterate_object(self);
        self.instruction(Instruction::Else);
        self.instruction(Instruction::LocalGet(counter.cursor));
        self.instruction(Instruction::I64Const(32));
        self.instruction(Instruction::I64ShrS);
        self.instruction(Instruction::LocalSet(counter.last));
        self.instruction(Instruction::LocalGet(counter.cursor));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I64ExtendI32S);
        self.instruction(Instruction::LocalSet(counter.cursor));
        counter.start_counting(self);
        self.instruction(Instruction::End);
        self.counting_loop(&counter, binding, body, span)?;
        self.instruction(Instruction::LocalGet(counter.generic));
        self.instruction(Instruction::I64Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::LocalGet(counter.range));
        self.instruction(Instruction::I32Const(range_type.cast_signed()));
        self.instruction(Instruction::LocalGet(counter.cursor));
        self.instruction(Instruction::LocalGet(counter.done));
        self.core_call("__exs_core_range_finish", span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Compiles the loop of `counter`: the next item is the cursor while counting, else the
    /// next item of its object; then the body.
    fn counting_loop(
        &mut self,
        counter: &Counter,
        binding: ForBinding<'a, '_>,
        body: &Block<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let item = self.value_local();
        self.instruction(Instruction::Block(BlockType::Empty));
        let break_depth = self.depth;
        self.instruction(Instruction::Loop(BlockType::Empty));
        let loop_depth = self.depth;
        // The fast path branches to `slow` once counting is over and else to `next` with the
        // cursor as the item.
        self.instruction(Instruction::Block(BlockType::Result(VALUE_TYPE)));
        let next = self.depth;
        self.instruction(Instruction::Block(BlockType::Empty));
        let slow = self.depth;
        self.instruction(Instruction::LocalGet(counter.done));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::BrIf(self.depth - slow));
        if !counter.precounted {
            let (ascending_past, descending_past) = if counter.inclusive {
                (Instruction::I64GtS, Instruction::I64LtS)
            } else {
                (Instruction::I64GeS, Instruction::I64LeS)
            };
            self.instruction(Instruction::LocalGet(counter.step));
            self.instruction(Instruction::I64Const(0));
            self.instruction(Instruction::I64GtS);
            self.instruction(Instruction::If(BlockType::Result(ValType::I32)));
            self.instruction(Instruction::LocalGet(counter.cursor));
            self.instruction(Instruction::LocalGet(counter.last));
            self.instruction(ascending_past);
            self.instruction(Instruction::Else);
            self.instruction(Instruction::LocalGet(counter.cursor));
            self.instruction(Instruction::LocalGet(counter.last));
            self.instruction(descending_past);
            self.instruction(Instruction::End);
            self.instruction(Instruction::BrIf(self.depth - slow));
        }
        immediate::push_small_int(&mut self.function, counter.cursor);
        self.instruction(Instruction::LocalGet(counter.cursor));
        self.instruction(Instruction::LocalGet(counter.last));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Empty));
        self.instruction(Instruction::I64Const(RANGE_STATE_LAST));
        self.instruction(Instruction::LocalSet(counter.done));
        self.instruction(Instruction::Else);
        self.instruction(Instruction::LocalGet(counter.cursor));
        self.instruction(Instruction::LocalGet(counter.step));
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::LocalSet(counter.cursor));
        self.instruction(Instruction::End);
        self.instruction(Instruction::Br(self.depth - next));
        self.instruction(Instruction::End);
        // Done counting, or an object that iterates through `Iterator::next`.
        self.instruction(Instruction::I64Const(RANGE_STATE_FINISHED));
        self.instruction(Instruction::LocalSet(counter.done));
        self.instruction(Instruction::LocalGet(counter.generic));
        self.instruction(Instruction::I64Eqz);
        self.instruction(Instruction::BrIf(self.depth - break_depth));
        self.iterator_next(counter.range, break_depth, span)?;
        self.instruction(Instruction::End);
        self.loop_body(binding, item, body, break_depth, loop_depth)
    }

    /// Compiles the rest of a loop whose next item is on the stack: binds it to `item`,
    /// compiles `body` in one block that `continue` leaves, and closes the loop.
    fn loop_body(
        &mut self,
        binding: ForBinding<'a, '_>,
        item: u32,
        body: &Block<'a>,
        break_depth: u32,
        loop_depth: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.store(item);
        self.check_for_binding(item, binding)?;
        if self.is_cell(&binding.identifier.name) {
            self.wrap_in_cell(item)?;
        }
        self.scopes
            .push(HashMap::from([(binding.identifier.name.clone(), item)]));
        self.instruction(Instruction::Block(BlockType::Empty));
        let continue_depth = self.depth;
        self.loops.push((break_depth, continue_depth));
        for statement in &body.statements {
            self.compile_statement(statement)?;
        }
        self.loops.pop();
        self.instruction(Instruction::End);
        self.branch_to(loop_depth);
        self.instruction(Instruction::End);
        self.instruction(Instruction::End);
        self.scopes.pop();
        Ok(())
    }

    /// Advances the Object in `iterator` through `Iterator::next` and pushes the
    /// item, or leaves the loop at `break_depth` on `IteratorStep::Done`.
    ///
    /// An Error returns from the function, an Object without an Iterator implementation
    /// yields the NotIterable Error, and a value that is no `IteratorStep` its MatchError.
    fn iterator_next(
        &mut self,
        iterator: u32,
        break_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.context;
        // Well-formed prelude Ranges step natively; others call `Range::next`.
        let range = context
            .types
            .get("Range")
            .filter(|_| context.functions.contains_key("Range::next"))
            .map(|range| range.id);
        let native = self.scalar_local();
        if let Some(range) = range {
            self.instruction(Instruction::LocalGet(iterator));
            self.instruction(Instruction::I32Const(range.cast_signed()));
            self.core_call("__exs_core_range_step", span)?;
            self.instruction(Instruction::LocalTee(native));
            self.instruction(Instruction::I64Eqz);
            self.instruction(Instruction::BrIf(self.depth - break_depth));
            self.instruction(Instruction::LocalGet(native));
            self.instruction(Instruction::I64Const(-1));
            self.instruction(Instruction::I64Eq);
            self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        }
        self.dispatched_next(iterator, break_depth, span)?;
        if range.is_some() {
            self.instruction(Instruction::Else);
            self.instruction(Instruction::LocalGet(native));
            self.instruction(Instruction::End);
        }
        Ok(())
    }

    /// Advances the Object in `iterator` through the `Iterator::next` dispatch and its
    /// `IteratorStep`, pushing the item or leaving the loop at `break_depth`.
    fn dispatched_next(
        &mut self,
        iterator: u32,
        break_depth: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.context;
        let step_type = |name: &str| {
            context
                .types
                .enum_variant(name)
                .map(|variant| (variant.type_id, variant.index))
                .ok_or_else(|| error("E0999", span, format!("missing prelude `{name}`")))
        };
        let (step_type_id, done_index) = step_type("IteratorStep::Done")?;
        let (_, item_index) = step_type("IteratorStep::Item")?;
        // Types without a reached implementation cannot be constructed, so they need no test.
        let targets: Vec<_> = context
            .dispatch
            .iterator_next()
            .iter()
            .filter(|target| context.functions.contains_key(&target.key))
            .cloned()
            .collect();
        self.dispatch_chain(&targets, iterator, &[], span, |compiler| {
            compiler.set_position(span)?;
            compiler.push_vm();
            compiler.instruction(Instruction::LocalGet(iterator));
            compiler.core_call("__exs_core_not_an_iterator", span)
        })?;
        let step = self.store_new();
        self.return_if_error(step, span)?;
        for (index, done) in [(done_index, true), (item_index, false)] {
            self.instruction(Instruction::LocalGet(step));
            self.type_tag_test(step_type_id, span)?;
            self.instruction(Instruction::If(BlockType::Result(ValType::I32)));
            self.push_address(step);
            self.instruction(Instruction::I32Load(u32_field(VARIANT)));
            self.instruction(Instruction::I32Const(index.cast_signed()));
            self.instruction(Instruction::I32Eq);
            self.instruction(Instruction::Else);
            self.instruction(Instruction::I32Const(0));
            self.instruction(Instruction::End);
            if done {
                self.instruction(Instruction::BrIf(self.depth - break_depth));
            }
        }
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.push_address(step);
        self.instruction(Instruction::I32Load(u32_field(FIELD_COUNT)));
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.push_address(step);
        self.instruction(Instruction::I64Load(value_field(FIELDS)));
        self.instruction(Instruction::Else);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(step));
        self.core_call("__exs_core_enum_missing_field", span)?;
        self.instruction(Instruction::End);
        self.instruction(Instruction::Else);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(step));
        self.core_call("__exs_core_match_error", span)?;
        let mismatch = self.store_new();
        self.return_if_error(mismatch, span)?;
        self.instruction(Instruction::LocalGet(mismatch));
        self.instruction(Instruction::End);
        Ok(())
    }
}

/// The locals of a `for` loop that counts a Range in locals or iterates an object.
struct Counter {
    /// The next item while counting.
    cursor: u32,
    /// The bound, or the last item when `precounted`.
    last: u32,
    /// The signed distance between items.
    step: u32,
    /// 0 before the last item, `RANGE_STATE_LAST` after it, `RANGE_STATE_FINISHED` once
    /// another item was requested.
    done: u32,
    /// 1 when the loop iterates the object in `range` instead of counting.
    generic: u32,
    /// The Range object, or the value the loop iterates.
    range: u32,
    /// Whether an exclusive bound is excluded from the items.
    inclusive: bool,
    /// Whether `last` is the last item, so no bound test is needed.
    precounted: bool,
}

impl Counter {
    /// Allocates the locals of a counting loop.
    fn new(compiler: &mut FunctionCompiler<'_, '_>, inclusive: bool, precounted: bool) -> Self {
        Self {
            cursor: compiler.scalar_local(),
            last: compiler.scalar_local(),
            step: compiler.scalar_local(),
            done: compiler.scalar_local(),
            generic: compiler.scalar_local(),
            range: compiler.value_local(),
            inclusive,
            precounted,
        }
    }

    /// Starts counting from `cursor` toward `last`; a start after the end counts down.
    fn start_counting(&self, compiler: &mut FunctionCompiler<'_, '_>) {
        compiler.instruction(Instruction::I64Const(-1));
        compiler.instruction(Instruction::I64Const(1));
        compiler.instruction(Instruction::LocalGet(self.cursor));
        compiler.instruction(Instruction::LocalGet(self.last));
        compiler.instruction(Instruction::I64GtS);
        compiler.instruction(Instruction::Select);
        compiler.instruction(Instruction::LocalSet(self.step));
        for local in [self.done, self.generic] {
            compiler.instruction(Instruction::I64Const(0));
            compiler.instruction(Instruction::LocalSet(local));
        }
    }

    /// Makes the loop iterate the object in `range`.
    fn iterate_object(&self, compiler: &mut FunctionCompiler<'_, '_>) {
        for local in [self.done, self.generic] {
            compiler.instruction(Instruction::I64Const(1));
            compiler.instruction(Instruction::LocalSet(local));
        }
    }
}
