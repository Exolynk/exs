//! The implementation methods of a module and their dispatch.
//!
//! Method calls, operators and formatted-string parts dispatch statically: a chain of
//! type-tag tests over every type implementing the method, in declaration order, before the
//! built-in behavior.

use std::collections::HashMap;

use wasm_encoder::{BlockType, Instruction};

use super::closures::accepts;
use super::function::{FunctionCompiler, error};
use crate::ast::{Module, Visibility};
use crate::codegen::VALUE_TYPE;
use crate::codegen::standard::{self, TO_STRING_METHOD, TO_STRING_TRAIT};
use crate::codegen::trait_registry::{TraitOperator, TraitRegistry};
use crate::codegen::types::TypeRegistry;
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

/// The prelude trait `for` loops advance Objects through.
const ITERATOR_TRAIT: &str = "Iterator";
/// The method of [`ITERATOR_TRAIT`] that yields the next `IteratorStep`.
const ITERATOR_METHOD: &str = "next";

/// One implementation method a dispatch chain may call.
#[derive(Clone, Debug)]
pub(super) struct Target {
    /// Nominal type id of the receivers it accepts.
    pub(super) type_id: u32,
    /// Function key of the method, `Type::method`.
    pub(super) key: String,
    /// The package a private method belongs to, `None` for a public one.
    pub(super) private_to: Option<String>,
}

impl Target {
    /// Returns whether code of `package` may call the method.
    fn visible_from(&self, package: &str) -> bool {
        self.private_to
            .as_deref()
            .is_none_or(|owner| owner == package)
    }
}

/// Every implementation method of a module, by the ways calls dispatch to them.
#[derive(Default)]
pub(super) struct Dispatch {
    /// Instance methods by method name.
    instance: HashMap<String, Vec<Target>>,
    /// `ToString::to_string` implementations, which formatted strings call.
    to_string: Vec<Target>,
    /// `Iterator::next` implementations, which `for` loops over Objects call.
    iterator_next: Vec<Target>,
    /// Implementations selected by each source operator.
    operators: HashMap<TraitOperator, Vec<Target>>,
    /// Keys of the static methods, with the package of the private ones.
    statics: HashMap<String, Option<String>>,
}

