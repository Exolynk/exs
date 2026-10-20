//! Direct runtime calls for built-in receiver methods, shared by both lowerings.
//!
//! A method call that no nominal implementation handles falls back to the runtime. Built-in
//! methods called with their exact arity skip the generic name-based dispatcher and its
//! argument List.

use exs_abi::BuiltinMethod;

/// One direct runtime call replacing the generic method dispatcher.
pub(super) struct BuiltinCall {
    /// Runtime export to call with the receiver, the optional method index and the arguments.
    pub(super) export: &'static str,
    /// `BuiltinMethod` index passed right after the receiver, for the indexed exports.
    pub(super) method: Option<i32>,
}

/// Returns the direct runtime call for `method` with `arity` arguments, if one exists.
pub(super) fn builtin_call(method: &str, arity: usize) -> Option<BuiltinCall> {
    let dedicated = match (method, arity) {
        ("length", 0) => Some("__exs_rt_length"),
        ("is_empty", 0) => Some("__exs_rt_is_empty"),
        ("slice", 2) => Some("__exs_rt_slice"),
        ("contains", 1) => Some("__exs_rt_contains"),
        ("starts_with", 1) => Some("__exs_rt_starts_with"),
        ("ends_with", 1) => Some("__exs_rt_ends_with"),
        ("trim", 0) => Some("__exs_rt_trim"),
        ("trim_start", 0) => Some("__exs_rt_trim_start"),
        ("trim_end", 0) => Some("__exs_rt_trim_end"),
        ("replace", 2) => Some("__exs_rt_replace"),
        ("split", 1) => Some("__exs_rt_split"),
        _ => None,
    };
    if let Some(export) = dedicated {
        return Some(BuiltinCall {
            export,
            method: None,
        });
    }
    let builtin = BuiltinMethod::from_name(method).filter(|builtin| builtin.arity() == arity)?;
    let export = match arity {
        0 => "__exs_rt_call_builtin0",
        1 => "__exs_rt_call_builtin1",
        _ => "__exs_rt_call_builtin2",
    };
    Some(BuiltinCall {
        export,
        method: Some(builtin.id().cast_signed()),
    })
}
