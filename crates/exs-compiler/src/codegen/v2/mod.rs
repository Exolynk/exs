//! Code generation for the runtime v2 prototype.
//!
//! Values live in compiler-owned linear memory laid out by `exs-layout`; the linked
//! `exs-core-template` only contributes stateless functions. The backend compiles the functions
//! reachable from public entry points and reports every construct it does not support yet as a
//! compile error, so a module either runs with v2 semantics or does not compile.

mod assertions;
mod closures;
mod collections;
mod dispatch;
mod entry;
mod enums;
mod errors;
mod expressions;
mod function;
mod intrinsics;
mod iteration;
mod linker;
mod methods;
mod objects;
mod reachability;
mod resumable;
mod statements;
mod static_keys;
mod type_table;
mod units;
mod untyped;
mod used_types;
mod validation;
mod wire_schema;

pub(super) use linker::compile;
