//! Page runs carved from linear memory, and the list of free runs.

use core::mem::offset_of;

use exs_layout::{PAGE_SIZE, PageHeader, PageKind};

use crate::memory::Memory;
use crate::vm::Vm;

/// Byte offset of [`PageHeader::kind`].
const KIND: u32 = offset_of!(PageHeader, kind) as u32;
/// Byte offset of [`PageHeader::run_pages`].
const RUN_PAGES: u32 = offset_of!(PageHeader, run_pages) as u32;
/// Byte offset of [`PageHeader::detail`].
const DETAIL: u32 = offset_of!(PageHeader, detail) as u32;
/// Byte offset of [`PageHeader::live`].
const LIVE: u32 = offset_of!(PageHeader, live) as u32;

/// The decoded header of one page run.
#[derive(Clone, Copy)]
pub(crate) struct Run {
    /// Use of the run.
    pub(crate) kind: PageKind,
    /// Number of pages in the run.
    pub(crate) pages: u32,
    /// Size class of class pages, next free run of free runs.
    pub(crate) detail: u32,
}

/// Reads the header of the run starting at `page`, or `None` when it is corrupt.
pub(crate) fn read_run(memory: &impl Memory, page: u32) -> Option<Run> {
    Some(Run {
        kind: PageKind::from_u32(memory.load_u32(page + KIND))?,
        pages: memory.load_u32(page + RUN_PAGES),
        detail: memory.load_u32(page + DETAIL),
    })
}

/// Writes the header of the run starting at `page`.
pub(crate) fn write_run(memory: &mut impl Memory, page: u32, run: Run, live: u32) {
    memory.store_u32(page + KIND, run.kind as u32);
    memory.store_u32(page + RUN_PAGES, run.pages);
    memory.store_u32(page + DETAIL, run.detail);
    memory.store_u32(page + LIVE, live);
}

/// Returns the byte size of `pages` pages, or `None` when it exceeds the address space.
pub(crate) fn pages_bytes(pages: u32) -> Option<u32> {
    pages.checked_mul(PAGE_SIZE)
}

/// Takes a run of `count` pages from the free runs (first fit) or from fresh memory.
///
/// Returns `None` when linear memory cannot grow far enough.
pub(crate) fn take_pages(memory: &mut impl Memory, vm: Vm, count: u32) -> Option<u32> {
    let mut previous = 0;
    let mut run_page = vm.free_runs(memory);
    while run_page != 0 {
        let run = read_run(memory, run_page)?;
        if run.pages >= count {
            let remainder = run.pages - count;
            let replacement = if remainder == 0 {
                run.detail
            } else {
                let tail = run_page + pages_bytes(count)?;
                write_run(
                    memory,
                    tail,
                    Run {
                        kind: PageKind::Free,
                        pages: remainder,
                        detail: run.detail,
                    },
                    0,
                );
                tail
            };
            if previous == 0 {
                vm.set_free_runs(memory, replacement);
            } else {
                memory.store_u32(previous + DETAIL, replacement);
            }
            return Some(run_page);
        }
        previous = run_page;
        run_page = run.detail;
    }
    carve_pages(memory, vm, count)
}

/// Appends a run of `count` pages at the end of the heap, growing memory when needed.
fn carve_pages(memory: &mut impl Memory, vm: Vm, count: u32) -> Option<u32> {
    let start = vm.heap_end(memory);
    let end = start.checked_add(pages_bytes(count)?)?;
    let size = memory.size();
    if u64::from(end) > size {
        let missing = u64::from(end) - size;
        let pages = u32::try_from(missing.div_ceil(u64::from(PAGE_SIZE))).ok()?;
        if !memory.grow(pages) {
            return None;
        }
    }
    vm.set_heap_end(memory, end);
    Some(start)
}

/// Returns one run of `pages` pages to the free runs.
pub(crate) fn release_run(memory: &mut impl Memory, vm: Vm, page: u32, pages: u32) {
    let next = vm.free_runs(memory);
    write_run(
        memory,
        page,
        Run {
            kind: PageKind::Free,
            pages,
            detail: next,
        },
        0,
    );
    vm.set_free_runs(memory, page);
}
