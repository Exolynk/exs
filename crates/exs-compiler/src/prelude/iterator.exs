/// The result of advancing one Iterator.
enum IteratorStep {
    /// Contains the next value yielded by the Iterator.
    Item(value: Any),
    /// Marks that the Iterator has no more values.
    Done,
}

/// A value that yields one item at a time and may suspend while advancing.
trait Iterator {
    /// Produces the next item or marks the Iterator complete.
    fn next(self) -> IteratorStep | Error;
}

/// A host-backed pull stream that implements Iterator.
type HostStream {
    handle: Int,
}

impl Iterator for HostStream {
    fn next(self) -> IteratorStep | Error {
        ret Host::call("__exs.host.stream_next", self.handle);
    }
}
