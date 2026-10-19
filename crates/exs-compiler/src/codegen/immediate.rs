//! Inline Wasm sequences that operate on 64-bit tagged `ValueRef`s without runtime calls.
//!
//! Both the direct and the continuation lowering use these helpers for type tests, Bool
//! conditions, scalar literals and Int arithmetic or comparisons on immediate operands. Every
//! helper reproduces the runtime result exactly; operands that are not immediates keep using the
//! existing runtime calls and operator dispatch.

use exs_value::ValueRef;
use exs_value::layout::{BOOL_TAG, EXTRA_SHIFT, KIND_SHIFT, SMALL_INT_TAG, object_tag};
use wasm_encoder::{BlockType, Function, Instruction};

use super::VALUE_TYPE;
use super::trait_registry::TraitOperator;
use crate::ast::BinaryOperator;

/// Pushes one constant `ValueRef`, such as an immediate literal, as its raw `i64`.
pub(super) fn push(function: &mut Function, value: ValueRef) {
    function.instruction(&Instruction::I64Const(value.raw().cast_signed()));
}

/// Replaces the `ValueRef` on the stack with 1 when its type flags intersect `mask`, else 0.
pub(super) fn type_mask_test(function: &mut Function, mask: u32) {
    function.instruction(&Instruction::I64Const(i64::from(mask) << KIND_SHIFT));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::I64Ne);
}

/// Replaces the `ValueRef` on the stack with 1 when it is an Object carrying `type_id`.
///
/// Returns `false` without emitting anything when the id is too large for the extra bits; the
/// caller must then keep the runtime test.
pub(super) fn object_type_test(function: &mut Function, type_id: u32) -> bool {
    let Some(tag) = object_tag(type_id) else {
        return false;
    };
    tag_test(function, tag);
    true
}

/// Replaces the `ValueRef` on the stack with 1 when it is an immediate Bool.
pub(super) fn is_bool_test(function: &mut Function) {
    tag_test(function, BOOL_TAG);
}

/// Replaces the `ValueRef` on the stack with 1 when its upper 32 bits equal `tag`.
fn tag_test(function: &mut Function, tag: u32) {
    function.instruction(&Instruction::I64Const(i64::from(EXTRA_SHIFT)));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I64Const(i64::from(tag)));
    function.instruction(&Instruction::I64Eq);
}

/// Converts the Wasm i32 condition (0 or 1) on the stack into an immediate Bool `ValueRef`.
pub(super) fn bool_from_condition(function: &mut Function) {
    function.instruction(&Instruction::I64ExtendI32U);
    push(function, ValueRef::FALSE);
    function.instruction(&Instruction::I64Or);
}

/// Pushes 1 when both `ValueRef` locals hold immediate Ints, else 0.
pub(super) fn both_small_ints_test(function: &mut Function, left: u32, right: u32) {
    function.instruction(&Instruction::LocalGet(left));
    tag_test(function, SMALL_INT_TAG);
    function.instruction(&Instruction::LocalGet(right));
    tag_test(function, SMALL_INT_TAG);
    function.instruction(&Instruction::I32And);
}

/// One binary source operator with an inline fast path for two immediate Int operands.
#[derive(Clone, Copy)]
pub(super) enum IntOperation {
    /// Checked addition.
    Add,
    /// Checked subtraction.
    Subtract,
    /// Checked multiplication.
    Multiply,
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Strict less-than.
    LessThan,
    /// Less-than-or-equal.
    LessOrEqual,
    /// Strict greater-than.
    GreaterThan,
    /// Greater-than-or-equal.
    GreaterOrEqual,
}

impl IntOperation {
    /// Returns the inline operation for one source binary operator, if it has one.
    pub(super) const fn from_binary(operator: BinaryOperator) -> Option<Self> {
        match operator {
            BinaryOperator::Add => Some(Self::Add),
            BinaryOperator::Subtract => Some(Self::Subtract),
            BinaryOperator::Multiply => Some(Self::Multiply),
            BinaryOperator::Equal => Some(Self::Equal),
            BinaryOperator::NotEqual => Some(Self::NotEqual),
            BinaryOperator::LessThan => Some(Self::LessThan),
            BinaryOperator::LessOrEqual => Some(Self::LessOrEqual),
            BinaryOperator::GreaterThan => Some(Self::GreaterThan),
            BinaryOperator::GreaterOrEqual => Some(Self::GreaterOrEqual),
            BinaryOperator::Divide | BinaryOperator::And | BinaryOperator::Or => None,
        }
    }

