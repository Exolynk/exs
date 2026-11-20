//! A temporary hash set of the keys of one wide Object being adopted from an image.
//!
//! Checking every new key against the keys before it takes quadratic time; the set keeps the
//! check constant per key for Objects with at least [`MIN_KEYS`] keys. Its table is the element
//! storage of a scratch List that is never exposed to the program: the List roots the table, and
//! the collector traces the key Strings in it.

use crate::lists::{allocate_list, item_address, list_reference};
use crate::memory::Memory;
use crate::strings::{string_object, text_equals};

/// Fewest keys of an Object whose adoption uses a key set.
pub(crate) const MIN_KEYS: u32 = 16;

/// Multiplier of the word hash.
const HASH_FACTOR: u64 = 0x517c_c1b7_2722_0a95;

/// A hash set of key Strings with open addressing; an empty slot holds 0.
#[derive(Clone, Copy)]
pub(crate) struct KeySet {
    /// Address of the scratch List whose element storage holds the table.
    list: u32,
    /// Slot count minus one; the slot count is a power of two.
    mask: u32,
}

impl KeySet {
    /// Returns an empty set for `count` keys and the raw reference of its List, which the
    /// caller must root while it uses the set. Returns `None` when memory is exhausted.
    pub(crate) fn new(memory: &mut impl Memory, vm: u32, count: u32) -> Option<(Self, u64)> {
        // At most half the slots are used, so every probe ends at an empty slot.
        let slots = count.checked_mul(2)?.checked_next_power_of_two()?;
        let list = allocate_list(memory, vm, slots)?;
        let raw = list_reference(list)?;
        Some((
            Self {
                list,
                mask: slots - 1,
            },
            raw,
        ))
    }

    /// Returns the hash of the key text of `len` bytes at `start`, which must be 8-byte aligned.
    pub(crate) fn hash(memory: &impl Memory, start: u32, len: u32) -> u32 {
        let words = len / 8;
        let mut hash = u64::from(len);
        for word in 0..words {
            hash =
                (hash.rotate_left(5) ^ memory.load_u64(start + word * 8)).wrapping_mul(HASH_FACTOR);
        }
        for offset in words * 8..len {
            hash = (hash.rotate_left(5) ^ u64::from(memory.load_u8(start + offset)))
                .wrapping_mul(HASH_FACTOR);
        }
        (hash >> 32) as u32
    }

    /// Returns whether the set holds a key equal to the text of `len` bytes at `start`, whose
    /// hash is `hash`.
    pub(crate) fn contains(self, memory: &impl Memory, hash: u32, start: u32, len: u32) -> bool {
        let mut slot = hash & self.mask;
        loop {
            let raw = memory.load_u64(item_address(memory, self.list, slot));
            if raw == 0 {
                return false;
            }
            if string_object(raw).is_some_and(|key| text_equals(memory, key, start, len)) {
                return true;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Adds the key String `name`, which the set must lack, whose text has the hash `hash`.
    pub(crate) fn insert(self, memory: &mut impl Memory, hash: u32, name: u64) {
        let mut slot = hash & self.mask;
        while memory.load_u64(item_address(memory, self.list, slot)) != 0 {
            slot = (slot + 1) & self.mask;
        }
        memory.store_u64(item_address(memory, self.list, slot), name);
    }
}
