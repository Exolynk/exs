//! Built-in receiver methods and static functions of built-in types, as stateless helpers
//! with the results, Error kinds and messages.

mod bytes;
mod clone;
mod dispatch;
mod float_parse;
mod lists;
mod numbers;
pub(crate) mod objects;
mod statics;
mod strings;
mod values;

pub(crate) use bytes::get as bytes_get;
pub use dispatch::call_builtin;
#[cfg(test)]
pub(crate) use float_parse::parse_f64;
pub(crate) use objects::{field_value, is_object};
pub use statics::call_static;