    /// Returns the inline operation for one standard operator trait, if it has one.
    pub(super) const fn from_trait(operator: TraitOperator) -> Option<Self> {
        match operator {
            TraitOperator::Add => Some(Self::Add),
            TraitOperator::Subtract => Some(Self::Subtract),
            TraitOperator::Multiply => Some(Self::Multiply),
            TraitOperator::Equal => Some(Self::Equal),
            TraitOperator::NotEqual => Some(Self::NotEqual),
            TraitOperator::LessThan => Some(Self::LessThan),
            TraitOperator::LessOrEqual => Some(Self::LessOrEqual),
            TraitOperator::GreaterThan => Some(Self::GreaterThan),
            TraitOperator::GreaterOrEqual => Some(Self::GreaterOrEqual),
            TraitOperator::Divide => None,
        }
    }

    /// Returns whether the operation produces a Bool rather than an Int.
    pub(super) const fn is_comparison(self) -> bool {
        !matches!(self, Self::Add | Self::Subtract | Self::Multiply)
    }

    /// Pushes the result `ValueRef` for two `ValueRef` locals that both hold immediate Ints.
    ///
    /// Arithmetic on two `i32` values cannot overflow `i64`, so the only remaining case is a
    /// result outside the `i32` range, which `int_new` boxes exactly like the runtime would.
    /// `temporary` is an i64 scratch local and `int_new` the `__exs_rt_int_new` function index.
    pub(super) fn emit(
        self,
        function: &mut Function,
        left: u32,
        right: u32,
        temporary: u32,
        int_new: u32,
    ) {
        match self {
            Self::Equal | Self::NotEqual => {
                // Immediate Ints are canonical, so equal values have equal raw references.
                function.instruction(&Instruction::LocalGet(left));
                function.instruction(&Instruction::LocalGet(right));
                function.instruction(&if matches!(self, Self::Equal) {
                    Instruction::I64Eq
                } else {
                    Instruction::I64Ne
                });
                bool_from_condition(function);
            }
            Self::LessThan | Self::LessOrEqual | Self::GreaterThan | Self::GreaterOrEqual => {
                function.instruction(&Instruction::LocalGet(left));
                function.instruction(&Instruction::I32WrapI64);
                function.instruction(&Instruction::LocalGet(right));
                function.instruction(&Instruction::I32WrapI64);
                function.instruction(&match self {
                    Self::LessThan => Instruction::I32LtS,
                    Self::LessOrEqual => Instruction::I32LeS,
                    Self::GreaterThan => Instruction::I32GtS,
                    _ => Instruction::I32GeS,
                });
                bool_from_condition(function);
            }
            Self::Add | Self::Subtract | Self::Multiply => {
                payload_as_i64(function, left);
                payload_as_i64(function, right);
                function.instruction(&match self {
                    Self::Add => Instruction::I64Add,
                    Self::Subtract => Instruction::I64Sub,
                    _ => Instruction::I64Mul,
                });
                function.instruction(&Instruction::LocalTee(temporary));
                function.instruction(&Instruction::I32WrapI64);
                function.instruction(&Instruction::I64ExtendI32S);
                function.instruction(&Instruction::LocalGet(temporary));
                function.instruction(&Instruction::I64Eq);
                function.instruction(&Instruction::If(BlockType::Result(VALUE_TYPE)));
                function.instruction(&Instruction::LocalGet(temporary));
                function.instruction(&Instruction::I64Const(0xFFFF_FFFF));
                function.instruction(&Instruction::I64And);
                function.instruction(&Instruction::I64Const(
                    i64::from(SMALL_INT_TAG) << EXTRA_SHIFT,
                ));
                function.instruction(&Instruction::I64Or);
                function.instruction(&Instruction::Else);
                function.instruction(&Instruction::LocalGet(temporary));
                function.instruction(&Instruction::Call(int_new));
                function.instruction(&Instruction::End);
            }
        }
    }
}

/// Pushes the sign-extended `i32` payload of one immediate Int local as an `i64`.
fn payload_as_i64(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I64ExtendI32S);
}
