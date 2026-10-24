//! Integration tests for the image boundary representation.

use exs_abi::value::{ExsStackFrame, SourcePositionId};
use exs_abi::{BoundaryError, BoundaryLimits, ErrorSeverity, ExsError, ExsValue};

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
        ExsValue::Typed {
            type_id: "test::Point".to_owned(),
            fields: vec![
                ("x".to_owned(), ExsValue::Int(1)),
                ("label".to_owned(), ExsValue::String("a".to_owned())),
            ],
        },
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
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    let read =
        ExsValue::from_image_with_limits(&image, BoundaryLimits::unrestricted()).expect("value");
    assert_eq!(read, value);
}

/// Applies the payload limit in the writer and the reader at exactly the same size.
#[test]
fn limits_the_payload_size() {
    let value = sample();
    let limits = |max_payload_bytes| BoundaryLimits {
        max_payload_bytes,
        ..BoundaryLimits::unrestricted()
    };
    let size = (1..10_000)
        .find(|size| value.to_image_with_limits(limits(*size)).is_ok())
        .expect("the sample has a payload size");
    let image = value.to_image_with_limits(limits(size)).expect("fits");
    assert_eq!(
        value.to_image_with_limits(limits(size - 1)),
        Err(BoundaryError::PayloadLimitExceeded)
    );
    assert!(ExsValue::from_image_with_limits(&image, limits(size)).is_ok());
    assert_eq!(
        ExsValue::from_image_with_limits(&image, limits(size - 1)),
        Err(BoundaryError::PayloadLimitExceeded)
    );
}

/// Applies the nesting and collection limits.
#[test]
fn limits_nesting_and_collections() {
    let nested = ExsValue::List(vec![ExsValue::List(vec![ExsValue::Int(1)])]);
    let image = nested
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    let nesting = BoundaryLimits {
        max_nesting: 2,
        ..BoundaryLimits::unrestricted()
    };
    assert_eq!(
        ExsValue::from_image_with_limits(&image, nesting),
        Err(BoundaryError::NestingLimitExceeded)
    );
    assert_eq!(
        nested.to_image_with_limits(nesting),
        Err(BoundaryError::NestingLimitExceeded)
    );
    let entries = BoundaryLimits {
        max_collection_entries: 0,
        ..BoundaryLimits::unrestricted()
    };
    assert_eq!(
        ExsValue::from_image_with_limits(&image, entries),
        Err(BoundaryError::CollectionLimitExceeded)
    );
}

/// Rejects truncated, misaligned, cyclic-free but corrupted and invalid-UTF-8 images.
#[test]
fn rejects_malformed_images() {
    let image = sample()
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    for len in [0, 4, 8, image.len() / 2] {
        assert!(
            ExsValue::from_image_with_limits(&image[..len], BoundaryLimits::default()).is_err()
        );
    }
    let mut bad_magic = image.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        ExsValue::from_image_with_limits(&bad_magic, BoundaryLimits::default()),
        Err(BoundaryError::Malformed)
    );
    let mut misaligned = image.clone();
    misaligned[4] = 9;
    assert_eq!(
        ExsValue::from_image_with_limits(&misaligned, BoundaryLimits::default()),
        Err(BoundaryError::Malformed)
    );
    let text = ExsValue::String("ab".to_owned())
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    let mut invalid = text.clone();
    let last = invalid.len() - 8;
    invalid[last] = 0xff;
    assert_eq!(
        ExsValue::from_image_with_limits(&invalid, BoundaryLimits::default()),
        Err(BoundaryError::Malformed)
    );
    // A List whose element points back at the List nests until the limit stops it.
    let mut cycle = ExsValue::List(vec![ExsValue::None])
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    let root = u32::from_le_bytes(cycle[4..8].try_into().expect("root")) as usize;
    let root_bytes = (root as u32).to_le_bytes();
    cycle[root + 8..root + 12].copy_from_slice(&root_bytes);
    assert_eq!(
        ExsValue::from_image_with_limits(&cycle, BoundaryLimits::default()),
        Err(BoundaryError::NestingLimitExceeded)
    );
}

/// Reads a None cause as no cause.
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
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("image");
    let ExsValue::Error(mut expected) = value else {
        unreachable!("the value is an Error");
    };
    expected.cause = None;
    assert_eq!(
        ExsValue::from_image_with_limits(&image, BoundaryLimits::unrestricted()),
        Ok(ExsValue::Error(expected))
    );
}
