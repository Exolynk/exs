//! Default text of values that formatted strings cannot render like `+` operands.

use exs_abi::{TYPE_ERROR, TYPE_FN, TYPE_LIST, TYPE_NONE, TYPE_OBJECT};
use exs_value::layout::{KIND_CELL, KIND_SHIFT};

/// Returns v1's default `ToString` text of a value that is no String, Bool, Int or Float.
pub(crate) fn default_text(raw: u64) -> &'static str {
    let kind = (raw >> KIND_SHIFT) as u32;
    if kind & TYPE_NONE != 0 {
        "None"
    } else if kind & TYPE_ERROR != 0 {
        "Error"
    } else if kind & TYPE_LIST != 0 {
        "[]"
    } else if kind & TYPE_OBJECT != 0 {
        "{}"
    } else if kind & TYPE_FN != 0 {
        "fn main()"
    } else if kind & u32::from(KIND_CELL) != 0 {
        "Cell"
    } else {
        // No other kind of value exists in runtime v2 yet.
        "None"
    }
}
