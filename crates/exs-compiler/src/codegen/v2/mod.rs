//! Code generation for the runtime v2 prototype.
//!
//! Values live in compiler-owned linear memory laid out by `exs-layout`; the linked
//! `exs-core-template` only contributes stateless functions. The backend compiles the functions
//! reachable from public entry points and reports every construct it does not support yet as a
//! compile error, so a module either runs with v2 semantics or does not compile.

mod collections;
mod entry;
mod errors;
mod expressions;
mod function;
mod linker;
mod objects;
mod protocols;
mod reachability;
mod statements;
mod static_keys;
mod type_table;
mod untyped;

pub(super) use linker::compile;
