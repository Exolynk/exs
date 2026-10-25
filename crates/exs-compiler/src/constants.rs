//! Module constants: `const NAME = value;` declarations are resolved at compile time and their
//! values substituted into every reference that no local binding shadows.
//!
//! A constant's value is a literal of None, Bool, Int, Float, String or Bytes, a negated
//! number literal, or the name of another constant.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    AssignmentTarget, Block, ConstDeclaration, ElseBranch, Expression, FormattedStringPart,
    MatchArmBody, MatchPattern, Module, Parameter, Statement, TestDeclaration, UnaryOperator,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// The constants of a project by canonical name, with the module that declares each.
pub(crate) struct Declared<'m, 'a> {
    /// Each constant's declaration and the index of its module.
    declarations: HashMap<String, (&'m ConstDeclaration<'a>, usize)>,
}

impl<'m, 'a> Declared<'m, 'a> {
    /// Collects the constants of `modules`, named by `canonical(module index, name)`.
    pub(crate) fn new(
        modules: &'m [Module<'a>],
        canonical: impl Fn(usize, &str) -> String,
    ) -> Self {
        let declarations = modules
            .iter()
            .enumerate()
            .flat_map(|(index, module)| {
                module
                    .constants
                    .iter()
                    .map(move |declaration| (index, declaration))
            })
            .map(|(index, declaration)| {
                (
                    canonical(index, &declaration.name.name),
                    (declaration, index),
                )
            })
            .collect();
        Self { declarations }
    }

    /// Resolves the value of every constant. `bindings[index]` maps the names module `index`
    /// may use to canonical names.
    pub(crate) fn resolve(
        &self,
        bindings: &[HashMap<String, String>],
    ) -> Result<HashMap<String, Expression<'a>>, CompileDiagnostics<'a>> {
        let mut resolver = Resolver {
            declared: self,
            bindings,
            values: HashMap::new(),
            visiting: HashSet::new(),
        };
        let mut names: Vec<&String> = self.declarations.keys().collect();
        names.sort();
        for name in names {
            resolver.value(name)?;
        }
        Ok(resolver.values)
    }
}

/// Resolves constant values, following references to other constants.
struct Resolver<'d, 'm, 'a> {
    /// The project's constants.
    declared: &'d Declared<'m, 'a>,
    /// Canonical names each module may use.
    bindings: &'d [HashMap<String, String>],
    /// Resolved values by canonical name.
    values: HashMap<String, Expression<'a>>,
    /// Constants whose value is being resolved, to report cycles.
    visiting: HashSet<String>,
}

impl<'a> Resolver<'_, '_, 'a> {
    /// Returns the value of the constant `canonical`.
    fn value(&mut self, canonical: &str) -> Result<Expression<'a>, CompileDiagnostics<'a>> {
        if let Some(value) = self.values.get(canonical) {
            return Ok(value.clone());
        }
        let Some((declaration, module)) = self.declared.declarations.get(canonical).copied() else {
            return Err(CompileDiagnostics::new());
        };
        if !self.visiting.insert(canonical.to_owned()) {
            return Err(error(
                "E0121",
                declaration.name.span,
                format!("constant `{}` refers to itself", declaration.name.name),
            ));
        }
        let value = self.evaluate(&declaration.value, module, declaration)?;
        self.visiting.remove(canonical);
        self.values.insert(canonical.to_owned(), value.clone());
        Ok(value)
    }

    /// Returns the value of the constant expression `expression` declared in `module`.
    fn evaluate(
        &mut self,
        expression: &Expression<'a>,
        module: usize,
        declaration: &ConstDeclaration<'a>,
    ) -> Result<Expression<'a>, CompileDiagnostics<'a>> {
        match expression {
            Expression::Integer(..)
            | Expression::Float(..)
            | Expression::String(..)
            | Expression::Bytes(..)
            | Expression::Bool(..)
            | Expression::None(..)
            | Expression::Unit(..) => Ok(expression.clone()),
            Expression::Unary {
                operator: UnaryOperator::Negate,
                operand,
                ..
            } if matches!(**operand, Expression::Integer(..) | Expression::Float(..)) => {
                Ok(expression.clone())
            }
            Expression::Variable(identifier) => {
                let canonical = self.bindings[module]
                    .get(&identifier.name)
                    .filter(|canonical| self.declared.declarations.contains_key(*canonical))
                    .cloned()
                    .ok_or_else(|| {
                        error(
                            "E0119",
                            identifier.span,
                            format!("`{}` is no constant", identifier.name),
                        )
                    })?;
                self.value(&canonical)
                    .map(|value| with_span(value, identifier.span))
            }
            _ => Err(error(
                "E0119",
                declaration.value_span(),
                format!(
                    "the value of constant `{}` must be a literal, a negated number, or another constant",
                    declaration.name.name
                ),
            )),
        }
    }
}

impl<'a> ConstDeclaration<'a> {
    /// Returns the span of the constant's value.
    fn value_span(&self) -> SourceSpan<'a> {
        crate::parser::expression_span(&self.value)
    }
}

