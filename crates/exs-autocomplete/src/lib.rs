//! Reusable, UI-independent completion support for ExS source text.

mod catalog;
mod engine;
mod syntax;

pub use engine::CompletionEngine;
pub use model::{CompletionItem, CompletionKind, CompletionRequest, CompletionResponse};

mod model;
