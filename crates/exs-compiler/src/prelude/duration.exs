/// An exact non-negative time interval represented as normalized seconds and nanoseconds.
type Duration {
    seconds: Int,
    nanoseconds: Int,
}

impl Duration {
    /// Returns the whole-second component of this Duration.
    fn as_seconds(self) -> Int {
        ret self.seconds;
    }

    /// Returns this Duration truncated to a whole number of milliseconds.
    fn as_milliseconds(self) -> Int | Error {
        let seconds = (self.seconds * 1000)?;
        let nanoseconds = (self.nanoseconds.div_euclid(1000000))?;
        ret (seconds + nanoseconds)?;
    }

    /// Returns this Duration truncated to a whole number of microseconds.
    fn as_microseconds(self) -> Int | Error {
        let seconds = (self.seconds * 1000000)?;
        let nanoseconds = (self.nanoseconds.div_euclid(1000))?;
        ret (seconds + nanoseconds)?;
    }

    /// Returns this Duration as an exact whole number of nanoseconds.
    fn as_nanoseconds(self) -> Int | Error {
        let seconds = (self.seconds * 1000000000)?;
        ret (seconds + self.nanoseconds)?;
    }

    /// Creates an exact Duration from a non-negative nanosecond count.
    fn nanoseconds(value: Int) -> Duration | Error {
        if value < 0 {
            ret Error("ValueError", "duration nanoseconds must not be negative", value);
        }
        let seconds = (value.div_euclid(1000000000))?;
        let nanoseconds = (value.rem_euclid(1000000000))?;
        ret Duration { seconds: seconds, nanoseconds: nanoseconds };
    }

    /// Creates an exact Duration from a non-negative microsecond count.
    fn microseconds(value: Int) -> Duration | Error {
        if value < 0 {
            ret Error("ValueError", "duration microseconds must not be negative", value);
        }
        ret Duration::nanoseconds((value * 1000)?);
    }

    /// Creates an exact Duration from a non-negative millisecond count.
    fn milliseconds(value: Int) -> Duration | Error {
        if value < 0 {
            ret Error("ValueError", "duration milliseconds must not be negative", value);
        }
        ret Duration::nanoseconds((value * 1000000)?);
    }

    /// Creates an exact Duration from a non-negative second count.
    fn seconds(value: Int) -> Duration | Error {
        if value < 0 {
            ret Error("ValueError", "duration seconds must not be negative", value);
        }
        ret Duration { seconds: value, nanoseconds: 0 };
    }
}

impl Compare for Duration {
    /// Orders two Durations by their length; other values have no order with a Duration.
    fn compare(self, other: Any) -> Ordering {
        if !(other is Duration) {
            ret Ordering::Unordered;
        }
        if self.seconds != other.seconds {
            ret self.seconds.compare(other.seconds);
        }
        ret self.nanoseconds.compare(other.nanoseconds);
    }
}

impl Add for Duration {
    /// Returns the sum of two Durations.
    fn add(self, other: Any) -> Any {
        if !(other is Duration) {
            ret Error("TypeError", "a Duration can only be added to a Duration", other);
        }
        let seconds = (self.seconds + other.seconds)?;
        let nanoseconds = self.nanoseconds + other.nanoseconds;
        if nanoseconds >= 1000000000 {
            seconds = (seconds + 1)?;
            nanoseconds = nanoseconds - 1000000000;
        }
        ret Duration { seconds: seconds, nanoseconds: nanoseconds };
    }
}

impl Sub for Duration {
    /// Returns the difference of two Durations, or ValueError when it would be negative.
    fn sub(self, other: Any) -> Any {
        if !(other is Duration) {
            ret Error("TypeError", "only a Duration can be subtracted from a Duration", other);
        }
        let seconds = (self.seconds - other.seconds)?;
        let nanoseconds = self.nanoseconds - other.nanoseconds;
        if nanoseconds < 0 {
            seconds = (seconds - 1)?;
            nanoseconds = nanoseconds + 1000000000;
        }
        if seconds < 0 {
            ret Error("ValueError", "Duration subtraction must not be negative", other);
        }
        ret Duration { seconds: seconds, nanoseconds: nanoseconds };
    }
}
