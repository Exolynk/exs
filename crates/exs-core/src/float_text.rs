//! The text of Floats, exactly like Rust's `{}` without lookup tables.
//!
//! `core::fmt` formats floats with Grisu and its table of cached powers of ten, and every byte
//! of the core template's data costs fuel on every execution. This formatter produces the same
//! text with Steele and White's shortest-digit algorithm on exact big integers, as Rust's
//! fallback does: the shortest digits that read back as the same Float, rounded to the
//! nearest, in positional notation.

use core::cmp::Ordering;

use crate::big::Big;

/// Byte capacity of a Float's text: a sign, `0.`, 323 zeros and 17 digits for the smallest
/// subnormal, or 309 digits for the largest finite Float.
pub(crate) const MAX_LEN: usize = 352;

/// One Float decoded into the integers of its rounding interval, like Rust's `flt2dec`.
struct Decoded {
    /// The scaled mantissa: the value is `mant × 2^exp`.
    mant: u64,
    /// Distance to the lower boundary of the rounding interval, in units of `2^exp`.
    minus: u64,
    /// Distance to the upper boundary of the rounding interval, in units of `2^exp`.
    plus: u64,
    /// The binary exponent.
    exp: i64,
    /// Whether the boundaries themselves read back as the value, for an even mantissa.
    inclusive: bool,
}

