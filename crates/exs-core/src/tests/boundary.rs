//! Runner input and result conversion tests.

use std::vec::Vec;

use exs_abi::ExsValue;
use exs_value::ValueRef;

use super::heap::{VM, VecMemory};
use crate::{
    CborError, Memory, Number, decode_inputs, encode_result, init_execution, number, reserve_input,
    result,
};

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

/// Returns the result bytes the VM produced.
fn result_bytes(memory: &VecMemory) -> Vec<u8> {
    let (address, length) = result(memory, VM);
    (0..length)
        .map(|offset| memory.load_u8(address + offset))
        .collect()
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
    let input = ExsValue::List(values.to_vec())
        .to_cbor()
        .expect("encodable");
    let mut memory = memory_with_input(&input);
    let count = decode_inputs(&mut memory, VM, SLOTS, 10).expect("decodable");
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
    let maps = ExsValue::List(std::vec![ExsValue::Object(std::vec![])])
        .to_cbor()
        .expect("encodable");
    let mut memory = memory_with_input(&maps);
    assert_eq!(
        decode_inputs(&mut memory, VM, SLOTS, 1),
        Err(CborError::Unsupported)
    );
}

/// Encodes scalar results exactly like the runner's CBOR decoder expects them.
#[test]
fn encodes_scalar_results() {
    let mut memory = memory_with_input(&[0x80]);
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
        encode_result(&mut memory, VM, raw).expect("encodable");
        assert_eq!(
            ExsValue::from_cbor(&result_bytes(&memory)),
            Ok(ExsValue::Int(value))
        );
    }
    let raw = crate::float_value(&mut memory, VM, -2.25).expect("memory has room");
    encode_result(&mut memory, VM, raw).expect("encodable");
    assert_eq!(
        ExsValue::from_cbor(&result_bytes(&memory)),
        Ok(ExsValue::Float(-2.25))
    );
    for (raw, expected) in [
        (ValueRef::NONE.raw(), ExsValue::None),
        (ValueRef::FALSE.raw(), ExsValue::Bool(false)),
        (ValueRef::TRUE.raw(), ExsValue::Bool(true)),
    ] {
        encode_result(&mut memory, VM, raw).expect("encodable");
        assert_eq!(ExsValue::from_cbor(&result_bytes(&memory)), Ok(expected));
    }
}

/// Round-trips Unicode text and rejects invalid UTF-8.
#[test]
fn round_trips_strings() {
    for text in ["", "ascii", "äöü €", "𝄞 and 中文"] {
        let input = ExsValue::List(std::vec![ExsValue::String(text.into())])
            .to_cbor()
            .expect("encodable");
        let mut memory = memory_with_input(&input);
        assert_eq!(decode_inputs(&mut memory, VM, SLOTS, 1), Ok(1));
        let raw = memory.load_u64(SLOTS);
        let object = crate::string_object(raw).expect("decoded a String");
        assert_eq!(memory.load_u32(object + 4), text.chars().count() as u32);
        encode_result(&mut memory, VM, raw).expect("encodable");
        assert_eq!(
            ExsValue::from_cbor(&result_bytes(&memory)),
            Ok(ExsValue::String(text.into()))
        );
    }
    // An array of one text string holding an unpaired surrogate (ED A0 80).
    let mut memory = memory_with_input(&[0x81, 0x63, 0xed, 0xa0, 0x80]);
    assert_eq!(
        decode_inputs(&mut memory, VM, SLOTS, 1),
        Err(CborError::Malformed)
    );
}
