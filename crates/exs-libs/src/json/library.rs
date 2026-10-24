//! Source and host functions of the `json` library.

use crate::error::HostFunction;

use super::{parse, write};

/// ExS source of the `json` library.
pub(crate) const SOURCE: &str = include_str!("json.exs");

/// Host functions of the `json` library by host name.
pub(crate) const HOST_FUNCTIONS: [(&str, HostFunction); 3] = [
    ("json.parse", parse::parse),
    ("json.stringify", write::stringify),
    ("json.pretty", write::pretty),
];
