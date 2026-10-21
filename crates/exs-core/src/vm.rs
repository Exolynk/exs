//! Typed access to the VM state block through a [`Memory`].

use core::mem::offset_of;

use exs_layout::VmState;

use crate::memory::Memory;

/// Address of one VM state block.
#[derive(Clone, Copy)]
pub(crate) struct Vm(pub(crate) u32);

/// Defines a getter and a setter for one `u32` field of [`VmState`].
macro_rules! vm_field {
    ($get:ident, $set:ident, $($field:tt)+) => {
        #[doc = concat!("Reads `VmState::", stringify!($($field)+), "`.")]
        pub(crate) fn $get(self, memory: &impl Memory) -> u32 {
            memory.load_u32(self.0 + offset_of!(VmState, $($field)+) as u32)
        }

        #[doc = concat!("Writes `VmState::", stringify!($($field)+), "`.")]
        pub(crate) fn $set(self, memory: &mut impl Memory, value: u32) {
            memory.store_u32(self.0 + offset_of!(VmState, $($field)+) as u32, value);
        }
    };
}

impl Vm {
    vm_field!(heap_start, set_heap_start, heap.heap_start);
    vm_field!(heap_end, set_heap_end, heap.heap_end);
    vm_field!(free_runs, set_free_runs, heap.free_runs);
    vm_field!(
        allocated_since_collection,
        set_allocated_since_collection,
        heap.allocated_since_collection
    );
    vm_field!(
        collection_threshold,
        set_collection_threshold,
        heap.collection_threshold
    );
    vm_field!(mark_stack, set_mark_stack, heap.mark_stack);
    vm_field!(
        mark_stack_capacity,
        set_mark_stack_capacity,
        heap.mark_stack_capacity
    );
    vm_field!(flags, set_flags, heap.flags);
    vm_field!(collections, set_collections, heap.collections);
    vm_field!(root_ranges, set_root_ranges, roots.ranges);
    vm_field!(root_range_count, set_root_range_count, roots.range_count);
    vm_field!(root_stack_end, set_root_stack_end, root_stack.range.end);
    vm_field!(root_stack_limit, set_root_stack_limit, root_stack.limit);
    vm_field!(program_types, set_program_types, program.types);
    vm_field!(
        program_type_count,
        set_program_type_count,
        program.type_count
    );
    vm_field!(root_shape, set_root_shape, program.root_shape);

    /// Pushes one raw reference onto the root stack and returns its slot address, or `None`
    /// when the stack is full.
    ///
    /// Core functions root intermediate objects this way while they allocate more; they must
    /// restore the stack with [`Vm::pop_roots`] before returning.
    pub(crate) fn push_root(self, memory: &mut impl Memory, raw: u64) -> Option<u32> {
        let slot = self.root_stack_end(memory);
        let end = slot.checked_add(8)?;
        if end > self.root_stack_limit(memory) {
            return None;
        }
        memory.store_u64(slot, raw);
        self.set_root_stack_end(memory, end);
        Some(slot)
    }

    /// Pops every root pushed since the one at `slot`, inclusive.
    pub(crate) fn pop_roots(self, memory: &mut impl Memory, slot: u32) {
        self.set_root_stack_end(memory, slot);
    }

    /// Returns the address of the free-list head of one size class.
    fn free_head_address(self, class: usize) -> u32 {
        self.0 + offset_of!(VmState, heap.free_heads) as u32 + class as u32 * 4
    }

    /// Reads the free-list head of one size class.
    pub(crate) fn free_head(self, memory: &impl Memory, class: usize) -> u32 {
        memory.load_u32(self.free_head_address(class))
    }

    /// Writes the free-list head of one size class.
    pub(crate) fn set_free_head(self, memory: &mut impl Memory, class: usize, value: u32) {
        memory.store_u32(self.free_head_address(class), value);
    }
}

/// Points the VM state block at `vm` to `count` [`exs_layout::RootRange`]s stored at `ranges`.
///
/// Generated code writes these fields directly; this is the same update for Rust callers.
pub fn set_root_ranges(memory: &mut impl Memory, vm: u32, ranges: u32, count: u32) {
    let vm = Vm(vm);
    vm.set_root_ranges(memory, ranges);
    vm.set_root_range_count(memory, count);
}

/// Replaces the heap option bits of the VM state block at `vm`.
pub fn set_heap_flags(memory: &mut impl Memory, vm: u32, flags: u32) {
    Vm(vm).set_flags(memory, flags);
}

/// Points the VM state block at `vm` to the program's type table of `count` entries.
///
/// Generated code writes these fields directly; this is the same update for Rust callers.
pub fn set_program_types(memory: &mut impl Memory, vm: u32, types: u32, count: u32) {
    let vm = Vm(vm);
    vm.set_program_types(memory, types);
    vm.set_program_type_count(memory, count);
}

/// Returns the number of collections the VM state block at `vm` has completed.
pub fn collection_count(memory: &impl Memory, vm: u32) -> u32 {
    Vm(vm).collections(memory)
}
