//! Integration tests for the image boundary representation.

use exs_abi::cbor::{ExsStackFrame, SourcePositionId};
use exs_abi::{CborError, CborLimits, ErrorSeverity, ExsError, ExsValue};

/// Returns one value of every kind, nested.
fn sample() -> ExsValue {
    let error = ExsError {
        severity: ErrorSeverity::Fatal,
        kind: "Kind".to_owned(),
        message: "message ü".to_owned(),
        data: Box::new(ExsValue::List(vec![ExsValue::Int(1)])),
        origin: Some(SourcePositionId(70_000)),
        trace: vec![ExsStackFrame {
            function_id: 300,
            call_site: SourcePositionId(5),
        }],
        cause: Some(Box::new(ExsValue::String("cause".to_owned()))),
    };
    ExsValue::List(vec![
        ExsValue::None,
        ExsValue::Bool(true),
        ExsValue::Bool(false),
        ExsValue::Int(-24),
        ExsValue::Int(23),
        ExsValue::Int(i64::MIN),
        ExsValue::Int(i64::MAX),
        ExsValue::Float(-0.5),
        ExsValue::String("aé𝄞".to_owned()),
        ExsValue::String(String::new()),
        ExsValue::Bytes(vec![0, 255, 7]),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("nested".to_owned(), ExsValue::Object(Vec::new())),
        ]),
        ExsValue::Enum {
            type_id: "test::Color".to_owned(),
            variant: "Rgb".to_owned(),
            fields: vec![ExsValue::Int(1), ExsValue::Int(2)],
        },
        ExsValue::Error(error),
    ])
}

/// Round-trips values of every kind.
#[test]
fn round_trips_every_kind() {
    let value = sample();
    let image = value
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    let read = ExsValue::from_image_with_limits(&image, CborLimits::unrestricted()).expect("value");
    assert_eq!(read, value);
}

/// Applies the payload limit to the size of the equivalent CBOR, exactly.
#[test]
fn limits_the_equivalent_cbor_size() {
    let value = sample();
    let cbor = value.to_cbor().expect("cbor").len();
    let limits = |max_payload_bytes| CborLimits {
        max_payload_bytes,
        ..CborLimits::unrestricted()
    };
    let image = value.to_image_with_limits(limits(cbor)).expect("fits");
    assert_eq!(
        value.to_image_with_limits(limits(cbor - 1)),
        Err(CborError::PayloadLimitExceeded)
    );
    assert!(ExsValue::from_image_with_limits(&image, limits(cbor)).is_ok());
    assert_eq!(
        ExsValue::from_image_with_limits(&image, limits(cbor - 1)),
        Err(CborError::PayloadLimitExceeded)
    );
}

/// Applies the nesting and collection limits like CBOR.
#[test]
fn limits_nesting_and_collections() {
    let nested = ExsValue::List(vec![ExsValue::List(vec![ExsValue::Int(1)])]);
    let image = nested
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    let nesting = CborLimits {
        max_nesting: 2,
        ..CborLimits::unrestricted()
    };
    assert_eq!(
        ExsValue::from_image_with_limits(&image, nesting),
        Err(CborError::NestingLimitExceeded)
    );
    assert_eq!(
        nested.to_image_with_limits(nesting),
        Err(CborError::NestingLimitExceeded)
    );
    let entries = CborLimits {
        max_collection_entries: 0,
        ..CborLimits::unrestricted()
    };
    assert_eq!(
        ExsValue::from_image_with_limits(&image, entries),
        Err(CborError::CollectionLimitExceeded)
    );
}

/// Rejects truncated, misaligned, cyclic-free but corrupted and invalid-UTF-8 images.
#[test]
fn rejects_malformed_images() {
    let image = sample()
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    for len in [0, 4, 8, image.len() / 2] {
        assert!(ExsValue::from_image_with_limits(&image[..len], CborLimits::default()).is_err());
    }
    let mut bad_magic = image.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        ExsValue::from_image_with_limits(&bad_magic, CborLimits::default()),
        Err(CborError::Malformed)
    );
    let mut misaligned = image.clone();
    misaligned[4] = 9;
    assert_eq!(
        ExsValue::from_image_with_limits(&misaligned, CborLimits::default()),
        Err(CborError::Malformed)
    );
    let text = ExsValue::String("ab".to_owned())
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    let mut invalid = text.clone();
    let last = invalid.len() - 8;
    invalid[last] = 0xff;
    assert_eq!(
        ExsValue::from_image_with_limits(&invalid, CborLimits::default()),
        Err(CborError::Malformed)
    );
    // A List whose element points back at the List nests until the limit stops it.
    let mut cycle = ExsValue::List(vec![ExsValue::None])
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    let root = u32::from_le_bytes(cycle[4..8].try_into().expect("root")) as usize;
    let root_bytes = (root as u32).to_le_bytes();
    cycle[root + 8..root + 12].copy_from_slice(&root_bytes);
    assert_eq!(
        ExsValue::from_image_with_limits(&cycle, CborLimits::default()),
        Err(CborError::NestingLimitExceeded)
    );
}

/// Reads a None cause as no cause, like CBOR.
#[test]
fn reads_a_none_cause_as_no_cause() {
    let value = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "K".to_owned(),
        message: "m".to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: Some(Box::new(ExsValue::None)),
    });
    let image = value
        .to_image_with_limits(CborLimits::unrestricted())
        .expect("image");
    let from_cbor = ExsValue::from_cbor(&value.to_cbor().expect("cbor")).expect("cbor value");
    assert_eq!(
        ExsValue::from_image_with_limits(&image, CborLimits::unrestricted()),
        Ok(from_cbor)
    );
}
