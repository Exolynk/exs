//! Random values from a cryptographically secure random source.
//!
//! The values are suitable for identifiers and one-time codes; they cannot be reproduced from
//! a seed.

/// Returns a random Int from `min` to `max`, both included.
///
/// A `min` above `max` returns a `ValueError`.
///
/// ```exs
/// fn main() -> Int | Error {
///     ret rand::int(1, 6);
/// }
/// ```
fn int(min: Int, max: Int) -> Int | Error { ret Host::call("rand.int", min, max); }

/// Returns a random Float from `min` included to `max` excluded; equal bounds return `min`.
///
/// Bounds that are not finite or a `min` above `max` return a `ValueError`.
fn float(min: Int | Float, max: Int | Float) -> Float | Error {
    ret Host::call("rand.float", min, max);
}

/// Returns a random text of `length` letters and digits, such as an identifier or a code.
///
/// The length must be from 0 to 1048576; other lengths return a `ValueError`.
fn text(length: Int) -> String | Error { ret Host::call("rand.text", length); }

/// Returns `length` random Bytes.
///
/// The length must be from 0 to 1048576; other lengths return a `ValueError`.
fn bytes(length: Int) -> Bytes | Error { ret Host::call("rand.bytes", length); }
