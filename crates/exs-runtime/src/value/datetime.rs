//! Fixed-offset RFC 3339 parsing used by the standard DateTime prelude.

use alloc::boxed::Box;
use alloc::vec;

use exs_value::ValueRef;

use crate::gc;
use crate::runtime;
use crate::value::{RtValue, RuntimeObject};

/// Parses ExS's strict RFC 3339 DateTime spelling into its raw prelude fields.
pub(crate) fn parse_rfc3339(value: ValueRef) -> ValueRef {
    let receiver = value;
    let RtValue::String(text) = runtime::value(value) else {
        return runtime::recoverable_error(
            "TypeError",
            "DateTime::parse_rfc3339 requires a String value",
            receiver,
        );
    };
    let bytes = text.as_str().as_bytes();
    let Some((year, month, day, hour, minute, second, nanoseconds, offset)) = parse(bytes) else {
        return parse_error(receiver);
    };
    let days = days_from_civil(year, month, day);
    let seconds_of_day = i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    let unix_seconds = days * 86_400 + seconds_of_day - i64::from(offset);
    let checkpoint = gc::temporary_root_checkpoint();
    let values = [
        runtime::allocate(RtValue::Int(unix_seconds)),
        runtime::allocate(RtValue::Int(i64::from(nanoseconds))),
        runtime::allocate(RtValue::Int(i64::from(offset))),
        runtime::allocate(RtValue::None),
    ];
    for value in values {
        gc::push_temporary_root(value);
    }
    let result = runtime::allocate(RtValue::Object(Box::new(RuntimeObject {
        type_id: None,
        entries: vec![
            ("unix_seconds".into(), values[0]),
            ("nanoseconds".into(), values[1]),
            ("utc_offset_seconds".into(), values[2]),
            ("timezone".into(), values[3]),
        ],
        enum_data: None,
    })));
    gc::restore_temporary_roots(checkpoint);
    result
}

/// Validates and decodes the strict RFC 3339 components accepted by ExS.
fn parse(bytes: &[u8]) -> Option<(i32, i32, i32, i32, i32, i32, i32, i32)> {
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
        Some(b'+' | b'-') if index + 6 == bytes.len() && bytes.get(index + 3) == Some(&b':') => {
            let hours = digits(bytes, index + 1, 2)?;
            let minutes = digits(bytes, index + 4, 2)?;
            let offset = hours * 3_600 + minutes * 60;
            if offset >= 86_400 {
                return None;
            }
            if bytes[index] == b'-' {
                -offset
            } else {
                offset
            }
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

/// Returns ExS's recoverable parse error for malformed DateTime source text.
fn parse_error(value: ValueRef) -> ValueRef {
    runtime::recoverable_error(
        "ParseError",
        "DateTime::parse_rfc3339 requires strict RFC 3339 text",
        value,
    )
}