/// Decodes the finite, nonzero, positive `value`.
fn decode(value: f64) -> Decoded {
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (mant, exp) = if biased == 0 {
        (fraction << 1, -1075)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let inclusive = mant & 1 == 0;
    if biased == 0 {
        Decoded {
            mant,
            minus: 1,
            plus: 1,
            exp,
            inclusive,
        }
    } else if mant == 1 << 52 {
        // A power of two has a closer lower neighbor than upper one.
        Decoded {
            mant: mant << 2,
            minus: 1,
            plus: 2,
            exp: exp - 2,
            inclusive,
        }
    } else {
        Decoded {
            mant: mant << 1,
            minus: 1,
            plus: 1,
            exp: exp - 1,
            inclusive,
        }
    }
}

/// Returns an estimate of `ceil(log10(mant × 2^exp))` that is exact or one too low.
fn estimate_scaling_factor(mant: u64, exp: i64) -> i64 {
    let bits = 64 - i64::from((mant - 1).leading_zeros());
    // 1292913986 = floor(2^32 × log10(2)), so the estimate never exceeds the exact value.
    ((bits + exp) * 1_292_913_986) >> 32
}

/// Returns whether `left` compares below `right`, or equal to it for an inclusive interval.
fn below(left: &Big, right: &Big, inclusive: bool) -> bool {
    match left.compare(right) {
        Ordering::Less => true,
        Ordering::Equal => inclusive,
        Ordering::Greater => false,
    }
}

/// Writes the shortest digits of the finite, nonzero, positive `value` into `digits` and
/// returns their count and the decimal exponent `k`, so that the value reads `0.digits × 10^k`.
fn shortest(value: f64, digits: &mut [u8; 18]) -> (usize, i64) {
    let decoded = decode(value);
    let inclusive = decoded.inclusive;
    let mut k = estimate_scaling_factor(decoded.mant + decoded.plus, decoded.exp);
    let mut mant = Big::from_u64(decoded.mant);
    let mut minus = Big::from_u64(decoded.minus);
    let mut plus = Big::from_u64(decoded.plus);
    let mut scale = Big::from_small(1);
    if decoded.exp < 0 {
        scale.shl(decoded.exp.unsigned_abs());
    } else {
        let shift = decoded.exp.unsigned_abs();
        mant.shl(shift);
        minus.shl(shift);
        plus.shl(shift);
    }
    if k >= 0 {
        scale.mul_pow10(k.unsigned_abs());
    } else {
        let power = k.unsigned_abs();
        mant.mul_pow10(power);
        minus.mul_pow10(power);
        plus.mul_pow10(power);
    }
    // Fix the estimate: afterwards `scale < mant + plus <= 10 × scale`.
    let mut high = mant.clone();
    high.add(&plus);
    if below(&scale, &high, inclusive) {
        k += 1;
    } else {
        mant.mul_small(10);
        minus.mul_small(10);
        plus.mul_small(10);
    }
    let mut scale2 = scale.clone();
    scale2.shl(1);
    let mut scale4 = scale.clone();
    scale4.shl(2);
    let mut scale8 = scale.clone();
    scale8.shl(3);
    let mut count = 0;
    let (down, up) = loop {
        let mut digit = 0;
        for (multiple, weight) in [(&scale8, 8), (&scale4, 4), (&scale2, 2), (&scale, 1)] {
            if mant.compare(multiple) != Ordering::Less {
                mant.sub(multiple);
                digit += weight;
            }
        }
        if let Some(slot) = digits.get_mut(count) {
            *slot = b'0' + digit;
        }
        count += 1;
        let down = below(&mant, &minus, inclusive);
        let mut high = mant.clone();
        high.add(&plus);
        let up = below(&scale, &high, inclusive);
        if down || up || count >= 17 {
            break (down, up);
        }
        mant.mul_small(10);
        minus.mul_small(10);
        plus.mul_small(10);
    };
    // Round up when only the upper neighbor is in reach or the rest is at least one half.
    let mut doubled = mant;
    doubled.shl(1);
    if up && (!down || doubled.compare(&scale) != Ordering::Less) {
        let used = digits.get_mut(..count).unwrap_or(&mut []);
        match used.iter().rposition(|digit| *digit != b'9') {
            Some(index) => {
                for (position, digit) in used.iter_mut().enumerate().skip(index) {
                    *digit = if position == index { *digit + 1 } else { b'0' };
                }
            }
            None => {
                // All nines: the value rounds up to the next power of ten.
                for (position, digit) in digits.iter_mut().enumerate().take(count + 1) {
                    *digit = if position == 0 { b'1' } else { b'0' };
                }
                count += 1;
                k += 1;
            }
        }
    }
    (count, k)
}

/// Writes the text of `value` like Rust's `{}` into `text` and returns its length.
pub(crate) fn float_text(value: f64, text: &mut [u8; MAX_LEN]) -> usize {
    if value.is_nan() {
        for (slot, byte) in text.iter_mut().zip(b"NaN") {
            *slot = *byte;
        }
        return 3;
    }
    let mut len = 0;
    let mut put = |bytes: &[u8], len: &mut usize| {
        for (slot, byte) in text.iter_mut().skip(*len).zip(bytes) {
            *slot = *byte;
        }
        *len += bytes.len();
    };
    if value.is_sign_negative() {
        put(b"-", &mut len);
    }
    if value.is_infinite() {
        put(b"inf", &mut len);
        return len;
    }
    if value == 0.0 {
        put(b"0", &mut len);
        return len;
    }
    let mut digits = [0_u8; 18];
    let (count, k) = shortest(value.abs(), &mut digits);
    let digits = digits.get(..count).unwrap_or(&[]);
    let mut push = |bytes: &[u8], len: &mut usize| {
        for (slot, byte) in text.iter_mut().skip(*len).zip(bytes) {
            *slot = *byte;
        }
        *len += bytes.len();
    };
    if k <= 0 {
        push(b"0.", &mut len);
        for _ in 0..k.unsigned_abs() {
            push(b"0", &mut len);
        }
        push(digits, &mut len);
    } else if (k as usize) < count {
        let (integer, fraction) = digits.split_at_checked(k as usize).unwrap_or((digits, &[]));
        push(integer, &mut len);
        push(b".", &mut len);
        push(fraction, &mut len);
    } else {
        push(digits, &mut len);
        for _ in count..k as usize {
            push(b"0", &mut len);
        }
    }
    len
}
