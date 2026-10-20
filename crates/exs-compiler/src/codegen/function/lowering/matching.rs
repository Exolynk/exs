use std::collections::HashMap;

use super::*;
use crate::ast::{Identifier, MatchArm, MatchArmBody, MatchPattern};
use crate::codegen::match_arms::{ArmTest, MatchArms, check_bindings};

impl<'a, 'module> FunctionCompiler<'a, 'module> {
    /// Lowers one enum-pattern `match` into a chain of variant tests inside one Wasm block.
    ///
    /// Each matching arm stores its value and branches past the remaining arms; without a
    /// wildcard, a value no arm matched returns the match error.
    pub(in crate::codegen::function) fn compile_match(
        &mut self,
        value: &Expression<'a>,
        arms: &[MatchArm<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(value)?;
        let subject = self.store_stack_value()?;
        let result = self.allocate_local();
        self.function
            .instruction(&Instruction::Block(BlockType::Empty));
        self.enter_control()?;
        let done_depth = self.control_depth;
        let mut validation = MatchArms::new(self.types);
        for arm in arms {
            match validation.arm(arm)? {
                ArmTest::Wildcard => {
                    self.compile_match_arm(subject, arm, &[], result)?;
                    self.branch_to(done_depth, arm.span)?;
                }
                ArmTest::Variant {
                    type_identity,
                    variant,
                } => {
                    let MatchPattern::Variant {
                        bindings,
                        span: pattern_span,
                        ..
                    } = &arm.pattern
                    else {
                        return Err(diagnostics(CompileDiagnostic::new(
                            "E0999",
                            arm.span,
                            "variant arm without a variant pattern",
                        )));
                    };
                    self.function.instruction(&Instruction::LocalGet(subject));
                    self.compile_string(&type_identity, *pattern_span)?;
                    self.compile_string(&variant, *pattern_span)?;
                    self.runtime_call("__exs_rt_enum_matches_literal", *pattern_span)?;
                    immediate::push(&mut self.function, ValueRef::boolean(true));
                    self.function.instruction(&Instruction::I64Eq);
                    self.function
                        .instruction(&Instruction::If(BlockType::Empty));
                    self.enter_control()?;
                    self.compile_match_arm(subject, arm, bindings, result)?;
                    self.branch_to(done_depth, arm.span)?;
                    self.function.instruction(&Instruction::End);
                    self.exit_control()?;
                }
            }
        }
        if validation.finish(span)? {
            self.function.instruction(&Instruction::LocalGet(subject));
            self.runtime_call("__exs_rt_match_error", span)?;
            self.return_if_error(span)?;
            self.store_stack_value_in(result)?;
        }
        self.function.instruction(&Instruction::End);
        self.exit_control()?;
        self.clear_root_slot(subject)?;
        self.function.instruction(&Instruction::LocalGet(result));
        self.clear_root_slot(result)
    }

    /// Binds one arm's payload fields in a new scope and stores the arm value in `result`.
    fn compile_match_arm(
        &mut self,
        subject: u32,
        arm: &MatchArm<'a>,
        bindings: &[Identifier<'a>],
        result: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        check_bindings(bindings)?;
        let mut scope = HashMap::new();
        for (index, binding) in bindings.iter().enumerate() {
            let index = i32::try_from(index).map_err(|_| {
                diagnostics(CompileDiagnostic::new(
                    "E0212",
                    binding.span,
                    "too many enum payload bindings",
                ))
            })?;
            self.function.instruction(&Instruction::LocalGet(subject));
            self.function.instruction(&Instruction::I32Const(index));
            self.runtime_call("__exs_rt_enum_field", binding.span)?;
            let local = self.store_stack_value()?;
            scope.insert(binding.name.clone(), local);
        }
        self.scopes.push(scope);
        match &arm.body {
            MatchArmBody::Expression(value) => self.compile_expression(value)?,
            MatchArmBody::Block(block) => {
                self.compile_block(block, true)?;
                self.emit_none();
            }
        }
        self.store_stack_value_in(result)?;
        if let Some(scope) = self.scopes.pop() {
            for local in scope.into_values() {
                self.clear_root_slot(local)?;
            }
        }
        Ok(())
    }
}