/// Returns a single diagnostic.
fn error<'a>(code: &'static str, span: SourceSpan<'a>, message: String) -> CompileDiagnostics<'a> {
    CompileDiagnostics::from(CompileDiagnostic::new(code, span, message))
}

/// Returns `value` reported at `span`, the reference it replaces.
fn with_span<'a>(value: Expression<'a>, span: SourceSpan<'a>) -> Expression<'a> {
    match value {
        Expression::Integer(number, _) => Expression::Integer(number, span),
        Expression::Float(number, _) => Expression::Float(number, span),
        Expression::String(text, _) => Expression::String(text, span),
        Expression::Bytes(text, _) => Expression::Bytes(text, span),
        Expression::Bool(flag, _) => Expression::Bool(flag, span),
        Expression::None(_) => Expression::None(span),
        Expression::Unit(_) => Expression::Unit(span),
        Expression::Unary {
            operator, operand, ..
        } => Expression::Unary {
            operator,
            operand: Box::new(with_span(*operand, span)),
            span,
        },
        other => other,
    }
}

/// Resolves the constants of the single source `module` and substitutes them into it.
pub(crate) fn inline_single<'a>(module: &mut Module<'a>) -> Result<(), CompileDiagnostics<'a>> {
    let mut declared: HashSet<&str> = module
        .functions
        .iter()
        .map(|declaration| declaration.name.name.as_str())
        .chain(
            module
                .types
                .iter()
                .map(|declaration| declaration.name.name.as_str()),
        )
        .chain(
            module
                .enums
                .iter()
                .map(|declaration| declaration.name.name.as_str()),
        )
        .chain(
            module
                .traits
                .iter()
                .map(|declaration| declaration.name.name.as_str()),
        )
        .collect();
    for declaration in &module.constants {
        if !declared.insert(&declaration.name.name) {
            return Err(error(
                "E0122",
                declaration.name.span,
                format!("duplicate declaration `{}`", declaration.name.name),
            ));
        }
    }
    let names: HashMap<String, String> = module
        .constants
        .iter()
        .map(|declaration| (declaration.name.name.clone(), declaration.name.name.clone()))
        .collect();
    let values = Declared::new(std::slice::from_ref(module), |_, name| name.to_owned())
        .resolve(std::slice::from_ref(&names))?;
    inline(module, &values, &HashSet::new())
}

/// Substitutes the constants `visible` names, by the names the module spells, into every
/// reference of `module` that no local binding shadows, and rejects assignments to them.
/// `private` names constants of other libraries that `module` may not use.
pub(crate) fn inline<'a>(
    module: &mut Module<'a>,
    visible: &HashMap<String, Expression<'a>>,
    private: &HashSet<String>,
) -> Result<(), CompileDiagnostics<'a>> {
    if visible.is_empty() && private.is_empty() {
        return Ok(());
    }
    let mut inliner = Inliner {
        visible,
        private,
        scopes: Vec::new(),
        diagnostics: CompileDiagnostics::new(),
    };
    for function in &mut module.functions {
        inliner.function(&function.parameters, &mut function.body);
    }
    for implementation in &mut module.implementations {
        for method in &mut implementation.methods {
            inliner.function(&method.parameters, &mut method.body);
        }
    }
    for declaration in &mut module.traits {
        for method in &mut declaration.methods {
            if let Some(body) = &mut method.body {
                inliner.function(&method.parameters, body);
            }
        }
    }
    inliner.tests(&mut module.tests);
    inliner.finish()
}

/// Substitutes constants into tests taken out of their module, like [`inline`].
pub(crate) fn inline_tests<'a>(
    tests: &mut [TestDeclaration<'a>],
    visible: &HashMap<String, Expression<'a>>,
    private: &HashSet<String>,
) -> Result<(), CompileDiagnostics<'a>> {
    let mut inliner = Inliner {
        visible,
        private,
        scopes: Vec::new(),
        diagnostics: CompileDiagnostics::new(),
    };
    inliner.tests(tests);
    inliner.finish()
}

/// Walks function bodies with their local bindings and substitutes constants.
struct Inliner<'v, 'a> {
    /// Constant values by the names the module spells.
    visible: &'v HashMap<String, Expression<'a>>,
    /// Names of constants private to another library.
    private: &'v HashSet<String>,
    /// Local binding names of the enclosing scopes, innermost last.
    scopes: Vec<HashSet<String>>,
    /// Errors found so far.
    diagnostics: CompileDiagnostics<'a>,
}

