//! Lowering of enum constructors, `match` and `is`.
//!
//! Enum values are `EnumObject`s whose references carry the enum's type tag like typed Objects,
//! so variant tests compare the tag inline and then the variant index in the object.

use std::collections::HashMap;

use exs_layout::TYPE_OBJECT;
use exs_layout::value::layout::{EXTRA_SHIFT, HEAP_TYPE_EXTRA, nominal_tag, tag, type_kind};
use wasm_encoder::{BlockType, Instruction, ValType};

use super::collections::{u32_field, value_field};
use super::function::{FunctionCompiler, error};
use crate::ast::{Expression, Identifier, MatchArm, MatchArmBody, MatchPattern, TypeAnnotation};
use crate::codegen::match_arms::{ArmTest, MatchArms, check_bindings};
use crate::codegen::types::{EnumVariant, NominalKind};
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the header word, which holds the type id of typed Objects and enums.
const HEADER_WORD: u64 = 4;
/// Byte offset of the variant index inside an enum object.
const VARIANT: u64 = core::mem::offset_of!(exs_layout::EnumObject, variant) as u64;
/// Byte offset of the payload field count inside an enum object.
const FIELD_COUNT: u64 = core::mem::offset_of!(exs_layout::EnumObject, field_count) as u64;
/// Byte offset of the first payload field of an enum object.
const FIELDS: u64 = size_of::<exs_layout::EnumObject>() as u64;

