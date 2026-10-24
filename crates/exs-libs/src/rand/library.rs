//! Source and host functions of the `rand` library.

use crate::error::HostFunction;

use super::functions;

/// ExS source of the `rand` library.
pub(crate) const SOURCE: &str = include_str!("rand.exs");

/// Host functions of the `rand` library by host name.
pub(crate) const HOST_FUNCTIONS: [(&str, HostFunction); 4] = [
    ("rand.int", functions::int),
    ("rand.float", functions::float),
    ("rand.text", functions::text),
    ("rand.bytes", functions::bytes),
];