impl Dispatch {
    /// Collects the implementation methods of `module` in declaration order.
    pub(super) fn build(
        module: &Module<'_>,
        traits: &TraitRegistry<'_>,
        types: &TypeRegistry,
    ) -> Self {
        let mut dispatch = Self::default();
        for implementation in &module.implementations {
            let Some(nominal) = types.get(&implementation.type_name.name) else {
                continue;
            };
            for method in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, method.name.name);
                let private_to =
                    (method.visibility == Visibility::Private).then(|| method.package.clone());
                let is_instance = method
                    .parameters
                    .first()
                    .is_some_and(|parameter| parameter.name.name == "self");
                if !is_instance {
                    dispatch.statics.insert(key, private_to);
                    continue;
                }
                let target = Target {
                    type_id: nominal.id,
                    key,
                    private_to,
                };
                dispatch
                    .instance
                    .entry(method.name.name.clone())
                    .or_default()
                    .push(target.clone());
                let Some(trait_name) = &implementation.trait_name else {
                    continue;
                };
                let trait_name =
                    standard::canonical_trait_name(&trait_name.name).unwrap_or(&trait_name.name);
                for operator in traits.operators_for(trait_name, &method.name.name) {
                    dispatch
                        .operators
                        .entry(*operator)
                        .or_default()
                        .push(target.clone());
                }
                if trait_name == TO_STRING_TRAIT && method.name.name == TO_STRING_METHOD {
                    dispatch.to_string.push(target.clone());
                }
                if trait_name == ITERATOR_TRAIT && method.name.name == ITERATOR_METHOD {
                    dispatch.iterator_next.push(target);
                }
            }
        }
        dispatch
    }

    /// Returns every instance method named `name`, private ones included.
    pub(super) fn instance_all(&self, name: &str) -> &[Target] {
        self.instance.get(name).map_or(&[], Vec::as_slice)
    }

    /// Returns the instance methods named `name` that code of `package` may call: a call
    /// outside a private method's package does not dispatch to it, so its receivers fall
    /// back to the built-in behavior, MethodNotFound for a nominal value.
    pub(super) fn instance(&self, name: &str, package: &str) -> Vec<Target> {
        self.instance_all(name)
            .iter()
            .filter(|target| target.visible_from(package))
            .cloned()
            .collect()
    }

    /// Returns the package the static method `key` is private to, if it is private.
    pub(super) fn static_private_to(&self, key: &str) -> Option<&str> {
        self.statics.get(key).and_then(Option::as_deref)
    }

    /// Returns the implementations `operator` dispatches to.
    pub(super) fn operator(&self, operator: TraitOperator) -> &[Target] {
        self.operators.get(&operator).map_or(&[], Vec::as_slice)
    }

    /// Returns the `ToString` implementations.
    pub(super) fn to_string(&self) -> &[Target] {
        &self.to_string
    }

    /// Returns the `Iterator::next` implementations.
    pub(super) fn iterator_next(&self) -> &[Target] {
        &self.iterator_next
    }

    /// Returns whether `key` names a static method.
    pub(super) fn is_static(&self, key: &str) -> bool {
        self.statics.contains_key(key)
    }
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Emits a chain of tag tests over `targets` that calls the first implementation whose
    /// type the receiver in `receiver` has with `arguments`, leaving its result on the stack,
    /// and otherwise runs `fallback`.
    pub(super) fn dispatch_chain(
        &mut self,
        targets: &[Target],
        receiver: u32,
        arguments: &[u32],
        span: SourceSpan<'a>,
        fallback: impl FnOnce(&mut Self) -> Result<(), CompileDiagnostics<'a>>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        for target in targets {
            self.instruction(Instruction::LocalGet(receiver));
            self.type_tag_test(target.type_id, span)?;
            self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
            self.call_method(&target.key, receiver, arguments, span)?;
            self.instruction(Instruction::Else);
        }
        fallback(self)?;
        for _ in targets {
            self.instruction(Instruction::End);
        }
        Ok(())
    }

    /// Calls the implementation method `key` with `receiver` and `arguments`, or creates the
    /// ArityError when it takes a different number of arguments.
    pub(super) fn call_method(
        &mut self,
        key: &str,
        receiver: u32,
        arguments: &[u32],
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let (index, arity, variadic) = self
            .context
            .functions
            .get(key)
            .map(|info| (info.index, info.arity, info.variadic.is_some()))
            .ok_or_else(|| error("E0999", span, format!("missing method `{key}`")))?;
        if !accepts(arity, variadic, arguments.len() + 1) {
            self.set_position(span)?;
            self.push_vm();
            self.instruction(Instruction::LocalGet(receiver));
            return self.core_call("__exs_core_nominal_arity_error", span);
        }
        self.instruction(Instruction::LocalGet(receiver));
        let fixed = arity.saturating_sub(1) as usize;
        for argument in arguments.iter().take(fixed) {
            self.instruction(Instruction::LocalGet(*argument));
        }
        if variadic {
            let rest = &arguments[fixed.min(arguments.len())..];
            let count = rest.len() as u32;
            let block = self.consecutive_locals(count.max(1));
            for (offset, argument) in (0..count).zip(rest) {
                self.instruction(Instruction::LocalGet(*argument));
                self.store(block + offset);
            }
            self.push_arguments(block, count, 0, true, span)?;
        }
        self.direct_call(index, span)
    }

    /// Calls the compiled function `index` with the arguments on the stack, recording the
    /// call site and returning fatal Errors, and leaves the result on the stack.
    pub(super) fn direct_call(
        &mut self,
        index: u32,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        self.set_call_site(span)?;
        self.instruction(Instruction::Call(index));
        let result = self.store_new();
        self.return_if_fatal(result);
        self.instruction(Instruction::LocalGet(result));
        Ok(())
    }

    /// Emits `operator` on `left` and `right`: the implementation of the left operand's type
    /// when it has one, else `fallback`. Comparison results are tested.
    pub(super) fn operator_chain(
        &mut self,
        operator: TraitOperator,
        left: u32,
        right: u32,
        span: SourceSpan<'a>,
        fallback: impl FnOnce(&mut Self) -> Result<(), CompileDiagnostics<'a>>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        let context = self.context;
        let targets = context.dispatch.operator(operator);
        for target in targets {
            self.instruction(Instruction::LocalGet(left));
            self.type_tag_test(target.type_id, span)?;
            self.instruction(Instruction::If(BlockType::Result(VALUE_TYPE)));
            self.call_method(&target.key, left, &[right], span)?;
            if let Some(test) = operator.comparison_test() {
                let ordering = self.store_new();
                self.return_if_error(ordering, span)?;
                self.set_position(span)?;
                self.push_vm();
                self.instruction(Instruction::LocalGet(ordering));
                self.instruction(Instruction::I32Const(test));
                self.core_call("__exs_core_ordering_test", span)?;
            }
            self.instruction(Instruction::Else);
        }
        fallback(self)?;
        for _ in targets {
            self.instruction(Instruction::End);
        }
        Ok(())
    }
}
