//! Encodes query strings and single parts of URLs.

/// Encodes an Object as a query string in the `application/x-www-form-urlencoded` format.
///
/// Strings, Ints, Floats, and Bools are written as text, a List writes its key once per item,
/// and None leaves the key out. Other values return a `TypeError`.
///
/// ```exs
/// fn main() -> String | Error {
///     ret url::encode_query({ q: "a b", tag: ["x", "y"], page: 2 });
/// }
/// ```
fn encode_query(values: Object) -> String | Error { ret Host::call("url.encode_query", values); }

/// Decodes a query string into an Object of Strings; a leading `?` is ignored, and a key that
/// occurs more than once becomes a List of its values.
///
/// ```exs
/// fn main() -> String | Error {
///     let query = url::decode_query("?q=a+b&tag=x&tag=y")?;
///     ret query.q;
/// }
/// ```
fn decode_query(text: String) -> Object | Error { ret Host::call("url.decode_query", text); }

/// Percent-encodes text for use as one part of a URL, such as a path segment; only letters,
/// digits, and `-`, `.`, `_`, `~` stay as they are.
fn encode(text: String) -> String | Error { ret Host::call("url.encode", text); }

/// Decodes percent-encoded text; a result that is not valid UTF-8 returns a `UrlDecode`
/// Error.
fn decode(text: String) -> String | Error { ret Host::call("url.decode", text); }
