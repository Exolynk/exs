//! Integration tests for the shared ExS CBOR boundary representation.

use exs_abi::{CborError, CborLimits, ErrorSeverity, ExsError, ExsValue};

/// Round-trips primitive and nested list values through the shared codec.
#[test]
fn round_trips_phase_one_values() {
    for value in [
        ExsValue::None,
        ExsValue::Bool(true),
        ExsValue::Bool(false),
        ExsValue::Int(-42),
        ExsValue::Int(42),
        ExsValue::Float(-1.5),
        ExsValue::Float(0.0),
        ExsValue::String("Ada\\nLovelace".to_owned()),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::String("Ada".to_owned()),
            ExsValue::List(vec![ExsValue::Bool(true)]),
        ]),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            (
                "values".to_owned(),
                ExsValue::List(vec![ExsValue::Int(1), ExsValue::Int(2)]),
            ),
        ]),
        ExsValue::Enum {
            type_id: "colors.exs::Color".to_owned(),
            variant: "Rgb".to_owned(),
            fields: vec![ExsValue::Int(255), ExsValue::Int(0), ExsValue::Int(128)],
        },
        ExsValue::Error(ExsError {
            severity: ErrorSeverity::Recoverable,
            kind: "MissingValue".to_owned(),
            message: "value was absent".to_owned(),
            data: Box::new(ExsValue::None),
            origin: None,
            trace: Vec::new(),
            cause: None,
        }),
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

/// Rejects CBOR byte strings, which are not represented by the ABI value.
#[test]
fn rejects_unsupported_cbor_values() {
    assert_eq!(
        ExsValue::from_cbor(&[0x40]),
        Err(CborError::UnsupportedType)
    );
}

/// Applies collection limits before allocating from a declared CBOR collection length.
#[test]
fn rejects_declared_collections_over_the_configured_limit() {
    assert_eq!(
        ExsValue::from_cbor_with_limits(
            &[0x98, 100],
            CborLimits {
                max_payload_bytes: 1024,
                max_nesting: 8,
                max_collection_entries: 4,
            },
        ),
        Err(CborError::CollectionLimitExceeded)
    );
}

/// Applies the default collection limit before allocating from hostile input.
#[test]
fn default_limits_reject_hostile_declared_collection() {
    assert_eq!(
        ExsValue::from_cbor(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
        Err(CborError::CollectionLimitExceeded)
    );
}

/// Rejects impossible collection headers even when callers opt out of resource limits.
#[test]
fn unrestricted_limits_reject_impossible_collection_length_without_allocating() {
    assert_eq!(
        ExsValue::from_cbor_with_limits(
            &[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            CborLimits::unrestricted(),
        ),
        Err(CborError::Malformed)
    );
}

/// Applies the default payload size limit before parsing input.
#[test]
fn default_limits_reject_oversized_payloads() {
    let bytes = vec![0xf6; 2 * 1024 * 1024 + 1];
    assert_eq!(
        ExsValue::from_cbor(&bytes),
        Err(CborError::PayloadLimitExceeded)
    );
}

/// Rejects nested values that cross the configured structural depth.
#[test]
fn rejects_values_over_the_configured_nesting_limit() {
    let value = ExsValue::List(vec![ExsValue::List(vec![ExsValue::Int(1)])]);
    let bytes = match value.to_cbor() {
        Ok(bytes) => bytes,
        Err(error) => panic!("could not encode test value: {error}"),
    };
    let limits = CborLimits {
        max_payload_bytes: 1024,
        max_nesting: 1,
        max_collection_entries: 8,
    };
    assert_eq!(
        value.to_cbor_with_limits(limits),
        Err(CborError::NestingLimitExceeded)
    );
    assert_eq!(
        ExsValue::from_cbor_with_limits(&bytes, limits),
        Err(CborError::NestingLimitExceeded)
    );
}
