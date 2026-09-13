/// A finite sequence of Int values that implements Iterator.
type Range {
    current: Int,
    end: Int,
    step: Int,
    inclusive: Bool,
    done: Bool,
}

impl Range {
    /// Creates a half-open Range from start toward end with an inferred unit direction.
    fn until(start: Int, end: Int) -> Range {
        let step = 1;
        if start > end {
            step = -1;
        }
        ret Range { current: start, end: end, step: step, inclusive: false, done: false };
    }

    /// Creates an inclusive Range from start toward end with an inferred unit direction.
    fn through(start: Int, end: Int) -> Range {
        let step = 1;
        if start > end {
            step = -1;
        }
        ret Range { current: start, end: end, step: step, inclusive: true, done: false };
    }

    /// Creates a half-open Range with an explicit non-zero step.
    fn step(start: Int, end: Int, step: Int) -> Range | Error {
        if step == 0 {
            ret Error("ValueError", "Range step must not be zero", step);
        }
        if (start < end && step < 0) || (start > end && step > 0) {
            ret Error("ValueError", "Range step must move toward the end", step);
        }
        ret Range { current: start, end: end, step: step, inclusive: false, done: false };
    }

    /// Creates an inclusive Range with an explicit non-zero step.
    fn step_through(start: Int, end: Int, step: Int) -> Range | Error {
        let range = Range::step(start, end, step)?;
        range.inclusive = true;
        ret range;
    }
}

impl Iterator for Range {
    /// Yields the next Int until this finite Range is exhausted.
    fn next(self) -> IteratorStep | Error {
        if self.done {
            ret IteratorStep::Done;
        }
        if self.step > 0 {
            if self.inclusive {
                if self.current > self.end {
                    self.done = true;
                    ret IteratorStep::Done;
                }
            } else if self.current >= self.end {
                self.done = true;
                ret IteratorStep::Done;
            }
        } else if self.inclusive {
            if self.current < self.end {
                self.done = true;
                ret IteratorStep::Done;
            }
        } else if self.current <= self.end {
            self.done = true;
            ret IteratorStep::Done;
        }
        let item = self.current;
        if item == self.end {
            self.done = true;
        } else {
            self.current = (self.current + self.step)?;
        }
        ret IteratorStep::Item(item);
    }
}
