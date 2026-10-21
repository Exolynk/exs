//! A byte-vector memory and helpers that build synthetic heaps.

use std::vec::Vec;

use exs_layout::{
    BufferObject, CellObject, ListObject, ObjectKind, PAGE_SIZE, PageKind, RootRange,
};
use exs_value::ValueRef;
use exs_value::layout::KIND_CELL;

use crate::{Memory, allocate, collect, init_heap, set_root_ranges};

/// Address of the VM state block in test memories.
pub(super) const VM: u32 = 256;
/// Address of the single root range descriptor.
const ROOT_RANGE: u32 = 4096;
/// Address of the first root slot.
pub(super) const ROOT_SLOTS: u32 = 8192;
/// Number of root slots.
pub(super) const ROOT_SLOT_COUNT: u32 = 64;
/// Address of the first heap page.
pub(super) const HEAP_START: u32 = PAGE_SIZE;

/// Linear memory backed by a byte vector with an optional page limit.
pub(crate) struct VecMemory {
    /// Memory contents.
    bytes: Vec<u8>,
    /// Largest number of 64 KiB pages `grow` may reach.
    max_pages: u32,
}

impl VecMemory {
    /// Returns a one-page memory that may grow to `max_pages` pages.
    pub(super) fn new(max_pages: u32) -> Self {
        Self {
            bytes: std::vec![0; PAGE_SIZE as usize],
            max_pages,
        }
    }

    /// Returns `N` bytes starting at `address`.
    fn read<const N: usize>(&self, address: u32) -> [u8; N] {
        let start = address as usize;
        let mut bytes = [0; N];
        bytes.copy_from_slice(&self.bytes[start..start + N]);
        bytes
    }

    /// Writes `bytes` starting at `address`.
    fn write(&mut self, address: u32, bytes: &[u8]) {
        let start = address as usize;
        self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
    }
}

impl Memory for VecMemory {
    fn load_u8(&self, address: u32) -> u8 {
        self.read::<1>(address)[0]
    }

    fn load_u16(&self, address: u32) -> u16 {
        u16::from_le_bytes(self.read(address))
    }

    fn load_u32(&self, address: u32) -> u32 {
        u32::from_le_bytes(self.read(address))
    }

    fn load_u64(&self, address: u32) -> u64 {
        u64::from_le_bytes(self.read(address))
    }

    fn store_u8(&mut self, address: u32, value: u8) {
        self.write(address, &[value]);
    }

    fn store_u16(&mut self, address: u32, value: u16) {
        self.write(address, &value.to_le_bytes());
    }

    fn store_u32(&mut self, address: u32, value: u32) {
        self.write(address, &value.to_le_bytes());
    }

    fn store_u64(&mut self, address: u32, value: u64) {
        self.write(address, &value.to_le_bytes());
    }

    fn fill(&mut self, address: u32, length: u32, value: u8) {
        let start = address as usize;
        self.bytes[start..start + length as usize].fill(value);
    }

    fn copy(&mut self, destination: u32, source: u32, length: u32) {
        let source = source as usize;
        self.bytes
            .copy_within(source..source + length as usize, destination as usize);
    }

    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn bytes(&self, address: u32, length: u32) -> &[u8] {
        let start = address as usize;
        &self.bytes[start..start + length as usize]
    }

    fn grow(&mut self, pages: u32) -> bool {
        let current = (self.bytes.len() / PAGE_SIZE as usize) as u32;
        if current + pages > self.max_pages {
            return false;
        }
        self.bytes
            .resize(self.bytes.len() + (pages * PAGE_SIZE) as usize, 0);
        true
    }
}

/// A test heap with one range of root slots.
pub(super) struct Heap {
    /// The backing memory.
    pub(super) memory: VecMemory,
}

