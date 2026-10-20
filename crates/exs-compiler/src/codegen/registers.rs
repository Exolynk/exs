//! Inline access to the runtime register block in linear memory.
//!
//! The runtime exports the block's constant address, so generated code reads and writes the
//! registers with plain loads and stores relative to address 0 instead of runtime calls.

use exs_abi::{REGISTERS_CALL_SITE_OFFSET, REGISTERS_QUANTUM_OFFSET};
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
