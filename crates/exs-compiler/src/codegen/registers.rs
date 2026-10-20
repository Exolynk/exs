//! Inline access to the runtime register block in linear memory.
//!
//! The runtime exports the block's constant address, so generated code reads and writes the
//! registers with plain loads and stores relative to address 0 instead of runtime calls.

use exs_abi::{
    REGISTERS_CALL_SITE_OFFSET, REGISTERS_QUANTUM_OFFSET, REGISTERS_ROOT_FLOOR_OFFSET,
    REGISTERS_ROOT_LIMIT_OFFSET, REGISTERS_ROOT_TOP_OFFSET, REGISTERS_TRACE_LIMIT_OFFSET,
    REGISTERS_TRACE_TOP_OFFSET, TRACE_ENTRY_SIZE,
};
use wasm_encoder::{BlockType, Function, Instruction, MemArg};

/// Returns the memory operand of one register at `offset` inside the block at `address`.
fn register(address: u32, offset: u32) -> MemArg {
    MemArg {
        offset: u64::from(address) + u64::from(offset),
        align: 2,
        memory_index: 0,
    }
}

/// Stores one call-site source position for the next generated function entry.
pub(super) fn store_call_site(function: &mut Function, address: u32, position: u32) {
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::I32Const(position.cast_signed()));
    function.instruction(&Instruction::I32Store(register(
        address,
        REGISTERS_CALL_SITE_OFFSET,
    )));
}

/// Returns the instructions that consume one scheduler checkpoint inline.
///
/// They leave an `if` open whose else branch must emit the runtime checkpoint slow path, which
/// runs when this checkpoint would exhaust the running task's quantum. The caller emits them
/// through its own builder so control depth stays tracked, and closes the `if` with `End`.
pub(super) fn checkpoint_countdown(address: u32) -> [Instruction<'static>; 12] {
    let quantum = register(address, REGISTERS_QUANTUM_OFFSET);
    [
        Instruction::I32Const(0),
        Instruction::I32Load(quantum),
        Instruction::I32Const(1),
        Instruction::I32GtU,
        Instruction::If(BlockType::Empty),
        Instruction::I32Const(0),
        Instruction::I32Const(0),
        Instruction::I32Load(quantum),
        Instruction::I32Const(1),
        Instruction::I32Sub,
        Instruction::I32Store(quantum),
        Instruction::Else,
    ]
}

/// Runtime exports used by the inline function entry and exit slow paths.
#[derive(Clone, Copy)]
pub(super) struct FrameExports {
    /// `__exs_rt_root_push`, which moves the root stack to a new chunk.
    pub(super) root_push: u32,
    /// `__exs_rt_root_pop`, which leaves the first frame of a chunk.
    pub(super) root_pop: u32,
    /// `__exs_rt_trace_grow`, which enlarges the error-trace buffer.
    pub(super) trace_grow: u32,
}

/// Returns the memory operand of one trace-entry field at `offset` from the entry address.
const fn trace_field(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 2,
        memory_index: 0,
    }
}

/// Returns the inline entry of one generated function.
///
/// Pushes a zeroed root frame of `slot_count` slots into `base_local` and one trace entry for
/// `function_id`, consuming the pending call site. The runtime only runs when the frame needs
/// a new chunk or the trace buffer is full. `trace_local` is an i32 scratch local.
pub(super) fn function_enter(
    address: u32,
    exports: FrameExports,
    function_id: u32,
    slot_count: u32,
    base_local: u32,
    trace_local: u32,
) -> Vec<Instruction<'static>> {
    let frame_bytes = slot_count.saturating_mul(8).cast_signed();
    let root_top = register(address, REGISTERS_ROOT_TOP_OFFSET);
    let call_site = register(address, REGISTERS_CALL_SITE_OFFSET);
    let trace_top = register(address, REGISTERS_TRACE_TOP_OFFSET);
    vec![
        // Root frame: bump the top inside the current chunk, or let the runtime switch chunks.
        Instruction::I32Const(0),
        Instruction::I32Load(root_top),
        Instruction::LocalTee(base_local),
        Instruction::I32Const(frame_bytes),
        Instruction::I32Add,
        Instruction::I32Const(0),
        Instruction::I32Load(register(address, REGISTERS_ROOT_LIMIT_OFFSET)),
        Instruction::I32GtU,
        Instruction::If(BlockType::Empty),
        Instruction::I32Const(slot_count.cast_signed()),
        Instruction::Call(exports.root_push),
        Instruction::LocalSet(base_local),
        Instruction::Else,
        Instruction::I32Const(0),
        Instruction::LocalGet(base_local),
        Instruction::I32Const(frame_bytes),
        Instruction::I32Add,
        Instruction::I32Store(root_top),
        Instruction::LocalGet(base_local),
        Instruction::I32Const(0),
        Instruction::I32Const(frame_bytes),
        Instruction::MemoryFill(0),
        Instruction::End,
        // Trace entry: function id and the pending call site, which is consumed.
        Instruction::I32Const(0),
        Instruction::I32Load(trace_top),
        Instruction::LocalTee(trace_local),
        Instruction::I32Const(0),
        Instruction::I32Load(register(address, REGISTERS_TRACE_LIMIT_OFFSET)),
        Instruction::I32GeU,
        Instruction::If(BlockType::Empty),
        Instruction::Call(exports.trace_grow),
        Instruction::LocalSet(trace_local),
        Instruction::End,
        Instruction::LocalGet(trace_local),
        Instruction::I32Const(function_id.cast_signed()),
        Instruction::I32Store(trace_field(0)),
        Instruction::LocalGet(trace_local),
        Instruction::I32Const(0),
        Instruction::I32Load(call_site),
        Instruction::I32Store(trace_field(4)),
        Instruction::I32Const(0),
        Instruction::I32Const(0),
        Instruction::I32Store(call_site),
        Instruction::I32Const(0),
        Instruction::LocalGet(trace_local),
        Instruction::I32Const(TRACE_ENTRY_SIZE.cast_signed()),
        Instruction::I32Add,
        Instruction::I32Store(trace_top),
    ]
}

/// Returns the inline exit of one generated function entered with [`function_enter`].
///
/// Pops the trace entry and the root frame at `base_local`; the runtime only runs when the
/// frame was the first one of its chunk.
pub(super) fn function_leave(
    address: u32,
    exports: FrameExports,
    base_local: u32,
) -> Vec<Instruction<'static>> {
    let trace_top = register(address, REGISTERS_TRACE_TOP_OFFSET);
    vec![
        Instruction::I32Const(0),
        Instruction::I32Const(0),
        Instruction::I32Load(trace_top),
        Instruction::I32Const(TRACE_ENTRY_SIZE.cast_signed()),
        Instruction::I32Sub,
        Instruction::I32Store(trace_top),
        Instruction::LocalGet(base_local),
        Instruction::I32Const(0),
        Instruction::I32Load(register(address, REGISTERS_ROOT_FLOOR_OFFSET)),
        Instruction::I32Eq,
        Instruction::If(BlockType::Empty),
        Instruction::LocalGet(base_local),
        Instruction::Call(exports.root_pop),
        Instruction::Else,
        Instruction::I32Const(0),
        Instruction::LocalGet(base_local),
        Instruction::I32Store(register(address, REGISTERS_ROOT_TOP_OFFSET)),
        Instruction::End,
    ]
}
