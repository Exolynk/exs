//! Errors and argument checks shared by the host functions.

use exs_abi::{ErrorSeverity, ExsError, ExsValue};

/// A synchronous host function of a library.
pub(crate) type HostFunction = fn(Vec<ExsValue>) -> ExsValue;

/// Returns a recoverable ExS error of `kind` that the script can handle with `?`.
pub(crate) fn error(kind: &str, message: impl Into<String>) -> ExsValue {
    ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: kind.to_owned(),
        message: message.into(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    })
}

/// Returns the single argument of `function`, or a type error when the call passes another
/// number of arguments.
pub(crate) fn single_argument(
    function: &str,
    arguments: Vec<ExsValue>,
) -> Result<ExsValue, ExsValue> {
    let mut arguments = arguments.into_iter();
    match (arguments.next(), arguments.next()) {
        (Some(argument), None) => Ok(argument),
        _ => Err(error(
            "TypeError",
            format!("{function} expects exactly one argument"),
        )),
    }
}

/// Returns the single String argument of `function`, or a type error.
pub(crate) fn string_argument(
    function: &str,
    arguments: Vec<ExsValue>,
) -> Result<String, ExsValue> {
    match single_argument(function, arguments)? {
        ExsValue::String(text) => Ok(text),
        _ => Err(error(
            "TypeError",
            format!("{function} expects a String argument"),
        )),
    }
}
