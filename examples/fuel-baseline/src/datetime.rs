//! Minimal RFC 3339 parsing and formatting matching ExS's native `DateTime`.

use exs_guest::{ExsValue, Vec};

/// One instant with the UTC offset it was written in.
#[derive(Clone, Copy)]
pub(crate) struct DateTime {
    /// Seconds since the Unix epoch.
    unix_seconds: i64,
    /// Nanoseconds within the second.
    nanoseconds: u32,
    /// UTC offset in seconds, kept for formatting.
    offset_seconds: i32,
}

impl DateTime {
    /// Parses strict `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)` text.
    pub(crate) fn parse_rfc3339(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
            return None;
        }
        if bytes[13] != b':' || bytes[16] != b':' {
            return None;
        }
        let year = digits(bytes, 0, 4)?;
        let month = digits(bytes, 5, 2)?;
        let day = digits(bytes, 8, 2)?;
        let hour = digits(bytes, 11, 2)?;
        let minute = digits(bytes, 14, 2)?;
        let second = digits(bytes, 17, 2)?;
        let mut position = 19;
        let mut nanoseconds = 0_u32;
        if bytes.get(position) == Some(&b'.') {
            position += 1;
            let mut scale = 100_000_000_u32;
            while let Some(digit) = bytes.get(position).filter(|byte| byte.is_ascii_digit()) {
                nanoseconds += u32::from(digit - b'0') * scale;
                scale /= 10;
                position += 1;
            }
        }
        let offset_seconds = match bytes.get(position)? {
            b'Z' => 0,
            sign @ (b'+' | b'-') => {
                let hours = digits(bytes, position + 1, 2)?;
                let minutes = digits(bytes, position + 4, 2)?;
                let offset = i32::try_from(hours * 3600 + minutes * 60).ok()?;
                if *sign == b'-' { -offset } else { offset }
            }
            _ => return None,
        };
        let days = days_from_civil(year, month, day);
        let local = days * 86_400 + hour * 3600 + minute * 60 + second;
        Some(Self {
            unix_seconds: local - i64::from(offset_seconds),
            nanoseconds,
            offset_seconds,
        })
    }

    /// Returns this instant moved by `seconds`, keeping its offset.
    pub(crate) fn add_seconds(self, seconds: i64) -> Self {
        Self {
            unix_seconds: self.unix_seconds + seconds,
            ..self
        }
    }

    /// Formats with nine fraction digits and `Z` for a zero offset, like ExS's `to_rfc3339`.
    pub(crate) fn to_rfc3339(self) -> String {
        let local = self.unix_seconds + i64::from(self.offset_seconds);
        let (year, month, day) = civil_from_days(local.div_euclid(86_400));
        let seconds = local.rem_euclid(86_400);
        let offset = if self.offset_seconds == 0 {
            String::from("Z")
        } else {
            let sign = if self.offset_seconds < 0 { '-' } else { '+' };
            let minutes = self.offset_seconds.unsigned_abs() / 60;
            format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
        };
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:09}{offset}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            self.nanoseconds
        )
    }
}

/// Parses `count` ASCII digits at `start`.
fn digits(bytes: &[u8], start: usize, count: usize) -> Option<i64> {
    bytes
        .get(start..start + count)?
        .iter()
        .try_fold(0_i64, |value, byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + i64::from(byte - b'0'))
        })
}

/// Returns the days since the Unix epoch of one proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Returns the proleptic Gregorian date of one day count since the Unix epoch.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Parses every input String as RFC 3339 and formats it back.
pub(crate) async fn datetime_roundtrip_100(inputs: Vec<ExsValue>) -> ExsValue {
    let Some(ExsValue::List(values)) = inputs.first() else {
        return ExsValue::None;
    };
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let ExsValue::String(text) = value else {
            return ExsValue::None;
        };
        match DateTime::parse_rfc3339(text) {
            Some(parsed) => output.push(ExsValue::String(parsed.to_rfc3339())),
            None => return ExsValue::None,
        }
    }
    ExsValue::List(output)
}
