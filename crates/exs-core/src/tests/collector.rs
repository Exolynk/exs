//! Collector tests.

use std::collections::HashSet;
use std::vec::Vec;

use exs_layout::ValueRef;
use exs_layout::{
    GC_STATIC, ObjectKind, PAGE_HEADER_SIZE, PAGE_SIZE, VM_FLAG_COLLECT_EVERY_ALLOCATION,
};

use super::heap::{Heap, ROOT_SLOT_COUNT, VM, reference};
use crate::{Memory, collection_count, set_heap_flags};

/// Keeps objects reachable through Lists, Buffers and Cells and frees all others.
#[test]
fn frees_unreachable_objects_and_keeps_reachable_ones() {
    let mut heap = Heap::new(64);
    let text = heap.string(5);
    let inner = heap.cell(reference(text));
    let items = heap.buffer(&[reference(inner), ValueRef::NONE.raw(), 0]);
    let list = heap.list(items, 2);
    let garbage_text = heap.string(40);
    let garbage = heap.cell(reference(garbage_text));
    heap.set_root(0, reference(list));
    heap.collect();
    for object in [list, items, inner, text] {
        assert!(heap.is_live(object), "reachable object {object} was freed");
        assert_eq!(heap.memory.load_u8(object + 1), 0, "mark bit survived");
    }
    assert!(!heap.is_live(garbage));
    assert!(!heap.is_live(garbage_text));
    assert_eq!(collection_count(&heap.memory, VM), 1);
}

/// Hands freed cells to later allocations of the same class.
#[test]
fn reuses_freed_cells() {
    let mut heap = Heap::new(64);
    let freed: HashSet<u32> = (0..10).map(|_| heap.cell(0)).collect();
    heap.collect();
    let reused: HashSet<u32> = (0..10).map(|_| heap.cell(0)).collect();
    assert_eq!(freed, reused);
}

/// Frees unreachable cycles and keeps reachable ones.
#[test]
fn handles_reference_cycles() {
    let mut heap = Heap::new(64);
    let first = heap.cell(0);
    let second = heap.cell(reference(first));
    heap.memory.store_u64(first + 8, reference(second));
    let kept_first = heap.cell(0);
    let kept_second = heap.cell(reference(kept_first));
    heap.memory
        .store_u64(kept_first + 8, reference(kept_second));
    heap.set_root(3, reference(kept_second));
    heap.collect();
    assert!(!heap.is_live(first) && !heap.is_live(second));
    assert!(heap.is_live(kept_first) && heap.is_live(kept_second));
}

/// Ignores immediates, empty slots and static objects outside the heap.
#[test]
fn ignores_immediates_and_static_objects() {
    let mut heap = Heap::new(64);
    // A static String below the heap, as a data segment would hold it.
    let static_text = 32_768;
    heap.memory.store_u8(static_text, ObjectKind::String as u8);
    heap.memory.store_u8(static_text + 1, GC_STATIC);
    let cell = heap.cell(reference(static_text));
    let small = ValueRef::small_int(7).expect("small Int").raw();
    heap.set_root(0, small);
    heap.set_root(1, ValueRef::TRUE.raw());
    heap.set_root(2, reference(cell));
    heap.collect();
    assert!(heap.is_live(cell));
    assert_eq!(heap.memory.load_u8(static_text + 1), GC_STATIC);
}

/// Returns empty pages and dead large objects to free runs that later allocations reuse.
#[test]
fn reuses_empty_pages_and_large_runs() {
    let mut heap = Heap::new(64);
    // The first collection takes one page for the mark stack.
    heap.collect();
    for _ in 0..2000 {
        heap.cell(0);
    }
    heap.allocate(2 * PAGE_SIZE, ObjectKind::Bytes);
    let carved = heap.heap_end();
    heap.collect();
    // One page of 16-byte cells and a three-page large run merge into four free pages, which
    // 2000 cells of the 128-byte class fill exactly (511 cells per page).
    for _ in 0..2000 {
        heap.string(100);
    }
    assert_eq!(heap.heap_end(), carved, "freed pages were not reused");
    heap.collect();
    heap.allocate(3 * PAGE_SIZE - 64, ObjectKind::Bytes);
    assert_eq!(heap.heap_end(), carved, "freed large run was not reused");
}

/// Keeps every reachable object of random graphs, also when collecting at every allocation.
#[test]
fn preserves_reachability_of_random_graphs() {
    for collect_every_allocation in [false, true] {
        let mut random = XorShift(0x9E37_79B9_7F4A_7C15);
        let mut heap = Heap::new(512);
        if collect_every_allocation {
            set_heap_flags(&mut heap.memory, VM, VM_FLAG_COLLECT_EVERY_ALLOCATION);
        }
        let mut objects = Vec::new();
        let mut edges = Vec::new();
        for round in 0..400 {
            // Only still-rooted objects are certainly alive and may be referenced.
            let width = (random.next() % 6) as usize;
            let rooted = objects.len().min(ROOT_SLOT_COUNT as usize - 1);
            let targets: Vec<usize> = (0..width)
                .filter(|_| rooted > 0)
                .map(|_| objects.len() - 1 - (random.next() as usize) % rooted)
                .collect();
            let values: Vec<u64> = targets
                .iter()
                .map(|target| reference(objects[*target]))
                .collect();
            // Root every fresh object first, so nothing it should keep can be collected while
            // the next ones are allocated.
            let object = heap.buffer(&values);
            heap.set_root(round % ROOT_SLOT_COUNT, reference(object));
            objects.push(object);
            edges.push(targets);
        }
        let roots: Vec<usize> = (0..objects.len())
            .filter(|index| objects.len() - index <= ROOT_SLOT_COUNT as usize)
            .collect();
        heap.collect();
        let mut reachable = HashSet::new();
        let mut stack = roots;
        while let Some(index) = stack.pop() {
            if reachable.insert(index) {
                stack.extend(edges[index].iter().copied());
            }
        }
        for (index, object) in objects.iter().enumerate() {
            if reachable.contains(&index) {
                assert!(heap.is_live(*object), "reachable object {index} was freed");
            }
        }
        // Collecting at every allocation reuses dead cells for later objects, so only a single
        // final collection leaves exactly the unreachable objects free.
        if !collect_every_allocation {
            let freed = (0..objects.len())
                .filter(|index| !heap.is_live(objects[*index]))
                .count();
            assert_eq!(freed, objects.len() - reachable.len());
        }
    }
}

