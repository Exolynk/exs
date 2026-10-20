//! Detection of `for` loops over Range variables that no source code can observe while they run.
//!
//! A binding initialized with a literal `a..b` or `a..=b` holds a fresh Range object that is
//! reachable only through that binding. When every other use of the binding is a plain
//! property read outside the loops that iterate it, those loops may count in Wasm locals and
//! write `current` and `done` back once they exit, because nothing can read the fields in
//! between. Such loops also never suspend, so they do not make their function resumable.

use std::collections::HashMap;

use crate::ast::{
    AssignmentTarget, Block, ElseBranch, Expression, FormattedStringPart, FunctionDeclaration,
    MatchArmBody, MatchPattern, Statement,
};
use crate::codegen::standard::is_unshared_range;
use crate::diagnostic::SourceSpan;

/// Returns the spans of the `for` statements in `function` that iterate an unobserved Range.
///
/// The analysis is conservative: any use it does not recognize as harmless, including uses
/// inside closures, marks the Range binding as observed.
pub(crate) fn unobserved_range_loops<'a>(
    function: &FunctionDeclaration<'a>,
) -> Vec<SourceSpan<'a>> {
    let mut walker = Walker {
        scopes: vec![HashMap::new()],
        candidates: Vec::new(),
        active: Vec::new(),
    };
    for parameter in &function.parameters {
        walker.declare(&parameter.name.name, None);
    }
    walker.block(&function.body);
    walker
        .candidates
        .into_iter()
        .filter(|candidate| !candidate.observed)
        .flat_map(|candidate| candidate.loops)
        .collect()
}

/// One binding initialized with a literal Range.
struct Candidate<'a> {
    /// Whether any use could observe or alias the Range object.
    observed: bool,
    /// Spans of the `for` statements iterating this binding.
    loops: Vec<SourceSpan<'a>>,
}

/// Lexical walk that resolves names to Range candidates.
struct Walker<'a, 'ast> {
    /// Visible names, each mapped to its candidate index when it is a literal Range binding.
    scopes: Vec<HashMap<&'ast str, Option<usize>>>,
    /// Every literal Range binding found so far.
    candidates: Vec<Candidate<'a>>,
    /// Candidates whose loop body is currently being walked.
    active: Vec<usize>,
}

impl<'a, 'ast> Walker<'a, 'ast> {
    /// Declares one name in the innermost scope, shadowing any outer binding.
    fn declare(&mut self, name: &'ast str, candidate: Option<usize>) {
        if let Some(scope) = self.scopes.last_mut() {
            let _previous = scope.insert(name, candidate);
        }
    }

    /// Returns the candidate a name currently resolves to.
    fn resolve(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .flatten()
    }

    /// Marks the candidate a name resolves to as observed.
    fn observe(&mut self, name: &str) {
        if let Some(candidate) = self
            .resolve(name)
            .and_then(|index| self.candidates.get_mut(index))
        {
            candidate.observed = true;
        }
    }

    /// Walks one block in its own scope.
    fn block(&mut self, block: &'ast Block<'a>) {
        self.scopes.push(HashMap::new());
        self.statements(&block.statements);
        let _scope = self.scopes.pop();
    }

