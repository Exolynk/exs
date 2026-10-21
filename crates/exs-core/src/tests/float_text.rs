//! Tests that the table-free Float text matches Rust's `{}` byte for byte.

use std::format;
use std::string::String;

use crate::float_text::{MAX_LEN, float_text};

/// Asserts that `value` renders exactly like Rust's `{}`.
fn check(value: f64) {
    let mut text = [0_u8; MAX_LEN];
    let len = float_text(value, &mut text);
    let actual = String::from_utf8_lossy(&text[..len]);
    assert_eq!(
        actual,
        format!("{value}"),
        "formatting {:#x}",
        value.to_bits()
    );
}

/// A small deterministic pseudo-random generator.
struct XorShift(u64);

impl XorShift {
    /// Returns the next pseudo-random number.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Matches Rust on special values, boundaries, powers and hard cases.
#[test]
fn matches_std_on_edge_cases() {
    let mut values = std::vec![
        0.0,
        -0.0,
        f64::NAN,
        -f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
        f64::EPSILON,
        f64::from_bits(1),
        f64::from_bits(2),
        f64::from_bits((1 << 52) - 1),
        f64::from_bits(1 << 52),
        f64::from_bits((1 << 52) + 1),
        1.0,
        0.1,
        0.2,
        0.3,
        0.1 + 0.2,
        1.5,
        2.5,
        100.0,
        1e15,
        1e16,
        1e17,
        1e21,
        1e22,
        1e23,
        1e-5,
        1e-7,
        5e-324,
        9007199254740993.0,
        123_456.789,
        0.000_001_234_5,
        2.0_f64.powi(-1022),
        2.0_f64.powi(1023),
        299_792_458.0,
        core::f64::consts::PI,
    ];
    for exponent in -1074..=1023 {
        let power = 2.0_f64.powi(exponent);
        values.extend([power, power * 3.0, power * 0.75, -power]);
    }
    for exponent in -325..=308 {
        let text = format!("1e{exponent}");
        let parsed: f64 = text.parse().unwrap_or(0.0);
        values.extend([parsed, f64::from_bits(parsed.to_bits().wrapping_add(1))]);
        values.push(f64::from_bits(parsed.to_bits().wrapping_sub(1)));
    }
    for value in values {
        check(value);
    }
}

/// Matches Rust on a million random bit patterns and random short decimals.
#[test]
fn matches_std_on_random_values() {
    let mut random = XorShift(0x9e37_79b9_7f4a_7c15);
    for _ in 0..1_000_000 {
        check(f64::from_bits(random.next()));
    }
    for _ in 0..200_000 {
        let digits = random.next() % 1_000_000;
        let exponent = (random.next() % 40) as i32 - 20;
        let text = format!("{digits}e{exponent}");
        let parsed: f64 = text.parse().unwrap_or(0.0);
        check(parsed);
    }
}
