//! Operations of the continuation graph that compute one value without suspending.
//!
//! Most of them rebuild the source expression over temporary bindings of their operands and
//! compile it with the synchronous lowering, so resumable code has its semantics exactly.

use std::collections::HashMap;

use core::mem::{offset_of, size_of};

use exs_abi::BuiltinMethod;
use exs_layout::{BufferObject, CellObject, ListObject};
use exs_value::ValueRef;
use wasm_encoder::{BlockType, Instruction};

use super::step::StepEmitter;
use crate::ast::{AssignmentTarget, Expression, Identifier, ObjectProperty, Statement};
use crate::codegen::continuation::{FormatPart, Operation};
use crate::codegen::v2::closures::closure_span_key;
use crate::codegen::v2::collections::{u32_field, value_field};
use crate::codegen::v2::function::error;
use crate::codegen::{VALUE_TYPE, immediate};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Byte offset of the value of a Cell.
const CELL_VALUE: u64 = offset_of!(CellObject, value) as u64;
/// Byte offset of the element count of a List.
const LIST_LEN: u64 = offset_of!(ListObject, len) as u64;
/// Byte offset of the element Buffer of a List.
const LIST_ITEMS: u64 = offset_of!(ListObject, items) as u64;
/// Byte offset of the first slot of a Buffer.
const BUFFER_SLOTS: u64 = size_of::<BufferObject>() as u64;
/// Byte offset of the variant index of an enum.
const VARIANT: u64 = offset_of!(exs_layout::EnumObject, variant) as u64;
/// Byte offset of the payload field count of an enum.
const FIELD_COUNT: u64 = offset_of!(exs_layout::EnumObject, field_count) as u64;
/// Byte offset of the first field slot of a typed Object.
const TYPED_SLOTS: u64 = size_of::<exs_layout::TypedObject>() as u64;
/// Byte offset of the first payload field of an enum.
const FIELDS: u64 = size_of::<exs_layout::EnumObject>() as u64;

/// The static functions v1's graph names by their runtime export, by type and method.
const STATIC_EXPORTS: [(&str, &str, &str); 22] = [
    ("__exs_rt_bytes_from_list", "Bytes", "from_list"),
    ("__exs_rt_bytes_from_utf8", "Bytes", "from_utf8"),
    ("__exs_rt_bytes_from_hex", "Bytes", "from_hex"),
    ("__exs_rt_bytes_from_base64", "Bytes", "from_base64"),
    ("__exs_rt_integer_parse", "Int", "parse"),
    ("__exs_rt_float_parse", "Float", "parse"),
    ("__exs_rt_object_from_entries", "Object", "from_entries"),
    ("__exs_rt_math_pi", "Math", "pi"),
    ("__exs_rt_math_tau", "Math", "tau"),
    ("__exs_rt_math_e", "Math", "e"),
    ("__exs_rt_math_sin", "Math", "sin"),
    ("__exs_rt_math_cos", "Math", "cos"),
    ("__exs_rt_math_tan", "Math", "tan"),
    ("__exs_rt_math_asin", "Math", "asin"),
    ("__exs_rt_math_acos", "Math", "acos"),
    ("__exs_rt_math_atan", "Math", "atan"),
    ("__exs_rt_math_exp", "Math", "exp"),
    ("__exs_rt_math_ln", "Math", "ln"),
    ("__exs_rt_math_log2", "Math", "log2"),
    ("__exs_rt_math_log10", "Math", "log10"),
    ("__exs_rt_math_atan2", "Math", "atan2"),
    ("__exs_rt_math_hypot", "Math", "hypot"),
];

/// Returns an identifier named `name` at `span`.
fn identifier<'a>(name: &str, span: SourceSpan<'a>) -> Identifier<'a> {
    Identifier {
        name: name.to_owned(),
        span,
    }
}

