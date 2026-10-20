//! Removal of functions that no runner-visible entry point can reach.
//!
//! The linked module contains the whole runtime template and every compiled prelude function,
//! although a script uses only a few of them. Compiling unused code costs wall-clock time on
//! every execution, so the final module keeps only functions reachable from its exports, its
//! start function, element segments and `ref.func` initializers. The templates' `__exs_rt_*`
//! and `__exs_core_*` exports only exist for linking and are dropped first, which makes their
//! functions prunable like any other.
//!
//! The resumable dispatcher calls every resumable step function, so its calls of steps are
//! weak: they keep no step alive, and calls to pruned steps become `unreachable`. Steps stay live through
//! direct calls, which every child call emits, or as linker-provided roots for the frames
//! created by function id alone: suspendable entry points and lifted closures.

use std::collections::HashMap;

use wasm_encoder::reencode::{self, Reencode};
use wasmparser::{
    ConstExpr, ElementItems, ExternalKind, FunctionBody, Name, Operator, Parser, Payload, TypeRef,
};

/// Export-name prefixes of template functions that only the linker resolves: the v1 runtime's
/// and the v2 core library's.
const LINK_EXPORT_PREFIXES: [&str; 2] = ["__exs_rt_", "__exs_core_"];

/// Returns whether one export exists only for linking.
fn is_link_export(name: &str) -> bool {
    LINK_EXPORT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Returns `wasm` without functions unreachable from its runner-visible roots.
///
/// # Errors
///
/// Returns a message when the module cannot be parsed or re-encoded, which indicates a linker
/// bug rather than a source error.
pub(super) fn prune_unreachable(wasm: &[u8], hints: &PruneHints) -> Result<Vec<u8>, String> {
    let live = live_functions(wasm, hints)?;
    let mut pruner = Pruner {
        live,
        weak_caller: hints.weak_caller,
        in_weak_body: false,
    };
    let mut module = wasm_encoder::Module::new();
    reencode::utils::parse_core_module(&mut pruner, &mut module, Parser::new(0), wasm)
        .map_err(|error| format!("could not prune the linked module: {error:?}"))?;
    Ok(module.finish())
}

/// Linker knowledge about functions reached without a direct call.
pub(super) struct PruneHints {
    /// The resumable dispatcher, whose calls of `weak_targets` keep nothing alive.
    pub(super) weak_caller: Option<u32>,
    /// Every resumable step function the dispatcher may select.
    pub(super) weak_targets: Vec<u32>,
    /// Step functions whose frames are created by function id, which keep them alive.
    pub(super) roots: Vec<u32>,
}

/// Reachability of every function and the index each live one keeps in the pruned module.
struct LiveFunctions {
    /// Number of imported functions, which are always kept and never renumbered.
    imports: u32,
    /// New index of every live function, by old index.
    new_index: HashMap<u32, u32>,
}

impl LiveFunctions {
    /// Returns whether the function with old index `index` is kept.
    fn is_live(&self, index: u32) -> bool {
        index < self.imports || self.new_index.contains_key(&index)
    }
}

/// Collects the roots of `wasm` and marks every function reachable from them.
fn live_functions(wasm: &[u8], hints: &PruneHints) -> Result<LiveFunctions, String> {
    let error = |error: wasmparser::BinaryReaderError| error.to_string();
    let mut imports = 0;
    let mut roots = hints.roots.clone();
    let mut bodies = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(error)? {
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(
                        import.map_err(error)?.ty,
                        TypeRef::Func(_) | TypeRef::FuncExact(_)
                    ) {
                        imports += 1;
                    }
                }
            }
            Payload::ExportSection(section) => {
                for export in section {
                    let export = export.map_err(error)?;
                    if export.kind == ExternalKind::Func && !is_link_export(export.name) {
                        roots.push(export.index);
                    }
                }
            }
            Payload::StartSection { func, .. } => roots.push(func),
            Payload::ElementSection(section) => {
                for element in section {
                    match element.map_err(error)?.items {
                        ElementItems::Functions(functions) => {
                            for function in functions {
                                roots.push(function.map_err(error)?);
                            }
                        }
                        ElementItems::Expressions(_, expressions) => {
                            for expression in expressions {
                                const_expression_functions(
                                    &expression.map_err(error)?,
                                    &mut roots,
                                )?;
                            }
                        }
                    }
                }
            }
            Payload::GlobalSection(section) => {
                for global in section {
                    const_expression_functions(&global.map_err(error)?.init_expr, &mut roots)?;
                }
            }
            Payload::CodeSectionEntry(body) => bodies.push(body),
            _ => {}
        }
    }
    let mut live = vec![false; bodies.len()];
    while let Some(index) = roots.pop() {
        let Some(defined) = index.checked_sub(imports) else {
            continue;
        };
        let Some(slot) = live.get_mut(defined as usize) else {
            return Err(format!("function index {index} is out of range"));
        };
        if std::mem::replace(slot, true) {
            continue;
        }
        if let Some(body) = bodies.get(defined as usize) {
            let first_new = roots.len();
            called_functions(body, &mut roots)?;
            if hints.weak_caller == Some(index) {
                let mut position = first_new;
                while position < roots.len() {
                    if hints.weak_targets.contains(&roots[position]) {
                        roots.swap_remove(position);
                    } else {
                        position += 1;
                    }
                }
            }
        }
    }
    let mut new_index = HashMap::new();
    let mut next = imports;
    for (defined, is_live) in (0_u32..).zip(&live) {
        if *is_live {
            new_index.insert(imports + defined, next);
            next += 1;
        }
    }
    Ok(LiveFunctions { imports, new_index })
}

