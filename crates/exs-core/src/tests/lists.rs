//! List, access and typed Object tests on an initialized execution.

use exs_abi::ExsValue;
use exs_layout::VM_FLAG_COLLECT_EVERY_ALLOCATION;
use exs_value::ValueRef;

use super::execution::Execution;
use super::heap::VM;
use crate::{
    AccessError, Memory, concat, decode_inputs, encode_result, index_get, index_set, list_copy,
    list_object, property_get, property_set, reserve_input, set_heap_flags,
};

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
    assert_eq!(property_set(memory, VM, point, z, two), Ok(()));
    assert_eq!(property_get(memory, VM, point, z), Ok(two));
    assert_eq!(
        property_get(memory, VM, point, one),
        Err(AccessError::InvalidKey)
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
