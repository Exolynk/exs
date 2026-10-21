//! Guards for standard-trait implementations the v2 backend does not dispatch yet.
//!
//! v1 dispatches operators and formatted-string parts to nominal `Add`, `Compare`, `ToString`
//! and related implementations before its built-in behavior. Until v2 compiles implementation
//! methods, it traps on exactly those receivers instead of silently applying the built-in
//! behavior.

use std::collections::HashMap;

use exs_value::layout::object_tag;
use wasm_encoder::{BlockType, Instruction};

use super::function::FunctionCompiler;
use crate::ast::{BinaryOperator, Module};
use crate::codegen::standard::{self, TO_STRING_METHOD, TO_STRING_TRAIT};
use crate::codegen::trait_registry::{TraitOperator, TraitRegistry};
use crate::codegen::types::TypeRegistry;

/// Nominal types whose standard-trait implementations v1 dispatches to.
#[derive(Default)]
pub(super) struct Protocols {
    /// Types implementing each source operator.
    operators: HashMap<TraitOperator, Vec<u32>>,
    /// Types implementing `ToString`, which formatted strings call.
    to_string: Vec<u32>,
}

impl Protocols {
    /// Collects the implementing types of every operator trait and of `ToString`, like v1's
    /// method registry.
    pub(super) fn build(
        module: &Module<'_>,
        traits: &TraitRegistry<'_>,
        types: &TypeRegistry,
    ) -> Self {
        let mut protocols = Self::default();
        for implementation in &module.implementations {
            let (Some(trait_name), Some(nominal)) = (
                &implementation.trait_name,
                types.get(&implementation.type_name.name),
            ) else {
                continue;
            };
            let trait_name =
                standard::canonical_trait_name(&trait_name.name).unwrap_or(&trait_name.name);
            for method in &implementation.methods {
                let is_instance = method
                    .parameters
                    .first()
                    .is_some_and(|parameter| parameter.name.name == "self");
                if !is_instance {
                    continue;
                }
                for operator in traits.operators_for(trait_name, &method.name.name) {
                    protocols
                        .operators
                        .entry(*operator)
                        .or_default()
                        .push(nominal.id);
                }
                if trait_name == TO_STRING_TRAIT && method.name.name == TO_STRING_METHOD {
                    protocols.to_string.push(nominal.id);
                }
            }
        }
        protocols
    }
}

impl FunctionCompiler<'_, '_> {
    /// Traps when the value in `local` has one of `type_ids`.
    fn guard_types(&mut self, local: u32, type_ids: &[u32]) {
        let tags: Vec<u32> = type_ids.iter().filter_map(|id| object_tag(*id)).collect();
        for tag in tags {
            self.tag_test(local, tag);
            self.instruction(Instruction::If(BlockType::Empty));
            self.instruction(Instruction::Unreachable);
            self.instruction(Instruction::End);
        }
    }

    /// Traps when the left operand in `left` has an implementation v1 dispatches `operator`
    /// to.
    pub(super) fn guard_operator(&mut self, operator: BinaryOperator, left: u32) {
        let Some(operator) = TraitOperator::from_binary(operator) else {
            return;
        };
        let context = self.context;
        if let Some(type_ids) = context.protocols.operators.get(&operator) {
            self.guard_types(left, type_ids);
        }
    }

    /// Traps when the formatted-string part in `local` has a `ToString` implementation.
    pub(super) fn guard_to_string(&mut self, local: u32) {
        let context = self.context;
        self.guard_types(local, &context.protocols.to_string);
    }
}
