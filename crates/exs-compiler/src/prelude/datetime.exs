/// A runner-captured wall-clock instant with optional IANA time-zone metadata.
///
/// `unix_seconds` and `nanoseconds` identify the instant. `utc_offset_seconds` is the offset
/// actually observed at that instant, while `timezone` is the runner-resolved IANA identifier
/// when one is available.
type DateTime {
    unix_seconds: Int,
    nanoseconds: Int,
    utc_offset_seconds: Int,
    timezone: String | None,
}

/// Returns whether `year` is a proleptic Gregorian leap year.
fn __exs_datetime_is_leap_year(year: Int) -> Bool {
    if (year.rem_euclid(4)) != 0 {
        ret false;
    }
    if (year.rem_euclid(100)) != 0 {
        ret true;
    }
    ret (year.rem_euclid(400)) == 0;
}

/// Returns the number of days in one validated proleptic Gregorian month.
fn __exs_datetime_days_in_month(year: Int, month: Int) -> Int | Error {
    if month < 1 || month > 12 {
        ret Error("ValueError", "month must be between 1 and 12", month);
    }
    if month == 2 {
        if __exs_datetime_is_leap_year(year) {
            ret 29;
        }
        ret 28;
    }
    if month == 4 || month == 6 || month == 9 || month == 11 {
        ret 30;
    }
    ret 31;
}

/// Converts one civil date to its signed day count relative to 1970-01-01.
fn __exs_datetime_days_from_civil(year: Int, month: Int, day: Int) -> Int | Error {
    let adjusted_year = year;
    if month <= 2 {
        adjusted_year = (adjusted_year - 1)?;
    }
    let era = adjusted_year.div_euclid(400)?;
    let year_of_era = (adjusted_year - (era * 400)?)?;
    let march_month = 0;
    if month > 2 {
        march_month = (month - 3)?;
    } else {
        march_month = (month + 9)?;
    }
    let day_of_year = ((((153 * march_month) + 2)?.div_euclid(5))? + day - 1)?;
    let day_of_era = ((year_of_era * 365) + (year_of_era.div_euclid(4))?)?;
    day_of_era = (day_of_era - (year_of_era.div_euclid(100))?)?;
    day_of_era = (day_of_era + day_of_year)?;
    ret ((era * 146097) + day_of_era - 719468)?;
}

/// Calculates all local calendar components using one recorded fixed UTC offset.
fn __exs_datetime_parts(
    unix_seconds: Int,
    nanoseconds: Int,
    utc_offset_seconds: Int,
 ) -> Object | Error {
    let local_seconds = (unix_seconds + utc_offset_seconds)?;
    let days = local_seconds.div_euclid(86400)?;
    let seconds_of_day = local_seconds.rem_euclid(86400)?;
    let shifted = (days + 719468)?;
    let era = shifted.div_euclid(146097)?;
    let day_of_era = (shifted - (era * 146097)?)?;
    let leap_four = day_of_era.div_euclid(1460)?;
    let leap_hundred = day_of_era.div_euclid(36524)?;
    let leap_four_hundred = day_of_era.div_euclid(146096)?;
    let year_numerator = (day_of_era - leap_four)?;
    year_numerator = (year_numerator + leap_hundred)?;
    year_numerator = (year_numerator - leap_four_hundred)?;
    let year_of_era = year_numerator.div_euclid(365)?;
    let year = (year_of_era + (era * 400)?)?;
    let leap_days = ((year_of_era.div_euclid(4))? - (year_of_era.div_euclid(100))?)?;
    let completed_year_days = ((365 * year_of_era) + leap_days)?;
    let day_of_year = (day_of_era - completed_year_days)?;
    let march_month_numerator = ((5 * day_of_year) + 2)?;
    let march_month = march_month_numerator.div_euclid(153)?;
    let completed_month_days = ((153 * march_month) + 2)?.div_euclid(5)?;
    let day = ((day_of_year - completed_month_days)? + 1)?;
    let month = 0;
    if march_month < 10 {
        month = (march_month + 3)?;
    } else {
        month = (march_month - 9)?;
    }
    let final_year = year;
    if month <= 2 {
        final_year = (year + 1)?;
    }
    let ordinal = day;
    if month == 2 {
        ordinal = (31 + day)?;
    } else if month > 2 {
        ordinal = (day_of_year + 59)?;
        if __exs_datetime_is_leap_year(final_year) {
            ordinal = (ordinal + 1)?;
        }
    }
    ret {
        year: final_year,
        month: month,
        day: day,
        hour: seconds_of_day.div_euclid(3600)?,
        minute: ((seconds_of_day.rem_euclid(3600))?.div_euclid(60))?,
        second: seconds_of_day.rem_euclid(60)?,
        nanosecond: nanoseconds,
        weekday: ((days + 3).rem_euclid(7) + 1)?,
        ordinal: ordinal,
    };
}

