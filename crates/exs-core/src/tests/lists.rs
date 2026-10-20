//! List, access and typed Object tests on an initialized execution.

use core::mem::offset_of;
use std::vec::Vec;

use exs_abi::ExsValue;
use exs_layout::{ObjectKind, TypeEntry, VM_FLAG_COLLECT_EVERY_ALLOCATION, VmState};
use exs_value::ValueRef;
use exs_value::layout::object_type_extra;

use super::heap::{VM, VecMemory};
use crate::{
    AccessError, Memory, allocate, concat, decode_inputs, encode_result, index_get, index_set,
    init_execution, list_copy, list_new, list_object, property_get, property_set, reserve_input,
    result, set_heap_flags, set_program_types,
};

/// Address of the program's type table in these tests.
const TYPES: u32 = 2048;
/// Address of the field-name slots of type 0.
const FIELD_NAMES: u32 = 2304;

/// One initialized execution whose values are rooted on the root stack.
struct Execution {
    /// The backing memory.
    memory: VecMemory,
}

impl Execution {
    /// Initializes a fresh execution.
    fn new() -> Self {
        let mut memory = VecMemory::new(256);
        init_execution(&mut memory, VM, exs_layout::PAGE_SIZE, 1).expect("memory has room");
        Self { memory }
    }

    /// Returns the address of the root stack's next free slot.
    fn stack_end(&self) -> u32 {
        self.memory
            .load_u32(VM + offset_of!(VmState, root_stack.range.end) as u32)
    }