impl Heap {
    /// Creates an empty heap that may grow to `max_pages` pages.
    pub(super) fn new(max_pages: u32) -> Self {
        let mut memory = VecMemory::new(max_pages);
        init_heap(&mut memory, VM, HEAP_START);
        memory.store_u32(ROOT_RANGE, ROOT_SLOTS);
        memory.store_u32(ROOT_RANGE + 4, ROOT_SLOTS + ROOT_SLOT_COUNT * 8);
        set_root_ranges(&mut memory, VM, ROOT_RANGE, 1);
        let range_size = core::mem::size_of::<RootRange>();
        assert_eq!(range_size, 8);
        Self { memory }
    }

    /// Allocates one object or panics when memory is exhausted.
    pub(super) fn allocate(&mut self, size: u32, kind: ObjectKind) -> u32 {
        allocate(&mut self.memory, VM, size, kind).expect("test heap exhausted")
    }

    /// Runs one collection.
    pub(super) fn collect(&mut self) {
        collect(&mut self.memory, VM);
    }

    /// Stores one raw reference in root slot `index`.
    pub(super) fn set_root(&mut self, index: u32, raw: u64) {
        assert!(index < ROOT_SLOT_COUNT);
        self.memory.store_u64(ROOT_SLOTS + index * 8, raw);
    }

    /// Allocates a Cell holding `value`.
    pub(super) fn cell(&mut self, value: u64) -> u32 {
        let cell = self.allocate(size_of::<CellObject>(), ObjectKind::Cell);
        self.memory.store_u64(
            cell + core::mem::offset_of!(CellObject, value) as u32,
            value,
        );
        cell
    }

    /// Allocates a Buffer holding `values`.
    pub(super) fn buffer(&mut self, values: &[u64]) -> u32 {
        let count = values.len() as u32;
        let buffer = self.allocate(size_of::<BufferObject>() + count * 8, ObjectKind::Buffer);
        self.memory.store_u32(buffer + 4, count);
        for (index, value) in (0_u32..).zip(values) {
            self.memory
                .store_u64(buffer + size_of::<BufferObject>() + index * 8, *value);
        }
        buffer
    }

    /// Allocates a List over the Buffer at `items`.
    pub(super) fn list(&mut self, items: u32, len: u32) -> u32 {
        let list = self.allocate(size_of::<ListObject>(), ObjectKind::List);
        self.memory
            .store_u32(list + core::mem::offset_of!(ListObject, len) as u32, len);
        self.memory.store_u32(
            list + core::mem::offset_of!(ListObject, items) as u32,
            items,
        );
        list
    }

    /// Allocates a String with `len` bytes.
    pub(super) fn string(&mut self, len: u32) -> u32 {
        self.allocate(16 + len, ObjectKind::String)
    }

    /// Returns the kind byte of the object at `address`.
    pub(super) fn kind(&self, address: u32) -> u8 {
        self.memory.load_u8(address)
    }

    /// Returns whether the object at `address` is still allocated.
    ///
    /// An object is dead when its cell is free or when its whole page run was released.
    pub(super) fn is_live(&self, address: u32) -> bool {
        let mut page = HEAP_START;
        while page < self.heap_end() {
            let pages = self.memory.load_u32(page + 4).max(1);
            let end = page + pages * PAGE_SIZE;
            if address < end {
                let kind = self.memory.load_u32(page);
                return match PageKind::from_u32(kind) {
                    Some(PageKind::Class) => self.kind(address) != ObjectKind::Free as u8,
                    Some(PageKind::Large) => true,
                    _ => false,
                };
            }
            page = end;
        }
        false
    }

    /// Returns the address just past the last carved heap page.
    pub(super) fn heap_end(&self) -> u32 {
        self.memory
            .load_u32(VM + core::mem::offset_of!(exs_layout::VmState, heap.heap_end) as u32)
    }
}

/// Returns the byte size of `T` as a `u32`.
pub(super) const fn size_of<T>() -> u32 {
    core::mem::size_of::<T>() as u32
}

/// Returns a raw heap reference to the object at `address`.
pub(super) fn reference(address: u32) -> u64 {
    ValueRef::from_parts(KIND_CELL, 0, address)
        .expect("internal kinds are nonzero")
        .raw()
}