/// Returns one local DateTime component using its recorded fixed UTC offset.
fn __exs_datetime_part(
    unix_seconds: Int,
    nanoseconds: Int,
    utc_offset_seconds: Int,
    part: Int,
) -> Int | Error {
    let parts = __exs_datetime_parts(unix_seconds, nanoseconds, utc_offset_seconds)?;
    if part == 0 { ret parts.year; }
    if part == 1 { ret parts.month; }
    if part == 2 { ret parts.day; }
    if part == 3 { ret parts.hour; }
    if part == 4 { ret parts.minute; }
    if part == 5 { ret parts.second; }
    if part == 6 { ret parts.nanosecond; }
    if part == 7 { ret parts.weekday; }
    if part == 8 { ret parts.ordinal; }
    ret Error("ValueError", "unknown DateTime component", part);
}

/// Produces a decimal integer padded with leading zeros to `width` digits.
fn __exs_datetime_pad(value: Int, width: Int) -> String {
    let digits = value.to_string();
    let result = "";
    while (result.length() + digits.length()) < width {
        result = result + "0";
    }
    ret result + digits;
}

/// Renders one signed UTC offset using RFC 3339's numeric offset spelling.
fn __exs_datetime_offset_string(offset: Int) -> String | Error {
    if offset == 0 {
        ret "Z";
    }
    let sign = "+";
    let absolute = offset;
    if offset < 0 {
        sign = "-";
        absolute = (0 - offset)?;
    }
    let hours = absolute.div_euclid(3600)?;
    let minutes = (absolute.rem_euclid(3600)?).div_euclid(60)?;
    ret sign + __exs_datetime_pad(hours, 2) + ":" + __exs_datetime_pad(minutes, 2);
}

/// Validates civil components and constructs a fixed-offset DateTime without zone database access.
fn __exs_datetime_from_fixed_components(
    year: Int,
    month: Int,
    day: Int,
    hour: Int,
    minute: Int,
    second: Int,
    nanosecond: Int,
    utc_offset_seconds: Int,
    timezone: String | None,
) -> DateTime | Error {
    let maximum_day = __exs_datetime_days_in_month(year, month)?;
    if day < 1 || day > maximum_day {
        ret Error("ValueError", "day is outside the selected month", day);
    }
    if hour < 0 || hour > 23 {
        ret Error("ValueError", "hour must be between 0 and 23", hour);
    }
    if minute < 0 || minute > 59 {
        ret Error("ValueError", "minute must be between 0 and 59", minute);
    }
    if second < 0 || second > 59 {
        ret Error("ValueError", "second must be between 0 and 59", second);
    }
    if nanosecond < 0 || nanosecond >= 1000000000 {
        ret Error("ValueError", "nanosecond must be between 0 and 999999999", nanosecond);
    }
    if utc_offset_seconds < -86399 || utc_offset_seconds > 86399 {
        ret Error("ValueError", "UTC offset must be within one day", utc_offset_seconds);
    }
    let days = __exs_datetime_days_from_civil(year, month, day)?;
    let seconds_of_day = (((hour * 3600) + (minute * 60))? + second)?;
    let unix_seconds = (((days * 86400) + seconds_of_day)? - utc_offset_seconds)?;
    ret DateTime {
        unix_seconds: unix_seconds,
        nanoseconds: nanosecond,
        utc_offset_seconds: utc_offset_seconds,
        timezone: timezone,
    };
}

