//! Exact conversion of decimal text to binary64 without lookup tables.
//!
//! `f64::from_str` carries a 10 KiB table of powers of five, and every byte of the core
//! template's data costs fuel on every execution. This parser accepts exactly the same syntax
//! and returns bit-identical results: short inputs take Clinger's exact fast path, all others
//! an exact big-integer division with round-half-to-even.

use core::cmp::Ordering;

use crate::big::Big;

/// Significant decimal digits kept; any rounding decision of a binary64 value needs at most
/// 767, and dropped digits are summarized in a sticky flag.
const MAX_DIGITS: usize = 800;
/// Binary exponent of the guard bit of the smallest subnormal, `2^-1075`.
const MIN_SCALE: i64 = 1075;

/// Parses `text` like `f64::from_str`, returning `None` where it returns an error.
pub(crate) fn parse_f64(text: &[u8]) -> Option<f64> {
    let (negative, rest) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, text),
    };
    let magnitude = special(rest).or_else(|| decimal(rest))?;
    Some(if negative { -magnitude } else { magnitude })
}

/// Returns infinity or NaN for their case-insensitive spellings.
fn special(text: &[u8]) -> Option<f64> {
    let matches = |word: &[u8]| text.eq_ignore_ascii_case(word);
    if matches(b"inf") || matches(b"infinity") {
        Some(f64::INFINITY)
    } else if matches(b"nan") {
        Some(f64::NAN)
    } else {
        None
    }
}

/// The significant digits of a decimal number: `digits[..count] × 10^scale`.
struct Decimal {
    /// Significant digits without leading zeros, each 0 to 9.
    digits: [u8; MAX_DIGITS],
    /// Number of digits kept.
    count: usize,
    /// Power of ten applied to the kept digits.
    scale: i64,
    /// Whether nonzero digits beyond `MAX_DIGITS` were dropped.
    truncated: bool,
}

/// Parses `digits[.digits][(e|E)[sign]digits]` and converts it.
fn decimal(text: &[u8]) -> Option<f64> {
    let mut number = Decimal {
        digits: [0; MAX_DIGITS],
        count: 0,
        scale: 0,
        truncated: false,
    };
    let mut position = 0;
    let mut seen = 0_usize;
    let mut after_point = false;
    while let Some(&byte) = text.get(position) {
        match byte {
            b'0'..=b'9' => {
                seen += 1;
                number.push(byte - b'0', after_point);
            }
            b'.' if !after_point => after_point = true,
            _ => break,
        }
        position += 1;
    }
    if seen == 0 {
        return None;
    }
    if let Some(b'e' | b'E') = text.get(position) {
        position += 1;
        let negative = match text.get(position) {
            Some(b'-') => {
                position += 1;
                true
            }
            Some(b'+') => {
                position += 1;
                false
            }
            _ => false,
        };
        let start = position;
        let mut exponent = 0_i64;
        while let Some(byte @ b'0'..=b'9') = text.get(position) {
            // Saturating far beyond any representable magnitude keeps the result exact.
            exponent = (exponent * 10 + i64::from(byte - b'0')).min(1 << 40);
            position += 1;
        }
        if position == start {
            return None;
        }
        number.scale += if negative { -exponent } else { exponent };
    }
    if position != text.len() {
        return None;
    }
    Some(number.convert())
}

impl Decimal {
    /// Appends one digit, before or after the decimal point.
    fn push(&mut self, digit: u8, after_point: bool) {
        if self.count == 0 && digit == 0 {
            if after_point {
                self.scale -= 1;
            }
        } else if self.count < MAX_DIGITS {
            self.digits[self.count] = digit;
            self.count += 1;
            if after_point {
                self.scale -= 1;
            }
        } else {
            self.truncated |= digit != 0;
            if !after_point {
                self.scale += 1;
            }
        }
    }

