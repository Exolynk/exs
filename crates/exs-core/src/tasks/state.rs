//! Access to the VM's task state.

use core::mem::offset_of;

use exs_layout::{TaskState, VmState};

use crate::memory::Memory;

/// Byte offset of one field of [`TaskState`] in the VM state block.
macro_rules! task_offset {
    ($field:ident) => {
        (offset_of!(VmState, task) + offset_of!(TaskState, $field)) as u32
    };
}

/// One `u64` field of [`TaskState`].
#[derive(Clone, Copy)]
pub(super) enum Field {
    /// [`TaskState::frame`].
    Frame,
    /// [`TaskState::ready`].
    Ready,
    /// [`TaskState::completed`].
    Completed,
    /// [`TaskState::current`].
    Current,
    /// [`TaskState::queue_head`].
    QueueHead,
    /// [`TaskState::queue_tail`].
    QueueTail,
    /// [`TaskState::tasks`].
    Tasks,
    /// [`TaskState::host_call`].
    HostCall,
    /// [`TaskState::next_host_call`].
    NextHostCall,
    /// [`TaskState::quantum`].
    Quantum,
}

impl Field {
    /// Returns the byte offset of the field in the VM state block.
    const fn offset(self) -> u32 {
        match self {
            Self::Frame => task_offset!(frame),
            Self::Ready => task_offset!(ready),
            Self::Completed => task_offset!(completed),
            Self::Current => task_offset!(current),
            Self::QueueHead => task_offset!(queue_head),
            Self::QueueTail => task_offset!(queue_tail),
            Self::Tasks => task_offset!(tasks),
            Self::HostCall => task_offset!(host_call),
            Self::NextHostCall => task_offset!(next_host_call),
            Self::Quantum => task_offset!(quantum),
        }
    }
}

/// Reads one task field of the VM at `vm`.
pub(super) fn load(memory: &impl Memory, vm: u32, field: Field) -> u64 {
    memory.load_u64(vm + field.offset())
}

/// Writes one task field of the VM at `vm`.
pub(super) fn store(memory: &mut impl Memory, vm: u32, field: Field, value: u64) {
    memory.store_u64(vm + field.offset(), value);
}

/// Reads the address in one reference-valued task field, 0 when it is empty.
pub(super) fn load_address(memory: &impl Memory, vm: u32, field: Field) -> u32 {
    load(memory, vm, field) as u32
}

/// Writes the address `address` as a reference into one task field, 0 meaning empty.
pub(super) fn store_address(memory: &mut impl Memory, vm: u32, field: Field, address: u32) {
    store(memory, vm, field, reference(address));
}

/// Returns the internal reference of the heap object at `address`, or 0 for address 0.
///
/// Frames and tasks are referenced with kind `KIND_FRAME`, which marks internal objects the
/// collector traces like any heap reference.
pub(super) fn reference(address: u32) -> u64 {
    if address == 0 {
        return 0;
    }
    exs_value::ValueRef::from_parts(exs_value::layout::KIND_FRAME, 0, address)
        .map_or(0, exs_value::ValueRef::raw)
}
