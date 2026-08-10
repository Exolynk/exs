//! Reusable, UI-independent completion support for ExS source text.
//!
//! Enable the optional `birei` feature to adapt the completion engine to
//! Birei's [`birei::code_editor::CodeLanguageService`] interface.

mod catalog;
mod engine;
mod syntax;

#[cfg(feature = "birei")]
mod birei;

#[cfg(feature = "birei")]
pub use birei::ExsBireiLanguageService;
pub use engine::CompletionEngine;
pub use model::{CompletionItem, CompletionKind, CompletionRequest, CompletionResponse};

mod model;
