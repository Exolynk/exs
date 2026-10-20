//! Allocator tests.

use std::collections::HashSet;

use exs_layout::{
    MAX_CLASS_SIZE, ObjectKind, PAGE_HEADER_SIZE, PAGE_SIZE, SIZE_CLASSES, class_for_size,
};

use super::heap::{HEAP_START, Heap};
use crate::Memory;

/// Maps sizes to the smallest fitting class and leaves larger sizes to page runs.
#[test]
fn selects_the_smallest_fitting_size_class() {
    assert_eq!(class_for_size(1), Some(0));
    assert_eq!(class_for_size(16), Some(0));
    assert_eq!(class_for_size(17), Some(1));
    assert_eq!(class_for_size(MAX_CLASS_SIZE), Some(SIZE_CLASSES.len() - 1));
    assert_eq!(class_for_size(MAX_CLASS_SIZE + 1), None);
}

/// Returns distinct, aligned and zeroed objects with their kind and class in the header.
#[test]
fn allocates_aligned_zeroed_objects_with_headers() {
    let mut heap = Heap::new(64);
    let mut seen = HashSet::new();
    for size in [8, 16, 24, 40, 100, 300, 2000] {
        for _ in 0..50 {
            let object = heap.allocate(size, ObjectKind::Bytes);
            assert_eq!(object % 8, 0);
            assert!(object >= HEAP_START + PAGE_HEADER_SIZE);
            assert!(seen.insert(object), "address {object} returned twice");
            assert_eq!(heap.kind(object), ObjectKind::Bytes as u8);
            let class = class_for_size(size.max(16).next_multiple_of(8)).expect("small size");
            assert_eq!(heap.memory.load_u16(object + 2), class as u16);
            assert_eq!(heap.memory.load_u32(object + 4), 0);
            for offset in 8..size {
                assert_eq!(heap.memory.load_u8(object + offset), 0);
            }
            // Dirty the object so a reused cell would show up as not zeroed.
            heap.memory.fill(object + 8, size.saturating_sub(8), 0xAB);
        }
    }
}

/// Serves objects above the largest class from their own page runs.
#[test]
fn serves_large_objects_from_page_runs() {
    let mut heap = Heap::new(64);
    let first = heap.allocate(3 * PAGE_SIZE, ObjectKind::Buffer);
    let second = heap.allocate(MAX_CLASS_SIZE + 8, ObjectKind::Buffer);
    assert_eq!((first - PAGE_HEADER_SIZE) % PAGE_SIZE, 0);
    assert_eq!((second - PAGE_HEADER_SIZE) % PAGE_SIZE, 0);
    assert_eq!(heap.memory.load_u16(first + 2), u16::MAX);
    // Three pages of payload plus the page header need four pages.
    assert_eq!(second, first + 4 * PAGE_SIZE);
    assert_eq!(heap.heap_end(), HEAP_START + 5 * PAGE_SIZE);
}

/// Reports exhaustion instead of growing past the memory limit.
#[test]
fn reports_exhausted_memory() {
    let mut heap = Heap::new(3);
    assert!(
        crate::allocate(
            &mut heap.memory,
            super::heap::VM,
            4 * PAGE_SIZE,
            ObjectKind::Bytes
        )
        .is_none()
    );
    let small = crate::allocate(&mut heap.memory, super::heap::VM, 32, ObjectKind::Bytes);
    assert!(small.is_some());
}
