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