/// Appends every function a body calls or references to `functions`.
fn called_functions(body: &FunctionBody<'_>, functions: &mut Vec<u32>) -> Result<(), String> {
    let mut reader = body
        .get_operators_reader()
        .map_err(|error| error.to_string())?;
    while !reader.eof() {
        match reader.read().map_err(|error| error.to_string())? {
            Operator::Call { function_index }
            | Operator::ReturnCall { function_index }
            | Operator::RefFunc { function_index } => functions.push(function_index),
            _ => {}
        }
    }
    Ok(())
}

/// Appends every function a constant expression references to `functions`.
fn const_expression_functions(
    expression: &ConstExpr<'_>,
    functions: &mut Vec<u32>,
) -> Result<(), String> {
    let mut reader = expression.get_operators_reader();
    while !reader.eof() {
        if let Operator::RefFunc { function_index } =
            reader.read().map_err(|error| error.to_string())?
        {
            functions.push(function_index);
        }
    }
    Ok(())
}

/// Re-encoder that skips dead functions and renumbers the live ones.
struct Pruner {
    /// Reachability computed before re-encoding.
    live: LiveFunctions,
    /// Function whose calls to pruned functions become `unreachable`.
    weak_caller: Option<u32>,
    /// Whether the body being re-encoded belongs to `weak_caller`.
    in_weak_body: bool,
}

impl Reencode for Pruner {
    type Error = String;

    fn function_index(&mut self, func: u32) -> Result<u32, reencode::Error<Self::Error>> {
        if func < self.live.imports {
            return Ok(func);
        }
        self.live.new_index.get(&func).copied().ok_or_else(|| {
            reencode::Error::UserError(format!("live code references pruned function {func}"))
        })
    }

    fn parse_function_section(
        &mut self,
        functions: &mut wasm_encoder::FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for (index, type_index) in (self.live.imports..).zip(section) {
            let type_index = type_index?;
            if self.live.is_live(index) {
                functions.function(self.type_index(type_index)?);
            }
        }
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        code: &mut wasm_encoder::CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for (index, body) in (self.live.imports..).zip(section) {
            let body = body?;
            if self.live.is_live(index) {
                self.in_weak_body = self.weak_caller == Some(index);
                self.parse_function_body(code, body)?;
            }
        }
        self.in_weak_body = false;
        Ok(())
    }

    fn instruction<'a>(
        &mut self,
        operator: Operator<'a>,
    ) -> Result<wasm_encoder::Instruction<'a>, reencode::Error<Self::Error>> {
        if self.in_weak_body
            && let Operator::Call { function_index } = operator
            && !self.live.is_live(function_index)
        {
            // The dispatcher never selects a pruned step, because no frame of it can exist.
            return Ok(wasm_encoder::Instruction::Unreachable);
        }
        reencode::utils::instruction(self, operator)
    }

    fn parse_export_section(
        &mut self,
        exports: &mut wasm_encoder::ExportSection,
        section: wasmparser::ExportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for export in section {
            let export = export?;
            if export.kind == ExternalKind::Func && is_link_export(export.name) {
                continue;
            }
            self.parse_export(exports, export)?;
        }
        Ok(())
    }

    fn parse_custom_name_subsection(
        &mut self,
        names: &mut wasm_encoder::NameSection,
        section: Name<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        match section {
            Name::Function(map) => {
                let mut functions = wasm_encoder::NameMap::new();
                for naming in map {
                    let naming = naming?;
                    if self.live.is_live(naming.index) {
                        functions.append(self.function_index(naming.index)?, naming.name);
                    }
                }
                names.functions(&functions);
                Ok(())
            }
            // Per-function local and label names are only debugging aids; dropping them avoids
            // remapping entries of pruned functions.
            Name::Local(_) | Name::Label(_) => Ok(()),
            section => reencode::utils::parse_custom_name_subsection(self, names, section),
        }
    }
}
