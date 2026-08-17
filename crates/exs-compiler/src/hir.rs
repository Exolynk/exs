//! Binding-resolved intermediate data used by suspendability analysis.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::ast::{
    AssignmentTarget, Block, Expression, FunctionDeclaration, Module, Parameter, Statement,
};
use crate::codegen::trait_registry::{TraitOperator, TraitRegistry};
use crate::diagnostic::SourceSpan;

/// One compiler-assigned lexical binding identity within a source module.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BindingId(pub(crate) u32);

/// One stable source-order identity for a closure lifted from a module.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ClosureId(pub(crate) u32);

/// The resolved data for every source function and implementation method.
pub(crate) struct HirModule<'a> {
    functions: HashMap<String, HirFunction<'a>>,
    closures: Vec<HirClosure<'a>>,
}

impl<'a> HirModule<'a> {
    /// Resolves lexical bindings and call edges without changing source execution semantics.
    #[must_use]
    pub(crate) fn lower(module: &'a Module<'a>, traits: &TraitRegistry<'a>) -> Self {
        let instance_targets = instance_method_targets(module, traits);
        let state = LoweringState::new();
        let mut functions = HashMap::new();
        for function in &module.functions {
            let key = function.name.name.clone();
            let lowered = HirFunction::lower(function, key.clone(), &instance_targets, &state);
            let _previous = functions.insert(key, lowered);
        }
        for implementation in &module.implementations {
            for function in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, function.name.name);
                let lowered = HirFunction::lower(function, key.clone(), &instance_targets, &state);
                let _previous = functions.insert(key, lowered);
            }
        }
        Self {
            functions,
            closures: state.into_closures(),
        }
    }

    /// Returns the resolved data for one direct function or implementation method.
    #[must_use]
    pub(crate) fn function(&self, key: &str) -> Option<&HirFunction<'a>> {
        self.functions.get(key)
    }

    /// Iterates over resolved functions in unspecified map order.
    pub(crate) fn functions(&self) -> impl Iterator<Item = (&str, &HirFunction<'a>)> {
        self.functions
            .iter()
            .map(|(key, function)| (key.as_str(), function))
    }

    /// Iterates over closures in stable lexical source order.
    pub(crate) fn closures(&self) -> impl Iterator<Item = &HirClosure<'a>> {
        self.closures.iter()
    }
}

/// The immediately enclosing callable that owns a lifted closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClosureOwner {
    /// A direct source function or implementation method compiler key.
    Function(String),
    /// Another lifted closure.
    Closure(ClosureId),
}

/// Resolved closure metadata used by later lifting and runtime-value lowering.
pub(crate) struct HirClosure<'a> {
    id: ClosureId,
    owner: ClosureOwner,
    parameters: Vec<BindingId>,
    bindings: Vec<Binding<'a>>,
    references: Vec<BindingReference<'a>>,
    captures: Vec<Capture<'a>>,
    calls: Vec<CallEdge<'a>>,
    callable_calls: Vec<CallableCall<'a>>,
    host_calls: Vec<HostCall<'a>>,
}

impl<'a> HirClosure<'a> {
    /// Returns the stable lifted identity assigned in lexical source order.
    #[must_use]
    pub(crate) fn id(&self) -> ClosureId {
        self.id
    }

    /// Returns the immediately enclosing callable that creates this closure.
    #[must_use]
    pub(crate) fn owner(&self) -> &ClosureOwner {
        &self.owner
    }

    /// Returns the compiler identities of this closure's parameter bindings.
    #[must_use]
    pub(crate) fn parameters(&self) -> &[BindingId] {
        &self.parameters
    }

    /// Returns the lexical bindings declared inside this closure.
    #[must_use]
    pub(crate) fn bindings(&self) -> &[Binding<'a>] {
        &self.bindings
    }

    /// Returns source variable references and their resolved binding identities.
    #[must_use]
    pub(crate) fn references(&self) -> &[BindingReference<'a>] {
        &self.references
    }

