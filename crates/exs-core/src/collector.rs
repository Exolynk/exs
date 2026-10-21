//! The non-moving mark-and-sweep collector.
//!
//! Marking starts from every `ValueRef` slot in the VM's root ranges and the root shape of
//! untyped Objects, and follows each object
//! kind's [`exs_layout::trace_layout`]. Objects outside the heap, such as static literal
//! Strings, and objects with [`GC_STATIC`] are never marked. Sweeping walks every page run once:
//! it rebuilds the size-class free lists, returns empty class pages and unmarked large objects
//! to the free runs and merges adjacent free pages. Objects never move, so addresses held in
//! Wasm locals stay valid across a collection.

use core::mem::{offset_of, size_of};

use exs_layout::{
    GC_MARK, GC_STATIC, ObjectKind, PAGE_HEADER_SIZE, PAGE_SIZE, PageKind, RootRange, SIZE_CLASSES,
    ValueSlots, trace_layout,
};
use exs_value::ValueRef;

use crate::allocator::{
    INITIAL_COLLECTION_THRESHOLD, LARGE_CLASS, page_of, used_cells, write_free_cell,
};
use crate::memory::Memory;
use crate::pages::{Run, pages_bytes, read_run, release_run, take_pages, write_run};
use crate::vm::Vm;

/// Byte offset of the GC bits in an object header.
const GC_OFFSET: u32 = 1;

/// Bytes of one mark-stack entry, an object address.
const ENTRY_SIZE: u32 = 4;

/// Collects every heap object unreachable from the roots of the VM state block at `vm`.
pub fn collect(memory: &mut impl Memory, vm: u32) {
    let vm = Vm(vm);
    let mut marker = Marker::open(memory, vm);
    marker.mark_roots(memory);
    marker.drain(memory);
    while marker.overflowed {
        marker.overflowed = false;
        marker.rescan(memory);
        marker.drain(memory);
    }
    marker.close(memory);
    sweep(memory, vm);
}

/// Marking state of one collection.
struct Marker {
    /// The VM being collected.
    vm: Vm,
    /// First heap address.
    heap_start: u32,
    /// Address just past the last heap page.
    heap_end: u32,
    /// Address of the first mark-stack entry, 0 when no stack storage exists.
    entries: u32,
    /// Capacity of the mark stack in entries.
    capacity: u32,
    /// Number of entries in use.
    len: u32,
    /// Whether an object was marked without being pushed because the stack was full.
    overflowed: bool,
}

impl Marker {
    /// Prepares marking, reusing or creating the VM's mark-stack storage.
    fn open(memory: &mut impl Memory, vm: Vm) -> Self {
        let mut marker = Self {
            vm,
            heap_start: vm.heap_start(memory),
            heap_end: vm.heap_end(memory),
            entries: 0,
            capacity: 0,
            len: 0,
            overflowed: false,
        };
        if vm.mark_stack(memory) == 0
            && let Some(page) = take_pages(memory, vm, 1)
        {
            write_run(memory, page, system_run(1), 0);
            vm.set_mark_stack(memory, page);
            vm.set_mark_stack_capacity(memory, stack_capacity(1));
            // The new run lies inside the heap, which the sweep below must walk too.
            marker.heap_end = vm.heap_end(memory);
        }
        let page = vm.mark_stack(memory);
        if page != 0 {
            marker.entries = page + PAGE_HEADER_SIZE;
            marker.capacity = vm.mark_stack_capacity(memory);
        }
        marker
    }

    /// Records the final mark-stack storage in the VM state block.
    fn close(&self, memory: &mut impl Memory) {
        if self.entries != 0 {
            self.vm
                .set_mark_stack(memory, self.entries - PAGE_HEADER_SIZE);
            self.vm.set_mark_stack_capacity(memory, self.capacity);
        }
    }

    /// Marks every object referenced from the VM's root ranges and the shape tree.
    fn mark_roots(&mut self, memory: &mut impl Memory) {
        let root_shape = self.vm.root_shape(memory);
        if root_shape != 0 {
            self.mark_address(memory, root_shape);
        }
        let ranges = self.vm.root_ranges(memory);
        let count = self.vm.root_range_count(memory);
        let range_size = size_of::<RootRange>() as u32;
        for index in 0..count {
            let range = ranges + index * range_size;
            let start = memory.load_u32(range + offset_of!(RootRange, start) as u32);
            let end = memory.load_u32(range + offset_of!(RootRange, end) as u32);
            let mut slot = start;
            while slot.checked_add(8).is_some_and(|next| next <= end) {
                let raw = memory.load_u64(slot);
                self.mark_value(memory, raw);
                slot += 8;
            }
        }
    }

    /// Traces every pushed object until the mark stack is empty.
    fn drain(&mut self, memory: &mut impl Memory) {
        while self.len > 0 {
            self.len -= 1;
            let object = memory.load_u32(self.entries + self.len * ENTRY_SIZE);
            self.trace(memory, object);
        }
    }

