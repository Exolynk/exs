//! Codes generated code and the core library exchange: the scheduler status of a runnable
//! task, the states of inline Range loops and the outcomes of the built-in equality.

/// A runnable execution that needs another runner turn.
pub const STATUS_READY: i32 = 0;
/// Returned by the core library's Range start when a for loop must iterate the Range Object itself.
pub const RANGE_BEGIN_FALLBACK: i64 = 0x8000_0000;
/// Inline Range loop state after the last item was yielded.
pub const RANGE_STATE_LAST: i64 = 1;
/// Inline Range loop state after another item was requested past the last one.
pub const RANGE_STATE_FINISHED: i64 = 2;

/// Built-in equality outcome: the values differ.
pub const EQUALITY_DIFFERENT: u32 = 0;
/// Built-in equality outcome: the values are equal.
pub const EQUALITY_SAME: u32 = 1;
/// Built-in equality outcome: the values are composites of one kind whose parts decide.
pub const EQUALITY_STRUCTURAL: u32 = 2;