impl<'a> StepEmitter<'a, '_> {
    /// Binds the values of the durable `slots` to temporary names and compiles the expression
    /// `build` makes of them.
    pub(super) fn synthesize(
        &mut self,
        slots: &[u32],
        span: SourceSpan<'a>,
        build: impl FnOnce(Vec<Expression<'a>>) -> Expression<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let operands = self.bind(slots, span);
        let expression = build(operands);
        let compiled = self.compiler.compile_expression(&expression);
        self.compiler.scopes.pop();
        compiled
    }

    /// Binds the values of the durable `slots` to temporary names in a new scope and returns
    /// the variables naming them.
    fn bind(&mut self, slots: &[u32], span: SourceSpan<'a>) -> Vec<Expression<'a>> {
        let mut scope = HashMap::new();
        let mut operands = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            let local = self.load(*slot);
            let name = format!("${index}");
            operands.push(Expression::Variable(identifier(&name, span)));
            scope.insert(name, local);
        }
        self.compiler.scopes.push(scope);
        operands
    }

    /// Emits `operation` when it computes a value without a control transition, storing the
    /// value in its destination. Returns whether it did.
    pub(super) fn value_operation(
        &mut self,
        operation: &Operation<'a, '_>,
    ) -> Result<bool, CompileDiagnostics<'a>> {
        match operation {
            Operation::Literal {
                expression,
                destination,
            } => {
                self.compiler.compile_expression(expression)?;
                self.set_slot(*destination);
            }
            Operation::String {
                value,
                destination,
                span,
            } => {
                self.compiler.push_string(value, *span)?;
                self.set_slot(*destination);
            }
            Operation::Integer {
                value,
                destination,
                span,
            } => {
                self.compiler.compile_integer(i128::from(*value), *span)?;
                self.set_slot(*destination);
            }
            Operation::None { destination, .. } => {
                immediate::push(&mut self.compiler.function, ValueRef::NONE);
                self.set_slot(*destination);
            }
            Operation::Boolean {
                value, destination, ..
            } => {
                immediate::push(&mut self.compiler.function, ValueRef::boolean(*value));
                self.set_slot(*destination);
            }
            Operation::Copy {
                source,
                destination,
                ..
            } => {
                self.get_slot(*source);
                self.set_slot(*destination);
            }
            Operation::WireDecode {
                source,
                destination,
                contract,
                span,
                ..
            } => {
                let local = self.load_rooted(*source);
                self.compiler.check_binding(local, contract, *span)?;
                self.instruction(Instruction::LocalGet(local));
                self.set_slot(*destination);
            }
            Operation::CellNew {
                value,
                destination,
                span,
            } => {
                self.compiler.push_vm();
                self.get_slot(*value);
                self.compiler.core_call("__exs_core_cell_new", *span)?;
                self.set_slot(*destination);
            }
            Operation::CellGet {
                cell, destination, ..
            } => {
                self.get_slot(*cell);
                self.instruction(Instruction::I32WrapI64);
                self.instruction(Instruction::I64Load(value_field(CELL_VALUE)));
                self.set_slot(*destination);
            }
            Operation::CellSet { cell, value, .. } => {
                self.get_slot(*cell);
                self.instruction(Instruction::I32WrapI64);
                self.get_slot(*value);
                self.instruction(Instruction::I64Store(value_field(CELL_VALUE)));
            }
            Operation::Closure {
                layout,
                captures,
                destination,
                span,
                ..
            } => self.closure(layout.function_id, captures, *destination, *span)?,
            Operation::Unary {
                operator,
                operand,
                destination,
                span,
            } => {
                self.synthesize(&[*operand], *span, |mut operands| Expression::Unary {
                    operator: *operator,
                    operand: Box::new(operands.remove(0)),
                    span: *span,
                })?;
                self.set_slot(*destination);
            }
            Operation::Binary {
                operator,
                left,
                right,
                destination,
                span,
            } => {
                self.synthesize(&[*left, *right], *span, |mut operands| {
                    let right = operands.remove(1);
                    Expression::Binary {
                        operator: *operator,
                        left: Box::new(operands.remove(0)),
                        right: Box::new(right),
                        span: *span,
                    }
                })?;
                self.set_slot(*destination);
            }
            Operation::List {
                elements,
                destination,
                span,
            } => {
                self.synthesize(elements, *span, |elements| Expression::List {
                    elements,
                    span: *span,
                })?;
                self.set_slot(*destination);
            }
            Operation::Object {
                properties,
                destination,
                span,
            } => {
                let slots: Vec<u32> = properties.iter().map(|(_, _, slot)| *slot).collect();
                self.synthesize(&slots, *span, |values| Expression::Object {
                    properties: properties
                        .iter()
                        .zip(values)
                        .map(|((key, key_span, _), value)| ObjectProperty {
                            key: (*key).to_owned(),
                            key_span: *key_span,
                            value,
                            span: *key_span,
                        })
                        .collect(),
                    span: *span,
                })?;
                self.set_slot(*destination);
            }
            Operation::Error {
                kind,
                message,
                data,
                destination,
                span,
            } => {
                self.synthesize(&[*kind, *message, *data], *span, |arguments| {
                    Expression::Call {
                        callee: identifier("Error", *span),
                        arguments,
                        span: *span,
                    }
                })?;
                self.set_slot(*destination);
            }
            Operation::TypedObject {
                type_id,
                keys,
                destination,
                span,
            } => self.typed_object(*type_id, keys, *destination, *span)?,
            Operation::Enum {
                type_id,
                variant,
                fields,
                destination,
                span,
                ..
            } => self.enumeration(*type_id, variant, fields, *destination, *span)?,
            Operation::EnumMatches {
                value,
                type_identity,
                variant,
                destination,
                span,
            } => self.enum_matches(*value, type_identity, variant, *destination, *span)?,
            Operation::EnumField {
                value,
                index,
                destination,
                span,
            } => self.enum_field(*value, *index, *destination, *span)?,
            Operation::MatchError {
                value,
                destination,
                span,
            } => {
                self.compiler.set_position(*span)?;
                self.compiler.push_vm();
                self.get_slot(*value);
                self.compiler.core_call("__exs_core_match_error", *span)?;
                self.set_slot(*destination);
                self.return_if_error(*destination, *span)?;
            }
            Operation::IsType {
                value,
                contract,
                destination,
                span,
            } => {
                let local = self.load(*value);
                self.compiler.contract_test(local, contract, *span)?;
                immediate::bool_from_condition(&mut self.compiler.function);
                self.set_slot(*destination);
            }
            Operation::Propagate {
                value,
                destination,
                span,
            } => {
                self.synthesize(&[*value], *span, |mut operands| Expression::Propagate {
                    value: Box::new(operands.remove(0)),
                    span: *span,
                })?;
                self.set_slot(*destination);
            }
            Operation::Index {
                receiver,
                index,
                destination,
                span,
            } => {
                self.synthesize(&[*receiver, *index], *span, |mut operands| {
                    let index = operands.remove(1);
                    Expression::Index {
                        receiver: Box::new(operands.remove(0)),
                        index: Box::new(index),
                        span: *span,
                    }
                })?;
                self.set_slot(*destination);
            }
            Operation::Property {
                receiver,
                property,
                property_span,
                destination,
                span,
            } => {
                self.synthesize(&[*receiver], *span, |mut operands| Expression::Property {
                    receiver: Box::new(operands.remove(0)),
                    property: identifier(property, *property_span),
                    span: *span,
                })?;
                self.set_slot(*destination);
            }
            Operation::IndexSet {
                receiver,
                index,
                value,
                span,
            } => {
                let mut operands = self.bind(&[*receiver, *index, *value], *span);
                let value = operands.remove(2);
                let index = operands.remove(1);
                let statement = Statement::Assign {
                    target: AssignmentTarget::Index {
                        receiver: Box::new(operands.remove(0)),
                        index: Box::new(index),
                        span: *span,
                    },
                    value,
                    span: *span,
                };
                let compiled = self.compiler.compile_statement(&statement);
                self.compiler.scopes.pop();
                compiled?;
            }
            Operation::PropertySet {
                receiver,
                property,
                property_span,
                value,
                span,
            } => {
                let mut operands = self.bind(&[*receiver, *value], *span);
                let value = operands.remove(1);
                let statement = Statement::Assign {
                    target: AssignmentTarget::Property {
                        receiver: Box::new(operands.remove(0)),
                        property: identifier(property, *property_span),
                        span: *span,
                    },
                    value,
                    span: *span,
                };
                let compiled = self.compiler.compile_statement(&statement);
                self.compiler.scopes.pop();
                compiled?;
            }
            Operation::RuntimeStatic {
                values,
                export,
                destination,
                span,
            } => {
                self.runtime_static(values, export, *span)?;
                self.set_slot(*destination);
            }
            Operation::FormatNew {
                destination, span, ..
            } => {
                self.compiler.push_vm();
                self.instruction(Instruction::I32Const(0));
                self.instruction(Instruction::I32Const(0));
                self.compiler.core_call("__exs_core_list_new", *span)?;
                self.set_slot(*destination);
            }
            Operation::FormatPush {
                builder,
                part,
                span,
            } => {
                match part {
                    FormatPart::Text(text) => self.compiler.push_string(text, *span)?,
                    FormatPart::Slot(slot) => self.get_slot(*slot),
                }
                let part = self.compiler.store_new();
                self.append(*builder, part, *span)?;
            }
            Operation::FormatFinish {
                builder,
                destination,
                span,
            } => {
                let list = self.load(*builder);
                self.compiler.push_vm();
                self.compiler.push_address(list);
                self.instruction(Instruction::I32Load(u32_field(LIST_ITEMS)));
                self.instruction(Instruction::I32Const(BUFFER_SLOTS as i32));
                self.instruction(Instruction::I32Add);
                self.compiler.push_address(list);
                self.instruction(Instruction::I32Load(u32_field(LIST_LEN)));
                self.compiler.core_call("__exs_core_format", *span)?;
                self.set_slot(*destination);
            }
            Operation::OrderingTest {
                value,
                test,
                destination,
                span,
            } => {
                // Like synchronous code, an Error from `compare` returns and the test's own
                // TypeError is a value.
                self.return_if_error(*value, *span)?;
                self.compiler.set_position(*span)?;
                self.compiler.push_vm();
                self.get_slot(*value);
                self.instruction(Instruction::I32Const(*test));
                self.compiler.core_call("__exs_core_ordering_test", *span)?;
                self.set_slot(*destination);
            }
            Operation::ValidateParameters {
                contracts,
                spans,
                offset,
                ..
            } => {
                for ((slot, contract), span) in (*offset..).zip(contracts).zip(spans) {
                    let local = self.load(slot);
                    self.compiler.check_contract(local, contract, *span)?;
                }
            }
            Operation::ValidateVariadicParameter {
                slot,
                contract,
                span,
            } => {
                let local = self.load(*slot);
                self.compiler.check_variadic(local, contract, *span)?;
            }
            Operation::ValidateSlot {
                slot,
                contract,
                span,
            } => {
                let local = self.load(*slot);
                self.compiler.check_contract(local, contract, *span)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Appends the value in the temporary `part` to the formatted-string builder List in
    /// durable slot `builder`.
    fn append(
        &mut self,
        builder: u32,
        part: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.compiler.push_vm();
        self.instruction(Instruction::I32Const(
            BuiltinMethod::Push.id().cast_signed(),
        ));
        self.get_slot(builder);
        self.instruction(Instruction::LocalGet(part));
        immediate::push(&mut self.compiler.function, ValueRef::NONE);
        self.compiler.core_call("__exs_core_call_builtin", span)?;
        self.instruction(Instruction::Drop);
        Ok(())
    }

    /// Appends the value in durable slot `value` to the builder in durable slot `builder`.
    pub(super) fn append_slot(
        &mut self,
        builder: u32,
        value: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let part = self.load(value);
        self.append(builder, part, span)
    }

    /// Creates the closure lifted as the unit with the trace id `function_id`, capturing the
    /// Cells in the durable `captures`.
    fn closure(
        &mut self,
        function_id: u32,
        captures: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let key = self.key_of(function_id, span)?;
        let lifted = self
            .resumable
            .lifted
            .iter()
            .find(|closure| closure.key == key)
            .ok_or_else(|| error("E0999", span, format!("missing lifted closure `{key}`")))?;
        let index = self
            .compiler
            .context
            .closures
            .get(&closure_span_key(lifted.declaration.span))
            .map(|closure| closure.index)
            .ok_or_else(|| error("E0999", span, format!("unreached closure `{key}`")))?;
        let count = captures.len() as u32;
        let block = self.compiler.consecutive_locals(count.max(1));
        for (offset, capture) in (0..count).zip(captures) {
            self.get_slot(*capture);
            self.compiler.store(block + offset);
        }
        self.compiler.push_vm();
        self.instruction(Instruction::I32Const(index.cast_signed()));
        self.compiler.push_slot_address(block);
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.compiler.core_call("__exs_core_closure_new", span)?;
        self.set_slot(destination);
        Ok(())
    }

    /// Creates the typed Object of `type_id` a literal naming `keys` builds, with every field
    /// None until the following property writes.
    fn typed_object(
        &mut self,
        type_id: u32,
        keys: &[String],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.compiler.context;
        let nominal = context
            .types
            .nominal_types()
            .find(|nominal| nominal.id == type_id)
            .ok_or_else(|| error("E0999", span, format!("missing nominal type {type_id}")))?;
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let object = self.compiler.allocate_typed_object(nominal, &keys, span)?;
        for index in 0..nominal.fields.len() as u64 {
            self.compiler.push_address(object);
            immediate::push(&mut self.compiler.function, ValueRef::NONE);
            self.instruction(Instruction::I64Store(value_field(TYPED_SLOTS + index * 8)));
        }
        self.instruction(Instruction::LocalGet(object));
        self.set_slot(destination);
        Ok(())
    }

    /// Creates the enum of `type_id` and the variant named `variant` holding the durable
    /// `fields`.
    fn enumeration(
        &mut self,
        type_id: u32,
        variant: &str,
        fields: &[u32],
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let index = self.variant_index(type_id, variant, span)?;
        let count = fields.len() as u32;
        let block = self.compiler.consecutive_locals(count.max(1));
        for (offset, field) in (0..count).zip(fields) {
            self.get_slot(*field);
            self.compiler.store(block + offset);
        }
        self.compiler.push_vm();
        self.instruction(Instruction::I32Const(type_id.cast_signed()));
        self.instruction(Instruction::I32Const(index.cast_signed()));
        if count == 0 {
            self.instruction(Instruction::I32Const(0));
        } else {
            self.compiler.push_slot_address(block);
        }
        self.instruction(Instruction::I32Const(count.cast_signed()));
        self.compiler.core_call("__exs_core_enum_new", span)?;
        self.set_slot(destination);
        Ok(())
    }

    /// Returns the declaration index of the variant named `variant` of the enum `type_id`.
    fn variant_index(
        &self,
        type_id: u32,
        variant: &str,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.compiler
            .context
            .types
            .enum_variants()
            .find(|candidate| candidate.type_id == type_id && candidate.name == variant)
            .map(|candidate| candidate.index)
            .ok_or_else(|| error("E0999", span, format!("missing enum variant `{variant}`")))
    }

    /// Stores whether the value in durable slot `value` is the variant `variant` of the enum
    /// with the identity `identity`.
    fn enum_matches(
        &mut self,
        value: u32,
        identity: &str,
        variant: &str,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let type_id = self.compiler.enum_type_id(identity, span)?;
        let index = self.variant_index(type_id, variant, span)?;
        let local = self.load(value);
        self.instruction(Instruction::LocalGet(local));
        self.compiler.type_tag_test(type_id, span)?;
        self.instruction(Instruction::If(BlockType::Result(
            wasm_encoder::ValType::I32,
        )));
        self.compiler.push_address(local);
        self.instruction(Instruction::I32Load(u32_field(VARIANT)));
        self.instruction(Instruction::I32Const(index.cast_signed()));
        self.instruction(Instruction::I32Eq);
        self.instruction(Instruction::Else);
        self.instruction(Instruction::I32Const(0));
        self.instruction(Instruction::End);
        immediate::bool_from_condition(&mut self.compiler.function);
        self.set_slot(destination);
        Ok(())
    }

    /// Stores payload field `index` of the matched enum in durable slot `value`, or v1's
    /// MatchError when it lacks the field.
    fn enum_field(
        &mut self,
        value: u32,
        index: u32,
        destination: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let local = self.load(value);
        self.compiler.push_address(local);
        self.instruction(Instruction::I32Load(u32_field(FIELD_COUNT)));
        self.instruction(Instruction::I32Const(index.cast_signed()));
        self.instruction(Instruction::I32GtU);
        self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
        self.compiler.push_address(local);
        self.instruction(Instruction::I64Load(value_field(
            FIELDS + u64::from(index) * 8,
        )));
        self.instruction(Instruction::Else);
        self.compiler.set_position(span)?;
        self.compiler.push_vm();
        self.instruction(Instruction::LocalGet(local));
        self.compiler
            .core_call("__exs_core_enum_missing_field", span)?;
        self.instruction(Instruction::End);
        self.set_slot(destination);
        Ok(())
    }

    /// Calls the static function v1's graph names by its runtime `export` with the durable
    /// `values`, leaving the result on the stack.
    fn runtime_static(
        &mut self,
        values: &[u32],
        export: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if export == "__exs_rt_datetime_parse_rfc3339" {
            return self.synthesize(values, span, |arguments| Expression::Call {
                callee: identifier("__exs_datetime_parse_rfc3339_runtime", span),
                arguments,
                span,
            });
        }
        let (_, type_name, method) = STATIC_EXPORTS
            .iter()
            .find(|(name, _, _)| *name == export)
            .copied()
            .ok_or_else(|| error("E0999", span, format!("unknown static export `{export}`")))?;
        self.synthesize(values, span, |arguments| Expression::StaticMethodCall {
            type_name: identifier(type_name, span),
            method: identifier(method, span),
            arguments,
            span,
        })
    }
}