/// Converts one ASCII decimal scalar into its integer value.
fn __exs_datetime_digit(value: String) -> Int | Error {
    if value == "0" { ret 0; }
    if value == "1" { ret 1; }
    if value == "2" { ret 2; }
    if value == "3" { ret 3; }
    if value == "4" { ret 4; }
    if value == "5" { ret 5; }
    if value == "6" { ret 6; }
    if value == "7" { ret 7; }
    if value == "8" { ret 8; }
    if value == "9" { ret 9; }
    ret Error("ParseError", "expected an ASCII decimal digit", value);
}

/// Parses RFC3339 calendar timestamps with optional fractional seconds and numeric offsets.
fn __exs_datetime_parse_rfc3339(value: String) -> DateTime | Error {
    let length = value.length();
    let year = 0;
    let month = 0;
    let day = 0;
    let hour = 0;
    let minute = 0;
    let second = 0;
    let offset_sign = 1;
    let offset_hour = 0;
    let offset_minute = 0;
    let nanosecond = 0;
    let fraction_digits = 0;
    let parsing_fraction = false;
    let parsing_offset = false;
    let offset_position = 0;
    let timezone_seen = false;
    let position = 0;
    for scalar in value {
        if position < 19 && (position == 4 || position == 7) {
            if scalar != "-" {
                ret Error("ParseError", "RFC 3339 date separators must be hyphens", value);
            }
        } else if position < 19 && position == 10 {
            if scalar != "T" && scalar != "t" {
                ret Error("ParseError", "RFC 3339 date and time must be separated by T", value);
            }
        } else if position < 19 && (position == 13 || position == 16) {
            if scalar != ":" {
                ret Error("ParseError", "RFC 3339 time separators must be colons", value);
            }
        } else if position < 19 {
            let digit = __exs_datetime_digit(scalar)?;
            if position < 4 {
                year = ((year * 10) + digit)?;
            } else if position < 7 {
                month = ((month * 10) + digit)?;
            } else if position < 10 {
                day = ((day * 10) + digit)?;
            } else if position < 13 {
                hour = ((hour * 10) + digit)?;
            } else if position < 16 {
                minute = ((minute * 10) + digit)?;
            } else if scalar == "-" {
                ret Error("ParseError", "RFC 3339 second must be an ASCII decimal digit", value);
            } else {
                second = ((second * 10) + digit)?;
            }
        } else if position == 19 {
            if scalar == "." {
                parsing_fraction = true;
            } else if scalar == "Z" || scalar == "z" {
                timezone_seen = true;
            } else if scalar == "+" || scalar == "-" {
                if scalar == "-" {
                    offset_sign = -1;
                }
                parsing_offset = true;
            } else {
                ret Error("ParseError", "RFC 3339 timestamp must end with Z, an offset, or fractional seconds", value);
            }
        } else if parsing_fraction {
            if scalar == "Z" || scalar == "z" {
                if fraction_digits == 0 {
                    ret Error("ParseError", "RFC 3339 fractional seconds require at least one digit", value);
                }
                parsing_fraction = false;
                timezone_seen = true;
            } else if scalar == "+" || scalar == "-" {
                if fraction_digits == 0 {
                    ret Error("ParseError", "RFC 3339 fractional seconds require at least one digit", value);
                }
                if scalar == "-" {
                    offset_sign = -1;
                }
                parsing_fraction = false;
                parsing_offset = true;
            } else {
                if fraction_digits >= 9 {
                    ret Error("ParseError", "RFC 3339 fractions support at most nine digits", value);
                }
                let digit = __exs_datetime_digit(scalar)?;
                nanosecond = ((nanosecond * 10) + digit)?;
                fraction_digits = (fraction_digits + 1)?;
            }
        } else if parsing_offset {
            if offset_position == 0 || offset_position == 1 {
                let digit = __exs_datetime_digit(scalar)?;
                offset_hour = ((offset_hour * 10) + digit)?;
            } else if offset_position == 2 {
                if scalar != ":" {
                    ret Error("ParseError", "RFC 3339 offset separators must be colons", value);
                }
            } else if offset_position == 3 || offset_position == 4 {
                let digit = __exs_datetime_digit(scalar)?;
                offset_minute = ((offset_minute * 10) + digit)?;
            } else {
                ret Error("ParseError", "RFC 3339 offset must use exactly HH:MM", value);
            }
            offset_position = (offset_position + 1)?;
            if offset_position == 5 {
                timezone_seen = true;
            }
        } else {
            ret Error("ParseError", "RFC 3339 timestamp contains trailing data", value);
        }
        position = (position + 1)?;
    }
    if position < 20 || parsing_fraction || !timezone_seen {
        ret Error("ParseError", "RFC 3339 timestamp is incomplete", value);
    }
    if parsing_offset && offset_position != 5 {
        ret Error("ParseError", "RFC 3339 offset must use exactly HH:MM", value);
    }
    while fraction_digits < 9 {
        nanosecond = (nanosecond * 10)?;
        fraction_digits = (fraction_digits + 1)?;
    }
    let offset = ((offset_hour * 3600) + (offset_minute * 60))?;
    offset = (offset * offset_sign)?;
    ret __exs_datetime_from_fixed_components(year, month, day, hour, minute, second, nanosecond, offset, None);
}

