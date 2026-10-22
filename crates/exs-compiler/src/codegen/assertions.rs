//! The test assertions `assert(condition[, description])` and
//! `assert_eq(actual, expected[, description])`.
//!
//! Like v1 they are standard intrinsics: a failed assertion returns a fatal AssertionFailed
//! Error from the function, and an omitted description is a compiler-provided String.

use wasm_encoder::Instruction;

use super::function::{FunctionCompiler, error};
use crate::ast::{BinaryOperator, Expression};
use crate::codegen::declarations::contains_statements;
use crate::codegen::standard::{ASSERT_DEFAULT_DESCRIPTION, ASSERT_EQ_DEFAULT_DESCRIPTION};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// One of the two assertion intrinsics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Assertion {
    /// `assert(condition[, description])`.
    Assert,
    /// `assert_eq(actual, expected[, description])`.
    AssertEq,
}

impl Assertion {
    /// Returns the assertion a call of `callee` evaluates, if any.
    pub(super) fn from_callee(callee: &str) -> Option<Self> {
        match callee {
            "assert" | "std::test::assert" => Some(Self::Assert),
            "assert_eq" | "std::test::assert_eq" => Some(Self::AssertEq),
            _ => None,
        }
    }

    /// Returns the description of an assertion that omits one.
    pub(super) const fn default_description(self) -> &'static str {
        match self {
            Self::Assert => ASSERT_DEFAULT_DESCRIPTION,
            Self::AssertEq => ASSERT_EQ_DEFAULT_DESCRIPTION,
        }
    }

    /// Returns the number of values the assertion tests, before its optional description.
    const fn operands(self) -> usize {
        match self {
            Self::Assert => 1,
            Self::AssertEq => 2,
        }
    }

    /// Returns the name diagnostics use.
    const fn name(self) -> &'static str {
        match self {
            Self::Assert => "assert",
            Self::AssertEq => "assert_eq",
        }
    }
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles one call of `assertion` and leaves its None result on the stack.
    pub(super) fn compile_assertion(
        &mut self,
        assertion: Assertion,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let operands = assertion.operands();
        if !(operands..=operands + 1).contains(&arguments.len()) {
            return Err(error(
                "E0208",
                span,
                format!(
                    "{} expects {} or {} arguments but received {}",
                    assertion.name(),
                    operands,
                    operands + 1,
                    arguments.len()
                ),
            ));
        }
        let mut values = Vec::with_capacity(operands);
        for (index, argument) in arguments[..operands].iter().enumerate() {
            let later = &arguments[index + 1..];
            values.push(self.operand(argument, !later.iter().any(contains_statements))?);
        }
        let (condition, compared) = if assertion == Assertion::AssertEq {
            self.compile_slow_binary(BinaryOperator::Equal, values[0], values[1], span)?;
            (self.store_new(), Some((values[0], values[1])))
        } else {
            (values[0], None)
        };
        match arguments.get(operands) {
            Some(description) => self.compile_expression(description)?,
            None => self.push_string(assertion.default_description(), span)?,
        }
        let description = self.store_new();
        self.assert_values(condition, compared, description, span)
    }

    /// Calls the core library's assertion on the rooted locals `condition` and `description`,
    /// with the compared `actual` and `expected` locals for `assert_eq`, and returns its fatal
    /// Error from the function. Leaves the None result on the stack.
    pub(super) fn assert_values(
        &mut self,
        condition: u32,
        compared: Option<(u32, u32)>,
        description: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::LocalGet(condition));
        if let Some((actual, expected)) = compared {
            self.instruction(Instruction::LocalGet(actual));
            self.instruction(Instruction::LocalGet(expected));
            self.instruction(Instruction::LocalGet(description));
            self.core_call("__exs_core_assert_eq", span)?;
        } else {
            self.instruction(Instruction::LocalGet(description));
            self.core_call("__exs_core_assert", span)?;
        }
        let result = self.store_new();
        self.return_if_fatal(result);
        self.instruction(Instruction::LocalGet(result));
        Ok(())
    }
}
