//! Numeric operation tests.

use exs_value::ValueRef;

use super::heap::{Heap, VM};
use crate::{
    Arithmetic, Number, NumericError, Ordering, arithmetic, compare, divide, equal, float_value,
    int_value, negate, number,
};

/// Returns the raw reference of one Int in `heap`.
fn int(heap: &mut Heap, value: i64) -> u64 {
    int_value(&mut heap.memory, VM, value).expect("heap has room")
}

/// Returns the raw reference of one Float in `heap`.
fn float(heap: &mut Heap, value: f64) -> u64 {
    float_value(&mut heap.memory, VM, value).expect("heap has room")
}

/// Keeps `i32` Ints immediate, boxes larger ones and reads both back.
#[test]
fn represents_small_and_large_ints() {
    let mut heap = Heap::new(64);
    let small = int(&mut heap, -7);
    assert_eq!(small, ValueRef::small_int(-7).expect("small").raw());
    for value in [i64::from(i32::MAX) + 1, i64::MIN, i64::MAX] {
        let raw = int(&mut heap, value);
        assert!(ValueRef::small_int(value).is_none());
        assert_eq!(number(&heap.memory, raw), Some(Number::Int(value)));
    }
    assert_eq!(
        number(&heap.memory, ValueRef::TRUE.raw()),
        Some(Number::Int(1))
    );
    assert_eq!(number(&heap.memory, ValueRef::NONE.raw()), None);
}

/// Applies checked Int arithmetic, Float promotion and Float division.
#[test]
fn applies_the_language_arithmetic_rules() {
    let mut heap = Heap::new(64);
    let (two, three) = (int(&mut heap, 2), int(&mut heap, 3));
    let half = float(&mut heap, 0.5);
    let product = arithmetic(&mut heap.memory, VM, Arithmetic::Multiply, two, three);
    assert_eq!(product, Some(int(&mut heap, 6)));
    let mixed = arithmetic(&mut heap.memory, VM, Arithmetic::Add, three, half).expect("numeric");
    assert_eq!(number(&heap.memory, mixed), Some(Number::Float(3.5)));
    let quotient = divide(&mut heap.memory, VM, three, two).expect("numeric");
    assert_eq!(number(&heap.memory, quotient), Some(Number::Float(1.5)));
    let wrapped = arithmetic(
        &mut heap.memory,
        VM,
        Arithmetic::Add,
        ValueRef::TRUE.raw(),
        two,
    );
    assert_eq!(wrapped, Some(three));
    let negated = negate(&mut heap.memory, VM, two).expect("numeric");
    assert_eq!(number(&heap.memory, negated), Some(Number::Int(-2)));
}

/// Compares and equates numbers across Int and Float, rejecting NaN orderings.
#[test]
fn compares_numbers() {
    let mut heap = Heap::new(64);
    let (one, one_float) = (int(&mut heap, 1), float(&mut heap, 1.0));
    let nan = float(&mut heap, f64::NAN);
    let big = int(&mut heap, i64::MAX);
    assert_eq!(compare(&heap.memory, Ordering::Less, one, big), Ok(true));
    assert_eq!(
        compare(&heap.memory, Ordering::GreaterOrEqual, one_float, one),
        Ok(true)
    );
    assert_eq!(
        compare(&heap.memory, Ordering::Less, nan, one),
        Err(NumericError::Unordered)
    );
    assert!(equal(&heap.memory, one, one_float));
    assert!(equal(&heap.memory, ValueRef::TRUE.raw(), one));
    assert!(!equal(&heap.memory, nan, nan));
    assert!(equal(
        &heap.memory,
        ValueRef::NONE.raw(),
        ValueRef::NONE.raw()
    ));
}
