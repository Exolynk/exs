//! Query strings in the `application/x-www-form-urlencoded` format.

use exs_abi::ExsValue;

use crate::error::{error, single_argument, string_argument};

/// Host function `url.encode_query`: encodes an Object as a query string.
pub(super) fn encode_query(arguments: Vec<ExsValue>) -> ExsValue {
    let entries = match single_argument("url::encode_query", arguments) {
        Ok(ExsValue::Object(entries)) => entries,
        Ok(_) => {
            return error("TypeError", "url::encode_query expects an Object argument");
        }
        Err(error) => return error,
    };
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (key, value) in &entries {
        let values = match value {
            ExsValue::List(items) => items.iter().collect::<Vec<_>>(),
            value => vec![value],
        };
        for value in values {
            match text(value) {
                Ok(Some(text)) => {
                    serializer.append_pair(key, &text);
                }
                Ok(None) => {}
                Err(()) => {
                    return error(
                        "TypeError",
                        format!(
                            "url::encode_query cannot encode the value of `{key}`; use Strings, Ints, Floats, Bools, None, or Lists of them"
                        ),
                    );
                }
            }
        }
    }
    ExsValue::String(serializer.finish())
}

/// Returns the query text of a scalar value, `None` for an omitted value, or `Err` for a value
/// without a query form.
fn text(value: &ExsValue) -> Result<Option<String>, ()> {
    match value {
        ExsValue::None => Ok(None),
        ExsValue::String(text) => Ok(Some(text.clone())),
        ExsValue::Int(number) => Ok(Some(number.to_string())),
        ExsValue::Float(number) => Ok(Some(number.to_string())),
        ExsValue::Bool(flag) => Ok(Some(flag.to_string())),
        _ => Err(()),
    }
}

/// Host function `url.decode_query`: decodes a query string into an Object of Strings.
pub(super) fn decode_query(arguments: Vec<ExsValue>) -> ExsValue {
    let query = match string_argument("url::decode_query", arguments) {
        Ok(query) => query,
        Err(error) => return error,
    };
    let query = query.strip_prefix('?').unwrap_or(&query);
    let mut entries: Vec<(String, ExsValue)> = Vec::new();
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        let value = ExsValue::String(value.into_owned());
        match entries.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, ExsValue::List(values))) => values.push(value),
            Some((_, existing)) => {
                let first = std::mem::replace(existing, ExsValue::None);
                *existing = ExsValue::List(vec![first, value]);
            }
            None => entries.push((key.into_owned(), value)),
        }
    }
    ExsValue::Object(entries)
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::{decode_query, encode_query};

    /// Shorthand for a String value.
    fn string(text: &str) -> ExsValue {
        ExsValue::String(text.to_owned())
    }

    /// Encodes scalars, repeats keys for Lists, and leaves out None.
    #[test]
    fn encodes_query_strings() {
        let query = encode_query(vec![ExsValue::Object(vec![
            ("q".to_owned(), string("a b&c")),
            (
                "tag".to_owned(),
                ExsValue::List(vec![string("x"), string("y")]),
            ),
            ("page".to_owned(), ExsValue::Int(2)),
            ("ratio".to_owned(), ExsValue::Float(0.5)),
            ("open".to_owned(), ExsValue::Bool(true)),
            ("skip".to_owned(), ExsValue::None),
        ])]);
        assert_eq!(
            query,
            string("q=a+b%26c&tag=x&tag=y&page=2&ratio=0.5&open=true")
        );
    }

    /// Rejects values without a query form.
    #[test]
    fn rejects_nested_values() {
        let ExsValue::Error(error) = encode_query(vec![ExsValue::Object(vec![(
            "nested".to_owned(),
            ExsValue::Object(Vec::new()),
        )])]) else {
            panic!("a nested Object must return an error");
        };
        assert_eq!(error.kind, "TypeError");
    }

    /// Decodes a query string and collects repeated keys into Lists.
    #[test]
    fn decodes_query_strings() {
        assert_eq!(
            decode_query(vec![string("?q=a+b%26c&tag=x&tag=y&tag=z&empty=")]),
            ExsValue::Object(vec![
                ("q".to_owned(), string("a b&c")),
                (
                    "tag".to_owned(),
                    ExsValue::List(vec![string("x"), string("y"), string("z")])
                ),
                ("empty".to_owned(), string("")),
            ])
        );
    }
}