    /// Walks statements in the current scope.
    fn statements(&mut self, statements: &'ast [Statement<'a>]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    /// Walks one statement.
    fn statement(&mut self, statement: &'ast Statement<'a>) {
        match statement {
            Statement::Let { name, value, .. } => {
                self.expression(value);
                let candidate = is_unshared_range(value).then(|| {
                    self.candidates.push(Candidate {
                        observed: false,
                        loops: Vec::new(),
                    });
                    self.candidates.len() - 1
                });
                self.declare(&name.name, candidate);
            }
            Statement::Assign { target, value, .. } => {
                match target {
                    AssignmentTarget::Variable(identifier) => self.observe(&identifier.name),
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
                span,
                ..
            } => {
                let candidate = match iterable {
                    Expression::Variable(identifier) => self.resolve(&identifier.name),
                    _ => None,
                };
                match candidate {
                    // Iterating the Range inside one of its own loops would observe it.
                    Some(index) if !self.active.contains(&index) => {
                        if let Some(candidate) = self.candidates.get_mut(index) {
                            candidate.loops.push(*span);
                        }
                    }
                    _ => self.expression(iterable),
                }
                self.active.extend(candidate);
                self.scopes.push(HashMap::new());
                self.declare(&binding.name, None);
                self.statements(&body.statements);
                let _scope = self.scopes.pop();
                if candidate.is_some() {
                    let _candidate = self.active.pop();
                }
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
            Statement::Expression { expression, .. } => self.expression(expression),
        }
    }

    /// Walks one expression.
    fn expression(&mut self, expression: &'ast Expression<'a>) {
        match expression {
            Expression::Variable(identifier) => self.observe(&identifier.name),
            Expression::Property { receiver, .. } => match receiver.as_ref() {
                // Reading a field between loops sees the written-back state.
                Expression::Variable(identifier) => {
                    if let Some(index) = self.resolve(&identifier.name)
                        && self.active.contains(&index)
                    {
                        self.observe(&identifier.name);
                    }
                }
                receiver => self.expression(receiver),
            },
            Expression::FormattedString { parts, .. } => {
                for part in parts {
                    if let FormattedStringPart::Expression(expression) = part {
                        self.expression(expression);
                    }
                }
            }
            Expression::IsType { value, .. }
            | Expression::Propagate { value, .. }
            | Expression::Unary { operand: value, .. } => self.expression(value),
            Expression::Binary { left, right, .. } => {
                self.expression(left);
                self.expression(right);
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                self.observe(&callee.name);
                self.expressions(arguments);
            }
            Expression::HostCall {
                name, arguments, ..
            } => {
                self.expression(name);
                self.expressions(arguments);
            }
            Expression::HostStream { arguments, .. }
            | Expression::HostTime { arguments, .. }
            | Expression::StaticMethodCall { arguments, .. } => self.expressions(arguments),
            Expression::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.expression(receiver);
                self.expressions(arguments);
            }
            Expression::List { elements, .. } => self.expressions(elements),
            Expression::Object { properties, .. } | Expression::TypedObject { properties, .. } => {
                for property in properties {
                    self.expression(&property.value);
                }
            }
            Expression::Match { value, arms, .. } => {
                self.expression(value);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    if let MatchPattern::Variant { bindings, .. } = &arm.pattern {
                        for binding in bindings {
                            self.declare(&binding.name, None);
                        }
                    }
                    match &arm.body {
                        MatchArmBody::Expression(value) => self.expression(value),
                        MatchArmBody::Block(block) => self.block(block),
                    }
                    let _scope = self.scopes.pop();
                }
            }
            Expression::Closure {
                parameters, body, ..
            } => {
                // A closure may run at any time, so it observes every outer Range it mentions,
                // even by property, and Ranges declared inside it are never inlined.
                let first_inner = self.candidates.len();
                let active = std::mem::replace(&mut self.active, (0..first_inner).collect());
                self.scopes.push(HashMap::new());
                for parameter in parameters {
                    self.declare(&parameter.name.name, None);
                }
                self.block(body);
                let _scope = self.scopes.pop();
                self.active = active;
                for candidate in self.candidates.iter_mut().skip(first_inner) {
                    candidate.observed = true;
                }
            }
            Expression::ParallelStatic { tasks, .. } => self.expressions(tasks),
            Expression::ParallelDynamic { functions, .. } => self.expression(functions),
            Expression::Index {
                receiver, index, ..
            } => {
                self.expression(receiver);
                self.expression(index);
            }
            Expression::Integer(_, _)
            | Expression::Float(_, _)
            | Expression::String(_, _)
            | Expression::Bytes(_, _)
            | Expression::Bool(_, _)
            | Expression::None(_) => {}
        }
    }

    /// Walks a list of expressions.
    fn expressions(&mut self, expressions: &'ast [Expression<'a>]) {
        for expression in expressions {
            self.expression(expression);
        }
    }
}