    /// Returns the non-local bindings captured in first-use source order.
    #[must_use]
    pub(crate) fn captures(&self) -> &[Capture<'a>] {
        &self.captures
    }

    /// Returns direct and statically selected call edges.
    #[must_use]
    pub(crate) fn calls(&self) -> &[CallEdge<'a>] {
        &self.calls
    }

    /// Returns local binding calls that will require dynamic closure invocation.
    #[must_use]
    pub(crate) fn callable_calls(&self) -> &[CallableCall<'a>] {
        &self.callable_calls
    }

    /// Returns dynamically named host-call suspend points.
    #[must_use]
    pub(crate) fn host_calls(&self) -> &[HostCall<'a>] {
        &self.host_calls
    }
}

/// Resolved lexical and call-edge information for one source function.
pub(crate) struct HirFunction<'a> {
    bindings: Vec<Binding<'a>>,
    references: Vec<BindingReference<'a>>,
    calls: Vec<CallEdge<'a>>,
    callable_calls: Vec<CallableCall<'a>>,
    host_calls: Vec<HostCall<'a>>,
    parallel_calls: Vec<SourceSpan<'a>>,
    matches: bool,
}

impl<'a> HirFunction<'a> {
    /// Lowers one source declaration into binding and suspend-point metadata.
    fn lower(
        function: &'a FunctionDeclaration<'a>,
        key: String,
        instance_targets: &HashMap<String, Vec<String>>,
        state: &LoweringState<'a>,
    ) -> Self {
        let mut lowerer = FunctionLowerer::new_root(function, key, instance_targets, state);
        lowerer.lower_block(&function.body);
        HirFunction {
            bindings: lowerer.bindings,
            references: lowerer.references,
            calls: lowerer.calls,
            callable_calls: lowerer.callable_calls,
            host_calls: lowerer.host_calls,
            parallel_calls: lowerer.parallel_calls,
            matches: lowerer.matches,
        }
    }

    /// Returns the lexical bindings allocated by this function.
    #[must_use]
    pub(crate) fn bindings(&self) -> &[Binding<'a>] {
        &self.bindings
    }

    /// Returns source variable references and their resolved binding identities.
    #[must_use]
    pub(crate) fn references(&self) -> &[BindingReference<'a>] {
        &self.references
    }

    /// Returns direct and statically selected call edges.
    #[must_use]
    pub(crate) fn calls(&self) -> &[CallEdge<'a>] {
        &self.calls
    }

    /// Returns local binding calls that will require dynamic closure invocation.
    #[must_use]
    pub(crate) fn callable_calls(&self) -> &[CallableCall<'a>] {
        &self.callable_calls
    }

    /// Returns dynamically named host-call suspend points.
    #[must_use]
    pub(crate) fn host_calls(&self) -> &[HostCall<'a>] {
        &self.host_calls
    }

    /// Returns parallel-expression sites that require resumable lowering.
    #[must_use]
    pub(crate) fn parallel_calls(&self) -> &[SourceSpan<'a>] {
        &self.parallel_calls
    }

    /// Returns whether this function contains a match expression.
    #[must_use]
    pub(crate) const fn has_matches(&self) -> bool {
        self.matches
    }
}

/// One source binding assigned a stable identity within its containing function.
pub(crate) struct Binding<'a> {
    /// The compiler-local binding identity.
    pub(crate) id: BindingId,
    /// Source spelling of the binding.
    pub(crate) name: &'a str,
    /// Declaration source span.
    pub(crate) span: SourceSpan<'a>,
}

/// One source variable use linked to its lexical declaration when it resolved successfully.
pub(crate) struct BindingReference<'a> {
    /// The resolved binding, or None when later semantic diagnostics report an unknown name.
    pub(crate) binding: Option<BindingId>,
    /// Reference source span.
    pub(crate) span: SourceSpan<'a>,
}

