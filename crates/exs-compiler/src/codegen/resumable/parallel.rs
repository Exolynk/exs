//! `par` in resumable code: every closure starts as a child task whose result fills the
//! parent's result List, and the parent waits until all of them completed.

use core::mem::{offset_of, size_of};

use exs_layout::{BufferObject, ListObject};
use exs_layout::{TYPE_FN, TYPE_LIST};
use wasm_encoder::{BlockType, Instruction};

use super::layouts::ClosureStep;
use super::step::StepEmitter;
use crate::codegen::closures::{CAPTURES, CLOSURE_INDEX};
use crate::codegen::collections::{u32_field, value_field};
use crate::codegen::immediate;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the element count of a List.
const LEN: u64 = offset_of!(ListObject, len) as u64;
/// Byte offset of the element Buffer of a List.
const ITEMS: u64 = offset_of!(ListObject, items) as u64;
/// Byte offset of the first slot of a Buffer.
const SLOTS: u64 = size_of::<BufferObject>() as u64;
/// Flag of a closure-arity table word marking a variadic closure.
const VARIADIC: u32 = 1 << 31;

/// Where the result index of a spawned task comes from.
#[derive(Clone, Copy)]
enum Index {
    /// A constant index.
    Constant(u32),
    /// The `i64` local holding the index.
    Local(u32),
}

