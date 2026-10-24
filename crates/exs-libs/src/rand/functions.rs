//! The host functions of the `rand` library.

use exs_abi::ExsValue;

use super::source::{fill, next_u64};
use crate::error::error;

/// Longest text or Bytes value `text` and `bytes` produce.
const MAX_LENGTH: i64 = 1_048_576;
/// Characters of the texts `text` produces.
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Host function `rand.int`: a random Int from `min` to `max`, both included.
pub(super) fn int(arguments: Vec<ExsValue>) -> ExsValue {
    let (min, max) = match arguments.as_slice() {
        [ExsValue::Int(min), ExsValue::Int(max)] => (*min, *max),
        _ => return error("TypeError", "rand::int expects two Int arguments"),
    };
    if min > max {
        return error(
            "ValueError",
            format!("rand::int: min {min} is above max {max}"),
        );
    }
    // Wrapping keeps the full Int range representable as an unsigned span.
    let span = max.wrapping_sub(min) as u64;
    let value = if span == u64::MAX {
        next_u64()
    } else {
        uniform_below(span + 1)
    };
    match value {
        Ok(offset) => ExsValue::Int(min.wrapping_add(offset as i64)),
        Err(cause) => unavailable(&cause),
    }
}

/// Returns a uniformly distributed number below `bound`, rejecting draws that would favor
/// small values.
fn uniform_below(bound: u64) -> Result<u64, String> {
    let zone = u64::MAX - u64::MAX % bound;
    loop {
        let draw = next_u64()?;
        if draw < zone {
            return Ok(draw % bound);
        }
    }
}

/// Host function `rand.float`: a random Float from `min` included to `max` excluded.
pub(super) fn float(arguments: Vec<ExsValue>) -> ExsValue {
    let number = |value: &ExsValue| match value {
        ExsValue::Int(value) => Some(*value as f64),
        ExsValue::Float(value) => Some(*value),
        _ => None,
    };
    let (min, max) = match arguments.as_slice() {
        [min, max] => match (number(min), number(max)) {
            (Some(min), Some(max)) => (min, max),
            _ => return error("TypeError", "rand::float expects two numbers"),
        },
        _ => return error("TypeError", "rand::float expects two numbers"),
    };
    if !min.is_finite() || !max.is_finite() {
        return error("ValueError", "rand::float expects finite bounds");
    }
    if min > max {
        return error(
            "ValueError",
            format!("rand::float: min {min} is above max {max}"),
        );
    }
    if min == max {
        return ExsValue::Float(min);
    }
    match next_u64() {
        Ok(draw) => {
            // The top 53 bits give an evenly spaced fraction in [0, 1).
            let fraction = (draw >> 11) as f64 / (1_u64 << 53) as f64;
            let value = min + fraction * (max - min);
            ExsValue::Float(if value < max { value } else { max.next_down() })
        }
        Err(cause) => unavailable(&cause),
    }
}

/// Host function `rand.text`: a random text of letters and digits.
pub(super) fn text(arguments: Vec<ExsValue>) -> ExsValue {
    let length = match length("rand::text", &arguments) {
        Ok(length) => length,
        Err(error) => return error,
    };
    // 248 is the largest multiple of the alphabet size below 256; higher bytes are redrawn.
    let limit = (256 / ALPHABET.len() * ALPHABET.len()) as u8;
    let mut text = String::with_capacity(length);
    let mut buffer = vec![0_u8; length.max(16)];
    while text.len() < length {
        if let Err(cause) = fill(&mut buffer) {
            return unavailable(&cause);
        }
        for byte in buffer.iter().filter(|byte| **byte < limit) {
            if text.len() == length {
                break;
            }
            text.push(char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]));
        }
    }
    ExsValue::String(text)
}

/// Host function `rand.bytes`: random Bytes.
pub(super) fn bytes(arguments: Vec<ExsValue>) -> ExsValue {
    let length = match length("rand::bytes", &arguments) {
        Ok(length) => length,
        Err(error) => return error,
    };
    let mut bytes = vec![0_u8; length];
    match fill(&mut bytes) {
        Ok(()) => ExsValue::Bytes(bytes),
        Err(cause) => unavailable(&cause),
    }
}