impl DateTime {
    /// Captures the current runner wall clock as a DateTime snapshot.
    fn now() -> DateTime {
        ret Host::now();
    }

    /// Constructs one fixed-offset DateTime from civil components without time-zone database access.
    fn from_components(
        year: Int,
        month: Int,
        day: Int,
        hour: Int,
        minute: Int,
        second: Int,
        nanosecond: Int,
        utc_offset_seconds: Int,
        timezone: String | None,
    ) -> DateTime | Error {
        if timezone != None {
            ret Error("ValueError", "use DateTime::from_components_in_timezone for IANA time zones", timezone);
        }
        ret __exs_datetime_from_fixed_components(
            year, month, day, hour, minute, second, nanosecond, utc_offset_seconds, None,
        );
    }

    /// Constructs one DateTime in an IANA zone, adjusting DST gaps and folds compatibly.
    fn from_components_in_timezone(
        year: Int,
        month: Int,
        day: Int,
        hour: Int,
        minute: Int,
        second: Int,
        nanosecond: Int,
        timezone: String,
    ) -> DateTime | Error {
        ret Host::date_time_from_components(year, month, day, hour, minute, second, nanosecond, timezone);
    }

    /// Constructs a DateTime directly from its Unix instant and recorded fixed-offset metadata.
    fn from_unix(
        unix_seconds: Int,
        nanoseconds: Int,
        utc_offset_seconds: Int,
        timezone: String | None,
    ) -> DateTime | Error {
        if nanoseconds < 0 || nanoseconds >= 1000000000 {
            ret Error("ValueError", "nanosecond must be between 0 and 999999999", nanoseconds);
        }
        if utc_offset_seconds < -86399 || utc_offset_seconds > 86399 {
            ret Error("ValueError", "UTC offset must be within one day", utc_offset_seconds);
        }
        ret DateTime {
            unix_seconds: unix_seconds,
            nanoseconds: nanoseconds,
            utc_offset_seconds: utc_offset_seconds,
            timezone: timezone,
        };
    }

    /// Parses a strict whole-second RFC 3339 timestamp with Z or a ±HH:MM offset.
    fn parse_rfc3339(value: String) -> DateTime | Error {
        ret __exs_datetime_parse_rfc3339(value);
    }

    /// Returns this instant rendered in one validated IANA time zone.
    fn in_timezone(self, timezone: String) -> DateTime | Error {
        ret Host::date_time_in_timezone(self, timezone);
    }

    /// Returns this instant rendered with UTC's fixed offset and IANA name.
    fn to_utc(self) -> DateTime {
        ret DateTime {
            unix_seconds: self.unix_seconds,
            nanoseconds: self.nanoseconds,
            utc_offset_seconds: 0,
            timezone: "UTC",
        };
    }

    /// Returns this instant's signed whole Unix-second component.
    fn as_unix_seconds(self) -> Int { ret self.unix_seconds; }

