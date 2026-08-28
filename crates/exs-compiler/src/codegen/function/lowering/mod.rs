//! Expression and built-in lowering for direct ExS functions.

use wasm_encoder::{BlockType, Instruction, ValType};

use crate::ast::{BinaryOperator, Expression, FormattedStringPart, ObjectProperty, UnaryOperator};
use crate::codegen::diagnostics;
use crate::codegen::trait_registry::TraitOperator;
use crate::codegen::types::{self, EnumVariant, NominalKind, TypeContract};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

use super::FunctionCompiler;
use super::analysis::{condition_span, runtime_operation};
use super::method;

mod builtins;
mod control;
mod dispatch;
mod expressions;
mod values;
