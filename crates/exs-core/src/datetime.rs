//! Fixed-offset RFC 3339 parsing for the standard DateTime prelude.
//!
//! The prelude's `__exs_datetime_parse_rfc3339_runtime` is an intrinsic: the compiler lowers
//! its calls to [`datetime_parse`] and builds the Object of raw DateTime fields itself.

use exs_layout::ValueRef;

use crate::errors::recoverable;
use crate::memory::Memory;
use crate::numbers::int_value;
use crate::strings::{string_object, text};

/// Number of raw DateTime fields [`datetime_parse`] writes: `unix_seconds`, `nanoseconds`,
/// `utc_offset_seconds` and `timezone`.
pub const DATETIME_FIELDS: u32 = 4;

/// Parses the String `raw` as ExS's strict RFC 3339 spelling.
///
/// On success writes the raw fields into the [`DATETIME_FIELDS`] rooted slots at `slots` and
/// returns None; otherwise returns the TypeError or ParseError. Returns `None` when memory
/// is exhausted.
pub fn datetime_parse(memory: &mut impl Memory, vm: u32, raw: u64, slots: u32) -> Option<u64> {
    let Some(object) = string_object(raw) else {
        return recoverable(
            memory,
            vm,
            "TypeError",
            "DateTime::parse_rfc3339 requires a String value",
            raw,
        );
    };
    let Some((year, month, day, hour, minute, second, nanoseconds, offset)) =
        parse(text(memory, object).as_bytes())
    else {
        return recoverable(
            memory,
            vm,
            "ParseError",
            "DateTime::parse_rfc3339 requires strict RFC 3339 text",
            raw,
        );
    };
    let days = days_from_civil(year, month, day);
    let seconds_of_day = i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    let unix_seconds = days * 86_400 + seconds_of_day - i64::from(offset);
    let fields = [unix_seconds, i64::from(nanoseconds), i64::from(offset)];
    for (index, value) in (0_u32..).zip(fields) {
        let value = int_value(memory, vm, value)?;
        memory.store_u64(slots + index * 8, value);
    }
    memory.store_u64(slots + 3 * 8, ValueRef::NONE.raw());
    Some(ValueRef::NONE.raw())
}

/// The fields of one parsed date: year, month, day, hour, minute, second, nanoseconds and
/// the UTC offset in seconds.
type Parsed = (i32, i32, i32, i32, i32, i32, i32, i32);

/// Validates and decodes the strict RFC 3339 components accepted by ExS.
fn parse(bytes: &[u8]) -> Option<Parsed> {
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = digits(bytes, 0, 4)?;
    let month = digits(bytes, 5, 2)?;
    let day = digits(bytes, 8, 2)?;
    let hour = digits(bytes, 11, 2)?;
    let minute = digits(bytes, 14, 2)?;
    let second = digits(bytes, 17, 2)?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut index = 19;
    let mut nanoseconds = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while let Some(byte) = bytes.get(index) {
            if !byte.is_ascii_digit() {
                break;
            }
            if index - fraction_start == 9 {
                return None;
            }
            nanoseconds = nanoseconds * 10 + i32::from(byte - b'0');
            index += 1;
        }
        let fraction_digits = index - fraction_start;
        if fraction_digits == 0 {
            return None;
        }
        for _ in fraction_digits..9 {
            nanoseconds *= 10;
        }
    }
    let offset = match bytes.get(index) {
        Some(b'Z' | b'z') if index + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-'))
            if index + 6 == bytes.len() && bytes.get(index + 3) == Some(&b':') =>
        {
            let hours = digits(bytes, index + 1, 2)?;
            let minutes = digits(bytes, index + 4, 2)?;
            let offset = hours * 3_600 + minutes * 60;
            if offset >= 86_400 {
                return None;
            }
            if *sign == b'-' { -offset } else { offset }
        }
        _ => return None,
    };
    Some((year, month, day, hour, minute, second, nanoseconds, offset))
}

/// Decodes one fixed-width ASCII decimal field.
fn digits(bytes: &[u8], start: usize, width: usize) -> Option<i32> {
    let slice = bytes.get(start..start.checked_add(width)?)?;
    let mut value = 0;
    for byte in slice {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i32::from(byte - b'0');
    }
    Some(value)
}

/// Returns the days in one proleptic Gregorian month.
fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Returns whether one proleptic Gregorian year is a leap year.
fn is_leap_year(year: i32) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

/// Converts one validated civil date to days since 1970-01-01.
fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let year = i64::from(year - i32::from(month <= 2));
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let march_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * i64::from(march_month) + 2).div_euclid(5) + i64::from(day) - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era.div_euclid(4) - year_of_era.div_euclid(100) + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
