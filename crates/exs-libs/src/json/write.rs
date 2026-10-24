//! Writing ExS values as JSON text.

use exs_abi::ExsValue;
use serde::Serialize;
use serde::ser::{Error as _, SerializeMap, SerializeSeq, Serializer};

use crate::error::{error, single_argument};

/// Host function `json.stringify`: writes its argument as compact JSON.
pub(super) fn stringify(arguments: Vec<ExsValue>) -> ExsValue {
    write("json::stringify", arguments, |value| {
        serde_json::to_string(value)
    })
}

/// Host function `json.pretty`: writes its argument as indented JSON.
pub(super) fn pretty(arguments: Vec<ExsValue>) -> ExsValue {
    write("json::pretty", arguments, |value| {
        serde_json::to_string_pretty(value)
    })
}

/// Writes `value` as compact JSON text, or returns why it has no JSON form.
#[cfg(feature = "http")]
pub(crate) fn to_text(value: &ExsValue) -> Result<String, String> {
    serde_json::to_string(&JsonOutput(value)).map_err(|cause| cause.to_string())
}

/// Writes the single argument of `function` with the serde_json `writer`.
fn write(
    function: &str,
    arguments: Vec<ExsValue>,
    writer: fn(&JsonOutput<'_>) -> serde_json::Result<String>,
) -> ExsValue {
    let value = match single_argument(function, arguments) {
        Ok(value) => value,
        Err(error) => return error,
    };
    match writer(&JsonOutput(&value)) {
        Ok(text) => ExsValue::String(text),
        Err(cause) => error("JsonUnsupported", cause.to_string()),
    }
}

/// An ExS value written as JSON; values without a JSON form fail to serialize.
struct JsonOutput<'a>(&'a ExsValue);

impl Serialize for JsonOutput<'_> {
    /// Writes the value, or fails for Bytes, enums, Errors, and non-finite Floats.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ExsValue::None => serializer.serialize_unit(),
            ExsValue::Bool(value) => serializer.serialize_bool(*value),
            ExsValue::Int(value) => serializer.serialize_i64(*value),
            ExsValue::Float(value) if value.is_finite() => serializer.serialize_f64(*value),
            ExsValue::Float(value) => Err(S::Error::custom(format!(
                "the Float {value} has no JSON form"
            ))),
            ExsValue::String(value) => serializer.serialize_str(value),
            ExsValue::List(items) => {
                let mut sequence = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    sequence.serialize_element(&JsonOutput(item))?;
                }
                sequence.end()
            }
            ExsValue::Object(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &JsonOutput(value))?;
                }
                map.end()
            }
            ExsValue::Bytes(_) => Err(S::Error::custom(
                "Bytes have no JSON form; convert them to a String first",
            )),
            ExsValue::Enum { variant, .. } => Err(S::Error::custom(format!(
                "the enum variant `{variant}` has no JSON form"
            ))),
            ExsValue::Error(error) => Err(S::Error::custom(format!(
                "the Error `{}` has no JSON form",
                error.kind
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::{pretty, stringify};

    /// A nested value written by `stringify` and `pretty`.
    fn sample() -> ExsValue {
        ExsValue::Object(vec![
            ("id".to_owned(), ExsValue::Int(7)),
            (
                "tags".to_owned(),
                ExsValue::List(vec![ExsValue::String("a\"b".to_owned()), ExsValue::None]),
            ),
            ("price".to_owned(), ExsValue::Float(1.5)),
            ("ok".to_owned(), ExsValue::Bool(false)),
        ])
    }

    /// Writes compact JSON in the key order of the Object.
    #[test]
    fn writes_compact_json() {
        assert_eq!(
            stringify(vec![sample()]),
            ExsValue::String(r#"{"id":7,"tags":["a\"b",null],"price":1.5,"ok":false}"#.to_owned())
        );
    }

    /// Writes JSON indented by two spaces.
    #[test]
    fn writes_pretty_json() {
        let ExsValue::String(text) = pretty(vec![sample()]) else {
            panic!("pretty must return a String");
        };
        assert!(
            text.starts_with("{\n  \"id\": 7,\n  \"tags\": [\n    \"a\\\"b\","),
            "{text}"
        );
    }

    /// Rejects values without a JSON form.
    #[test]
    fn rejects_values_without_json_form() {
        for value in [
            ExsValue::Bytes(vec![1]),
            ExsValue::Float(f64::NAN),
            ExsValue::List(vec![ExsValue::Float(f64::INFINITY)]),
        ] {
            let ExsValue::Error(error) = stringify(vec![value]) else {
                panic!("a value without JSON form must return an error");
            };
            assert_eq!(error.kind, "JsonUnsupported");
        }
    }
}
