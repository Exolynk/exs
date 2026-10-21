//! An initialized execution whose test values are rooted on the root stack.

use core::mem::offset_of;
use std::vec::Vec;

use exs_abi::ExsValue;
use exs_layout::{ObjectKind, TypeEntry, VmState};
use exs_value::ValueRef;
use exs_value::layout::object_type_extra;

use super::heap::{VM, VecMemory};
use crate::{
    CborError, Memory, allocate, decode_inputs, encode_result, init_execution, list_new,
    list_object, reserve_input, result, set_program_types,
};

/// Address of the program's type table in these tests.
pub(super) const TYPES: u32 = 2048;
/// Address of the field-name slots of type 0.
pub(super) const FIELD_NAMES: u32 = 2304;

/// One initialized execution whose values are rooted on the root stack.
pub(super) struct Execution {
    /// The backing memory.
    pub(super) memory: VecMemory,
}

impl Execution {
    /// Initializes a fresh execution.
    pub(super) fn new() -> Self {
        let mut memory = VecMemory::new(256);
        init_execution(&mut memory, VM, exs_layout::PAGE_SIZE, 1).expect("memory has room");
        Self { memory }
    }

    /// Returns the address of the root stack's next free slot.
    pub(super) fn stack_end(&self) -> u32 {
        self.memory
            .load_u32(VM + offset_of!(VmState, root_stack.range.end) as u32)
    }

    /// Roots `values` in consecutive new root-stack slots and returns the first slot address.
    pub(super) fn root(&mut self, values: &[u64]) -> u32 {
        let first = self.stack_end();
        for (index, value) in (0_u32..).zip(values) {
            self.memory.store_u64(first + index * 8, *value);
        }
        self.memory.store_u32(
            VM + offset_of!(VmState, root_stack.range.end) as u32,
            first + values.len() as u32 * 8,
        );
        first
    }

    /// Returns a small Int reference.
    pub(super) fn int(value: i64) -> u64 {
        ValueRef::small_int(value).expect("small Int").raw()
    }

    /// Creates and roots a List of `values`.
    pub(super) fn list(&mut self, values: &[u64]) -> u64 {
        let slots = self.root(values);
        let list = list_new(&mut self.memory, VM, slots, values.len() as u32).expect("room");
        self.root(&[list]);
        list
    }

    /// Returns the elements of one List.
    pub(super) fn elements(&self, raw: u64) -> Vec<u64> {
        let list = list_object(raw).expect("a List");
        let len = self.memory.load_u32(list + 8);
        let items = self.memory.load_u32(list + 12);
        (0..len)
            .map(|index| self.memory.load_u64(items + 8 + index * 8))
            .collect()
    }

    /// Creates and roots a String of `text`.
    pub(super) fn string(&mut self, text: &str) -> u64 {
        let object = allocate(
            &mut self.memory,
            VM,
            16 + text.len() as u32,
            ObjectKind::String,
        )
        .expect("room");
        self.memory
            .store_u32(object + 4, text.chars().count() as u32);
        self.memory.store_u32(object + 8, text.len() as u32);
        for (offset, byte) in (0_u32..).zip(text.bytes()) {
            self.memory.store_u8(object + 16 + offset, byte);
        }
        let raw = crate::strings::string_reference(object).expect("nonzero kind");
        self.root(&[raw]);
        raw
    }

    /// Declares type 0 with the field names `fields` and returns a typed Object of it holding
    /// `values`.
    pub(super) fn typed_object(&mut self, fields: &[&str], values: &[u64]) -> u64 {
        for (index, field) in (0_u32..).zip(fields) {
            let name = self.string(field);
            self.memory.store_u64(FIELD_NAMES + index * 8, name);
        }
        self.memory.store_u32(
            TYPES + offset_of!(TypeEntry, field_count) as u32,
            fields.len() as u32,
        );
        self.memory.store_u32(
            TYPES + offset_of!(TypeEntry, field_names) as u32,
            FIELD_NAMES,
        );
        set_program_types(&mut self.memory, VM, TYPES, 1);
        let size = 16 + values.len() as u32 * 8;
        let object = allocate(&mut self.memory, VM, size, ObjectKind::TypedObject).expect("room");
        self.memory.store_u32(object + 8, values.len() as u32);
        for (index, value) in (0_u32..).zip(values) {
            self.memory.store_u64(object + 16 + index * 8, *value);
        }
        let raw = ValueRef::from_parts(
            exs_abi::TYPE_OBJECT as u16,
            object_type_extra(Some(0)),
            object,
        )
        .expect("nonzero kind")
        .raw();
        self.root(&[raw]);
        raw
    }

    /// Encodes `raw` as the result and decodes it with the runner's codec.
    pub(super) fn encoded(&mut self, raw: u64) -> ExsValue {
        encode_result(&mut self.memory, VM, raw).expect("encodable");
        let (address, length) = result(&self.memory, VM);
        let bytes: Vec<u8> = (0..length)
            .map(|offset| self.memory.load_u8(address + offset))
            .collect();
        ExsValue::from_cbor(&bytes).expect("valid CBOR")
    }

    /// Decodes the single runner argument of `value` into a rooted slot and returns it.
    pub(super) fn decoded(&mut self, value: &ExsValue) -> Result<u64, CborError> {
        let input = ExsValue::List(std::vec![value.clone()])
            .to_cbor()
            .expect("encodable");
        self.decoded_bytes(&input)
    }

    /// Decodes the runner argument array `input` into a rooted slot and returns its first value.
    pub(super) fn decoded_bytes(&mut self, input: &[u8]) -> Result<u64, CborError> {
        let address =
            reserve_input(&mut self.memory, VM, input.len() as u32).expect("memory has room");
        for (offset, byte) in (0_u32..).zip(input) {
            self.memory.store_u8(address + offset, *byte);
        }
        let slot = self.root(&[0]);
        decode_inputs(&mut self.memory, VM, slot, 1)?;
        Ok(self.memory.load_u64(slot))
    }
}