    /// Returns the local proleptic Gregorian year at this DateTime's recorded offset.
    fn year(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 0); }

    /// Returns the local Gregorian month in the range 1 through 12.
    fn month(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 1); }

    /// Returns the local day of month in the range 1 through 31.
    fn day(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 2); }

    /// Returns the local hour in the range 0 through 23.
    fn hour(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 3); }

    /// Returns the local minute in the range 0 through 59.
    fn minute(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 4); }

    /// Returns the local second in the range 0 through 59.
    fn second(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 5); }

    /// Returns the fractional second in nanoseconds.
    fn nanosecond(self) -> Int { ret self.nanoseconds; }

    /// Returns the ISO weekday where Monday is 1 and Sunday is 7.
    fn weekday(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 7); }

    /// Returns the local one-based day number within the year.
    fn ordinal(self) -> Int | Error { ret __exs_datetime_part(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds, 8); }

    /// Returns the observed UTC offset in whole seconds.
    fn offset_seconds(self) -> Int { ret self.utc_offset_seconds; }

    /// Returns this snapshot's resolved IANA zone name when the runner supplied one.
    fn timezone_name(self) -> String | None { ret self.timezone; }

    /// Returns a copy with a new local year, resolving named-zone DST transitions compatibly.
    fn with_year(self, year: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(year, parts.month, parts.day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(year, parts.month, parts.day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new local month, resolving named-zone DST transitions compatibly.
    fn with_month(self, month: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, month, parts.day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, month, parts.day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new local day, resolving named-zone DST transitions compatibly.
    fn with_day(self, day: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, parts.month, day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, parts.month, day, parts.hour, parts.minute, parts.second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new local hour, resolving named-zone DST transitions compatibly.
    fn with_hour(self, hour: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, parts.month, parts.day, hour, parts.minute, parts.second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, parts.month, parts.day, hour, parts.minute, parts.second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new local minute, resolving named-zone DST transitions compatibly.
    fn with_minute(self, minute: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, parts.month, parts.day, parts.hour, minute, parts.second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, parts.month, parts.day, parts.hour, minute, parts.second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new local second, resolving named-zone DST transitions compatibly.
    fn with_second(self, second: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, parts.month, parts.day, parts.hour, parts.minute, second, parts.nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, parts.month, parts.day, parts.hour, parts.minute, second, parts.nanosecond, self.timezone);
    }

    /// Returns a copy with a new fractional second, resolving named-zone DST transitions compatibly.
    fn with_nanosecond(self, nanosecond: Int) -> DateTime | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        if self.timezone == None {
            ret DateTime::from_components(parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second, nanosecond, self.utc_offset_seconds, None);
        }
        ret DateTime::from_components_in_timezone(parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second, nanosecond, self.timezone);
    }

    /// Returns this instant plus one non-negative Duration while retaining its recorded offset.
    fn add_duration(self, duration: Duration) -> DateTime | Error {
        let nanoseconds = (self.nanoseconds + duration.nanoseconds)?;
        let carry = nanoseconds.div_euclid(1000000000)?;
        ret DateTime {
            unix_seconds: ((self.unix_seconds + duration.seconds)? + carry)?,
            nanoseconds: nanoseconds.rem_euclid(1000000000)?,
            utc_offset_seconds: self.utc_offset_seconds,
            timezone: self.timezone,
        };
    }

    /// Returns this instant minus one non-negative Duration while retaining its recorded offset.
    fn subtract_duration(self, duration: Duration) -> DateTime | Error {
        let nanoseconds = (self.nanoseconds - duration.nanoseconds)?;
        let borrow = 0;
        if nanoseconds < 0 {
            borrow = 1;
        }
        ret DateTime {
            unix_seconds: ((self.unix_seconds - duration.seconds)? - borrow)?,
            nanoseconds: nanoseconds.rem_euclid(1000000000)?,
            utc_offset_seconds: self.utc_offset_seconds,
            timezone: self.timezone,
        };
    }

    /// Returns whether this instant precedes `other` independently of their display offsets.
    fn is_before(self, other: DateTime) -> Bool {
        if self.unix_seconds != other.unix_seconds {
            ret self.unix_seconds < other.unix_seconds;
        }
        ret self.nanoseconds < other.nanoseconds;
    }

    /// Returns whether this instant follows `other` independently of their display offsets.
    fn is_after(self, other: DateTime) -> Bool {
        if self.unix_seconds != other.unix_seconds {
            ret self.unix_seconds > other.unix_seconds;
        }
        ret self.nanoseconds > other.nanoseconds;
    }

    /// Returns whether this and `other` represent the same instant independently of their zones.
    fn is_same_instant(self, other: DateTime) -> Bool {
        ret self.unix_seconds == other.unix_seconds && self.nanoseconds == other.nanoseconds;
    }

    /// Renders this DateTime as a fixed-width RFC 3339 timestamp with nanosecond precision.
    fn to_rfc3339(self) -> String | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        ret __exs_datetime_pad(parts.year, 4) + "-" + __exs_datetime_pad(parts.month, 2) + "-" + __exs_datetime_pad(parts.day, 2) + "T" + __exs_datetime_pad(parts.hour, 2) + ":" + __exs_datetime_pad(parts.minute, 2) + ":" + __exs_datetime_pad(parts.second, 2) + "." + __exs_datetime_pad(parts.nanosecond, 9) + __exs_datetime_offset_string(self.utc_offset_seconds)?;
    }

    /// Renders this DateTime's local calendar date as YYYY-MM-DD.
    fn to_date_string(self) -> String | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        ret __exs_datetime_pad(parts.year, 4) + "-" + __exs_datetime_pad(parts.month, 2) + "-" + __exs_datetime_pad(parts.day, 2);
    }

    /// Renders this DateTime's local clock time with numeric UTC offset.
    fn to_time_string(self) -> String | Error {
        let parts = __exs_datetime_parts(self.unix_seconds, self.nanoseconds, self.utc_offset_seconds)?;
        ret __exs_datetime_pad(parts.hour, 2) + ":" + __exs_datetime_pad(parts.minute, 2) + ":" + __exs_datetime_pad(parts.second, 2) + "." + __exs_datetime_pad(parts.nanosecond, 9) + __exs_datetime_offset_string(self.utc_offset_seconds)?;
    }

    /// Returns this instant truncated to a whole count of Unix microseconds.
    fn as_unix_microseconds(self) -> Int | Error {
        let seconds = (self.unix_seconds * 1000000)?;
        let nanoseconds = self.nanoseconds.div_euclid(1000)?;
        ret (seconds + nanoseconds)?;
    }

    /// Returns this instant as an exact whole count of Unix nanoseconds.
    fn as_unix_nanoseconds(self) -> Int | Error {
        let seconds = (self.unix_seconds * 1000000000)?;
        ret (seconds + self.nanoseconds)?;
    }
    /// Returns this instant truncated to a whole count of Unix milliseconds.
    fn as_unix_milliseconds(self) -> Int | Error {
        let seconds = (self.unix_seconds * 1000)?;
        let nanoseconds = (self.nanoseconds.div_euclid(1000000))?;
        ret (seconds + nanoseconds)?;
    }

    /// Returns the non-negative duration from `earlier` to this instant.
    fn duration_since(self, earlier: DateTime) -> Duration | Error {
        let seconds = (self.unix_seconds - earlier.unix_seconds)?;
        let nanoseconds = (self.nanoseconds - earlier.nanoseconds)?;
        if nanoseconds < 0 {
            seconds = (seconds - 1)?;
            nanoseconds = (nanoseconds + 1000000000)?;
        }
        if seconds < 0 {
            ret Error("ValueError", "DateTime duration must not be negative", seconds);
        }
        ret Duration { seconds: seconds, nanoseconds: nanoseconds };
    }
}

impl Add for DateTime {
    /// Adds one Duration supplied through the standard arithmetic protocol.
    fn add(self, other: Any) -> Any {
        let seconds = other.seconds?;
        let nanoseconds = other.nanoseconds?;
        ret self.add_duration(Duration { seconds: seconds, nanoseconds: nanoseconds });
    }
}

impl Sub for DateTime {
    /// Subtracts one Duration supplied through the standard arithmetic protocol.
    fn sub(self, other: Any) -> Any {
        let seconds = other.seconds?;
        let nanoseconds = other.nanoseconds?;
        ret self.subtract_duration(Duration { seconds: seconds, nanoseconds: nanoseconds });
    }
}

impl ToString for DateTime {
    /// Renders an RFC3339 value or a stable fallback when unchecked fields cannot be rendered.
    fn to_string(self) -> String {
        let rendered = self.to_rfc3339();
        if rendered is Error {
            ret "<invalid DateTime>";
        }
        ret rendered;
    }
}