/// Returns the single length argument of `function` when it is from 0 to the maximum length.
fn length(function: &str, arguments: &[ExsValue]) -> Result<usize, ExsValue> {
    match arguments {
        [ExsValue::Int(length)] if (0..=MAX_LENGTH).contains(length) => {
            usize::try_from(*length).map_err(|_| error("ValueError", "length is out of range"))
        }
        [ExsValue::Int(length)] => Err(error(
            "ValueError",
            format!("{function}: length {length} is not from 0 to {MAX_LENGTH}"),
        )),
        _ => Err(error(
            "TypeError",
            format!("{function} expects one Int argument"),
        )),
    }
}

/// Returns the error for a failed random source.
fn unavailable(cause: &str) -> ExsValue {
    error(
        "RandomUnavailable",
        format!("the random source failed: {cause}"),
    )
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::{bytes, float, int, text};

    /// Returns the kind of an error value.
    fn kind(value: ExsValue) -> String {
        match value {
            ExsValue::Error(error) => error.kind,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    /// Draws Ints inside the inclusive range, including both bounds over many draws.
    #[test]
    fn draws_ints_in_range() {
        let mut seen = [false; 3];
        for _ in 0..500 {
            let ExsValue::Int(value) = int(vec![ExsValue::Int(-1), ExsValue::Int(1)]) else {
                panic!("rand::int must return an Int");
            };
            assert!((-1..=1).contains(&value));
            seen[(value + 1) as usize] = true;
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(
            int(vec![ExsValue::Int(5), ExsValue::Int(5)]),
            ExsValue::Int(5)
        );
        assert!(matches!(
            int(vec![ExsValue::Int(i64::MIN), ExsValue::Int(i64::MAX)]),
            ExsValue::Int(_)
        ));
        assert_eq!(
            kind(int(vec![ExsValue::Int(2), ExsValue::Int(1)])),
            "ValueError"
        );
    }

    /// Draws Floats from the half-open range and accepts Int bounds.
    #[test]
    fn draws_floats_in_range() {
        for _ in 0..500 {
            let ExsValue::Float(value) = float(vec![ExsValue::Int(1), ExsValue::Float(1.5)]) else {
                panic!("rand::float must return a Float");
            };
            assert!((1.0..1.5).contains(&value));
        }
        assert_eq!(
            float(vec![ExsValue::Float(2.0), ExsValue::Float(2.0)]),
            ExsValue::Float(2.0)
        );
        assert_eq!(
            kind(float(vec![ExsValue::Float(f64::NAN), ExsValue::Float(1.0)])),
            "ValueError"
        );
        assert_eq!(
            kind(float(vec![ExsValue::Int(3), ExsValue::Int(1)])),
            "ValueError"
        );
    }

    /// Produces texts of the requested length from letters and digits.
    #[test]
    fn produces_texts() {
        let ExsValue::String(value) = text(vec![ExsValue::Int(40)]) else {
            panic!("rand::text must return a String");
        };
        assert_eq!(value.len(), 40);
        assert!(value.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        assert_eq!(
            text(vec![ExsValue::Int(0)]),
            ExsValue::String(String::new())
        );
        assert_eq!(kind(text(vec![ExsValue::Int(-1)])), "ValueError");
    }

    /// Produces Bytes of the requested length and rejects lengths above the maximum.
    #[test]
    fn produces_bytes() {
        let ExsValue::Bytes(value) = bytes(vec![ExsValue::Int(32)]) else {
            panic!("rand::bytes must return Bytes");
        };
        assert_eq!(value.len(), 32);
        assert_eq!(kind(bytes(vec![ExsValue::Int(1_048_577)])), "ValueError");
        assert_eq!(kind(bytes(vec![ExsValue::String("1".into())])), "TypeError");
    }
}
