//! Lowering of method calls on built-in values and of static built-in functions.
//!
//! A method call evaluates its receiver and arguments, then calls the core library's built-in
//! method by its `exs_layout::BuiltinMethod` index. A name no built-in provides is a
//! MethodNotFound Error and a built-in called with another number of arguments is an
//! ArityError; both are decided at compile time from the static method name.

use exs_layout::BuiltinMethod;
use exs_layout::StaticFunction;
use exs_layout::ValueRef;
use wasm_encoder::Instruction;

use super::closures::{accepts, arity_description};
use super::function::{FunctionCompiler, error, unsupported};
use crate::ast::{Expression, Identifier};
use crate::codegen::immediate;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// Returns whether the core library implements `method`.
const fn is_supported(method: BuiltinMethod) -> bool {
    matches!(
        method,
        BuiltinMethod::Add
            | BuiltinMethod::Sub
            | BuiltinMethod::Mul
            | BuiltinMethod::Div
            | BuiltinMethod::DivEuclid
            | BuiltinMethod::RemEuclid
            | BuiltinMethod::ToString
            | BuiltinMethod::Debug
            | BuiltinMethod::Abs
            | BuiltinMethod::Floor
            | BuiltinMethod::Ceil
            | BuiltinMethod::Round
            | BuiltinMethod::Trunc
            | BuiltinMethod::Fract
            | BuiltinMethod::Signum
            | BuiltinMethod::IsEven
            | BuiltinMethod::IsOdd
            | BuiltinMethod::Gcd
            | BuiltinMethod::Lcm
            | BuiltinMethod::Pow
            | BuiltinMethod::Min
            | BuiltinMethod::Max
            | BuiltinMethod::Clamp
            | BuiltinMethod::ToFloat
            | BuiltinMethod::ToInt
            | BuiltinMethod::IsNan
            | BuiltinMethod::IsFinite
            | BuiltinMethod::IsInfinite
            | BuiltinMethod::Sqrt
            | BuiltinMethod::Length
            | BuiltinMethod::IsEmpty
            | BuiltinMethod::Kind
            | BuiltinMethod::Message
            | BuiltinMethod::Data
            | BuiltinMethod::Cause
            | BuiltinMethod::Clone
            | BuiltinMethod::Compare
            | BuiltinMethod::Slice
            | BuiltinMethod::Contains
            | BuiltinMethod::StartsWith
            | BuiltinMethod::EndsWith
            | BuiltinMethod::Repeat
            | BuiltinMethod::Trim
            | BuiltinMethod::TrimStart
            | BuiltinMethod::TrimEnd
            | BuiltinMethod::Replace
            | BuiltinMethod::Split
            | BuiltinMethod::Push
            | BuiltinMethod::Get
            | BuiltinMethod::First
            | BuiltinMethod::Last
            | BuiltinMethod::Extend
            | BuiltinMethod::Reverse
            | BuiltinMethod::Reversed
            | BuiltinMethod::IndexOf
            | BuiltinMethod::LastIndexOf
            | BuiltinMethod::Join
            | BuiltinMethod::Pop
            | BuiltinMethod::Insert
            | BuiltinMethod::Remove
            | BuiltinMethod::Clear
            | BuiltinMethod::Has
            | BuiltinMethod::Delete
            | BuiltinMethod::Keys
            | BuiltinMethod::Values
            | BuiltinMethod::GetOr
            | BuiltinMethod::Entries
            | BuiltinMethod::Merge
            | BuiltinMethod::ToList
            | BuiltinMethod::Concat
            | BuiltinMethod::DecodeUtf8
            | BuiltinMethod::EncodeUtf8
            | BuiltinMethod::ToHex
            | BuiltinMethod::ToBase64
            | BuiltinMethod::ToLowercase
            | BuiltinMethod::ToUppercase
    )
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Compiles `receiver.method(arguments)`.
    pub(super) fn compile_method_call(
        &mut self,
        receiver: &Expression<'a>,
        method: &Identifier<'a>,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.context;
        let targets = context
            .dispatch
            .instance(&method.name, &self.declaration.package);
        let builtin = BuiltinMethod::from_name(&method.name);
        // Receivers no implementation accepts fall back to built-in behavior the backend may
        // not support yet; without implementations that is a compile error, with them a trap.
        let helper = crate::prelude::iterator_helper(&method.name);
        let unsupported_fallback = builtin
            .filter(|builtin| {
                helper.is_none() && builtin.arity() == arguments.len() && !is_supported(*builtin)
            })
            .map(|_| format!("calls of the built-in `{}` method", method.name));
        if let Some(what) = &unsupported_fallback
            && targets.is_empty()
        {
            return Err(unsupported(span, what));
        }
        self.compile_expression(receiver)?;
        let receiver = self.store_new();
        let count = u32::try_from(arguments.len())
            .map_err(|_| error("E0212", span, "too many method arguments".to_owned()))?;
        let block = self.consecutive_locals(count.max(1));
        for (offset, argument) in (0..count).zip(arguments) {
            self.compile_expression(argument)?;
            self.store(block + offset);
        }
        let arguments: Vec<u32> = (0..count).map(|offset| block + offset).collect();
        self.dispatch_chain(&targets, receiver, &arguments, span, |compiler| {
            // The prelude's eager iterator helpers come before built-in methods.
            if let Some(helper) = helper {
                return compiler.call_method(helper, receiver, &arguments, span);
            }
            if unsupported_fallback.is_some() {
                compiler.instruction(Instruction::Unreachable);
                return Ok(());
            }
            compiler.builtin_method_call(builtin, receiver, block, count, span)
        })
    }

    /// Calls the built-in method `builtin` of the receiver in `receiver` with the `count`
    /// arguments in the consecutive locals at `block`, or creates the Error for a method
    /// the receiver lacks or an argument count it does not take.
    pub(super) fn builtin_method_call(
        &mut self,
        builtin: Option<BuiltinMethod>,
        receiver: u32,
        block: u32,
        count: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.set_position(span)?;
        self.push_vm();
        match builtin {
            Some(builtin) if builtin.arity() == count as usize => {
                self.instruction(Instruction::I32Const(builtin.id().cast_signed()));
                self.instruction(Instruction::LocalGet(receiver));
                for offset in 0..2 {
                    if offset < count {
                        self.instruction(Instruction::LocalGet(block + offset));
                    } else {
                        immediate::push(&mut self.function, ValueRef::NONE);
                    }
                }
                self.core_call("__exs_core_call_builtin", span)
            }
            Some(builtin) => {
                self.push_slot_address(block);
                self.instruction(Instruction::I32Const(count.cast_signed()));
                self.instruction(Instruction::I32Const(
                    u32::try_from(builtin.arity()).unwrap_or(2).cast_signed(),
                ));
                self.core_call("__exs_core_method_arity_error", span)
            }
            None => {
                self.instruction(Instruction::LocalGet(receiver));
                self.core_call("__exs_core_method_not_found", span)
            }
        }
    }

    /// Compiles `type_name::method(arguments)` for the static functions of built-in types.
    pub(super) fn compile_static_call(
        &mut self,
        type_name: &Identifier<'a>,
        method: &Identifier<'a>,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let Some(function) = StaticFunction::from_path(&type_name.name, &method.name) else {
            let key = format!("{}::{}", type_name.name, method.name);
            if let Some(variant) = self.context.types.enum_variant(&key) {
                return self.compile_enum_variant(variant, arguments, span);
            }
            return self.compile_user_static_call(type_name, method, &key, arguments, span);
        };
        if function.arity() != arguments.len() {
            return Err(error(
                "E0208",
                span,
                format!(
                    "static method `{}::{}` expects {} arguments but received {}",
                    type_name.name,
                    method.name,
                    function.arity(),
                    arguments.len()
                ),
            ));
        }
        let mut locals = Vec::with_capacity(2);
        for argument in arguments {
            self.compile_expression(argument)?;
            locals.push(self.store_new());
        }
        self.set_position(span)?;
        self.push_vm();
        self.instruction(Instruction::I32Const(function.id().cast_signed()));
        for index in 0..2 {
            match locals.get(index) {
                Some(local) => self.instruction(Instruction::LocalGet(*local)),
                None => immediate::push(&mut self.function, ValueRef::NONE),
            }
        }
        self.core_call("__exs_core_call_static", span)
    }

    /// Rejects a call of `key` when it is a static method private to another package.
    pub(super) fn check_static_access(
        &self,
        key: &str,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        match self.context.dispatch.static_private_to(key) {
            Some(owner) if owner != self.declaration.package => Err(error(
                "E0118",
                span,
                format!(
                    "static method `{}` is private to its library",
                    key.rsplit("::").next().unwrap_or(key)
                ),
            )),
            _ => Ok(()),
        }
    }

    /// Compiles a call of the static implementation method `key`, with the diagnostics for
    /// unknown types, unknown methods, instance methods and argument counts.
    fn compile_user_static_call(
        &mut self,
        type_name: &Identifier<'a>,
        method: &Identifier<'a>,
        key: &str,
        arguments: &[Expression<'a>],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if matches!(
            type_name.name.as_str(),
            "Bytes" | "Int" | "Float" | "Object" | "Math"
        ) {
            return Err(unsupported(span, "static method calls"));
        }
        if self.context.types.get(&type_name.name).is_none() {
            return Err(error(
                "E0216",
                type_name.span,
                crate::diagnostic::unknown_name("type", &type_name.name),
            ));
        }
        let Some((index, arity, variadic)) = self
            .context
            .functions
            .get(key)
            .map(|info| (info.index, info.arity, info.variadic.is_some()))
        else {
            return Err(error(
                "E0225",
                method.span,
                format!(
                    "type `{}` has no static method `{}`",
                    type_name.name, method.name
                ),
            ));
        };
        self.check_static_access(key, method.span)?;
        if !self.context.dispatch.is_static(key) {
            return Err(error(
                "E0225",
                method.span,
                format!(
                    "type `{}` method `{}` requires a receiver",
                    type_name.name, method.name
                ),
            ));
        }
        if !accepts(arity, variadic, arguments.len()) {
            return Err(error(
                "E0208",
                span,
                format!(
                    "static method `{key}` expects {} arguments but received {}",
                    arity_description(arity, variadic),
                    arguments.len()
                ),
            ));
        }
        let count = arguments.len() as u32;
        let block = self.consecutive_locals(count.max(1));
        for (offset, argument) in (0..count).zip(arguments) {
            self.compile_expression(argument)?;
            self.store(block + offset);
        }
        self.push_arguments(block, count, arity, variadic, span)?;
        self.direct_call(index, span)
    }
}
