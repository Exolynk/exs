//! The linked context of resumable units: v1's signatures, method registry and frame
//! layouts, which its continuation graph builder needs, and the step function of every
//! resumable unit.

use std::collections::{HashMap, HashSet};

use crate::ast::Module;
use crate::codegen::continuation::{FrameLayout, frame_slot_count};
use crate::codegen::function::{
    FunctionSignature, LiftedFunction, MethodRegistry, build_signatures,
};
use crate::codegen::trait_registry::TraitRegistry;
use crate::codegen::types::TypeRegistry;
use crate::codegen::v2::units::Unit;
use crate::diagnostic::CompileDiagnostics;

/// One resumable unit as frames and the dispatcher see it.
#[derive(Clone, Copy)]
pub(in crate::codegen::v2) struct StepInfo {
    /// Dense index of its step function, the dispatcher's case and a frame's header word.
    pub(in crate::codegen::v2) step: u32,
    /// Identifier recorded in Error traces.
    pub(in crate::codegen::v2) function_id: u32,
    /// Number of durable slots of its frames.
    pub(in crate::codegen::v2) slot_count: u32,
    /// Wasm index of its step function.
    pub(in crate::codegen::v2) index: u32,
}

/// A closure that may suspend, as a resumable closure call starts its frames.
#[derive(Clone, Copy)]
pub(in crate::codegen::v2) struct ClosureStep {
    /// The step of its lifted function.
    pub(in crate::codegen::v2) step: StepInfo,
    /// Number of captured Cells, its first frame slots.
    pub(in crate::codegen::v2) captures: u32,
    /// Number of fixed parameters.
    pub(in crate::codegen::v2) arity: u32,
    /// Whether a trailing parameter collects the remaining arguments.
    pub(in crate::codegen::v2) variadic: bool,
}

/// The resumable units of a module and the context their graphs are built in.
pub(in crate::codegen::v2) struct Resumable<'l, 'a> {
    /// Every resumable unit by key.
    pub(in crate::codegen::v2) steps: HashMap<String, StepInfo>,
    /// The key of every unit by its trace id, which v1's graph names callees by.
    pub(in crate::codegen::v2) keys: HashMap<u32, String>,
    /// v1's signature of every function, method and lifted closure.
    pub(in crate::codegen::v2) signatures: HashMap<String, FunctionSignature>,
    /// v1's implementation-method registry.
    pub(in crate::codegen::v2) methods: MethodRegistry,
    /// The layouts v1's graph builder sees: resumable units and every lifted closure.
    pub(in crate::codegen::v2) frame_layouts: HashMap<String, FrameLayout>,
    /// The functions lifted from closures.
    pub(in crate::codegen::v2) lifted: &'l [LiftedFunction<'a>],
    /// The closures that may suspend, by their dense closure index.
    pub(in crate::codegen::v2) closures: HashMap<u32, ClosureStep>,
    /// Whether the module has `par` expressions, so tasks yield at scheduler checkpoints.
    pub(in crate::codegen::v2) parallel: bool,
}

impl<'l, 'a> Resumable<'l, 'a> {
    /// Builds the context of the `resumable` keys among `reached`, whose Wasm indices
    /// `index_of` returns.
    pub(in crate::codegen::v2) fn build(
        module: &Module<'a>,
        traits: &TraitRegistry<'a>,
        types: &TypeRegistry,
        lifted: &'l [LiftedFunction<'a>],
        reached: &[&Unit<'_, 'a>],
        resumable: &HashSet<String>,
        index_of: impl Fn(&str) -> Option<u32>,
    ) -> Result<Self, CompileDiagnostics<'a>> {
        let signatures = build_signatures(module, lifted, 0, types)?;
        let methods = MethodRegistry::build(module, traits, types, &signatures)?;
        let mut frame_layouts = HashMap::new();
        for (key, signature) in &signatures {
            if resumable.contains(key) || key.starts_with("$closure:") {
                frame_layouts.insert(
                    key.clone(),
                    FrameLayout {
                        function_id: signature.function_id,
                        slot_count: 0,
                    },
                );
            }
        }
        let keys = reached
            .iter()
            .map(|unit| (unit.function_id, unit.key.clone()))
            .collect();
        let mut steps = HashMap::new();
        for unit in reached.iter().filter(|unit| resumable.contains(&unit.key)) {
            let slot_count = frame_slot_count(
                unit.declaration,
                &unit.key,
                &signatures,
                &frame_layouts,
                lifted,
                &methods,
                types,
            )
            .unwrap_or_default();
            let step = steps.len() as u32;
            let index = index_of(&unit.key).unwrap_or_default();
            steps.insert(
                unit.key.clone(),
                StepInfo {
                    step,
                    function_id: unit.function_id,
                    slot_count,
                    index,
                },
            );
        }
        for (key, info) in &steps {
            if let Some(layout) = frame_layouts.get_mut(key) {
                layout.slot_count = info.slot_count;
            }
        }
        Ok(Self {
            steps,
            keys,
            signatures,
            methods,
            frame_layouts,
            lifted,
            closures: HashMap::new(),
            parallel: false,
        })
    }

    /// Returns the step of the unit with the trace id `function_id`, if it is resumable.
    pub(in crate::codegen::v2) fn step_of(&self, function_id: u32) -> Option<StepInfo> {
        self.keys
            .get(&function_id)
            .and_then(|key| self.steps.get(key))
            .copied()
    }
}
