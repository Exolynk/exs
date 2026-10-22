//! The nominal types whose names the static type table holds.
//!
//! Every name in the table is a static String and adds to a program's initialized data, which
//! costs fuel on every execution. The table names the types reached code uses, together with
//! the types their fields and payloads contain: values of other types cannot exist, and an
//! input enum of a type no code names is a host enum, which behaves the same.

use std::collections::HashSet;

use exs_abi::STANDARD_ORDERING_TYPE_ID;

use super::units::Unit;
use crate::ast::{
    Block, ElseBranch, Expression, MatchArmBody, MatchPattern, Module, Statement, TypeAnnotation,
};
use crate::codegen::types::{TypeContract, TypeRegistry};

/// Prefix of the source ids of the prelude.
const PRELUDE_SOURCE: &str = "<std>/";

/// Returns the ids of the types the type table names for the `reached` units.
pub(super) fn used_types(types: &TypeRegistry, reached: &[&Unit<'_, '_>]) -> HashSet<u32> {
    let mut used = HashSet::from([STANDARD_ORDERING_TYPE_ID]);
    let mut collector = Collector {
        types,
        used: &mut used,
    };
    for unit in reached {
        if let Some((type_name, _)) = unit.key.split_once("::")
            && !unit.key.starts_with('$')
        {
            collector.name(type_name);
        }
        for parameter in &unit.declaration.parameters {
            collector.annotation(parameter.type_annotation.as_ref());
        }
        collector.annotation(unit.declaration.return_type.as_ref());
        collector.block(&unit.declaration.body);
    }
    contained_types(types, &mut used);
    used
}

/// Returns the ids of the types and enums the program declares, without the prelude's.
pub(super) fn declared_types(module: &Module<'_>, types: &TypeRegistry) -> HashSet<u32> {
    module
        .types
        .iter()
        .map(|declaration| &declaration.name)
        .chain(module.enums.iter().map(|declaration| &declaration.name))
        .filter(|name| !name.span.source_id.starts_with(PRELUDE_SOURCE))
        .filter_map(|name| types.get(&name.name).map(|nominal| nominal.id))
        .collect()
}

/// Adds the types the fields and payloads of the types in `used` contain, transitively.
fn contained_types(types: &TypeRegistry, used: &mut HashSet<u32>) {
    let mut pending: Vec<u32> = used.iter().copied().collect();
    while let Some(id) = pending.pop() {
        let mut contracts: Vec<&TypeContract> = types
            .nominal_types()
            .filter(|nominal| nominal.id == id)
            .flat_map(|nominal| nominal.fields.iter().map(|field| &field.contract))
            .collect();
        contracts.extend(
            types
                .enum_variants()
                .filter(|variant| variant.type_id == id)
                .flat_map(|variant| variant.fields.iter()),
        );
        let mut found = Vec::new();
        for contract in contracts {
            contract_types(types, contract, &mut found);
        }
        for id in found {
            if used.insert(id) {
                pending.push(id);
            }
        }
    }
}

/// Appends the nominal and enum types `contract` names, including List element types.
fn contract_types(types: &TypeRegistry, contract: &TypeContract, found: &mut Vec<u32>) {
    found.extend(&contract.nominal_type_ids);
    for identity in &contract.enum_type_ids {
        found.extend(
            types
                .nominal_types()
                .filter(|nominal| nominal.enum_type_id.as_deref() == Some(identity.as_str()))
                .map(|nominal| nominal.id),
        );
    }
    if let Some(item) = &contract.list_item {
        contract_types(types, item, found);
    }
}

/// Collects the types source code refers to.
struct Collector<'r> {
    /// The resolved types.
    types: &'r TypeRegistry,
    /// The ids found so far.
    used: &'r mut HashSet<u32>,
}

impl Collector<'_> {
    /// Adds the type named `name`, if it is one.
    fn name(&mut self, name: &str) {
        if let Some(nominal) = self.types.get(name) {
            self.used.insert(nominal.id);
        }
    }

    /// Adds the enum of the variant named `name`, if it is one.
    fn variant(&mut self, name: &str) {
        if let Some(variant) = self.types.enum_variant(name) {
            self.used.insert(variant.type_id);
        }
    }

    /// Adds the types an annotation names.
    fn annotation(&mut self, annotation: Option<&TypeAnnotation<'_>>) {
        let Some(annotation) = annotation else {
            return;
        };
        if let Ok(contract) = self.types.resolve(Some(annotation), annotation.span) {
            let mut found = Vec::new();
            contract_types(self.types, &contract, &mut found);
            self.used.extend(found);
        }
    }

    /// Visits every statement of `block`.
    fn block(&mut self, block: &Block<'_>) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    /// Visits one statement, its annotations and its expressions.
    fn statement(&mut self, statement: &Statement<'_>) {
        match statement {
            Statement::Let {
                type_annotation,
                value,
                ..
            } => {
                self.annotation(type_annotation.as_ref());
                self.expression(value);
            }
            Statement::Assign { value, .. }
            | Statement::Expression {
                expression: value, ..
            } => self.expression(value),
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
                type_annotation,
                iterable,
                body,
                ..
            } => {
                self.annotation(type_annotation.as_ref());
                self.expression(iterable);
                self.block(body);
            }
            Statement::Break { .. } | Statement::Continue { .. } => {}
        }
    }

    /// Visits one expression and the statements and expressions inside it.
    fn expression(&mut self, expression: &Expression<'_>) {
        super::reachability::visit_expression_tree(
            expression,
            &mut |expression| match expression {
                Expression::TypedObject { type_name, .. } => self.name(&type_name.name),
                Expression::StaticMethodCall {
                    type_name, method, ..
                } => {
                    self.name(&type_name.name);
                    self.variant(&format!("{}::{}", type_name.name, method.name));
                }
                Expression::Variable(identifier) => self.variant(&identifier.name),
                Expression::Call { callee, .. } => self.variant(&callee.name),
                Expression::HostStream { .. } => self.name("HostStream"),
                Expression::HostTime { operation, .. } => {
                    self.name(super::reachability::host_time_target(*operation).1);
                }
                Expression::IsType {
                    type_annotation, ..
                } => self.annotation(Some(type_annotation)),
                Expression::Match { arms, .. } => {
                    for arm in arms {
                        if let MatchPattern::Variant { type_name, .. } = &arm.pattern {
                            self.name(&type_name.name);
                        }
                        if let MatchArmBody::Block(block) = &arm.body {
                            self.block(block);
                        }
                    }
                }
                _ => {}
            },
        );
    }
}
