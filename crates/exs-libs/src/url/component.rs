//! Percent-encoding of single URL parts.

use exs_abi::ExsValue;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::error::{error, string_argument};

/// Characters encoded in a URL part: everything except the unreserved characters of RFC 3986.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Host function `url.encode`: percent-encodes text for one URL part.
pub(super) fn encode(arguments: Vec<ExsValue>) -> ExsValue {
    match string_argument("url::encode", arguments) {
        Ok(text) => ExsValue::String(utf8_percent_encode(&text, COMPONENT).to_string()),
        Err(error) => error,
    }
}

/// Host function `url.decode`: decodes percent-encoded text.
pub(super) fn decode(arguments: Vec<ExsValue>) -> ExsValue {
    let text = match string_argument("url::decode", arguments) {
        Ok(text) => text,
        Err(error) => return error,
    };
    match percent_decode_str(&text).decode_utf8() {
        Ok(decoded) => ExsValue::String(decoded.into_owned()),
        Err(cause) => error(
            "UrlDecode",
            format!("the decoded text is not UTF-8: {cause}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::{decode, encode};

    /// Encodes reserved characters and spaces but keeps unreserved ones.
    #[test]
    fn encodes_url_parts() {
        assert_eq!(
            encode(vec![ExsValue::String("a b/ü-._~".to_owned())]),
            ExsValue::String("a%20b%2F%C3%BC-._~".to_owned())
        );
    }

    /// Decodes percent-encoded text and keeps `+` as it is.
    #[test]
    fn decodes_url_parts() {
        assert_eq!(
            decode(vec![ExsValue::String("a%20b%2F%C3%BC+".to_owned())]),
            ExsValue::String("a b/ü+".to_owned())
        );
    }

    /// Reports decoded text that is not UTF-8.
    #[test]
    fn rejects_invalid_utf8() {
        let ExsValue::Error(error) = decode(vec![ExsValue::String("%FF".to_owned())]) else {
            panic!("invalid UTF-8 must return an error");
        };
        assert_eq!(error.kind, "UrlDecode");
    }
}
