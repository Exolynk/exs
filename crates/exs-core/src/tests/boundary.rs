//! Runner input and result conversion tests.

use exs_abi::{CborLimits, ExsValue};
use exs_value::ValueRef;

use super::heap::{VM, VecMemory};
use super::walk::walked;
use crate::{Memory, Number, image_inputs, init_execution, number, reserve_input};

/// Address of the root slots receiving decoded arguments in these tests.
const SLOTS: u32 = 2048;

/// Returns an initialized memory holding `input` as the runner's argument bytes.
fn memory_with_input(input: &[u8]) -> VecMemory {
    let mut memory = VecMemory::new(64);
    init_execution(&mut memory, VM, exs_layout::PAGE_SIZE, 1).expect("memory has room");
    let address = reserve_input(&mut memory, VM, input.len() as u32).expect("memory has room");
    for (offset, byte) in (0_u32..).zip(input) {
        memory.store_u8(address + offset, *byte);
    }
    memory
}

/// Returns the image of the runner argument List `values`.
fn image(values: std::vec::Vec<ExsValue>) -> std::vec::Vec<u8> {
    ExsValue::List(values)
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("encodable")
}

/// Decodes every scalar the runner encodes and pads missing arguments with None.
#[test]
fn decodes_scalar_arguments() {
    let values = [
        ExsValue::Int(0),
        ExsValue::Int(-1),
        ExsValue::Int(1_000_000),
        ExsValue::Int(i64::MIN),
        ExsValue::Int(i64::MAX),
        ExsValue::Float(1.5),
        ExsValue::Bool(true),
        ExsValue::None,
    ];
    let mut memory = memory_with_input(&image(values.to_vec()));
    let count = image_inputs(&mut memory, VM, SLOTS, 10).expect("decodable");
    assert_eq!(count, values.len() as u32);
    for (index, value) in (0_u32..).zip(&values) {
        let raw = memory.load_u64(SLOTS + index * 8);
        match value {
            ExsValue::Int(expected) => {
                assert_eq!(number(&memory, raw), Some(Number::Int(*expected)))
            }
            ExsValue::Float(expected) => {
                assert_eq!(number(&memory, raw), Some(Number::Float(*expected)));
            }
            ExsValue::Bool(true) => assert_eq!(raw, ValueRef::TRUE.raw()),
            _ => assert_eq!(raw, ValueRef::NONE.raw()),
        }
    }
    for index in values.len() as u32..10 {
        assert_eq!(memory.load_u64(SLOTS + index * 8), ValueRef::NONE.raw());
    }
}

/// Reports scalar results exactly as the runner reads them.
#[test]
fn encodes_scalar_results() {
    let mut memory = memory_with_input(&image(std::vec::Vec::new()));
    for value in [
        0,
        23,
        24,
        255,
        256,
        65_536,
        -1,
        -24,
        -25,
        i64::MIN,
        i64::MAX,
    ] {
        let raw = crate::int_value(&mut memory, VM, value).expect("memory has room");
        assert_eq!(walked(&memory, VM, raw), Ok(ExsValue::Int(value)));
    }
    let raw = crate::float_value(&mut memory, VM, -2.25).expect("memory has room");
    assert_eq!(walked(&memory, VM, raw), Ok(ExsValue::Float(-2.25)));
    for (raw, expected) in [
        (ValueRef::NONE.raw(), ExsValue::None),
        (ValueRef::FALSE.raw(), ExsValue::Bool(false)),
        (ValueRef::TRUE.raw(), ExsValue::Bool(true)),
    ] {
        assert_eq!(walked(&memory, VM, raw), Ok(expected));
    }
}

/// Round-trips Unicode text.
#[test]
fn round_trips_strings() {
    for text in ["", "ascii", "äöü €", "𝄞 and 中文"] {
        let mut memory = memory_with_input(&image(std::vec![ExsValue::String(text.into())]));
        assert_eq!(image_inputs(&mut memory, VM, SLOTS, 1), Ok(1));
        let raw = memory.load_u64(SLOTS);
        let object = crate::string_object(raw).expect("decoded a String");
        assert_eq!(memory.load_u32(object + 4), text.chars().count() as u32);
        assert_eq!(walked(&memory, VM, raw), Ok(ExsValue::String(text.into())));
    }
}