impl<'a> FunctionCompiler<'a, '_> {
    /// Returns the variant a unit-variant name, a call or a static call `name` constructs.
    pub(super) fn enum_variant(&self, name: &str) -> Option<&EnumVariant> {
        self.context.types.enum_variant(name)
    }

    /// Returns the type id of the enum with the host-boundary identity `identity`.
    pub(super) fn enum_type_id(
        &self,
        identity: &str,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.context
            .types
            .nominal_types()
            .find(|nominal| {
                nominal.kind == NominalKind::Enum
                    && nominal.enum_type_id.as_deref() == Some(identity)
            })
            .map(|nominal| nominal.id)
            .ok_or_else(|| error("E0999", span, format!("unknown enum identity `{identity}`")))
    }

    /// Replaces the `ValueRef` on the stack with 1 when it is a value of the nominal type
    /// `type_id`: through its inline tag, or for a type id too large to inline through the
    /// type id in its heap header.
    pub(super) fn type_tag_test(
        &mut self,
        type_id: u32,
        _span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if let Some(expected) = nominal_tag(type_id) {
            self.instruction(Instruction::I64Const(i64::from(EXTRA_SHIFT)));
            self.instruction(Instruction::I64ShrU);
            self.instruction(Instruction::I64Const(i64::from(expected)));
            self.instruction(Instruction::I64Eq);
            return Ok(());
        }
        let value = self.scalar_local();
        self.instruction(Instruction::LocalTee(value));
        self.instruction(Instruction::I64Const(i64::from(EXTRA_SHIFT)));
        self.instruction(Instruction::I64ShrU);
        self.instruction(Instruction::I64Const(i64::from(tag(
            type_kind(TYPE_OBJECT),
            HEAP_TYPE_EXTRA,
        ))));
        self.instruction(Instruction::I64Eq);
        self.instruction(Instruction::If(BlockType::Result(ValType::I32)));
        // Typed Objects and enums keep their type id in the header word.
        self.instruction(Instruction::LocalGet(value));
        self.instruction(Instruction::I32WrapI64);
        self.instruction(Instruction::I32Load(u32_field(HEADER_WORD)));
        self.instruction(Instruction::I32Const(type_id.cast_signed()));
        self.instruction(Instruction::I32Eq);
        self.instruction(Instruction::Else);
        self.instruction(Instruction::I32Const(0));
        self.instruction(Instruction::End);
        Ok(())
    }

    /// Constructs one enum value after checking each payload against its declared contract
    /// in source order.
    pub(super) fn compile_enum_variant(
        &mut self,
        variant: &EnumVariant,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if variant.fields.len() != arguments.len() {
            return Err(error(
                "E0208",
                span,
                format!(
                    "enum variant `{}` expects {} arguments but received {}",
                    variant.name,
                    variant.fields.len(),
                    arguments.len()
                ),
            ));
        }
        let count = u32::try_from(arguments.len())
            .map_err(|_| error("E0212", span, "too many enum payload fields".to_owned()))?;
        let block = (count > 0).then(|| self.consecutive_locals(count));
        if let Some(block) = block {
            for ((offset, argument), contract) in (0..count).zip(arguments).zip(&variant.fields) {
                self.compile_expression(argument)?;
                self.store(block + offset);
                self.check_contract(block + offset, contract, span)?;
            }
        }
        self.push_vm();
        self.instruction(Instruction::I32Const(variant.type_id.cast_signed()));
        self.instruction(Instruction::I32Const(variant.index.cast_signed()));
        match block {
            Some(block) => self.push_slot_address(block),
            None => self.instruction(Instruction::I32Const(0)),
        }
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.core_call("__exs_core_enum_new", span)
    }

    /// Compiles a unit variant named by a bare identifier, which resolves before bindings.
    pub(super) fn compile_unit_variant(
        &mut self,
        variant: &EnumVariant,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if !variant.fields.is_empty() {
            return Err(error(
                "E0208",
                span,
                format!(
                    "enum variant `{}` expects {} arguments",
                    variant.name,
                    variant.fields.len()
                ),
            ));
        }
        self.compile_enum_variant(variant, &[], span)
    }

    /// Lowers one `match` into a chain of variant tests inside one Wasm block, validating each
    /// arm right before it.
    ///
    /// Each matching arm stores its value and branches past the remaining arms; when no
    /// wildcard covers unmatched values, they produce the MatchError, which returns.
    pub(super) fn compile_match(
        &mut self,
        value: &Expression<'a>,
        arms: &[MatchArm<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(value)?;
        let subject = self.store_new();
        let result = self.value_local();
        self.instruction(Instruction::Block(BlockType::Empty));
        let done = self.depth;
        let mut validation = MatchArms::new(self.context.types);
        for arm in arms {
            match validation.arm(arm)? {
                ArmTest::Wildcard => {
                    self.compile_match_arm(subject, arm, &[], result)?;
                    self.branch_to(done);
                }
                ArmTest::Literal => {
                    let MatchPattern::Literal(literal) = &arm.pattern else {
                        return Err(error(
                            "E0999",
                            arm.span,
                            "literal arm without a literal pattern".to_owned(),
                        ));
                    };
                    self.compile_expression(literal)?;
                    let expected = self.store_new();
                    self.push_vm();
                    self.instruction(Instruction::LocalGet(subject));
                    self.instruction(Instruction::LocalGet(expected));
                    self.core_call("__exs_core_equal", arm.span)?;
                    self.instruction(Instruction::I32Const(
                        exs_layout::EQUALITY_SAME.cast_signed(),
                    ));
                    self.instruction(Instruction::I32Eq);
                    self.instruction(Instruction::If(BlockType::Empty));
                    self.compile_match_arm(subject, arm, &[], result)?;
                    self.branch_to(done);
                    self.instruction(Instruction::End);
                }
                ArmTest::Variant { .. } => {
                    let MatchPattern::Variant {
                        type_name,
                        variant,
                        bindings,
                        span: pattern_span,
                    } = &arm.pattern
                    else {
                        return Err(error(
                            "E0999",
                            arm.span,
                            "variant arm without a variant pattern".to_owned(),
                        ));
                    };
                    let key = format!("{}::{}", type_name.name, variant.name);
                    let (type_id, index) = self
                        .enum_variant(&key)
                        .map(|variant| (variant.type_id, variant.index))
                        .ok_or_else(|| {
                            error(
                                "E0216",
                                *pattern_span,
                                crate::diagnostic::unknown_name("enum variant", &key),
                            )
                        })?;
                    self.instruction(Instruction::LocalGet(subject));
                    self.type_tag_test(type_id, *pattern_span)?;
                    self.instruction(Instruction::If(BlockType::Empty));
                    self.push_address(subject);
                    self.instruction(Instruction::I32Load(u32_field(VARIANT)));
                    self.instruction(Instruction::I32Const(index.cast_signed()));
                    self.instruction(Instruction::I32Eq);
                    self.instruction(Instruction::If(BlockType::Empty));
                    self.compile_match_arm(subject, arm, bindings, result)?;
                    self.branch_to(done);
                    self.instruction(Instruction::End);
                    self.instruction(Instruction::End);
                }
            }
        }
        if validation.finish(span)? {
            self.set_position(span)?;
            self.push_vm();
            self.instruction(Instruction::LocalGet(subject));
            self.core_call("__exs_core_match_error", span)?;
            self.store(result);
            self.return_if_error(result, span)?;
        }
        self.instruction(Instruction::End);
        self.instruction(Instruction::LocalGet(result));
        Ok(())
    }

    /// Binds one arm's payload fields in a new scope and stores the arm's value in `result`.
    ///
    /// A field the matched value lacks binds the MatchError.
    fn compile_match_arm(
        &mut self,
        subject: u32,
        arm: &MatchArm<'a>,
        bindings: &[Identifier<'a>],
        result: u32,
    ) -> Result<(), CompileDiagnostics<'a>> {
        check_bindings(bindings)?;
        let mut scope = HashMap::with_capacity(bindings.len());
        for (index, binding) in (0_u32..).zip(bindings) {
            self.push_address(subject);
            self.instruction(Instruction::I32Load(u32_field(FIELD_COUNT)));
            self.instruction(Instruction::I32Const(index.cast_signed()));
            self.instruction(Instruction::I32GtU);
            self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
            self.push_address(subject);
            self.instruction(Instruction::I64Load(value_field(
                FIELDS + u64::from(index) * 8,
            )));
            self.instruction(Instruction::Else);
            self.set_position(binding.span)?;
            self.push_vm();
            self.instruction(Instruction::LocalGet(subject));
            self.core_call("__exs_core_enum_missing_field", binding.span)?;
            self.instruction(Instruction::End);
            let local = self.store_new();
            if self.is_cell(&binding.name) {
                self.wrap_in_cell(local)?;
            }
            scope.insert(binding.name.clone(), local);
        }
        self.scopes.push(scope);
        match &arm.body {
            MatchArmBody::Expression(value) => self.compile_expression(value)?,
            MatchArmBody::Block(block) => {
                self.compile_block(block)?;
                immediate::push(&mut self.function, exs_layout::ValueRef::NONE);
            }
        }
        self.store(result);
        self.scopes.pop();
        Ok(())
    }

    /// Compiles a non-throwing test of one value against a resolved type contract.
    pub(super) fn compile_is_type(
        &mut self,
        value: &Expression<'a>,
        annotation: &TypeAnnotation<'a>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compile_expression(value)?;
        let local = self.store_new();
        let contract = self
            .context
            .types
            .resolve(Some(annotation), annotation.span)?;
        self.contract_test(local, &contract, span)?;
        immediate::bool_from_condition(&mut self.function);
        Ok(())
    }
}
