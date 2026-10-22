//! Shape, untyped Object and Object conversion tests.

use std::vec::Vec;

use exs_abi::ExsValue;
use exs_layout::VM_FLAG_COLLECT_EVERY_ALLOCATION;
use exs_layout::ValueRef;

use super::execution::Execution;
use super::heap::VM;
use crate::{
    BoundaryError, collect, index_get, object_lookup, object_with_shape, property_get,
    property_set, set_heap_flags, shape_for,
};

/// Returns an Object value of `entries`.
fn object(entries: &[(&str, ExsValue)]) -> ExsValue {
    ExsValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).into(), value.clone()))
            .collect(),
    )
}

/// Shares one shape between Objects with the same key sequence and grows their values.
#[test]
fn shares_shapes_and_grows_objects() {
    let mut execution = Execution::new();
    let names: Vec<u64> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|name| execution.string(name))
        .collect();
    let keys = execution.root(&names[..2]);
    let shape = shape_for(&mut execution.memory, VM, keys, 2).expect("room");
    assert_eq!(shape_for(&mut execution.memory, VM, keys, 2), Some(shape));
    let values = execution.root(&[Execution::int(1), Execution::int(2)]);
    let first = object_with_shape(&mut execution.memory, VM, shape, values, 2).expect("room");
    execution.root(&[first]);
    let second = execution.decoded(&object(&[("a", ExsValue::Int(3)), ("b", ExsValue::None)]));
    let second = second.expect("decodable");
    let memory = &mut execution.memory;
    assert_eq!(
        object_lookup(memory, first, names[1]),
        object_lookup(memory, second, names[1]),
        "equal key sequences have one shape"
    );
    assert_eq!(
        object_lookup(memory, first, names[1]),
        Some((u64::from(shape) << 32) | 1)
    );
    for (index, name) in (0..).zip(&names[2..]) {
        property_set(memory, VM, first, *name, Execution::int(10 + index)).expect("room");
    }
    property_set(memory, VM, first, names[0], Execution::int(7)).expect("room");
    assert_eq!(
        property_get(memory, VM, first, names[5]),
        Some(Execution::int(13))
    );
    assert_eq!(
        index_get(memory, VM, first, names[0]),
        Some(Execution::int(7))
    );
    let missing = execution.string("missing");
    let memory = &mut execution.memory;
    assert_eq!(
        property_get(memory, VM, first, missing),
        Some(ValueRef::NONE.raw())
    );
    assert_eq!(object_lookup(memory, first, missing), None);
    let error = property_get(memory, VM, first, Execution::int(1)).expect("room");
    assert_eq!(execution.error_kind(error), "TypeError");
    assert_eq!(
        execution.encoded(first),
        object(&[
            ("a", ExsValue::Int(7)),
            ("b", ExsValue::Int(2)),
            ("c", ExsValue::Int(10)),
            ("d", ExsValue::Int(11)),
            ("e", ExsValue::Int(12)),
            ("f", ExsValue::Int(13)),
        ])
    );
}

/// Keeps Objects, their values and every shape alive under constant collection.
#[test]
fn builds_objects_while_collecting_at_every_allocation() {
    let mut execution = Execution::new();
    set_heap_flags(&mut execution.memory, VM, VM_FLAG_COLLECT_EVERY_ALLOCATION);
    let input = object(&[
        ("name", ExsValue::String("Ada".into())),
        (
            "tags",
            ExsValue::List(std::vec![object(&[("x", ExsValue::Float(0.5))])]),
        ),
        ("big", ExsValue::Int(i64::MAX)),
    ]);
    let decoded = execution.decoded(&input).expect("decodable");
    execution.root(&[decoded]);
    for index in 0..20 {
        let name = execution.string(&std::format!("key{index}"));
        property_set(&mut execution.memory, VM, decoded, name, name).expect("room");
    }
    collect(&mut execution.memory, VM);
    let ExsValue::Object(entries) = execution.encoded(decoded) else {
        panic!("an Object");
    };
    assert_eq!(entries.len(), 23);
    assert_eq!(ExsValue::Object(entries[..3].to_vec()), input);
    assert_eq!(
        entries[22],
        ("key19".into(), ExsValue::String("key19".into()))
    );
}

/// Refuses keys a nominal type does not declare and encodes only the declared fields.
#[test]
fn refuses_undeclared_keys_of_typed_objects() {
    let mut execution = Execution::new();
    let point = execution.typed_object(&["x"], &[Execution::int(1)]);
    let label = execution.string("label");
    let error = property_set(&mut execution.memory, VM, point, label, label).expect("room");
    assert_eq!(execution.error_kind(error), "TypeError");
    assert_eq!(execution.encoded(point), object(&[("x", ExsValue::Int(1))]));
}

/// Rejects Objects with repeated keys.
#[test]
fn rejects_invalid_maps() {
    let mut execution = Execution::new();
    let repeated = object(&[("a", ExsValue::Int(1)), ("a", ExsValue::Int(2))]);
    assert_eq!(execution.decoded(&repeated), Err(BoundaryError::Malformed));
}