    /// Traces every marked heap object again after the mark stack overflowed.
    fn rescan(&mut self, memory: &mut impl Memory) {
        let mut page = self.heap_start;
        while page < self.heap_end {
            let Some(run) = read_run(memory, page) else {
                return;
            };
            match run.kind {
                PageKind::Class => {
                    let class = run.detail as usize;
                    for index in 0..used_cells(memory, self.vm, page, class) {
                        let cell = page
                            + PAGE_HEADER_SIZE
                            + index * crate::allocator::class_size(class).get();
                        if is_marked(memory, cell) {
                            self.trace(memory, cell);
                        }
                    }
                }
                PageKind::Large => {
                    let object = page + PAGE_HEADER_SIZE;
                    if is_marked(memory, object) {
                        self.trace(memory, object);
                    }
                }
                PageKind::Free | PageKind::System => {}
            }
            page = next_run(page, run.pages);
        }
    }

    /// Marks the object behind one raw `ValueRef`, ignoring immediates and empty slots.
    fn mark_value(&mut self, memory: &mut impl Memory, raw: u64) {
        // SAFETY: the reference is only decoded to inspect its bits, never dereferenced.
        let Some(value) = (unsafe { ValueRef::from_raw(raw) }) else {
            return;
        };
        if value.immediate().is_none() {
            self.mark_address(memory, value.payload());
        }
    }

    /// Marks one heap object and queues it for tracing.
    fn mark_address(&mut self, memory: &mut impl Memory, object: u32) {
        if object < self.heap_start || object >= self.heap_end || !object.is_multiple_of(8) {
            return;
        }
        let gc = memory.load_u8(object + GC_OFFSET);
        if gc & (GC_MARK | GC_STATIC) != 0 || memory.load_u8(object) == ObjectKind::Free as u8 {
            return;
        }
        memory.store_u8(object + GC_OFFSET, gc | GC_MARK);
        if self.len == self.capacity && !self.grow(memory) {
            self.overflowed = true;
            return;
        }
        memory.store_u32(self.entries + self.len * ENTRY_SIZE, object);
        self.len += 1;
    }

    /// Doubles the mark stack, returning `false` when no memory is left for it.
    fn grow(&mut self, memory: &mut impl Memory) -> bool {
        let old_page = if self.entries == 0 {
            0
        } else {
            self.entries - PAGE_HEADER_SIZE
        };
        let old_pages = if old_page == 0 {
            0
        } else {
            read_run(memory, old_page).map_or(0, |run| run.pages)
        };
        let pages = old_pages.max(1) * 2;
        let Some(page) = take_pages(memory, self.vm, pages) else {
            return false;
        };
        write_run(memory, page, system_run(pages), 0);
        self.heap_end = self.vm.heap_end(memory);
        let entries = page + PAGE_HEADER_SIZE;
        if self.len > 0 {
            memory.copy(entries, self.entries, self.len * ENTRY_SIZE);
        }
        if old_page != 0 {
            release_run(memory, self.vm, old_page, old_pages);
        }
        self.entries = entries;
        self.capacity = stack_capacity(pages);
        true
    }

    /// Marks every object one marked object references.
    fn trace(&mut self, memory: &mut impl Memory, object: u32) {
        let Some(kind) = ObjectKind::from_byte(memory.load_u8(object)) else {
            return;
        };
        let layout = trace_layout(kind);
        match layout.values {
            ValueSlots::None => {}
            ValueSlots::Fixed(offsets) => {
                for offset in offsets {
                    let raw = memory.load_u64(object + offset);
                    self.mark_value(memory, raw);
                }
            }
            ValueSlots::Counted { count, first } => {
                // A count beyond the object's own storage is clamped, so a corrupt count can
                // never make the collector read past the object.
                let capacity = object_capacity(memory, object).saturating_sub(first) / 8;
                let count = memory.load_u32(object + count).min(capacity);
                for index in 0..count {
                    let raw = memory.load_u64(object + first + index * 8);
                    self.mark_value(memory, raw);
                }
            }
        }
        for offset in layout.addresses {
            let address = memory.load_u32(object + offset);
            if address != 0 {
                self.mark_address(memory, address);
            }
        }
    }
}

/// Returns the header of a collector-owned run of `pages` pages.
const fn system_run(pages: u32) -> Run {
    Run {
        kind: PageKind::System,
        pages,
        detail: 0,
    }
}

/// Returns the mark-stack entries that fit a system run of `pages` pages.
const fn stack_capacity(pages: u32) -> u32 {
    (pages * PAGE_SIZE - PAGE_HEADER_SIZE) / ENTRY_SIZE
}

/// Returns the address of the run following the run of `pages` pages at `page`.
fn next_run(page: u32, pages: u32) -> u32 {
    page.saturating_add(pages_bytes(pages.max(1)).unwrap_or(u32::MAX))
}

