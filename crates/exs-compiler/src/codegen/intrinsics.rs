//! Prelude functions the compiler lowers itself.

use wasm_encoder::{BlockType, Instruction};

use super::function::{FunctionCompiler, error};
use crate::ast::Expression;
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Keys of the Object of raw DateTime fields the RFC 3339 parser returns, in order.
pub(super) const DATETIME_KEYS: [&str; 4] = [
    "unix_seconds",
    "nanoseconds",
    "utc_offset_seconds",
    "timezone",
];

/// Returns whether `name` is the prelude's RFC 3339 parser intrinsic.
pub(super) fn is_datetime_parser(name: &str) -> bool {
    name == "__exs_datetime_parse_rfc3339_runtime"
        || name.ends_with("::__exs_datetime_parse_rfc3339_runtime")
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles a call of the RFC 3339 parser intrinsic: the core parses the text into raw
    /// fields, which become an untyped Object.
    pub(super) fn compile_datetime_parse(
        &mut self,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let [text] = arguments else {
            return Err(error(
                "E0208",
                span,
                "DateTime parser intrinsic expects 1 argument".to_owned(),
            ));
        };
        self.compile_expression(text)?;
        let text = self.store_new();
        let count = DATETIME_KEYS.len() as u32;
        let block = self.consecutive_locals(count);
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(text));
        self.push_slot_address(block);
        self.core_call("__exs_core_datetime_parse", span)?;
        let parsed = self.store_new();
        self.instruction(Instruction::LocalGet(parsed));
        immediate::type_mask_test(&mut self.function, exs_layout::TYPE_ERROR);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.instruction(Instruction::LocalGet(parsed));
        self.instruction(Instruction::Else);
        let keys: Vec<String> = DATETIME_KEYS.iter().map(|key| (*key).to_owned()).collect();
        self.object_from_slots(&keys, block, span)?;
        self.instruction(Instruction::End);
        Ok(())
    }
}