    /// Converts the number to the nearest binary64, ties to even.
    fn convert(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        // The value lies in [10^(magnitude - 1), 10^magnitude).
        let magnitude = self.count as i64 + self.scale;
        if magnitude > 310 {
            return f64::INFINITY;
        }
        if magnitude <= -324 {
            return 0.0;
        }
        if let Some(value) = self.fast_path() {
            return value;
        }
        self.slow_path()
    }

    /// Converts exactly in binary64 arithmetic when both the digits and the power of ten are
    /// exact binary64 values, so the single multiplication or division rounds correctly.
    fn fast_path(&self) -> Option<f64> {
        if self.truncated || self.count > 15 || self.scale.unsigned_abs() > 22 {
            return None;
        }
        let digits = self.digits[..self.count]
            .iter()
            .fold(0_u64, |value, digit| value * 10 + u64::from(*digit));
        let mut power = 1.0_f64;
        for _ in 0..self.scale.unsigned_abs() {
            power *= 10.0;
        }
        let value = digits as f64;
        Some(if self.scale >= 0 {
            value * power
        } else {
            value / power
        })
    }

    /// Converts with exact big-integer arithmetic: the value is `numerator / denominator ×
    /// 2^binary` and its binary64 neighbors are found by one division.
    fn slow_path(&self) -> f64 {
        let mut numerator = Big::default();
        for digit in self.digits.iter().take(self.count) {
            numerator.mul_small(10);
            numerator.add_small(u32::from(*digit));
        }
        let mut denominator = Big::from_small(1);
        if self.scale >= 0 {
            numerator.mul_pow5(self.scale.unsigned_abs());
        } else {
            denominator.mul_pow5(self.scale.unsigned_abs());
        }
        let binary = self.scale;
        // Pick `scale` so that `q = floor(value × 2^scale)` has 54 bits: 53 mantissa bits and
        // one rounding bit; subnormals stop at the scale of the smallest subnormal.
        let estimate = i64::from(numerator.bit_len()) - i64::from(denominator.bit_len()) + binary;
        let mut scale = (54 - estimate).min(MIN_SCALE);
        let (mut quotient, mut remainder) = divide(&numerator, &denominator, binary + scale);
        while quotient >= 1 << 54 {
            scale -= 1;
            (quotient, remainder) = divide(&numerator, &denominator, binary + scale);
        }
        while quotient < 1 << 53 && scale < MIN_SCALE {
            scale += 1;
            (quotient, remainder) = divide(&numerator, &denominator, binary + scale);
        }
        let sticky = remainder || self.truncated;
        let mut mantissa = quotient >> 1;
        if quotient & 1 == 1 && (sticky || mantissa & 1 == 1) {
            mantissa += 1;
        }
        // The mantissa's unit is 2^(1 - scale).
        if mantissa == 1 << 53 {
            mantissa = 1 << 52;
            scale -= 1;
        }
        if mantissa < 1 << 52 {
            // A subnormal or, after rounding up to 2^52, the smallest normal.
            return f64::from_bits(mantissa);
        }
        let biased = 1076 - scale;
        if biased >= 2047 {
            return f64::INFINITY;
        }
        f64::from_bits((biased.cast_unsigned() << 52) | (mantissa - (1 << 52)))
    }
}

/// Returns `floor(numerator × 2^shift / denominator)`, which must be below 2^58, and whether
/// the division left a remainder.
fn divide(numerator: &Big, denominator: &Big, shift: i64) -> (u64, bool) {
    let mut remainder = numerator.clone();
    let mut divisor = denominator.clone();
    if shift >= 0 {
        remainder.shl(shift.unsigned_abs());
    } else {
        divisor.shl(shift.unsigned_abs());
    }
    divisor.shl(58);
    let mut quotient = 0_u64;
    // Each step halves the shifted divisor, testing quotient bits 57 down to 0.
    for _ in 0..58 {
        divisor.shr1();
        quotient <<= 1;
        if remainder.compare(&divisor) != Ordering::Less {
            remainder.sub(&divisor);
            quotient |= 1;
        }
    }
    (quotient, !remainder.is_zero())
}
