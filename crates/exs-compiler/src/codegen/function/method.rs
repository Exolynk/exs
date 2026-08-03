//! Nominal implementation method lookup for generated dispatch.

use std::collections::{HashMap, HashSet};

use crate::ast::Module;
use crate::codegen::trait_registry::{TraitOperator, TraitRegistry};
use crate::codegen::types::TypeRegistry;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics};

use super::FunctionSignature;

/// One compiler-resolved instance method target.
#[derive(Debug, Clone)]
pub(in crate::codegen) struct InstanceMethod {
    /// Runtime nominal Object tag required for this implementation.
    pub(in crate::codegen) type_id: u32,
    /// Linked direct Wasm function signature.
    pub(in crate::codegen) signature: FunctionSignature,
}

/// All implementation methods declared by one compiled module.
#[derive(Debug, Clone)]
pub(in crate::codegen) struct MethodRegistry {
    instance_methods: HashMap<String, Vec<InstanceMethod>>,
    operator_methods: HashMap<TraitOperator, Vec<InstanceMethod>>,
    static_methods: HashSet<String>,
}

impl MethodRegistry {
    /// Builds implementation method lookup tables after function linking assigns indexes.
    pub(in crate::codegen) fn build<'a>(
        module: &Module<'a>,
        traits: &TraitRegistry<'a>,
        types: &TypeRegistry,
        signatures: &HashMap<String, FunctionSignature>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let mut instance_methods: HashMap<String, Vec<InstanceMethod>> = HashMap::new();
        let mut operator_methods: HashMap<TraitOperator, Vec<InstanceMethod>> = HashMap::new();
        let mut static_methods = HashSet::new();
        for implementation in &module.implementations {
            let nominal = types.get(&implementation.type_name.name).ok_or_else(|| {
                CompileDiagnostics::from(CompileDiagnostic::new(
                    "E0999",
                    implementation.type_name.span,
                    "missing resolved implementation type",
                ))
            })?;
            for method in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, method.name.name);
                let signature = signatures.get(&key).cloned().ok_or_else(|| {
                    CompileDiagnostics::from(CompileDiagnostic::new(
                        "E0999",
                        method.name.span,
                        "missing implementation method signature",
                    ))
                })?;
                if method
                    .parameters
                    .first()
                    .is_some_and(|parameter| parameter.name.name == "self")
                {
                    let target = InstanceMethod {
                        type_id: nominal.id,
                        signature,
                    };
                    instance_methods
                        .entry(method.name.name.clone())
                        .or_default()
                        .push(target.clone());
                    if let Some(trait_name) = &implementation.trait_name {
                        for operator in traits.operators_for(&trait_name.name, &method.name.name) {
                            operator_methods
                                .entry(*operator)
                                .or_default()
                                .push(target.clone());
                        }
                    }
                } else {
                    static_methods.insert(key);
                }
            }
        }
        Ok(Self {
            instance_methods,
            operator_methods,
            static_methods,
        })
    }

    /// Returns every nominal implementation matching one instance method name.
    pub(in crate::codegen) fn instance(&self, name: &str) -> Option<&[InstanceMethod]> {
        self.instance_methods.get(name).map(Vec::as_slice)
    }

    /// Returns nominal implementations selected exclusively by one source operator.
    pub(in crate::codegen) fn operator(&self, operator: TraitOperator) -> &[InstanceMethod] {
        self.operator_methods
            .get(&operator)
            .map_or(&[], Vec::as_slice)
    }

    /// Returns whether a qualified implementation method is static.
    pub(in crate::codegen) fn is_static(&self, key: &str) -> bool {
        self.static_methods.contains(key)
    }
}
