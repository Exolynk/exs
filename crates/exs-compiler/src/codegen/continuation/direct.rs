//! Direct statements: `while`, `for` and `if` statements of a resumable function that cannot
//! suspend, which the step function runs as synchronous code instead of one state per
//! operation.
//!
//! Whether a statement can suspend is decided by lowering it as usual and inspecting the
//! operations that produces, so the decision matches what the graph would do. A direct
//! statement keeps its values in Wasm locals and only reads and writes the frame slots of the
//! bindings it uses before and after it runs.
//!
//! In modules with `par` expressions, loops yield at scheduler checkpoints. A direct `while`
//! loop checks the scheduler before every iteration and, when it yields, stores its assigned
//! bindings and resumes at its own state, which restarts it at its condition. Only innermost
//! `while` loops can restart that way, so `for` loops and statements holding other loops stay
//! in the graph there.

use std::collections::{BTreeSet, HashSet};

use crate::ast::{AssignmentTarget, Block, ElseBranch, Expression, MatchArmBody, Statement};
use crate::codegen::CompileDiagnostics;
use crate::codegen::reachability::visit_expression_tree;

use super::graph::{DirectBinding, GraphBuilder, Operation};
use crate::codegen::trait_registry::TraitOperator;

/// What a statement contains, as far as running it directly depends on it.
#[derive(Default)]
struct StatementFacts {
    /// Every name the statement reads or assigns, in a stable order.
    names: BTreeSet<String>,
    /// The names the statement assigns.
    assigned: HashSet<String>,
    /// Whether a `break` or `continue` leaves the statement.
    escapes: bool,
    /// Whether the statement holds a loop besides its own.
    nested_loops: bool,
    /// Whether the statement holds a closure, a host operation or a `par` expression.
    excluded: bool,
    /// Match arm blocks already walked, by address.
    arms: HashSet<usize>,
}

impl StatementFacts {
    /// Collects the facts of the statement a direct operation would run.
    fn of(statement: &Statement<'_>) -> Self {
        let mut facts = Self::default();
        match statement {
            Statement::While {
                condition, body, ..
            } => {
                facts.expression(condition, 0);
                facts.block(body, 1);
            }
            Statement::For { iterable, body, .. } => {
                facts.expression(iterable, 0);
                facts.block(body, 1);
            }
            statement => facts.statement(statement, 0),
        }
        facts
    }

    /// Walks the statements of `block` inside `loops` enclosing loops of the statement.
    fn block(&mut self, block: &Block<'_>, loops: u32) {
        for statement in &block.statements {
            self.statement(statement, loops);
        }
    }

    /// Walks one statement inside `loops` enclosing loops of the statement.
    fn statement(&mut self, statement: &Statement<'_>, loops: u32) {
        match statement {
            Statement::Let { value, .. }
            | Statement::Expression {
                expression: value, ..
            } => self.expression(value, loops),
            Statement::Assign { target, value, .. } => {
                match target {
                    AssignmentTarget::Variable(name) => {
                        self.names.insert(name.name.clone());
                        self.assigned.insert(name.name.clone());
                    }
                    AssignmentTarget::Index {
                        receiver, index, ..
                    } => {
                        self.expression(receiver, loops);
                        self.expression(index, loops);
                    }
                    AssignmentTarget::Property { receiver, .. } => {
                        self.expression(receiver, loops);
                    }
                }
                self.expression(value, loops);
            }
            Statement::Return { value, .. } => {
                if let Some(value) = value {
                    self.expression(value, loops);
                }
            }
            Statement::Block { block, .. } => self.block(block, loops),
            Statement::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.expression(condition, loops);
                self.block(then_block, loops);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.block(block, loops),
                    Some(ElseBranch::If(statement)) => self.statement(statement, loops),
                    None => {}
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                self.nested_loops = true;
                self.expression(condition, loops);
                self.block(body, loops + 1);
            }
            Statement::For { iterable, body, .. } => {
                self.nested_loops = true;
                self.expression(iterable, loops);
                self.block(body, loops + 1);
            }
            Statement::Break { .. } | Statement::Continue { .. } => {
                if loops == 0 {
                    self.escapes = true;
                }
            }
        }
    }

    /// Walks one expression inside `loops` enclosing loops of the statement, including the
    /// statements of its match arm blocks.
    ///
    /// Arms nested in loops of other arm blocks count `loops` too, which can only report an
    /// escaping branch too many and keep a statement in the graph.
    fn expression(&mut self, expression: &Expression<'_>, loops: u32) {
        let mut arms: Vec<&Block<'_>> = Vec::new();
        visit_expression_tree(expression, &mut |expression| match expression {
            Expression::Variable(identifier) => {
                self.names.insert(identifier.name.clone());
            }
            Expression::Match { arms: cases, .. } => {
                arms.extend(cases.iter().filter_map(|arm| match &arm.body {
                    MatchArmBody::Block(block) => Some(block),
                    MatchArmBody::Expression(_) => None,
                }));
            }
            Expression::Closure { .. }
            | Expression::HostCall { .. }
            | Expression::HostStream { .. }
            | Expression::HostTime { .. }
            | Expression::ParallelStatic { .. }
            | Expression::ParallelDynamic { .. } => self.excluded = true,
            _ => {}
        });
        for block in arms {
            if self.arms.insert(core::ptr::from_ref(block) as usize) {
                self.block(block, loops);
            }
        }
    }
}

