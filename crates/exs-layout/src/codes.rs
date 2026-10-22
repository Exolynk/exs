//! Codes generated code and the core library exchange: the scheduler status of a runnable
//! task and the states of inline Range loops.

/// A runnable execution that needs another runner turn.
pub const STATUS_READY: i32 = 0;
/// Returned by the core library's Range start when a for loop must iterate the Range Object itself.
pub const RANGE_BEGIN_FALLBACK: i64 = 0x8000_0000;
/// Inline Range loop state after the last item was yielded.
pub const RANGE_STATE_LAST: i64 = 1;
/// Inline Range loop state after another item was requested past the last one.
pub const RANGE_STATE_FINISHED: i64 = 2;
