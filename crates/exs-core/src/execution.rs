//! Start-up of one execution: heap, root stack and runner buffers.

use core::mem::offset_of;

use exs_layout::{PAGE_HEADER_SIZE, PageKind, VmState};

use crate::allocator::init_heap;
use crate::memory::Memory;
use crate::pages::{Run, pages_bytes, release_run, take_pages, write_run};

/// Byte offset of one `VmState` field.
macro_rules! vm_offset {
    ($($field:tt)+) => {
        offset_of!(VmState, $($field)+) as u32
    };
}

/// Initializes the VM state block at `vm`: an empty heap from `heap_start` and a root stack of
/// `root_stack_pages` pages. Returns `None` when memory cannot hold the root stack.
///
/// Does nothing when the block is already initialized, so every runner entry may call it.
pub fn init_execution(
    memory: &mut impl Memory,
    vm: u32,
    heap_start: u32,
    root_stack_pages: u32,
) -> Option<()> {
    if memory.load_u32(vm + vm_offset!(boundary.initialized)) != 0 {
        return Some(());
    }
    init_heap(memory, vm, heap_start);
    let stack = take_system_pages(memory, vm, root_stack_pages)?;
    let start = stack + PAGE_HEADER_SIZE;
    memory.store_u32(vm + vm_offset!(root_stack.range.start), start);
    let state = crate::vm::Vm(vm);
    state.set_root_stack_end(memory, start);
    state.set_root_stack_limit(memory, stack + pages_bytes(root_stack_pages)?);
    memory.store_u32(
        vm + vm_offset!(roots.ranges),
        vm + vm_offset!(root_stack.range),
    );
    memory.store_u32(vm + vm_offset!(roots.range_count), 1);
    memory.store_u32(vm + vm_offset!(boundary.initialized), 1);
    Some(())
}

/// Takes a run of `pages` pages that the collector keeps but never scans.
fn take_system_pages(memory: &mut impl Memory, vm: u32, pages: u32) -> Option<u32> {
    let vm_ref = crate::vm::Vm(vm);
    let page = take_pages(memory, vm_ref, pages)?;
    write_run(
        memory,
        page,
        Run {
            kind: PageKind::System,
            pages,
            detail: 0,
        },
        0,
    );
    Some(page)
}

/// Returns storage for `length` input bytes the runner writes, reusing earlier storage.
///
/// Returns the address of the first byte, or `None` when memory is exhausted.
pub fn reserve_input(memory: &mut impl Memory, vm: u32, length: u32) -> Option<u32> {
    let address = reserve(
        memory,
        vm,
        vm_offset!(boundary.input),
        vm_offset!(boundary.input_capacity),
        length,
    )?;
    memory.store_u32(vm + vm_offset!(boundary.input_len), length);
    Some(address)
}

/// Returns storage for a result of `length` bytes, reusing earlier storage.
pub(crate) fn reserve_result(memory: &mut impl Memory, vm: u32, length: u32) -> Option<u32> {
    let address = reserve(
        memory,
        vm,
        vm_offset!(boundary.result),
        vm_offset!(boundary.result_capacity),
        length,
    )?;
    memory.store_u32(vm + vm_offset!(boundary.result_len), length);
    Some(address)
}

/// Returns the address and length of the current input.
pub(crate) fn input(memory: &impl Memory, vm: u32) -> (u32, u32) {
    (
        memory.load_u32(vm + vm_offset!(boundary.input)),
        memory.load_u32(vm + vm_offset!(boundary.input_len)),
    )
}

/// Returns the address and length of the current result.
pub fn result(memory: &impl Memory, vm: u32) -> (u32, u32) {
    (
        memory.load_u32(vm + vm_offset!(boundary.result)),
        memory.load_u32(vm + vm_offset!(boundary.result_len)),
    )
}

/// Ensures the boundary buffer described by two VM fields holds `length` bytes.
fn reserve(
    memory: &mut impl Memory,
    vm: u32,
    address_field: u32,
    capacity_field: u32,
    length: u32,
) -> Option<u32> {
    let address = memory.load_u32(vm + address_field);
    let capacity = memory.load_u32(vm + capacity_field);
    if address != 0 && length <= capacity {
        return Some(address);
    }
    if address != 0 {
        let page = address - PAGE_HEADER_SIZE;
        let pages = (capacity + PAGE_HEADER_SIZE) / exs_layout::PAGE_SIZE;
        release_run(memory, crate::vm::Vm(vm), page, pages);
    }
    let pages = length
        .checked_add(PAGE_HEADER_SIZE)?
        .div_ceil(exs_layout::PAGE_SIZE)
        .max(1);
    let page = take_system_pages(memory, vm, pages)?;
    let address = page + PAGE_HEADER_SIZE;
    memory.store_u32(vm + address_field, address);
    memory.store_u32(vm + capacity_field, pages_bytes(pages)? - PAGE_HEADER_SIZE);
    Some(address)
}