impl<'source, 'function> GraphBuilder<'source, 'function> {
    /// Lowers `statement`, a `while`, `for` or `if` statement, as one direct operation when
    /// it cannot suspend, and as usual otherwise.
    pub(super) fn lower_compound(
        &mut self,
        statement: &'function Statement<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        let facts = StatementFacts::of(statement);
        let candidate = !facts.escapes && !facts.excluded;
        let checkpoint = self.parallel && matches!(statement, Statement::While { .. });
        let restartable =
            !self.parallel || (!facts.nested_loops && !matches!(statement, Statement::For { .. }));
        let first = self.operations.len();
        let slots = self.next_slot;
        self.lower_compound_states(statement)?;
        if !candidate
            || !restartable
            || self.operations[first..]
                .iter()
                .any(|operation| self.may_suspend(operation))
        {
            return Ok(());
        }
        let span = statement_span(statement);
        self.operations.truncate(first);
        self.next_slot = slots;
        let bindings = facts
            .names
            .iter()
            .filter_map(|name| {
                self.lookup_optional(name).map(|binding| DirectBinding {
                    name: name.clone(),
                    slot: binding.slot,
                    assigned: !binding.cell && facts.assigned.contains(name),
                })
            })
            .collect();
        self.operations.push(Operation::Direct {
            statement,
            bindings,
            checkpoint,
            span,
        });
        Ok(())
    }

    /// Lowers one `while`, `for` or `if` statement into graph states.
    fn lower_compound_states(
        &mut self,
        statement: &'function Statement<'source>,
    ) -> Result<(), CompileDiagnostics<'source>> {
        match statement {
            Statement::If {
                condition,
                then_block,
                else_branch,
                span,
            } => self.lower_if(condition, then_block, else_branch.as_ref(), *span),
            Statement::While {
                condition,
                body,
                span,
            } => self.lower_while(condition, body, *span),
            Statement::For {
                binding,
                type_annotation,
                iterable,
                body,
                span,
            } => self.lower_for(binding, type_annotation.as_ref(), iterable, body, *span),
            statement => Err(crate::codegen::diagnostics(
                crate::codegen::CompileDiagnostic::new(
                    "E0999",
                    statement_span(statement),
                    "only while, for and if statements run directly",
                ),
            )),
        }
    }

    /// Returns whether `operation` may suspend its frame or calls a unit that runs as a frame,
    /// which synchronous code cannot call.
    fn may_suspend(&self, operation: &Operation<'_, '_>) -> bool {
        let frames = |targets: &[crate::codegen::declarations::InstanceMethod]| {
            targets
                .iter()
                .any(|target| self.frame_functions.contains(&target.signature.function_id))
        };
        match operation {
            Operation::HostCall { .. }
            | Operation::HostResume { .. }
            | Operation::HostStream { .. }
            | Operation::HostTime { .. }
            | Operation::ChildCall { .. }
            | Operation::ClosureCall { .. }
            | Operation::Closure { .. }
            | Operation::ParallelStart { .. }
            | Operation::ParallelDynamicStart { .. }
            | Operation::ParallelTake { .. } => true,
            Operation::DirectCall { signature, .. } => {
                self.frame_functions.contains(&signature.function_id)
            }
            Operation::InstanceCall { targets, .. } => frames(targets),
            Operation::Operator {
                operator, targets, ..
            } => {
                frames(targets)
                    || (matches!(operator, TraitOperator::Equal | TraitOperator::NotEqual)
                        && self
                            .frame_layouts
                            .contains_key(crate::prelude::EQUALITY_HELPER))
            }
            _ => false,
        }
    }
}

/// Returns the span of one statement.
fn statement_span<'source>(statement: &Statement<'source>) -> crate::codegen::SourceSpan<'source> {
    match statement {
        Statement::Let { span, .. }
        | Statement::Assign { span, .. }
        | Statement::Return { span, .. }
        | Statement::Expression { span, .. }
        | Statement::Block { span, .. }
        | Statement::If { span, .. }
        | Statement::While { span, .. }
        | Statement::For { span, .. }
        | Statement::Break { span }
        | Statement::Continue { span } => *span,
    }
}
