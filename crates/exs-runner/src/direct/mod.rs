//! The direct boundary of runtime v2 modules.
//!
//! The runner reads results and host-call arguments from the module's memory instead of CBOR
//! the module encoded, and passes inputs and host-call responses as `exs_abi::image` images the
//! module adopts without parsing.

mod reader;

pub(crate) use reader::{is_direct, limit_kind, read, result};