/// One non-local binding retained by a closure environment.
pub(crate) struct Capture<'a> {
    /// The shared lexical binding identity retained by the closure.
    pub(crate) binding: BindingId,
    /// Source spelling resolved to this lexical binding at the first capture use.
    pub(crate) name: &'a str,
    /// First source use that required this capture.
    pub(crate) span: SourceSpan<'a>,
}

/// A direct source function or static implementation-method invocation.
pub(crate) struct CallEdge<'a> {
    /// The source name used as the callee lookup key.
    pub(crate) key: String,
    /// Full call source span.
    pub(crate) span: SourceSpan<'a>,
}

/// One source call whose callee resolved to a local lexical binding.
pub(crate) struct CallableCall<'a> {
    /// The local binding used as the dynamic callee.
    pub(crate) binding: BindingId,
    /// Full call source span.
    pub(crate) span: SourceSpan<'a>,
}

/// A dynamic host invocation that is always a potential suspend point.
pub(crate) struct HostCall<'a> {
    /// Full source span for diagnostics and runtime source position.
    pub(crate) span: SourceSpan<'a>,
}

/// Shared identity and closure collection state for one module lowering pass.
struct LoweringState<'a> {
    next_binding: Cell<u32>,
    next_closure: Cell<u32>,
    closures: RefCell<Vec<HirClosure<'a>>>,
}

impl<'a> LoweringState<'a> {
    /// Creates an empty source-order identity allocator.
    fn new() -> Self {
        Self {
            next_binding: Cell::new(0),
            next_closure: Cell::new(0),
            closures: RefCell::new(Vec::new()),
        }
    }

    /// Allocates the next module-wide binding identity.
    fn allocate_binding(&self) -> BindingId {
        let next = self.next_binding.get();
        self.next_binding.set(next.saturating_add(1));
        BindingId(next)
    }

    /// Allocates the next lexical source-order closure identity.
    fn allocate_closure(&self) -> ClosureId {
        let next = self.next_closure.get();
        self.next_closure.set(next.saturating_add(1));
        ClosureId(next)
    }

    /// Retains one discovered closure for later lifting.
    fn push_closure(&self, closure: HirClosure<'a>) {
        self.closures.borrow_mut().push(closure);
    }

    /// Releases every discovered closure after module lowering completes.
    fn into_closures(self) -> Vec<HirClosure<'a>> {
        let mut closures = self.closures.into_inner();
        closures.sort_by_key(|closure| closure.id.0);
        closures
    }
}

/// Mutable lexical resolver for one source function.
struct FunctionLowerer<'a, 'state> {
    scopes: Vec<HashMap<&'a str, BindingId>>,
    root_key: String,
    owner: Option<ClosureId>,
    parameters: Vec<BindingId>,
    bindings: Vec<Binding<'a>>,
    references: Vec<BindingReference<'a>>,
    captures: Vec<Capture<'a>>,
    calls: Vec<CallEdge<'a>>,
    callable_calls: Vec<CallableCall<'a>>,
    host_calls: Vec<HostCall<'a>>,
    parallel_calls: Vec<SourceSpan<'a>>,
    matches: bool,
    instance_targets: &'state HashMap<String, Vec<String>>,
    state: &'state LoweringState<'a>,
}

impl<'a, 'state> FunctionLowerer<'a, 'state> {
    /// Creates a resolver with parameter bindings in the outermost function scope.
    fn new_root(
        function: &'a FunctionDeclaration<'a>,
        root_key: String,
        instance_targets: &'state HashMap<String, Vec<String>>,
        state: &'state LoweringState<'a>,
    ) -> Self {
        let mut lowerer = Self {
            scopes: vec![HashMap::new()],
            root_key,
            owner: None,
            parameters: Vec::new(),
            bindings: Vec::new(),
            references: Vec::new(),
            captures: Vec::new(),
            calls: Vec::new(),
            callable_calls: Vec::new(),
            host_calls: Vec::new(),
            parallel_calls: Vec::new(),
            matches: false,
            instance_targets,
            state,
        };
        for parameter in &function.parameters {
            lowerer.declare_parameter(&parameter.name.name, parameter.name.span);
        }
        lowerer
    }

