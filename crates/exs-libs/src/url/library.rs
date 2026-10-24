//! Source and host functions of the `url` library.

use crate::error::HostFunction;

use super::{component, query};

/// ExS source of the `url` library.
pub(crate) const SOURCE: &str = include_str!("url.exs");

/// Host functions of the `url` library by host name.
pub(crate) const HOST_FUNCTIONS: [(&str, HostFunction); 4] = [
    ("url.encode_query", query::encode_query),
    ("url.decode_query", query::decode_query),
    ("url.encode", component::encode),
    ("url.decode", component::decode),
];
