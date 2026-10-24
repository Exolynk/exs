//! Parsing JSON text into ExS values.

use std::fmt;

use exs_abi::ExsValue;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use crate::error::{error, string_argument};

/// Host function `json.parse`: parses its String argument as JSON.
pub(super) fn parse(arguments: Vec<ExsValue>) -> ExsValue {
    let text = match string_argument("json::parse", arguments) {
        Ok(text) => text,
        Err(error) => return error,
    };
    match serde_json::from_str::<JsonValue>(&text) {
        Ok(JsonValue(value)) => value,
        Err(cause) => error("JsonSyntax", format!("invalid JSON: {cause}")),
    }
}

/// An ExS value read from JSON; objects keep their keys in document order.
struct JsonValue(ExsValue);

impl<'de> Deserialize<'de> for JsonValue {
    /// Reads any JSON value.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor).map(JsonValue)
    }
}

/// Builds ExS values from the events of a JSON deserializer.
struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = ExsValue;

    /// Describes the expected input in deserializer errors.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    /// Reads `true` or `false`.
    fn visit_bool<E>(self, value: bool) -> Result<ExsValue, E> {
        Ok(ExsValue::Bool(value))
    }

    /// Reads a negative whole number.
    fn visit_i64<E>(self, value: i64) -> Result<ExsValue, E> {
        Ok(ExsValue::Int(value))
    }

    /// Reads a non-negative whole number; one above the Int range becomes a Float.
    fn visit_u64<E>(self, value: u64) -> Result<ExsValue, E> {
        Ok(i64::try_from(value).map_or(ExsValue::Float(value as f64), ExsValue::Int))
    }

    /// Reads a number with a fraction or exponent.
    fn visit_f64<E>(self, value: f64) -> Result<ExsValue, E> {
        Ok(ExsValue::Float(value))
    }

    /// Reads a string.
    fn visit_str<E>(self, value: &str) -> Result<ExsValue, E> {
        Ok(ExsValue::String(value.to_owned()))
    }

    /// Reads an owned string.
    fn visit_string<E>(self, value: String) -> Result<ExsValue, E> {
        Ok(ExsValue::String(value))
    }

    /// Reads `null`.
    fn visit_unit<E>(self) -> Result<ExsValue, E> {
        Ok(ExsValue::None)
    }

    /// Reads an array.
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<ExsValue, A::Error> {
        let mut items = Vec::with_capacity(sequence.size_hint().unwrap_or_default());
        while let Some(JsonValue(item)) = sequence.next_element()? {
            items.push(item);
        }
        Ok(ExsValue::List(items))
    }

    /// Reads an object; a repeated key keeps its first position and its last value.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ExsValue, A::Error> {
        let mut entries: Vec<(String, ExsValue)> = Vec::new();
        while let Some((key, JsonValue(value))) = map.next_entry::<String, JsonValue>()? {
            match entries.iter_mut().find(|(existing, _)| *existing == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }
        Ok(ExsValue::Object(entries))
    }
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::parse;

    /// Parses the JSON value kinds into their ExS counterparts in document order.
    #[test]
    fn parses_json_values() {
        let value = parse(vec![ExsValue::String(
            r#"{"b": [1, -2, 1.5, 18446744073709551615], "a": {"x": null, "y": true}, "s": "ü"}"#
                .to_owned(),
        )]);
        assert_eq!(
            value,
            ExsValue::Object(vec![
                (
                    "b".to_owned(),
                    ExsValue::List(vec![
                        ExsValue::Int(1),
                        ExsValue::Int(-2),
                        ExsValue::Float(1.5),
                        ExsValue::Float(18_446_744_073_709_551_615.0),
                    ])
                ),
                (
                    "a".to_owned(),
                    ExsValue::Object(vec![
                        ("x".to_owned(), ExsValue::None),
                        ("y".to_owned(), ExsValue::Bool(true)),
                    ])
                ),
                ("s".to_owned(), ExsValue::String("ü".to_owned())),
            ])
        );
    }

    /// Reports invalid JSON as a recoverable error with its position.
    #[test]
    fn reports_syntax_errors() {
        let ExsValue::Error(error) = parse(vec![ExsValue::String("{\"a\": }".to_owned())]) else {
            panic!("invalid JSON must return an error");
        };
        assert_eq!(error.kind, "JsonSyntax");
        assert!(
            error.message.contains("line 1 column 7"),
            "{}",
            error.message
        );
    }

    /// Rejects a call without a String argument.
    #[test]
    fn rejects_non_string_arguments() {
        let ExsValue::Error(error) = parse(vec![ExsValue::Int(1)]) else {
            panic!("a non-string argument must return an error");
        };
        assert_eq!(error.kind, "TypeError");
    }
}