impl<'a> Inliner<'_, 'a> {
    /// Walks the bodies of `tests`.
    fn tests(&mut self, tests: &mut [TestDeclaration<'a>]) {
        for test in tests {
            self.function(&[], &mut test.body);
        }
    }

    /// Returns the errors found, if any.
    fn finish(self) -> Result<(), CompileDiagnostics<'a>> {
        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(self.diagnostics)
        }
    }

    /// Walks one function body whose parameters are `parameters`.
    fn function(&mut self, parameters: &[Parameter<'a>], body: &mut Block<'a>) {
        self.scopes.push(
            parameters
                .iter()
                .map(|parameter| parameter.name.name.clone())
                .collect(),
        );
        self.block(body);
        self.scopes.pop();
    }

    /// Reports whether `name` is a local binding of an enclosing scope.
    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    /// Adds `name` to the innermost scope.
    fn bind(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned());
        }
    }

    /// Walks one block in its own scope.
    fn block(&mut self, block: &mut Block<'a>) {
        self.scopes.push(HashSet::new());
        for statement in &mut block.statements {
            self.statement(statement);
        }
        self.scopes.pop();
    }

    /// Walks one statement.
    fn statement(&mut self, statement: &mut Statement<'a>) {
        match statement {
            Statement::Let { name, value, .. } => {
                self.expression(value);
                self.bind(&name.name);
            }
            Statement::Expression { expression, .. } => self.expression(expression),
            Statement::Assign { target, value, .. } => {
                match target {
                    AssignmentTarget::Variable(identifier) => {
                        if !self.is_local(&identifier.name)
                            && self.visible.contains_key(&identifier.name)
                        {
                            self.diagnostics.push(CompileDiagnostic::new(
                                "E0120",
                                identifier.span,
                                format!("cannot assign to the constant `{}`", identifier.name),
                            ));
                        }
                    }
                    AssignmentTarget::Index {
                        receiver, index, ..
                    } => {
                        self.expression(receiver);
                        self.expression(index);
                    }
                    AssignmentTarget::Property { receiver, .. } => self.expression(receiver),
                }
                self.expression(value);
            }
            Statement::Return { value, .. } => {
                if let Some(value) = value {
                    self.expression(value);
                }
            }
            Statement::Block { block, .. } => self.block(block),
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.expression(condition);
                self.block(then_block);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.block(block),
                    Some(ElseBranch::If(statement)) => self.statement(statement),
                    None => {}
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                self.expression(condition);
                self.block(body);
            }
            Statement::For {
                binding,
                iterable,
                body,
                ..
            } => {
                self.expression(iterable);
                self.scopes.push(HashSet::from([binding.name.clone()]));
                self.block(body);
                self.scopes.pop();
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
        }
    }

    /// Walks one expression, replacing constant references.
    fn expression(&mut self, expression: &mut Expression<'a>) {
        match expression {
            Expression::Variable(identifier) => {
                if self.is_local(&identifier.name) {
                    return;
                }
                if let Some(value) = self.visible.get(&identifier.name) {
                    *expression = with_span(value.clone(), identifier.span);
                } else if self.private.contains(&identifier.name) {
                    self.diagnostics.push(CompileDiagnostic::new(
                        "E0118",
                        identifier.span,
                        format!("`{}` is private to its library", identifier.name),
                    ));
                }
            }
            Expression::FormattedString { parts, .. } => {
                for part in parts {
                    if let FormattedStringPart::Expression(expression) = part {
                        self.expression(expression);
                    }
                }
            }
            Expression::Closure {
                parameters, body, ..
            } => self.function(parameters, body),
            Expression::Match { value, arms, .. } => {
                self.expression(value);
                for arm in arms {
                    let bindings = match &arm.pattern {
                        MatchPattern::Variant { bindings, .. } => bindings
                            .iter()
                            .map(|binding| binding.name.clone())
                            .collect(),
                        _ => HashSet::new(),
                    };
                    self.scopes.push(bindings);
                    match &mut arm.body {
                        MatchArmBody::Expression(value) => self.expression(value),
                        MatchArmBody::Block(block) => self.block(block),
                    }
                    self.scopes.pop();
                }
            }
            Expression::IsType { value, .. }
            | Expression::Propagate { value, .. }
            | Expression::Unary { operand: value, .. }
            | Expression::ParallelDynamic {
                functions: value, ..
            }
            | Expression::Property {
                receiver: value, ..
            } => self.expression(value),
            Expression::Binary { left, right, .. }
            | Expression::Index {
                receiver: left,
                index: right,
                ..
            } => {
                self.expression(left);
                self.expression(right);
            }
            Expression::List { elements, .. }
            | Expression::ParallelStatic {
                tasks: elements, ..
            }
            | Expression::Call {
                arguments: elements,
                ..
            }
            | Expression::HostStream {
                arguments: elements,
                ..
            }
            | Expression::HostTime {
                arguments: elements,
                ..
            }
            | Expression::StaticMethodCall {
                arguments: elements,
                ..
            } => {
                for element in elements {
                    self.expression(element);
                }
            }
            Expression::HostCall {
                name, arguments, ..
            } => {
                self.expression(name);
                for argument in arguments {
                    self.expression(argument);
                }
            }
            Expression::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.expression(receiver);
                for argument in arguments {
                    self.expression(argument);
                }
            }
            Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
                for property in properties {
                    self.expression(&mut property.value);
                }
            }
            Expression::Integer(..)
            | Expression::Float(..)
            | Expression::String(..)
            | Expression::Bytes(..)
            | Expression::Bool(..)
            | Expression::None(..)
            | Expression::Unit(..) => {}
        }
    }
}
