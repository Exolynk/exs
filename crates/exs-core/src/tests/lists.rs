//! List, access and typed Object tests on an initialized execution.

use exs_abi::ExsValue;
use exs_layout::VM_FLAG_COLLECT_EVERY_ALLOCATION;
use exs_value::ValueRef;

use super::execution::Execution;
use super::heap::VM;
use crate::{
    Memory, concat, image_inputs, index_get, index_set, list_copy, list_object, property_get,
    property_set, reserve_input, set_heap_flags,
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
    let sum = concat(&mut execution.memory, VM, slots, 4, 0).expect("List addition");
    assert_eq!(execution.elements(sum), [one, two, two, one, one, two]);
    assert_eq!(execution.elements(left), [one, two], "an operand changed");
    let copy = list_copy(&mut execution.memory, VM, left).expect("room");
    assert_ne!(copy, left);
    assert_eq!(execution.elements(copy), [one, two]);
    let text = execution.string("x");
    let slots = execution.root(&[text, left]);
    let error = concat(&mut execution.memory, VM, slots, 2, 0).expect("room");
    assert_eq!(execution.error_kind(error), "TypeError");
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
        list = concat(&mut execution.memory, VM, slots, 2, 0).expect("List addition");
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
    assert_eq!(index_get(memory, VM, list, Execution::int(1)), Some(two));
    for (index, message) in [
        (Execution::int(2), "List index is outside the List bounds"),
        (
            Execution::int(-1),
            "List index requires a non-negative Int value",
        ),
        (
            ValueRef::TRUE.raw(),
            "List index requires a non-negative Int value",
        ),
    ] {
        let error = index_get(&mut execution.memory, VM, list, index).expect("room");
        let (fatal, kind, text, data) = execution.error(error);
        assert_eq!(
            (fatal, kind.as_str(), text.as_str(), data),
            (false, "IndexError", message, index)
        );
    }
    let memory = &mut execution.memory;
    assert_eq!(
        index_set(memory, VM, list, Execution::int(0), two),
        Some(two)
    );
    assert_eq!(execution.elements(list), [two, two]);
    let point = execution.typed_object(&["x", "y"], &[one, ValueRef::NONE.raw()]);
    let (x, y, z) = (
        execution.string("x"),
        execution.string("y"),
        execution.string("z"),
    );
    let memory = &mut execution.memory;
    assert_eq!(property_get(memory, VM, point, x), Some(one));
    assert_eq!(
        property_get(memory, VM, point, z),
        Some(ValueRef::NONE.raw())
    );
    assert_eq!(property_set(memory, VM, point, y, two), Some(two));
    assert_eq!(index_get(memory, VM, point, y), Some(two));
    assert_eq!(property_set(memory, VM, point, z, two), Some(two));
    assert_eq!(property_get(memory, VM, point, z), Some(two));
    let error = property_get(memory, VM, point, one).expect("room");
    assert_eq!(
        execution.error(error).2,
        "Object keys and method names require a String value"
    );
    // A property of a List is an index with a String, which is no valid List index.
    let error = property_get(&mut execution.memory, VM, list, x).expect("room");
    assert_eq!(execution.error_kind(error), "IndexError");
    let error = property_get(&mut execution.memory, VM, one, x).expect("room");
    assert_eq!(
        execution.error(error).2,
        "index access requires a Bytes, List, or Object receiver"
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
        .to_image_with_limits(exs_abi::BoundaryLimits::unrestricted())
        .expect("encodable");
    let mut execution = Execution::new();
    let address =
        reserve_input(&mut execution.memory, VM, input.len() as u32).expect("memory has room");
    for (offset, byte) in (0_u32..).zip(&input) {
        execution.memory.store_u8(address + offset, *byte);
    }
    let slots = execution.root(&[0]);
    assert_eq!(image_inputs(&mut execution.memory, VM, slots, 1), Ok(1));
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
    // A List containing itself cannot cross; the runner reports v1's SerializationError.
    let cyclic = execution.list(&[0]);
    let slot = list_object(cyclic).map(|list| execution.memory.load_u32(list + 12) + 8);
    execution.memory.store_u64(slot.expect("a List"), cyclic);
    assert_eq!(
        super::walk::walked(&execution.memory, VM, cyclic),
        Err(crate::WalkError::Unsupported)
    );
    assert_eq!(
        crate::serialization_message(&execution.memory, cyclic),
        "value graph contains a cycle and cannot cross the runner boundary"
    );
}
