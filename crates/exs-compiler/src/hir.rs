//! Binding-resolved intermediate data used by suspendability analysis.

use std::collections::HashMap;

use crate::ast::{AssignmentTarget, Block, Expression, FunctionDeclaration, Module, Statement};
use crate::diagnostic::SourceSpan;

/// One compiler-assigned lexical binding identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BindingId(pub(crate) u32);

/// The resolved data for every source function and implementation method.
pub(crate) struct HirModule<'a> {
    functions: HashMap<String, HirFunction<'a>>,
}

impl<'a> HirModule<'a> {
    /// Resolves lexical bindings and call edges without changing source execution semantics.
    #[must_use]
    pub(crate) fn lower(module: &'a Module<'a>) -> Self {
        let instance_targets = instance_method_targets(module);
        let mut functions = HashMap::new();
        for function in &module.functions {
            let key = function.name.name.clone();
            let _previous = functions.insert(key, HirFunction::lower(function, &instance_targets));
        }
        for implementation in &module.implementations {
            for function in &implementation.methods {
                let key = format!("{}::{}", implementation.type_name.name, function.name.name);
                let _previous =
                    functions.insert(key, HirFunction::lower(function, &instance_targets));
            }
        }
        Self { functions }
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
}

/// Resolved lexical and call-edge information for one source function.
pub(crate) struct HirFunction<'a> {
    bindings: Vec<Binding<'a>>,
    references: Vec<BindingReference<'a>>,
    calls: Vec<CallEdge<'a>>,
    host_calls: Vec<HostCall<'a>>,
}

impl<'a> HirFunction<'a> {
    /// Lowers one source declaration into binding and suspend-point metadata.
    fn lower(
        function: &'a FunctionDeclaration<'a>,
        instance_targets: &HashMap<String, Vec<String>>,
    ) -> Self {
        let mut lowerer = FunctionLowerer::new(function, instance_targets);
        lowerer.lower_block(&function.body);
        HirFunction {
            bindings: lowerer.bindings,
            references: lowerer.references,
            calls: lowerer.calls,
            host_calls: lowerer.host_calls,
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

    /// Returns dynamically named host-call suspend points.
    #[must_use]
    pub(crate) fn host_calls(&self) -> &[HostCall<'a>] {
        &self.host_calls
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

/// A direct source function or static implementation-method invocation.
pub(crate) struct CallEdge<'a> {
    /// The source name used as the callee lookup key.
    pub(crate) key: String,
    /// Full call source span.
    pub(crate) span: SourceSpan<'a>,
}

/// A dynamic host invocation that is always a potential suspend point.
pub(crate) struct HostCall<'a> {
    /// Full source span for diagnostics and runtime source position.
    pub(crate) span: SourceSpan<'a>,
}

/// Mutable lexical resolver for one source function.
struct FunctionLowerer<'a> {
    scopes: Vec<HashMap<&'a str, BindingId>>,
    next_binding: u32,
    bindings: Vec<Binding<'a>>,
    references: Vec<BindingReference<'a>>,
    calls: Vec<CallEdge<'a>>,
    host_calls: Vec<HostCall<'a>>,
    instance_targets: HashMap<String, Vec<String>>,
}

impl<'a> FunctionLowerer<'a> {
    /// Creates a resolver with parameter bindings in the outermost function scope.
    fn new(
        function: &'a FunctionDeclaration<'a>,
        instance_targets: &HashMap<String, Vec<String>>,
    ) -> Self {
        let mut lowerer = Self {
            scopes: vec![HashMap::new()],
            next_binding: 0,
            bindings: Vec::new(),
            references: Vec::new(),
            calls: Vec::new(),
            host_calls: Vec::new(),
            instance_targets: instance_targets.clone(),
        };
        for parameter in &function.parameters {
            lowerer.declare(&parameter.name.name, parameter.name.span);
        }
        lowerer
    }

    /// Allocates one lexical binding in the innermost scope.
    fn declare(&mut self, name: &'a str, span: SourceSpan<'a>) {
        let id = BindingId(self.next_binding);
        self.next_binding = self.next_binding.saturating_add(1);
        if let Some(scope) = self.scopes.last_mut() {
            let _previous = scope.insert(name, id);
        }
        self.bindings.push(Binding { id, name, span });
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
            Statement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.lower_expression(condition);
                self.lower_block(then_block);
                if let Some(else_block) = else_block {
                    self.lower_block(else_block);
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
            Expression::IsError { value, .. }
            | Expression::Propagate { value, .. }
            | Expression::Unary { operand: value, .. } => self.lower_expression(value),
            Expression::Binary { left, right, .. } => {
                self.lower_expression(left);
                self.lower_expression(right);
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                self.calls.push(CallEdge {
                    key: callee.name.clone(),
                    span: *span,
                });
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
            Expression::Index {
                receiver, index, ..
            } => {
                self.lower_expression(receiver);
                self.lower_expression(index);
            }
            Expression::Property { receiver, .. } => self.lower_expression(receiver),
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {}
        }
    }

    /// Records one variable reference with its nearest lexical binding identity.
    fn reference(&mut self, name: &'a str, span: SourceSpan<'a>) {
        let binding = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied());
        self.references.push(BindingReference { binding, span });
    }
}

/// Collects every statically known implementation target for each dynamic method name.
fn instance_method_targets<'a>(module: &'a Module<'a>) -> HashMap<String, Vec<String>> {
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
            }
        }
    }
    targets
}