    /// Creates a resolver for one closure with its enclosing lexical scopes visible.
    fn new_closure(
        scopes: Vec<HashMap<&'a str, BindingId>>,
        root_key: String,
        owner: ClosureId,
        parameters: &'a [Parameter<'a>],
        instance_targets: &'state HashMap<String, Vec<String>>,
        state: &'state LoweringState<'a>,
    ) -> Self {
        let mut lowerer = Self {
            scopes,
            root_key,
            owner: Some(owner),
            parameters: Vec::new(),
            bindings: Vec::new(),
            references: Vec::new(),
            captures: Vec::new(),
            calls: Vec::new(),
            callable_calls: Vec::new(),
            host_calls: Vec::new(),
            parallel_calls: Vec::new(),
            matches: false,
            instance_targets,
            state,
        };
        lowerer.scopes.push(HashMap::new());
        for parameter in parameters {
            lowerer.declare_parameter(&parameter.name.name, parameter.name.span);
        }
        lowerer
    }

    /// Allocates one lexical binding in the innermost scope.
    fn declare(&mut self, name: &'a str, span: SourceSpan<'a>) {
        let id = self.state.allocate_binding();
        if let Some(scope) = self.scopes.last_mut() {
            let _previous = scope.insert(name, id);
        }
        self.bindings.push(Binding { id, name, span });
    }

    /// Allocates one parameter binding and retains its declaration order.
    fn declare_parameter(&mut self, name: &'a str, span: SourceSpan<'a>) {
        self.declare(name, span);
        if let Some(binding) = self.bindings.last() {
            self.parameters.push(binding.id);
        }
    }

    /// Resolves every statement in a lexical block.
    fn lower_block(&mut self, block: &'a Block<'a>) {
        self.scopes.push(HashMap::new());
        for statement in &block.statements {
            self.lower_statement(statement);
        }
        let _scope = self.scopes.pop();
    }

    /// Resolves bindings and call edges inside one statement.
    fn lower_statement(&mut self, statement: &'a Statement<'a>) {
        match statement {
            Statement::Let { name, value, .. } => {
                self.lower_expression(value);
                self.declare(&name.name, name.span);
            }
            Statement::Assign { target, value, .. } => {
                self.lower_assignment_target(target);
                self.lower_expression(value);
            }
            Statement::Return { value, .. } => {
                if let Some(value) = value {
                    self.lower_expression(value);
                }
            }
            Statement::Block { block, .. } => self.lower_block(block),
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.lower_expression(condition);
                self.lower_block(then_block);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        crate::ast::ElseBranch::Block(block) => self.lower_block(block),
                        crate::ast::ElseBranch::If(statement) => self.lower_statement(statement),
                    }
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                self.lower_expression(condition);
                self.lower_block(body);
            }
            Statement::For {
                binding,
                iterable,
                body,
                ..
            } => {
                self.lower_expression(iterable);
                self.scopes.push(HashMap::new());
                self.declare(&binding.name, binding.span);
                for statement in &body.statements {
                    self.lower_statement(statement);
                }
                let _scope = self.scopes.pop();
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
            Statement::Expression { expression, .. } => self.lower_expression(expression),
        }
    }

    /// Resolves all value dependencies in one assignment target.
    fn lower_assignment_target(&mut self, target: &'a AssignmentTarget<'a>) {
        match target {
            AssignmentTarget::Variable(identifier) => {
                self.reference(identifier.name.as_str(), identifier.span)
            }
            AssignmentTarget::Index {
                receiver, index, ..
            } => {
                self.lower_expression(receiver);
                self.lower_expression(index);
            }
            AssignmentTarget::Property { receiver, .. } => self.lower_expression(receiver),
        }
    }

    /// Resolves all bindings and calls contained in one source expression.
    fn lower_expression(&mut self, expression: &'a Expression<'a>) {
        match expression {
            Expression::Variable(identifier) => {
                self.reference(identifier.name.as_str(), identifier.span)
            }
            Expression::FormattedString { parts, .. } => {
                for part in parts {
                    if let crate::ast::FormattedStringPart::Expression(expression) = part {
                        self.lower_expression(expression);
                    }
                }
            }
            Expression::IsError { value, .. }
            | Expression::Propagate { value, .. }
            | Expression::Unary { operand: value, .. } => self.lower_expression(value),
            Expression::Binary {
                operator,
                left,
                right,
                span,
            } => {
                if let Some(operator) = TraitOperator::from_binary(*operator)
                    && let Some(targets) = self.instance_targets.get(operator.target_key())
                {
                    self.calls.extend(
                        targets
                            .iter()
                            .cloned()
                            .map(|key| CallEdge { key, span: *span }),
                    );
                }
                self.lower_expression(left);
                self.lower_expression(right);
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                if let Some(binding) = self.resolve(&callee.name) {
                    self.reference_binding(Some(binding), callee.span);
                    self.callable_calls.push(CallableCall {
                        binding,
                        span: *span,
                    });
                    // Preserve the current direct-call lowering path until dynamic invocation lands.
                    self.calls.push(CallEdge {
                        key: callee.name.clone(),
                        span: *span,
                    });
                } else {
                    self.calls.push(CallEdge {
                        key: callee.name.clone(),
                        span: *span,
                    });
                }
                for argument in arguments {
                    self.lower_expression(argument);
                }
            }
            Expression::HostCall {
                name,
                arguments,
                span,
            } => {
                self.host_calls.push(HostCall { span: *span });
                self.lower_expression(name);
                for argument in arguments {
                    self.lower_expression(argument);
                }
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                ..
            } => {
                if let Some(targets) = self.instance_targets.get(&method.name) {
                    self.calls
                        .extend(targets.iter().cloned().map(|key| CallEdge {
                            key,
                            span: method.span,
                        }));
                }
                self.lower_expression(receiver);
                for argument in arguments {
                    self.lower_expression(argument);
                }
            }
            Expression::StaticMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => {
                self.calls.push(CallEdge {
                    key: format!("{}::{}", type_name.name, method.name),
                    span: *span,
                });
                for argument in arguments {
                    self.lower_expression(argument);
                }
            }
            Expression::List { elements, .. } => {
                for element in elements {
                    self.lower_expression(element);
                }
            }
            Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
                for property in properties {
                    self.lower_expression(&property.value);
                }
            }
            Expression::Match { value, arms, .. } => {
                self.matches = true;
                self.lower_expression(value);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    if let crate::ast::MatchPattern::Variant { bindings, .. } = &arm.pattern {
                        for binding in bindings {
                            self.declare(&binding.name, binding.span);
                        }
                    }
                    match &arm.body {
                        crate::ast::MatchArmBody::Expression(value) => self.lower_expression(value),
                        crate::ast::MatchArmBody::Block(block) => self.lower_block(block),
                    }
                    let _scope = self.scopes.pop();
                }
            }
            Expression::Index {
                receiver, index, ..
            } => {
                self.lower_expression(receiver);
                self.lower_expression(index);
            }
            Expression::Property { receiver, .. } => self.lower_expression(receiver),
            Expression::Closure {
                parameters,
                body,
                span,
            } => self.lower_closure(parameters, body, *span),
            Expression::ParallelStatic { tasks, span } => {
                self.parallel_calls.push(*span);
                for task in tasks {
                    self.lower_expression(task);
                }
            }
            Expression::ParallelDynamic { functions, span } => {
                self.parallel_calls.push(*span);
                self.lower_expression(functions);
            }
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {}
        }
    }

    /// Records one variable reference with its nearest lexical binding identity.
    fn reference(&mut self, name: &'a str, span: SourceSpan<'a>) {
        let binding = self.resolve(name);
        self.reference_binding(binding, span);
    }

    /// Resolves one name using the innermost visible lexical declaration.
    fn resolve(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Records a resolved reference and captures non-local bindings for closures.
    fn reference_binding(&mut self, binding: Option<BindingId>, span: SourceSpan<'a>) {
        self.references.push(BindingReference { binding, span });
        if let Some(binding) = binding {
            self.capture(binding, None, span);
        }
    }

    /// Records one capture unless this callable owns the referenced binding itself.
    fn capture(&mut self, binding: BindingId, name: Option<&'a str>, span: SourceSpan<'a>) {
        if self.owner.is_none() || self.bindings.iter().any(|local| local.id == binding) {
            return;
        }
        if self.capture_index(binding).is_none() {
            let name = name.unwrap_or_else(|| {
                self.scopes
                    .iter()
                    .rev()
                    .find_map(|scope| {
                        scope
                            .iter()
                            .find_map(|(name, candidate)| (*candidate == binding).then_some(*name))
                    })
                    .unwrap_or("")
            });
            self.captures.push(Capture {
                binding,
                name,
                span,
            });
        }
    }

    /// Returns the first-use capture index for one binding, when present.
    fn capture_index(&self, binding: BindingId) -> Option<usize> {
        self.captures
            .iter()
            .position(|capture| capture.binding == binding)
    }

    /// Discovers one nested closure and propagates inherited captures to its parent closure.
    fn lower_closure(
        &mut self,
        parameters: &'a [Parameter<'a>],
        body: &'a Block<'a>,
        _span: SourceSpan<'a>,
    ) {
        let id = self.state.allocate_closure();
        let owner = self.owner.map_or_else(
            || ClosureOwner::Function(self.root_key.clone()),
            ClosureOwner::Closure,
        );
        let mut lowerer = Self::new_closure(
            self.scopes.clone(),
            self.root_key.clone(),
            id,
            parameters,
            self.instance_targets,
            self.state,
        );
        lowerer.lower_block(body);

        for capture in &lowerer.captures {
            self.capture(capture.binding, Some(capture.name), capture.span);
        }

        self.state.push_closure(HirClosure {
            id,
            owner,
            parameters: lowerer.parameters,
            bindings: lowerer.bindings,
            references: lowerer.references,
            captures: lowerer.captures,
            calls: lowerer.calls,
            callable_calls: lowerer.callable_calls,
            host_calls: lowerer.host_calls,
        });
    }
}

