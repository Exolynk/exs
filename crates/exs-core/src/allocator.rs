//! The size-class page allocator.
//!
//! Objects up to [`exs_layout::MAX_CLASS_SIZE`] bytes live in cells of single pages devoted to
//! one size class; each class keeps a free list of cells in the VM state block. Larger objects
//! get a run of whole pages. New memory is only carved or grown when the free lists and free
//! runs are exhausted, after a collection once the allocation volume reaches its threshold.

use core::mem::{offset_of, size_of};

use exs_layout::{
    FreeCell, ObjectKind, PAGE_HEADER_SIZE, PAGE_SIZE, PageKind, SIZE_CLASSES,
    VM_FLAG_COLLECT_EVERY_ALLOCATION, VmState, class_for_size,
};

use crate::collector::collect;
use crate::memory::Memory;
use crate::pages::{Run, pages_bytes, take_pages, write_run};
use crate::vm::Vm;

/// Allocation volume after which the first collection runs.
pub(crate) const INITIAL_COLLECTION_THRESHOLD: u32 = 1 << 20;

/// Number of cells one refill links into a class's free list.
const LINK_CHUNK: u32 = 64;

/// Header class value of objects that occupy their own page run.
pub(crate) const LARGE_CLASS: u16 = u16::MAX;

/// Byte offset of [`FreeCell::next`].
pub(crate) const FREE_NEXT: u32 = offset_of!(FreeCell, next) as u32;

/// Initializes the VM state block at `vm` with an empty heap starting at the first page
/// boundary at or after `heap_start`.
pub fn init_heap(memory: &mut impl Memory, vm: u32, heap_start: u32) {
    memory.fill(vm, size_of::<VmState>() as u32, 0);
    let vm = Vm(vm);
    let start = heap_start.next_multiple_of(PAGE_SIZE);
    vm.set_heap_start(memory, start);
    vm.set_heap_end(memory, start);
    vm.set_collection_threshold(memory, INITIAL_COLLECTION_THRESHOLD);
}

/// Allocates one zeroed object of `size` bytes and writes its header with `kind`.
///
/// `size` includes the 8-byte header. The returned address is 8-byte aligned; header class and
/// GC bits are set, the header word is 0. Returns `None` when memory is exhausted. Every value
/// the caller still needs must be reachable from the VM's roots, because this may collect.
pub fn allocate(memory: &mut impl Memory, vm: u32, size: u32, kind: ObjectKind) -> Option<u32> {
    let vm = Vm(vm);
    if vm.flags(memory) & VM_FLAG_COLLECT_EVERY_ALLOCATION != 0 {
        collect(memory, vm.0);
    }
    let size = size.max(size_of::<FreeCell>() as u32).next_multiple_of(8);
    let (object, class, bytes) = match class_for_size(size) {
        Some(class) => (
            allocate_cell(memory, vm, class)?,
            class as u16,
            SIZE_CLASSES[class],
        ),
        None => {
            let (object, pages) = allocate_large(memory, vm, size)?;
            (object, LARGE_CLASS, pages_bytes(pages)?)
        }
    };
    let allocated = vm.allocated_since_collection(memory);
    vm.set_allocated_since_collection(memory, allocated.saturating_add(bytes));
    memory.fill(object, size, 0);
    memory.store_u8(object, kind as u8);
    memory.store_u16(object + 2, class);
    Some(object)
}

/// Pops one cell of `class`, refilling the class from a fresh page when its list is empty.
fn allocate_cell(memory: &mut impl Memory, vm: Vm, class: usize) -> Option<u32> {
    if vm.free_head(memory, class) == 0 && over_threshold(memory, vm) {
        collect(memory, vm.0);
    }
    if vm.free_head(memory, class) == 0 {
        refill_class(memory, vm, class)?;
    }
    let cell = vm.free_head(memory, class);
    vm.set_free_head(memory, class, memory.load_u32(cell + FREE_NEXT));
    Some(cell)
}

/// Links the next [`LINK_CHUNK`] never-used cells of `class` into its free list, taking a new
/// page for the class when its current page has none left.
///
/// Linking a whole page at once would make the first allocation of every size class pay for
/// thousands of cells.
fn refill_class(memory: &mut impl Memory, vm: Vm, class: usize) -> Option<()> {
    let mut cursor = vm.class_cursor(memory, class);
    if cursor == 0 {
        let page = take_pages(memory, vm, 1)?;
        write_run(
            memory,
            page,
            Run {
                kind: PageKind::Class,
                pages: 1,
                detail: class as u32,
            },
            0,
        );
        cursor = page + PAGE_HEADER_SIZE;
    }
    let size = SIZE_CLASSES[class];
    let page_end = page_of(cursor) + PAGE_HEADER_SIZE + cells_per_page(class) * size;
    let count = ((page_end - cursor) / size).min(LINK_CHUNK);
    let end = cursor + count * size;
    vm.set_class_cursor(memory, class, if end == page_end { 0 } else { end });
    let mut next = vm.free_head(memory, class);
    // Linking from the end keeps the list in address order.
    for index in (0..count).rev() {
        let cell = cursor + index * size;
        write_free_cell(memory, cell, class, next);
        next = cell;
    }
    vm.set_free_head(memory, class, next);
    Some(())
}

/// Returns the first address of the page containing `address`.
pub(crate) const fn page_of(address: u32) -> u32 {
    address & !(PAGE_SIZE - 1)
}

/// Returns how many cells of the class page at `page` may hold objects: all of them, unless
/// the page is its class's partly linked page, whose cells from the cursor on are unused.
pub(crate) fn used_cells(memory: &impl Memory, vm: Vm, page: u32, class: usize) -> u32 {
    let cursor = vm.class_cursor(memory, class);
    if cursor != 0 && page_of(cursor) == page {
        (cursor - page - PAGE_HEADER_SIZE) / SIZE_CLASSES[class]
    } else {
        cells_per_page(class)
    }
}

/// Allocates a page run for one object of `size` bytes and returns it with its page count.
fn allocate_large(memory: &mut impl Memory, vm: Vm, size: u32) -> Option<(u32, u32)> {
    if over_threshold(memory, vm) {
        collect(memory, vm.0);
    }
    let pages = size.checked_add(PAGE_HEADER_SIZE)?.div_ceil(PAGE_SIZE);
    let page = take_pages(memory, vm, pages)?;
    write_run(
        memory,
        page,
        Run {
            kind: PageKind::Large,
            pages,
            detail: 0,
        },
        0,
    );
    Some((page + PAGE_HEADER_SIZE, pages))
}

/// Returns whether enough was allocated since the last collection to collect again.
fn over_threshold(memory: &impl Memory, vm: Vm) -> bool {
    vm.allocated_since_collection(memory) >= vm.collection_threshold(memory)
}

/// Returns the number of cells of `class` in one page.
pub(crate) const fn cells_per_page(class: usize) -> u32 {
    (PAGE_SIZE - PAGE_HEADER_SIZE) / SIZE_CLASSES[class]
}

/// Writes one free cell of `class` linking to `next`.
pub(crate) fn write_free_cell(memory: &mut impl Memory, cell: u32, class: usize, next: u32) {
    memory.store_u8(cell, ObjectKind::Free as u8);
    memory.store_u8(cell + 1, 0);
    memory.store_u16(cell + 2, class as u16);
    memory.store_u32(cell + FREE_NEXT, next);
}
