//! The shape transitions one image adoption has already taken.
//!
//! Writers store a repeated Object key once and let every Object point to that String record,
//! so within one image the same key offset always holds the same text. Adopting the next
//! Object with that key then moves from a shape to its child by comparing two numbers, instead
//! of reading and comparing the key text. Offsets only mean something inside one image, so
//! every adoption starts with an empty cache. Shapes are never collected, so the cached
//! addresses stay valid while the adoption allocates.

/// Number of cache entries, a power of two.
const ENTRIES: usize = 128;

/// One cached transition.
#[derive(Clone, Copy, Default)]
struct Entry {
    /// Address of the shape the transition starts from; 0 for an empty entry.
    parent: u32,
    /// Image offset of the key's String record.
    key: u32,
    /// Address of the shape the transition leads to.
    child: u32,
}

/// A direct-mapped cache of the shape transitions of one image adoption.
pub(crate) struct ShapeCache {
    /// The entries, indexed by a hash of the shape and key offset.
    entries: [Entry; ENTRIES],
}

impl ShapeCache {
    /// Returns an empty cache.
    pub(crate) fn new() -> Self {
        Self {
            entries: [Entry::default(); ENTRIES],
        }
    }

    /// Returns the entry index of the transition from `parent` by the key at offset `key`.
    fn index(parent: u32, key: u32) -> usize {
        let mixed = (parent ^ key.rotate_left(16)).wrapping_mul(0x9e37_79b1);
        (mixed >> (u32::BITS - ENTRIES.trailing_zeros())) as usize
    }

    /// Returns the shape the transition from `parent` by the key at offset `key` led to.
    pub(crate) fn child(&self, parent: u32, key: u32) -> Option<u32> {
        let entry = self.entries[Self::index(parent, key)];
        (entry.parent == parent && entry.key == key && parent != 0).then_some(entry.child)
    }

    /// Records that the key at offset `key` led from `parent` to `child`.
    pub(crate) fn insert(&mut self, parent: u32, key: u32, child: u32) {
        self.entries[Self::index(parent, key)] = Entry { parent, key, child };
    }
}
