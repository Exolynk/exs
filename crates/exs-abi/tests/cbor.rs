//! Integration tests for the shared ExS CBOR boundary representation.

use exs_abi::{CborError, ExsValue};

/// Round-trips every Phase-1 ABI primitive through the shared codec.
#[test]
fn round_trips_phase_one_values() {
    for value in [
        ExsValue::Null,
        ExsValue::Bool(true),
        ExsValue::Bool(false),
        ExsValue::Int(-42),
        ExsValue::Int(42),
        ExsValue::Float(-1.5),
        ExsValue::Float(0.0),
    ] {
        let encoded = match value.to_cbor() {
            Ok(encoded) => encoded,
            Err(error) => panic!("could not encode value: {error}"),
        };
        let decoded = match ExsValue::from_cbor(&encoded) {
            Ok(decoded) => decoded,
            Err(error) => panic!("could not decode value: {error}"),
        };
        assert_eq!(decoded, value);
    }
}

/// Rejects concatenated CBOR items at the ABI boundary.
#[test]
fn rejects_trailing_cbor_data() {
    assert_eq!(
        ExsValue::from_cbor(&[0xf6, 0xf6]),
        Err(CborError::TrailingData)
    );
}

/// Rejects CBOR types not yet represented by the Phase-1 ABI value.
#[test]
fn rejects_unsupported_cbor_values() {
    assert_eq!(
        ExsValue::from_cbor(&[0x60]),
        Err(CborError::UnsupportedType)
    );
}