    /// Roots `values` in consecutive new root-stack slots and returns the first slot address.
    fn root(&mut self, values: &[u64]) -> u32 {
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
    fn int(value: i64) -> u64 {
        ValueRef::small_int(value).expect("small Int").raw()
    }

    /// Creates and roots a List of `values`.
    fn list(&mut self, values: &[u64]) -> u64 {
        let slots = self.root(values);
        let list = list_new(&mut self.memory, VM, slots, values.len() as u32).expect("room");
        self.root(&[list]);
        list
    }

    /// Returns the elements of one List.
    fn elements(&self, raw: u64) -> Vec<u64> {
        let list = list_object(raw).expect("a List");
        let len = self.memory.load_u32(list + 8);
        let items = self.memory.load_u32(list + 12);
        (0..len)
            .map(|index| self.memory.load_u64(items + 8 + index * 8))
            .collect()
    }

    /// Creates and roots a String of `text`.
    fn string(&mut self, text: &str) -> u64 {
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
    fn typed_object(&mut self, fields: &[&str], values: &[u64]) -> u64 {
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
    fn encoded(&mut self, raw: u64) -> ExsValue {
        encode_result(&mut self.memory, VM, raw).expect("encodable");
        let (address, length) = result(&self.memory, VM);
        let bytes: Vec<u8> = (0..length)
            .map(|offset| self.memory.load_u8(address + offset))
            .collect();
        ExsValue::from_cbor(&bytes).expect("valid CBOR")
    }
}

/// Builds Lists, adds Lists and values like v1 and copies Lists for iteration.
#[test]
fn creates_adds_and_copies_lists() {
    let mut execution = Execution::new();
    let one = Execution::int(1);
    let two = Execution::int(2);
    let left = execution.list(&[one, two]);
    assert_eq!(execution.elements(left), [one, two]);
    let empty = execution.list(&[]);
    assert!(execution.elements(empty).is_empty());
    let right = execution.list(&[two]);
    let slots = execution.root(&[left, right, one, left]);
    let sum = concat(&mut execution.memory, VM, slots, 4).expect("List addition");
    assert_eq!(execution.elements(sum), [one, two, two, one, one, two]);
    assert_eq!(execution.elements(left), [one, two], "an operand changed");
    let copy = list_copy(&mut execution.memory, VM, left).expect("room");
    assert_ne!(copy, left);
    assert_eq!(execution.elements(copy), [one, two]);
    let text = execution.string("x");
    let slots = execution.root(&[text, left]);
    assert!(concat(&mut execution.memory, VM, slots, 2).is_err());
}

/// Keeps every List and element alive while allocating under constant collection.
#[test]
fn builds_lists_while_collecting_at_every_allocation() {
    let mut execution = Execution::new();
    set_heap_flags(&mut execution.memory, VM, VM_FLAG_COLLECT_EVERY_ALLOCATION);
    let text = execution.string("kept");
    let mut list = execution.list(&[text]);
    for _ in 0..50 {
        let slots = execution.root(&[list, text]);
        list = concat(&mut execution.memory, VM, slots, 2).expect("List addition");
        execution.root(&[list]);
    }
    let elements = execution.elements(list);
    assert_eq!(elements.len(), 51);
    assert!(elements.iter().all(|element| *element == text));
    assert_eq!(
        execution.encoded(text),
        ExsValue::String("kept".into()),
        "the shared String was freed"
    );
}

/// Reads and writes List elements and typed Object fields with the language's checks.
#[test]
fn accesses_list_elements_and_fields() {
    let mut execution = Execution::new();
    let (one, two) = (Execution::int(1), Execution::int(2));
    let list = execution.list(&[one, two]);
    let memory = &mut execution.memory;
    assert_eq!(index_get(memory, VM, list, Execution::int(1)), Ok(two));
    assert_eq!(
        index_get(memory, VM, list, Execution::int(2)),
        Err(AccessError::OutOfBounds)
    );
    assert_eq!(
        index_get(memory, VM, list, Execution::int(-1)),
        Err(AccessError::InvalidIndex)
    );
    assert_eq!(
        index_get(memory, VM, list, ValueRef::TRUE.raw()),
        Err(AccessError::InvalidIndex)
    );
    assert_eq!(index_set(memory, VM, list, Execution::int(0), two), Ok(()));
    assert_eq!(execution.elements(list), [two, two]);
    let point = execution.typed_object(&["x", "y"], &[one, ValueRef::NONE.raw()]);
    let (x, y, z) = (
        execution.string("x"),
        execution.string("y"),
        execution.string("z"),
    );
    let memory = &mut execution.memory;
    assert_eq!(property_get(memory, VM, point, x), Ok(one));
    assert_eq!(property_get(memory, VM, point, z), Ok(ValueRef::NONE.raw()));
    assert_eq!(property_set(memory, VM, point, y, two), Ok(()));
    assert_eq!(index_get(memory, VM, point, y), Ok(two));
    assert_eq!(
        property_set(memory, VM, point, z, two),
        Err(AccessError::UnsupportedKey)
    );
    assert_eq!(
        property_get(memory, VM, list, x),
        Err(AccessError::NotAccessible)
    );
}

/// Round-trips nested Lists through the runner's codec and encodes typed Objects as maps.
#[test]
fn converts_lists_and_typed_objects() {
    let nested = ExsValue::List(std::vec![
        ExsValue::Int(1),
        ExsValue::List(std::vec![ExsValue::String("ä".into()), ExsValue::None]),
        ExsValue::List(std::vec![]),
    ]);
    let input = ExsValue::List(std::vec![nested.clone()])
        .to_cbor()
        .expect("encodable");
    let mut execution = Execution::new();
    let address =
        reserve_input(&mut execution.memory, VM, input.len() as u32).expect("memory has room");
    for (offset, byte) in (0_u32..).zip(&input) {
        execution.memory.store_u8(address + offset, *byte);
    }
    let slots = execution.root(&[0]);
    assert_eq!(decode_inputs(&mut execution.memory, VM, slots, 1), Ok(1));
    let decoded = execution.memory.load_u64(slots);
    assert_eq!(execution.encoded(decoded), nested);
    let point = execution.typed_object(&["x", "label"], &[Execution::int(3), decoded]);
    assert_eq!(
        execution.encoded(point),
        ExsValue::Object(std::vec![
            ("x".into(), ExsValue::Int(3)),
            ("label".into(), nested),
        ])
    );
    // A List containing itself cannot be encoded.
    let cyclic = execution.list(&[0]);
    let slot = list_object(cyclic).map(|list| execution.memory.load_u32(list + 12) + 8);
    execution.memory.store_u64(slot.expect("a List"), cyclic);
    assert!(encode_result(&mut execution.memory, VM, cyclic).is_err());
}
