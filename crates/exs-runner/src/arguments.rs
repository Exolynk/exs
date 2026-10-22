//! Decoding the argument List of a host call that arrives as an image.

use std::fmt;

use exs_abi::{BoundaryError, BoundaryLimits, ExsValue};

/// Why the arguments of a host call could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArgumentsError {
    /// The request is no valid image within the limits.
    Invalid(BoundaryError),
    /// The request is a valid image but no ordered argument List.
    NotAList,
}

impl fmt::Display for ArgumentsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "invalid host-call arguments: {error}"),
            Self::NotAList => formatter.write_str("host-call arguments must be a List"),
        }
    }
}

/// Decodes the image of one host request of a Rust guest within `limits`.
///
/// # Errors
///
/// Returns an error when the payload is not a valid bounded image of a List.
pub(crate) fn decode_arguments(
    bytes: &[u8],
    limits: BoundaryLimits,
) -> Result<Vec<ExsValue>, ArgumentsError> {
    match ExsValue::from_image_with_limits(bytes, limits).map_err(ArgumentsError::Invalid)? {
        ExsValue::List(arguments) => Ok(arguments),
        _ => Err(ArgumentsError::NotAList),
    }
}

#[cfg(test)]
mod tests {
    use exs_abi::image::{IMAGE_MAGIC, RECORD_LIST};
    use exs_abi::{BoundaryError, BoundaryLimits, ExsValue};

    use super::{ArgumentsError, decode_arguments};

    /// Returns the image of `value`.
    fn image(value: &ExsValue) -> Vec<u8> {
        value
            .to_image_with_limits(BoundaryLimits::unrestricted())
            .expect("image")
    }

    /// Decodes an ordered argument List.
    #[test]
    fn decodes_ordered_arguments() {
        let arguments = vec![ExsValue::Int(1), ExsValue::String("Ada".to_owned())];
        assert_eq!(
            decode_arguments(
                &image(&ExsValue::List(arguments.clone())),
                BoundaryLimits::default()
            ),
            Ok(arguments)
        );
    }

    /// Rejects a request whose root is no argument List.
    #[test]
    fn rejects_non_list_arguments() {
        assert_eq!(
            decode_arguments(&image(&ExsValue::Int(1)), BoundaryLimits::default()),
            Err(ArgumentsError::NotAList)
        );
    }

    /// Applies the collection limit before reading a hostile declared argument count.
    #[test]
    fn rejects_a_hostile_argument_count() {
        let mut bytes = Vec::new();
        for word in [IMAGE_MAGIC, 8, RECORD_LIST, u32::MAX] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(
            decode_arguments(&bytes, BoundaryLimits::default()),
            Err(ArgumentsError::Invalid(
                BoundaryError::CollectionLimitExceeded
            ))
        );
    }
}
