//! Converts between ExS values and JSON text.
//!
//! JSON objects become Objects with their keys in document order, arrays become Lists, whole
//! numbers that fit an Int become Ints, all other numbers Floats, and `null` becomes None.

/// Parses JSON text into an ExS value.
///
/// Invalid JSON returns a `JsonSyntax` Error that names the line and column.
///
/// ```exs
/// fn main() -> Int | Error {
///     let order = json::parse("{\"id\": 7, \"items\": [1, 2]}")?;
///     ret order.id;
/// }
/// ```
fn parse(text: String) -> Any | Error { ret Host::call("json.parse", text); }

/// Writes a value as compact JSON text.
///
/// Bytes, enums, Errors, and Floats that are not finite have no JSON form and return a
/// `JsonUnsupported` Error.
///
/// ```exs
/// fn main() -> String | Error {
///     ret json::stringify({ id: 7, tags: ["new"] });
/// }
/// ```
fn stringify(value: Any) -> String | Error { ret Host::call("json.stringify", value); }

/// Writes a value as JSON text indented by two spaces per level.
///
/// The same values as for `stringify` have no JSON form.
fn pretty(value: Any) -> String | Error { ret Host::call("json.pretty", value); }
