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
    /// [`TaskState::host_call`].
    HostCall,
    /// [`TaskState::next_host_call`].
    NextHostCall,
    /// [`TaskState::cancelled_call`].
    CancelledCall,
}

impl Field {
    /// Returns the byte offset of the field in the VM state block.
    const fn offset(self) -> u32 {
        match self {
            Self::Frame => task_offset!(frame),
            Self::Ready => task_offset!(ready),
            Self::Completed => task_offset!(completed),
            Self::HostCall => task_offset!(host_call),
            Self::NextHostCall => task_offset!(next_host_call),
            Self::CancelledCall => task_offset!(cancelled_call),
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
