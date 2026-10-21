//! String concatenation and comparison tests.

use exs_value::ValueRef;

use super::heap::{Heap, VM};
use crate::{Memory, Ordering, compare, concat, equal, float_value, int_value, string_object};

/// Builds a String from `text` in `heap`.
fn string(heap: &mut Heap, text: &str) -> u64 {
    let object = heap.string(text.len() as u32);
    heap.memory
        .store_u32(object + 4, text.chars().count() as u32);
    heap.memory.store_u32(object + 8, text.len() as u32);
    for (offset, byte) in (0_u32..).zip(text.bytes()) {
        heap.memory.store_u8(object + 16 + offset, byte);
    }
    crate::strings::string_reference(object).expect("nonzero kind")
}

/// Returns the text of one String reference.
fn text(heap: &Heap, raw: u64) -> std::string::String {
    let object = string_object(raw).expect("a String");
    let len = heap.memory.load_u32(object + 8);
    let bytes: std::vec::Vec<u8> = (0..len)
        .map(|offset| heap.memory.load_u8(object + 16 + offset))
        .collect();
    std::string::String::from_utf8(bytes).expect("valid UTF-8")
}

/// Evaluates one `+` chain over `values` stored in root slots.
fn chain(heap: &mut Heap, values: &[u64]) -> Option<u64> {
    for (index, value) in (0_u32..).zip(values) {
        heap.set_root(index, *value);
    }
    concat(
        &mut heap.memory,
        VM,
        super::heap::ROOT_SLOTS,
        values.len() as u32,
        0,
    )
}

/// Renders scalars like v1 and joins whole runs into one String.
#[test]
fn concatenates_strings_and_scalars() {
    let mut heap = Heap::new(64);
    let prefix = string(&mut heap, "Wert ä: ");
    let big = int_value(&mut heap.memory, VM, -3_000_000_000).expect("room");
    let half = float_value(&mut heap.memory, VM, 0.5).expect("room");
    let whole = float_value(&mut heap.memory, VM, 2.0).expect("room");
    let values = [
        prefix,
        ValueRef::small_int(7).expect("small").raw(),
        ValueRef::TRUE.raw(),
        big,
        half,
        whole,
    ];
    let result = chain(&mut heap, &values).expect("renderable");
    assert_eq!(text(&heap, result), "Wert ä: 7true-30000000000.52");
    let object = string_object(result).expect("a String");
    assert_eq!(
        heap.memory.load_u32(object + 4),
        "Wert ä: 7true-30000000000.52".chars().count() as u32
    );
}

/// Adds numbers before a String like successive `+`.
#[test]
fn folds_numbers_before_strings() {
    let mut heap = Heap::new(64);
    let one = ValueRef::small_int(1).expect("small").raw();
    let sum = chain(&mut heap, &[one, one, one]).expect("numeric");
    assert_eq!(sum, ValueRef::small_int(3).expect("small").raw());
}

/// Compares Strings by text and orders them byte-wise.
#[test]
fn compares_strings() {
    let mut heap = Heap::new(64);
    let (apple, apple_copy, banana) = (
        string(&mut heap, "apple"),
        string(&mut heap, "apple"),
        string(&mut heap, "banana"),
    );
    assert!(equal(&heap.memory, apple, apple_copy));
    assert!(!equal(&heap.memory, apple, banana));
    assert_eq!(
        compare(&heap.memory, Ordering::Less, apple, banana),
        Ok(true)
    );
    assert_eq!(
        compare(&heap.memory, Ordering::GreaterOrEqual, apple, apple_copy),
        Ok(true)
    );
    let one = ValueRef::small_int(1).expect("small").raw();
    assert!(compare(&heap.memory, Ordering::Less, apple, one).is_err());
    assert_eq!(
        compare(
            &heap.memory,
            Ordering::LessOrEqual,
            ValueRef::NONE.raw(),
            ValueRef::NONE.raw()
        ),
        Ok(true)
    );
}

/// Renders every value like v1's formatted strings, using default texts for non-scalars.
#[test]
fn formats_every_value_with_default_texts() {
    let mut execution = super::execution::Execution::new();
    let prefix = execution.string("v: ");
    let list = execution.list(&[ValueRef::TRUE.raw()]);
    let values = [
        prefix,
        ValueRef::NONE.raw(),
        list,
        ValueRef::small_int(-4).expect("small").raw(),
        ValueRef::FALSE.raw(),
    ];
    let slots = execution.root(&values);
    let formatted =
        crate::format(&mut execution.memory, VM, slots, values.len() as u32).expect("room");
    assert_eq!(execution.text(formatted), "v: None[]-4false");
}
