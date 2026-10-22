//! Prelude functions the compiler lowers itself.

use wasm_encoder::{BlockType, Instruction};

use super::function::{FunctionCompiler, error};
use crate::ast::Expression;
use crate::codegen::trait_registry::TraitOperator;
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

/// A prelude function the compiler lowers to core calls or inline code, other than the
/// RFC 3339 parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Intrinsic {
    /// `__exs_equality(left, right) -> Int`: the core's equality outcome, 0 different, 1 equal
    /// or 2 when the parts decide.
    Equality,
    /// `__exs_parts(value) -> List | None`: the parts of a composite value.
    Parts,
    /// `__exs_same(left, right) -> Bool`: whether both are the same reference.
    Same,
    /// `__exs_has_eq(value) -> Bool`: whether the value's type implements `Eq` or `Compare`.
    HasEq,
}

impl Intrinsic {
    /// Every intrinsic.
    const ALL: [Self; 4] = [Self::Equality, Self::Parts, Self::Same, Self::HasEq];

    /// Returns the intrinsic a call of `name` names, with or without a module prefix.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        let base = name.rsplit("::").next().unwrap_or(name);
        Self::ALL
            .into_iter()
            .find(|intrinsic| intrinsic.name() == base)
    }

    /// Returns the prelude spelling of the intrinsic.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Equality => "__exs_equality",
            Self::Parts => "__exs_parts",
            Self::Same => "__exs_same",
            Self::HasEq => "__exs_has_eq",
        }
    }

    /// Returns the number of arguments the intrinsic takes.
    const fn arity(self) -> usize {
        match self {
            Self::Equality | Self::Same => 2,
            Self::Parts | Self::HasEq => 1,
        }
    }
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

    /// Compiles a call of `intrinsic` with `arguments`, leaving its result on the stack.
    pub(super) fn compile_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if arguments.len() != intrinsic.arity() {
            return Err(error(
                "E0208",
                span,
                format!(
                    "`{}` expects {} arguments",
                    intrinsic.name(),
                    intrinsic.arity()
                ),
            ));
        }
        let mut locals = Vec::with_capacity(arguments.len());
        for argument in arguments {
            self.compile_expression(argument)?;
            locals.push(self.store_new());
        }
        match intrinsic {
            Intrinsic::Equality => {
                self.push_vm();
                self.instruction(Instruction::LocalGet(locals[0]));
                self.instruction(Instruction::LocalGet(locals[1]));
                self.core_call("__exs_core_equal", span)?;
                self.instruction(Instruction::I64ExtendI32U);
                let equality = self.scalar_local();
                self.instruction(Instruction::LocalSet(equality));
                immediate::push_small_int(&mut self.function, equality);
            }
            Intrinsic::Parts => {
                self.set_position(span)?;
                self.push_vm();
                self.instruction(Instruction::LocalGet(locals[0]));
                self.core_call("__exs_core_parts", span)?;
            }
            Intrinsic::Same => {
                self.instruction(Instruction::LocalGet(locals[0]));
                self.instruction(Instruction::LocalGet(locals[1]));
                self.instruction(Instruction::I64Eq);
                immediate::bool_from_condition(&mut self.function);
            }
            Intrinsic::HasEq => {
                let context = self.context;
                let targets = context.dispatch.operator(TraitOperator::Equal);
                self.instruction(Instruction::I32Const(0));
                for target in targets {
                    self.instruction(Instruction::LocalGet(locals[0]));
                    self.type_tag_test(target.type_id, span)?;
                    self.instruction(Instruction::I32Or);
                }
                immediate::bool_from_condition(&mut self.function);
            }
        }
        Ok(())
    }
}
