//! Deep `clone` with v1's semantics: mutable values (Lists, Objects, Errors) are copied,
//! immutable ones shared, and aliases and cycles of the source reappear in the copy.
//!
//! A hash table from source to copy lives in a List rooted on the root stack, so every copy is
//! reachable while its contents are cloned.

use crate::enums::{
    allocate_enum, enum_object, enum_type, field_address as enum_field, field_count, variant,
};
use crate::errors::{copy_error, error_object, error_value_fields};
use crate::lists::{allocate_list, item_address, len, list_object, list_reference, set_len};
use crate::memory::Memory;
use crate::objects::{allocate_object, object_reference, shape, untyped_object, value_address};
use crate::shapes::key_count;
use crate::typed::{
    allocate_typed, extra, order, set_extra, set_order, slot_address, slot_count, type_id,
    typed_object,
};
use crate::vm::Vm;

/// Initial number of entries of the source-to-copy table.
const INITIAL_ENTRIES: u32 = 8;

/// Evaluates `clone()` of `source`, which must be rooted.
pub(crate) fn deep_clone(memory: &mut impl Memory, vm: u32, source: u64) -> Option<u64> {
    let table = allocate_list(memory, vm, INITIAL_ENTRIES * 2)?;
    set_len(memory, table, INITIAL_ENTRIES * 2);
    let state = Vm(vm);
    let slot = state.push_root(memory, list_reference(table)?)?;
    let mut cloner = Cloner { slot, count: 0 };
    let result = cloner.value(memory, vm, source);
    state.pop_roots(memory, slot);
    result
}

/// One clone traversal.
struct Cloner {
    /// Root slot holding the List of the source-to-copy table.
    slot: u32,
    /// Number of entries in the table.
    count: u32,
}

/// Returns the table slot index where `key` is or belongs, in a table of `entries` entries.
fn probe(memory: &impl Memory, table: u32, entries: u32, key: u64) -> u32 {
    let mut index = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32 & (entries - 1);
    loop {
        let stored = memory.load_u64(item_address(memory, table, index * 2));
        if stored == 0 || stored == key {
            return index;
        }
        index = (index + 1) & (entries - 1);
    }
}

impl Cloner {
    /// Returns the List of the table.
    fn table(&self, memory: &impl Memory) -> u32 {
        list_object(memory.load_u64(self.slot)).unwrap_or(0)
    }

    /// Returns the copy already made of `source`.
    fn existing(&self, memory: &impl Memory, source: u64) -> Option<u64> {
        let table = self.table(memory);
        let entries = len(memory, table) / 2;
        let index = probe(memory, table, entries, source);
        let stored = memory.load_u64(item_address(memory, table, index * 2));
        (stored == source).then(|| memory.load_u64(item_address(memory, table, index * 2 + 1)))
    }

    /// Records `copy` as the copy of `source`, growing the table at half load.
    fn register(
        &mut self,
        memory: &mut impl Memory,
        vm: u32,
        source: u64,
        copy: u64,
    ) -> Option<()> {
        let table = self.table(memory);
        let entries = len(memory, table) / 2;
        let index = probe(memory, table, entries, source);
        memory.store_u64(item_address(memory, table, index * 2), source);
        memory.store_u64(item_address(memory, table, index * 2 + 1), copy);
        self.count += 1;
        if self.count * 2 < entries {
            return Some(());
        }
        let grown_entries = entries.checked_mul(2)?;
        let grown = allocate_list(memory, vm, grown_entries.checked_mul(2)?)?;
        set_len(memory, grown, grown_entries * 2);
        let table = self.table(memory);
        for old in 0..entries {
            let key = memory.load_u64(item_address(memory, table, old * 2));
            if key != 0 {
                let value = memory.load_u64(item_address(memory, table, old * 2 + 1));
                let index = probe(memory, grown, grown_entries, key);
                memory.store_u64(item_address(memory, grown, index * 2), key);
                memory.store_u64(item_address(memory, grown, index * 2 + 1), value);
            }
        }
        memory.store_u64(self.slot, list_reference(grown)?);
        Some(())
    }

    /// Clones one value.
    fn value(&mut self, memory: &mut impl Memory, vm: u32, source: u64) -> Option<u64> {
        if let Some(list) = list_object(source) {
            if let Some(copy) = self.existing(memory, source) {
                return Some(copy);
            }
            let count = len(memory, list);
            let copy_list = allocate_list(memory, vm, count)?;
            let copy = list_reference(copy_list)?;
            self.register(memory, vm, source, copy)?;
            for index in 0..count {
                let item = memory.load_u64(item_address(memory, list, index));
                let item = self.value(memory, vm, item)?;
                memory.store_u64(item_address(memory, copy_list, index), item);
                set_len(memory, copy_list, index + 1);
            }
            return Some(copy);
        }
        if let Some(object) = untyped_object(memory, source) {
            if let Some(copy) = self.existing(memory, source) {
                return Some(copy);
            }
            let object_shape = shape(memory, object);
            let count = key_count(memory, object_shape);
            let copy_object = allocate_object(memory, vm, object_shape, count)?;
            let copy = object_reference(copy_object)?;
            self.register(memory, vm, source, copy)?;
            for index in 0..count {
                let item = memory.load_u64(value_address(memory, object, index));
                let item = self.value(memory, vm, item)?;
                memory.store_u64(value_address(memory, copy_object, index), item);
            }
            return Some(copy);
        }
        if let Some(object) = typed_object(memory, source) {
            if let Some(copy) = self.existing(memory, source) {
                return Some(copy);
            }
            let slots = slot_count(memory, object);
            let copy = allocate_typed(memory, vm, type_id(memory, object), slots)?;
            let copy_object = copy as u32;
            set_order(memory, copy_object, order(memory, object));
            self.register(memory, vm, source, copy)?;
            for index in 0..slots {
                let item = memory.load_u64(slot_address(object, index));
                // Removed declared fields stay removed.
                if item != 0 {
                    let item = self.value(memory, vm, item)?;
                    memory.store_u64(slot_address(copy_object, index), item);
                }
            }
            let keys = extra(memory, object);
            if keys != 0 {
                let extra_copy = self.value(memory, vm, object_reference(keys)?)?;
                set_extra(memory, copy_object, extra_copy as u32);
            }
            return Some(copy);
        }
        if let Some(object) = enum_object(memory, source) {
            if let Some(copy) = self.existing(memory, source) {
                return Some(copy);
            }
            let count = field_count(memory, object);
            let copy = allocate_enum(
                memory,
                vm,
                enum_type(memory, object),
                variant(memory, object),
                count,
            )?;
            self.register(memory, vm, source, copy)?;
            for index in 0..count {
                let item = memory.load_u64(enum_field(object, index));
                let item = self.value(memory, vm, item)?;
                memory.store_u64(enum_field(copy as u32, index), item);
            }
            return Some(copy);
        }
        if let Some(object) = error_object(source) {
            if let Some(copy) = self.existing(memory, source) {
                return Some(copy);
            }
            let copy = copy_error(memory, vm, object)?;
            self.register(memory, vm, source, copy)?;
            for (source_field, copy_field) in error_value_fields(object)
                .into_iter()
                .zip(error_value_fields(copy as u32))
            {
                let item = memory.load_u64(source_field);
                let item = self.value(memory, vm, item)?;
                memory.store_u64(copy_field, item);
            }
            return Some(copy);
        }
        // Strings, Bytes, numbers and immediates are immutable and shared.
        Some(source)
    }
}