impl<'a> StepEmitter<'a, '_> {
    /// Starts the closures in the durable `tasks` as children of the running task and waits
    /// for their results, which fill the List stored in `destination`.
    pub(super) fn parallel_start(
        &mut self,
        next: u32,
        tasks: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let count = u32::try_from(tasks.len()).unwrap_or(u32::MAX);
        self.compiler.push_vm();
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.compiler.core_call("__exs_core_parallel_new", span)?;
        self.set_slot(destination);
        for (index, task) in (0_u32..).zip(tasks) {
            let closure = self.load(*task);
            self.spawn(destination, Index::Constant(index), closure, span)?;
        }
        self.wait(next, span)
    }

    /// Starts every closure of the List in durable slot `functions` like v1's dynamic `par`:
    /// a value that is no List, an element that is no closure or one that takes arguments
    /// returns its Error from the function before any task starts.
    pub(super) fn parallel_dynamic_start(
        &mut self,
        next: u32,
        functions: u32,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let list = self.load(functions);
        self.instruction(Instruction::LocalGet(list));
        immediate::type_mask_test(&mut self.compiler.function, TYPE_LIST);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(list));
        self.compiler
            .core_call("__exs_core_parallel_list_error", span)?;
        self.complete(destination, span)?;
        self.instruction(Instruction::End);
        let index = self.compiler.scalar_local();
        let item = self.compiler.scalar_local();
        self.each_item(list, index, item, |emitter| {
            emitter.validate_task(item, destination, span)
        })?;
        self.compiler.push_vm();
        self.compiler.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(LEN)));
        self.compiler.core_call("__exs_core_parallel_new", span)?;
        self.set_slot(destination);
        self.each_item(list, index, item, |emitter| {
            emitter.spawn(destination, Index::Local(index), item, span)
        })?;
        self.wait(next, span)
    }

    /// Runs `body` for every element of the List in the temporary `list`, with the element's
    /// index in the `i64` local `index` and the element in the unrooted `item`, which the List
    /// keeps alive.
    fn each_item(
        &mut self,
        list: u32,
        index: u32,
        item: u32,
        body: impl FnOnce(&mut Self) -> Result<(), CompileDiagnostics<'a>>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.instruction(Instruction::I64Const(0));
        self.instruction(Instruction::LocalSet(index));
        self.instruction(Instruction::Block(BlockType::Empty));
        let done = self.compiler.depth;
        self.instruction(Instruction::Loop(BlockType::Empty));
        let head = self.compiler.depth;
        self.instruction(Instruction::LocalGet(index));
        self.compiler.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(LEN)));
        self.instruction(Instruction::I64ExtendI32U);
        self.instruction(Instruction::I64GeU);
        self.instruction(Instruction::BrIf(self.compiler.depth - done));
        self.compiler.push_address(list);
        self.instruction(Instruction::I32Load(u32_field(ITEMS)));
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Const(3));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Add);
        self.instruction(Instruction::I64Load(value_field(SLOTS)));
        self.instruction(Instruction::LocalSet(item));
        body(self)?;
        self.instruction(Instruction::LocalGet(index));
        self.instruction(Instruction::I64Const(1));
        self.instruction(Instruction::I64Add);
        self.instruction(Instruction::LocalSet(index));
        self.instruction(Instruction::Br(self.compiler.depth - head));
        self.instruction(Instruction::End);
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Returns v1's Error from the function unless the value in `item` is a closure that takes
    /// no fixed arguments or is variadic.
    fn validate_task(
        &mut self,
        item: u32,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.instruction(Instruction::LocalGet(item));
        immediate::type_mask_test(&mut self.compiler.function, TYPE_FN);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(item));
        self.compiler.core_call("__exs_core_not_callable", span)?;
        self.complete(destination, span)?;
        self.instruction(Instruction::End);
        let arities = self.closure_arities();
        self.instruction(Instruction::LocalGet(item));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Load(u32_field(CLOSURE_INDEX)));
        self.instruction(Instruction::I32Const(2));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Load(u32_field(u64::from(arities))));
        // A fixed arity other than 0 without a variadic tail is an ArityError.
        self.instruction(Instruction::I32Const((!VARIADIC).cast_signed()));
        self.instruction(Instruction::I32And);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::LocalGet(item));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Load(u32_field(CLOSURE_INDEX)));
        self.instruction(Instruction::I32Const(2));
        self.instruction(Instruction::I32Shl);
        self.instruction(Instruction::I32Load(u32_field(u64::from(arities))));
        self.instruction(Instruction::I32Const(VARIADIC.cast_signed()));
        self.instruction(Instruction::I32And);
        self.instruction(Instruction::I32Or);
        self.instruction(Instruction::I32Eqz);
        self.instruction(Instruction::If(BlockType::Empty));
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.compiler
            .core_call("__exs_core_closure_arity_error", span)?;
        self.complete(destination, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Returns the address of the static table of every closure's fixed arity, with
    /// [`VARIADIC`] set for variadic closures, by dense closure index.
    fn closure_arities(&self) -> u32 {
        let count = self
            .resumable
            .closures
            .keys()
            .max()
            .map_or(0, |index| index + 1);
        let words = (0..count)
            .map(|index| {
                self.resumable
                    .closures
                    .get(&index)
                    .map_or(u32::MAX, |closure| {
                        closure.arity | if closure.variadic { VARIADIC } else { 0 }
                    })
            })
            .collect();
        self.compiler.static_array(words)
    }

    /// Stores the Error on the stack in durable slot `destination` and returns it from the
    /// function, like v1 completing the frame.
    fn complete(
        &mut self,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.set_slot(destination);
        self.get_slot(destination);
        self.compiler.return_value(span)
    }

    /// Starts the closure in the temporary `closure` as a child task completing element
    /// `index` of the result List in durable slot `group`, with its captured Cells and, for a
    /// variadic closure, an empty rest List.
    fn spawn(
        &mut self,
        group: u32,
        index: Index,
        closure: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let mut cases: Vec<(u32, ClosureStep)> = self
            .resumable
            .closures
            .iter()
            .map(|(index, step)| (*index, *step))
            .collect();
        cases.sort_by_key(|(index, _)| *index);
        self.instruction(Instruction::Block(BlockType::Empty));
        let done = self.compiler.depth;
        for _ in &cases {
            self.instruction(Instruction::Block(BlockType::Empty));
        }
        // Label 0 leaves the closures `par` cannot start; label `k + 1` selects case `k`.
        self.instruction(Instruction::Block(BlockType::Empty));
        self.instruction(Instruction::LocalGet(closure));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Load(u32_field(CLOSURE_INDEX)));
        let limit = cases.last().map_or(0, |(index, _)| index + 1);
        let targets: Vec<u32> = (0..limit)
            .map(|index| {
                cases
                    .iter()
                    .position(|(case, _)| *case == index)
                    .map_or(0, |case| case as u32 + 1)
            })
            .collect();
        self.instruction(Instruction::BrTable(targets.into(), 0));
        self.instruction(Instruction::End);
        self.instruction(Instruction::Unreachable);
        for (_, case) in &cases {
            self.instruction(Instruction::End);
            self.compiler.push_vm();
            self.get_slot(group);
            match index {
                Index::Constant(index) => {
                    self.instruction(Instruction::I32Const(index.cast_signed()));
                }
                Index::Local(local) => {
                    self.instruction(Instruction::LocalGet(local));
                    self.instruction(Instruction::I32WrapI64);
                }
            }
            for value in [case.step.step, case.step.slot_count] {
                self.instruction(Instruction::I32Const(value.cast_signed()));
            }
            self.compiler.core_call("__exs_core_task_spawn", span)?;
            let child = self.compiler.scalar_local();
            self.instruction(Instruction::I64ExtendI32U);
            self.instruction(Instruction::LocalSet(child));
            for capture in 0..case.captures {
                self.child_slot(child, capture, |emitter| {
                    emitter.instruction(Instruction::LocalGet(closure));
                    emitter.instruction(Instruction::I32WrapI64);
                    emitter.instruction(Instruction::I64Load(value_field(
                        CAPTURES + u64::from(capture) * 8,
                    )));
                });
            }
            if case.variadic {
                let mut result = Ok(());
                self.child_slot(child, case.captures + case.arity, |emitter| {
                    emitter.compiler.push_vm();
                    emitter.instruction(Instruction::I32Const(0));
                    emitter.instruction(Instruction::I32Const(0));
                    result = emitter.compiler.core_call("__exs_core_list_new", span);
                });
                result?;
            }
            self.instruction(Instruction::Br(self.compiler.depth - done));
        }
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Waits for the running task's children, continuing at `next` once they completed.
    fn wait(&mut self, next: u32, span: SourceSpan<'a>) -> Result<(), CompileDiagnostics<'a>> {
        self.store_state(next);
        self.compiler.push_vm();
        self.compiler.core_call("__exs_core_parallel_wait", span)?;
        self.compiler.pop_frame();
        self.instruction(Instruction::Return);
        Ok(())
    }
}
