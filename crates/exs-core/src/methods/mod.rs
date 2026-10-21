//! Built-in receiver methods and static functions of built-in types, as stateless helpers
//! with v1's results, Error kinds and messages.

mod dispatch;
mod float_parse;
mod numbers;
mod statics;
mod values;

pub use dispatch::call_builtin;
#[cfg(test)]
pub(crate) use float_parse::parse_f64;
pub use statics::call_static;
