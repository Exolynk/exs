//! Inline Wasm sequences that operate on 64-bit tagged `ValueRef`s without runtime calls.
//!
//! Both the direct and the continuation lowering use these helpers for type tests, Bool
//! conditions, scalar literals and Int arithmetic or comparisons on immediate operands. Every
//! helper reproduces the runtime result exactly; operands that are not immediates keep using the
//! existing runtime calls and operator dispatch.

use exs_layout::ValueRef;
use exs_layout::value::layout::{BOOL_TAG, EXTRA_SHIFT, KIND_SHIFT, SMALL_INT_TAG};
use wasm_encoder::{BlockType, Function, Instruction};

use super::VALUE_TYPE;
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

/// Pushes the sign-extended `i32` payload of one immediate Int local as a plain `i64`.
pub(super) fn push_payload(function: &mut Function, local: u32) {
    payload_as_i64(function, local);
}

/// Pushes the immediate Int `ValueRef` of one plain `i64` local whose value fits `i32`.
pub(super) fn push_small_int(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(0xFFFF_FFFF));
    function.instruction(&Instruction::I64And);
    function.instruction(&Instruction::I64Const(
        i64::from(SMALL_INT_TAG) << EXTRA_SHIFT,
    ));
    function.instruction(&Instruction::I64Or);
}

/// One operand of an inline operation: a `ValueRef` local, or an immediate Int known at
/// compile time, which needs no local, root slot or type test.
#[derive(Clone, Copy)]
pub(super) enum Operand {
    /// A local holding any `ValueRef`.
    Local(u32),
    /// An immediate Int with this payload.
    SmallInt(i32),
}

impl Operand {
    /// Pushes the operand's `ValueRef`.
    pub(super) fn push(self, function: &mut Function) {
        match self {
            Self::Local(local) => function.instruction(&Instruction::LocalGet(local)),
            Self::SmallInt(value) => {
                function.instruction(&Instruction::I64Const(small_int_raw(value).cast_signed()))
            }
        };
    }

    /// Pushes the operand's Int payload as an `i32`; the operand must be an immediate Int.
    fn push_i32(self, function: &mut Function) {
        match self {
            Self::Local(local) => {
                function.instruction(&Instruction::LocalGet(local));
                function.instruction(&Instruction::I32WrapI64);
            }
            Self::SmallInt(value) => {
                function.instruction(&Instruction::I32Const(value));
            }
        }
    }

    /// Pushes the operand's sign-extended Int payload as an `i64`; the operand must be an
    /// immediate Int.
    fn push_i64(self, function: &mut Function) {
        match self {
            Self::Local(local) => payload_as_i64(function, local),
            Self::SmallInt(value) => {
                function.instruction(&Instruction::I64Const(i64::from(value)));
            }
        }
    }
}

/// Returns the raw `ValueRef` of the immediate Int `value`.
const fn small_int_raw(value: i32) -> u64 {
    ((SMALL_INT_TAG as u64) << EXTRA_SHIFT) | value.cast_unsigned() as u64
}

/// Returns the payload of a raw `ValueRef` when it is an immediate Int.
pub(super) fn small_int_payload(raw: u64) -> Option<i32> {
    if raw >> EXTRA_SHIFT != u64::from(SMALL_INT_TAG) {
        return None;
    }
    u32::try_from(raw & 0xFFFF_FFFF).ok().map(u32::cast_signed)
}

/// Pushes 1 when both operands hold immediate Ints, else 0. Constant operands need no test.
pub(super) fn both_small_ints_test(function: &mut Function, left: Operand, right: Operand) {
    let mut tested = 0;
    for operand in [left, right] {
        if let Operand::Local(local) = operand {
            function.instruction(&Instruction::LocalGet(local));
            tag_test(function, SMALL_INT_TAG);
            tested += 1;
        }
    }
    match tested {
        0 => {
            function.instruction(&Instruction::I32Const(1));
        }
        2 => {
            function.instruction(&Instruction::I32And);
        }
        _ => {}
    }
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
            BinaryOperator::Divide
            | BinaryOperator::Remainder
            | BinaryOperator::And
            | BinaryOperator::Or => None,
        }
    }

    /// Returns whether the operation compares its operands and yields a Bool.
    pub(super) const fn is_comparison(self) -> bool {
        !matches!(self, Self::Add | Self::Subtract | Self::Multiply)
    }

    /// Pushes the Wasm `i32` condition (0 or 1) of a comparison of two operands that both
    /// hold immediate Ints. Arithmetic operations push nothing.
    pub(super) fn emit_test(self, function: &mut Function, left: Operand, right: Operand) {
        match self {
            Self::Equal | Self::NotEqual => {
                // Immediate Ints are canonical, so equal values have equal raw references.
                left.push(function);
                right.push(function);
                function.instruction(&if matches!(self, Self::Equal) {
                    Instruction::I64Eq
                } else {
                    Instruction::I64Ne
                });
            }
            Self::LessThan | Self::LessOrEqual | Self::GreaterThan | Self::GreaterOrEqual => {
                left.push_i32(function);
                right.push_i32(function);
                function.instruction(&match self {
                    Self::LessThan => Instruction::I32LtS,
                    Self::LessOrEqual => Instruction::I32LeS,
                    Self::GreaterThan => Instruction::I32GtS,
                    _ => Instruction::I32GeS,
                });
            }
            Self::Add | Self::Subtract | Self::Multiply => {}
        }
    }

    /// Pushes the result `ValueRef` for two operands that both hold immediate Ints.
    ///
    /// Arithmetic on two `i32` values cannot overflow `i64`, so the only remaining case is a
    /// result outside the `i32` range, which `int_new` boxes exactly like the runtime would.
    /// `temporary` is an i64 scratch local and `int_new` the `__exs_rt_int_new` function index.
    pub(super) fn emit(
        self,
        function: &mut Function,
        left: Operand,
        right: Operand,
        temporary: u32,
        int_new: u32,
    ) {
        match self {
            Self::Add | Self::Subtract | Self::Multiply => {
                left.push_i64(function);
                right.push_i64(function);
                function.instruction(&match self {
                    Self::Add => Instruction::I64Add,
                    Self::Subtract => Instruction::I64Sub,
                    _ => Instruction::I64Mul,
                });
                push_int_result(function, temporary, int_new);
            }
            _ => {
                self.emit_test(function, left, right);
                bool_from_condition(function);
            }
        }
    }
}

/// Converts the `i64` Int on the stack into its `ValueRef`.
///
/// Values in the `i32` range become immediates; `int_new` boxes the rest exactly like the
/// runtime would. `temporary` is an i64 scratch local.
fn push_int_result(function: &mut Function, temporary: u32, int_new: u32) {
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

/// Pushes the sign-extended `i32` payload of one immediate Int local as an `i64`.
fn payload_as_i64(function: &mut Function, local: u32) {
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I64ExtendI32S);
}

#[cfg(test)]
mod tests {
    use exs_layout::ValueRef;

    use super::{small_int_payload, small_int_raw};

    /// Constant Int operands encode like the runtime's immediate Ints and decode back.
    #[test]
    fn small_int_constants_match_value_refs() {
        for value in [0, 1, -1, 42, i32::MIN, i32::MAX] {
            let raw = ValueRef::small_int(i64::from(value)).map(ValueRef::raw);
            assert_eq!(raw, Some(small_int_raw(value)));
            assert_eq!(small_int_payload(small_int_raw(value)), Some(value));
        }
        assert_eq!(small_int_payload(ValueRef::TRUE.raw()), None);
    }
}