/// Returns whether one heap object is marked.
fn is_marked(memory: &impl Memory, object: u32) -> bool {
    memory.load_u8(object) != ObjectKind::Free as u8
        && memory.load_u8(object + GC_OFFSET) & GC_MARK != 0
}

/// Returns the byte size of the cell or page run holding one object.
fn object_capacity(memory: &impl Memory, object: u32) -> u32 {
    let class = memory.load_u16(object + 2);
    if class == LARGE_CLASS {
        let page = object - PAGE_HEADER_SIZE;
        read_run(memory, page)
            .and_then(|run| pages_bytes(run.pages))
            .map_or(0, |bytes| bytes - PAGE_HEADER_SIZE)
    } else {
        SIZE_CLASSES.get(usize::from(class)).copied().unwrap_or(0)
    }
}

/// Frees every unmarked object, clears the marks of the others and resets the threshold.
fn sweep(memory: &mut impl Memory, vm: Vm) {
    for class in 0..SIZE_CLASSES.len() {
        vm.set_free_head(memory, class, 0);
    }
    vm.set_free_runs(memory, 0);
    let heap_end = vm.heap_end(memory);
    let mut free = FreePages::default();
    let mut live_bytes: u32 = 0;
    let mut page = vm.heap_start(memory);
    while page < heap_end {
        let Some(run) = read_run(memory, page) else {
            break;
        };
        match run.kind {
            PageKind::Class => {
                let live = sweep_class_page(memory, vm, page, run.detail as usize);
                if live == 0 {
                    free.add(memory, vm, page, 1);
                } else {
                    free.flush(memory, vm);
                    write_run(memory, page, run, live);
                    live_bytes = live_bytes.saturating_add(
                        live * crate::allocator::class_size(run.detail as usize).get(),
                    );
                }
            }
            PageKind::Large => {
                let object = page + PAGE_HEADER_SIZE;
                if is_marked(memory, object) {
                    clear_mark(memory, object);
                    free.flush(memory, vm);
                    live_bytes = live_bytes.saturating_add(pages_bytes(run.pages).unwrap_or(0));
                } else {
                    free.add(memory, vm, page, run.pages);
                }
            }
            PageKind::Free => free.add(memory, vm, page, run.pages),
            PageKind::System => free.flush(memory, vm),
        }
        page = next_run(page, run.pages);
    }
    free.flush(memory, vm);
    vm.set_collection_threshold(
        memory,
        live_bytes
            .saturating_mul(2)
            .max(INITIAL_COLLECTION_THRESHOLD),
    );
    vm.set_allocated_since_collection(memory, 0);
    let collections = vm.collections(memory);
    vm.set_collections(memory, collections.wrapping_add(1));
}

/// Clears the marks of one class page's live cells and links its other cells into the free
/// list. Returns the number of live cells; a page without any is left for the free runs.
fn sweep_class_page(memory: &mut impl Memory, vm: Vm, page: u32, class: usize) -> u32 {
    let size = crate::allocator::class_size(class).get();
    let cell_at = |index: u32| page + PAGE_HEADER_SIZE + index * size;
    let cells = used_cells(memory, vm, page, class);
    let live = (0..cells)
        .filter(|index| is_marked(memory, cell_at(*index)))
        .count() as u32;
    if live == 0 {
        // The page returns to the free runs, so it can no longer be its class's current page.
        let cursor = vm.class_cursor(memory, class);
        if cursor != 0 && page_of(cursor) == page {
            vm.set_class_cursor(memory, class, 0);
        }
        return 0;
    }
    let mut head = vm.free_head(memory, class);
    for index in 0..cells {
        let cell = cell_at(index);
        if is_marked(memory, cell) {
            clear_mark(memory, cell);
        } else {
            write_free_cell(memory, cell, class, head);
            head = cell;
        }
    }
    vm.set_free_head(memory, class, head);
    live
}

/// Clears the mark bit of one object.
fn clear_mark(memory: &mut impl Memory, object: u32) {
    let gc = memory.load_u8(object + GC_OFFSET);
    memory.store_u8(object + GC_OFFSET, gc & !GC_MARK);
}

/// Adjacent free pages found while sweeping, merged into one run.
#[derive(Default)]
struct FreePages {
    /// First page of the pending run, 0 when none.
    start: u32,
    /// Pages in the pending run.
    pages: u32,
}

impl FreePages {
    /// Adds the free run at `page`, merging it with the pending run when adjacent.
    fn add(&mut self, memory: &mut impl Memory, vm: Vm, page: u32, pages: u32) {
        if self.pages > 0 && next_run(self.start, self.pages) == page {
            self.pages += pages;
            return;
        }
        self.flush(memory, vm);
        self.start = page;
        self.pages = pages;
    }

    /// Releases the pending run to the VM's free runs.
    fn flush(&mut self, memory: &mut impl Memory, vm: Vm) {
        if self.pages > 0 {
            release_run(memory, vm, self.start, self.pages);
            self.pages = 0;
        }
    }
}