/// Marks correctly when the mark stack cannot grow, by rescanning marked objects.
#[test]
fn survives_mark_stack_overflow() {
    // Wide fan-out needs a larger mark stack than the memory limit allows.
    // Cells, the fan-out Buffer and a one-page mark stack fit, a two-page mark stack does not.
    let mut heap = Heap::new(15);
    let mut leaves = Vec::new();
    for _ in 0..30_000 {
        leaves.push(heap.cell(0));
    }
    let values: Vec<u64> = leaves.iter().map(|leaf| reference(*leaf)).collect();
    let fan = heap.buffer(&values);
    heap.set_root(0, reference(fan));
    let unreachable = heap.cell(0);
    heap.collect();
    assert!(leaves.iter().all(|leaf| heap.is_live(*leaf)));
    assert!(!heap.is_live(unreachable));
    assert!(heap.heap_end() <= 15 * PAGE_SIZE);
}

/// A small deterministic pseudo-random generator for graph tests.
struct XorShift(u64);

impl XorShift {
    /// Returns the next pseudo-random number.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Grows the mark stack when a wide object has more children than one stack page holds.
#[test]
fn grows_the_mark_stack_for_wide_objects() {
    let mut heap = Heap::new(64);
    let leaves: Vec<u32> = (0..30_000).map(|_| heap.cell(0)).collect();
    let values: Vec<u64> = leaves.iter().map(|leaf| reference(*leaf)).collect();
    let fan = heap.buffer(&values);
    heap.set_root(0, reference(fan));
    heap.collect();
    heap.collect();
    assert!(leaves.iter().all(|leaf| heap.is_live(*leaf)));
    assert_eq!(collection_count(&heap.memory, VM), 2);
}

/// Never looks at the unused cells of a partly linked page, even when a reused page still holds
/// bytes that look like marked objects, also when marking overflows and rescans.
#[test]
fn ignores_stale_bytes_behind_the_link_cursor() {
    let mut heap = Heap::new(15);
    // A dead one-page object leaves bytes that look like marked Cells referencing the heap.
    let stale = heap.allocate(PAGE_SIZE - PAGE_HEADER_SIZE - 64, ObjectKind::Bytes);
    for offset in (8..PAGE_SIZE - PAGE_HEADER_SIZE - 64).step_by(8) {
        heap.memory.store_u8(stale + offset, ObjectKind::Cell as u8);
        heap.memory
            .store_u8(stale + offset + 1, exs_layout::GC_MARK);
        heap.memory.store_u32(stale + offset + 4, 0);
    }
    heap.collect();
    // The 128-byte class reuses the page and links only its first chunk.
    let text = heap.string(100);
    assert_eq!(text, stale);
    heap.set_root(1, reference(text));
    let untouched = stale + 64 * 128;
    let before = heap.memory.load_u8(untouched + 1);
    // Wide fan-out overflows the one-page mark stack, so marking rescans every class page.
    let leaves: Vec<u32> = (0..30_000).map(|_| heap.cell(0)).collect();
    let values: Vec<u64> = leaves.iter().map(|leaf| reference(*leaf)).collect();
    let fan = heap.buffer(&values);
    heap.set_root(0, reference(fan));
    heap.collect();
    assert_eq!(
        heap.memory.load_u32(
            VM + core::mem::offset_of!(exs_layout::VmState, heap.mark_stack_capacity) as u32
        ),
        (PAGE_SIZE - PAGE_HEADER_SIZE) / 4,
        "the mark stack grew, so marking did not overflow"
    );
    assert!(heap.is_live(text));
    assert!(leaves.iter().all(|leaf| heap.is_live(*leaf)));
    assert_eq!(
        heap.memory.load_u8(untouched + 1),
        before,
        "a stale cell was swept"
    );
    // The page keeps handing out its remaining cells, zeroed.
    for _ in 0..100 {
        let next = heap.string(100);
        // The allocator zeroes the requested 120 bytes; the header occupies the first 8.
        assert!((8..120).all(|offset| heap.memory.load_u8(next + offset) == 0));
    }
}

/// Forgets the link cursor of a partly linked page that becomes empty and is released.
#[test]
fn resets_the_cursor_of_a_released_page() {
    let mut heap = Heap::new(64);
    let first = heap.string(100);
    heap.collect();
    assert!(!heap.is_live(first));
    // A large run reuses the released page, so the class must start a new page.
    let large = heap.allocate(PAGE_SIZE - PAGE_HEADER_SIZE - 64, ObjectKind::Bytes);
    heap.set_root(0, reference(large));
    assert_eq!(large, first);
    let next = heap.string(100);
    assert_ne!(next & !(PAGE_SIZE - 1), first & !(PAGE_SIZE - 1));
}
