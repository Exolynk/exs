//! Structured runtime Error payloads.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use exs_abi::{ErrorSeverity, ExsStackFrame, SourcePositionId};
use exs_value::ValueRef;

/// A language Error stored in the runtime heap.
pub(crate) struct RuntimeError {
    /// Whether the Error permits execution to continue.
    pub(crate) severity: ErrorSeverity,
    /// Stable machine-readable Error kind.
    pub(crate) kind: Box<str>,
    /// Human-readable Error description.
    pub(crate) message: Box<str>,
    /// Language value containing additional Error data.
    pub(crate) data: ValueRef,
    /// Source position active when the Error was created.
    pub(crate) origin: Option<SourcePositionId>,
    /// Language-level frames from innermost to outermost.
    pub(crate) trace: Vec<ExsStackFrame>,
    /// Optional related prior Error or language value.
    pub(crate) cause: Option<ValueRef>,
}

impl RuntimeError {
    /// Creates one recoverable runtime Error with no trace or explicit cause.
    pub(crate) fn recoverable(
        kind: &str,
        message: &str,
        data: ValueRef,
        origin: Option<SourcePositionId>,
    ) -> Self {
        Self {
            severity: ErrorSeverity::Recoverable,
            kind: String::from(kind).into_boxed_str(),
            message: String::from(message).into_boxed_str(),
            data,
            origin,
            trace: unsafe { crate::state::runtime() }
                .frames
                .iter()
                .rev()
                .copied()
                .collect(),
            cause: None,
        }
    }
}
