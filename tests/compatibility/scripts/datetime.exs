//! Dates, times and durations, through the host's time zone data.

fn main() {
    let instant = DateTime::parse_rfc3339("2026-03-04T05:06:07.5+01:00")?;
    let later = instant.add_duration(Duration::hours(25)?)?;
    let tokyo = instant.in_timezone("Asia/Tokyo")?;
    let civil = DateTime::from_components_in_timezone(2026, 7, 1, 12, 0, 0, 0, "Europe/Zurich")?;
    ret [
        instant.to_utc().to_rfc3339()?,
        instant.year()?,
        instant.weekday()?,
        instant.ordinal()?,
        DateTime::parse_rfc3339("2024-12-31T00:00:00Z")?.ordinal()?,
        instant.as_unix_seconds(),
        later.to_rfc3339()?,
        tokyo.to_rfc3339()?,
        civil.offset_seconds(),
        later.duration_since(instant)?.as_minutes(),
        instant.is_before(later),
    ];
}