/// Collects every statically known implementation target for each dynamic method name.
fn instance_method_targets<'a>(
    module: &'a Module<'a>,
    traits: &TraitRegistry<'a>,
) -> HashMap<String, Vec<String>> {
    let mut targets: HashMap<String, Vec<String>> = HashMap::new();
    for implementation in &module.implementations {
        for method in &implementation.methods {
            if method
                .parameters
                .first()
                .is_some_and(|parameter| parameter.name.name == "self")
            {
                targets
                    .entry(method.name.name.clone())
                    .or_default()
                    .push(format!(
                        "{}::{}",
                        implementation.type_name.name, method.name.name
                    ));
                if let Some(trait_name) = &implementation.trait_name {
                    for operator in traits.operators_for(&trait_name.name, &method.name.name) {
                        targets
                            .entry(operator.target_key().to_owned())
                            .or_default()
                            .push(format!(
                                "{}::{}",
                                implementation.type_name.name, method.name.name
                            ));
                    }
                }
            }
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::{ClosureId, ClosureOwner, HirModule};
    use crate::SourceInput;
    use crate::ast::Module;

    /// Parses one source fixture for direct HIR inspection.
    fn parse_module(source: &str) -> Module<'_> {
        let lexed = crate::lexer::lex(SourceInput {
            source_id: "hir-test.exs",
            text: source,
        });
        assert!(lexed.diagnostics.is_empty());
        match crate::parser::parse("hir-test.exs", lexed.tokens, true) {
            Ok(module) => module,
            Err(diagnostics) => panic!("source did not parse: {diagnostics}"),
        }
    }

    /// Lowers one parsed fixture with its normalized trait declarations.
    fn lower_module<'a>(module: &'a Module<'a>) -> HirModule<'a> {
        let traits = crate::codegen::trait_registry::TraitRegistry::build(module);
        HirModule::lower(module, &traits)
    }

    /// Resolves one function binding by its source spelling.
    fn function_binding(hir: &HirModule<'_>, name: &str) -> super::BindingId {
        match hir.function("main").and_then(|function| {
            function
                .bindings()
                .iter()
                .find(|binding| binding.name == name)
        }) {
            Some(binding) => binding.id,
            None => panic!("missing main binding {name}"),
        }
    }

    #[test]
    fn records_direct_closure_captures_in_first_use_order() {
        let module = parse_module(
            "fn main(input) { let first = input; let second = 2; let f = (value) => { ret value + second + first; }; ret 0; }",
        );
        let hir = lower_module(&module);
        let closures = hir.closures().collect::<Vec<_>>();

        assert_eq!(closures.len(), 1);
        assert_eq!(closures[0].id(), ClosureId(0));
        assert_eq!(
            closures[0].owner(),
            &ClosureOwner::Function("main".to_owned())
        );
        assert_eq!(closures[0].captures().len(), 2);
        assert_eq!(
            closures[0].captures()[0].binding,
            function_binding(&hir, "second")
        );
        assert_eq!(
            closures[0].captures()[1].binding,
            function_binding(&hir, "first")
        );
    }

    #[test]
    fn excludes_shadowed_bindings_from_closure_captures() {
        let module = parse_module(
            "fn main(input) { let value = input; let f = (input) => { let value = input; ret value; }; ret 0; }",
        );
        let hir = lower_module(&module);
        let closure = match hir.closures().next() {
            Some(closure) => closure,
            None => panic!("missing closure"),
        };

        assert!(closure.captures().is_empty());
    }

    #[test]
    fn propagates_nested_captures_through_the_enclosing_closure() {
        let module = parse_module(
            "fn main(input) { let offset = input; let outer = (value) => { ret () => { ret value + offset; }; }; ret 0; }",
        );
        let hir = lower_module(&module);
        let closures = hir.closures().collect::<Vec<_>>();
        let offset = function_binding(&hir, "offset");

        assert_eq!(closures.len(), 2);
        assert_eq!(closures[0].id(), ClosureId(0));
        assert_eq!(closures[1].id(), ClosureId(1));
        assert_eq!(closures[1].owner(), &ClosureOwner::Closure(ClosureId(0)));
        assert_eq!(closures[0].captures().len(), 1);
        assert_eq!(closures[0].captures()[0].binding, offset);
        assert_eq!(closures[1].captures().len(), 2);
        assert_eq!(
            closures[1].captures()[0].binding,
            closures[0].parameters()[0]
        );
        assert_eq!(closures[1].captures()[1].binding, offset);
    }

    #[test]
    fn classifies_local_callee_bindings_as_dynamic_calls() {
        let module =
            parse_module("fn main(input) { let f = (value) => { ret value; }; ret f(input); }");
        let hir = lower_module(&module);
        let function = match hir.function("main") {
            Some(function) => function,
            None => panic!("missing main function"),
        };

        assert_eq!(function.calls().len(), 1);
        assert_eq!(function.calls()[0].key, "f");
        assert_eq!(function.callable_calls().len(), 1);
        assert_eq!(
            function.callable_calls()[0].binding,
            function_binding(&hir, "f")
        );
    }
}
