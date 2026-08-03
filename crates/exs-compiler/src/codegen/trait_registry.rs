//! Normalized trait declarations shared by validation, contracts, and operator bindings.

use std::collections::HashMap;

use crate::ast::{FunctionDeclaration, Module, TraitMethodDeclaration, TypeAnnotation};
use crate::codegen::standard::{self, StandardOperator};
use crate::diagnostic::SourceSpan;

/// One source operator whose dispatch is provided by a trait method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraitOperator {
    /// The binary `+` operator.
    Add,
}

/// One trait declaration normalized independently of its source representation.
#[derive(Clone, Debug)]
pub(crate) struct TraitDefinition<'a> {
    /// Canonical source-visible trait name.
    pub(crate) name: String,
    /// Built-in runtime categories satisfying the trait contract.
    pub(crate) builtin_mask: u32,
    /// Required and default methods in declaration order.
    pub(crate) methods: Vec<TraitMethodDefinition<'a>>,
    /// Optional source operator binding selected by one trait method.
    pub(crate) operator: Option<TraitOperator>,
}

/// One normalized trait method declaration.
#[derive(Clone, Debug)]
pub(crate) struct TraitMethodDefinition<'a> {
    /// Source-visible method name.
    pub(crate) name: String,
    /// Required receiver, parameter, and result contract.
    pub(crate) signature: TraitMethodSignature,
    /// Source default body inherited by missing implementations.
    pub(crate) default_implementation: Option<FunctionDeclaration<'a>>,
    /// Source declaration span used for related diagnostics when available.
    pub(crate) declaration_span: Option<SourceSpan<'a>>,
    /// Optional explicit signature text for compiler-owned diagnostic messages.
    pub(crate) display_signature: Option<String>,
}

/// The annotation-level contract for one trait method.
#[derive(Clone, Debug)]
pub(crate) struct TraitMethodSignature {
    /// Whether the first parameter must be the bare `self` receiver.
    pub(crate) has_receiver: bool,
    /// Ordered parameter annotation members, including the receiver when present.
    parameter_types: Vec<Option<Vec<String>>>,
    /// Optional ordered result annotation members.
    return_types: Option<Vec<String>>,
}

/// All trait definitions visible to one source module.
#[derive(Clone, Debug)]
pub(crate) struct TraitRegistry<'a> {
    definitions: HashMap<String, TraitDefinition<'a>>,
}

impl<'a> TraitRegistry<'a> {
    /// Lowers compiler-owned and source-declared traits into one normalized registry.
    #[must_use]
    pub(crate) fn build(module: &Module<'a>) -> Self {
        let mut definitions = standard::traits()
            .iter()
            .map(|descriptor| {
                (
                    descriptor.name.to_owned(),
                    TraitDefinition {
                        name: descriptor.name.to_owned(),
                        builtin_mask: descriptor.builtin_mask,
                        methods: descriptor
                            .methods
                            .iter()
                            .map(|method| TraitMethodDefinition {
                                name: method.name.to_owned(),
                                signature: TraitMethodSignature {
                                    has_receiver: method.has_receiver,
                                    parameter_types: method
                                        .parameter_types
                                        .iter()
                                        .map(|annotation| {
                                            annotation.map(|name| vec![name.to_owned()])
                                        })
                                        .collect(),
                                    return_types: Some(
                                        method
                                            .return_types
                                            .iter()
                                            .map(|name| (*name).to_owned())
                                            .collect(),
                                    ),
                                },
                                default_implementation: None,
                                declaration_span: None,
                                display_signature: Some(method.signature.to_owned()),
                            })
                            .collect(),
                        operator: descriptor.operator.map(TraitOperator::from),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        for declaration in &module.traits {
            if standard::canonical_trait_name(&declaration.name.name).is_some() {
                continue;
            }
            definitions.insert(
                declaration.name.name.clone(),
                TraitDefinition {
                    name: declaration.name.name.clone(),
                    builtin_mask: 0,
                    methods: declaration
                        .methods
                        .iter()
                        .map(TraitMethodDefinition::from_source)
                        .collect(),
                    operator: None,
                },
            );
        }
        Self { definitions }
    }

    /// Resolves one source spelling to a normalized trait definition.
    pub(crate) fn definition(&self, name: &str) -> Option<&TraitDefinition<'a>> {
        let canonical = standard::canonical_trait_name(name).unwrap_or(name);
        self.definitions.get(canonical)
    }

    /// Iterates over every normalized trait definition.
    pub(crate) fn definitions(&self) -> impl Iterator<Item = &TraitDefinition<'a>> {
        self.definitions.values()
    }

    /// Returns whether one trait method supplies the given source operator binding.
    pub(crate) fn binds_operator(
        &self,
        trait_name: &str,
        method_name: &str,
        operator: TraitOperator,
    ) -> bool {
        self.definition(trait_name).is_some_and(|definition| {
            definition.operator == Some(operator)
                && definition
                    .methods
                    .iter()
                    .any(|method| method.name == method_name)
        })
    }
}

impl<'a> TraitMethodDefinition<'a> {
    /// Lowers one source trait method into the normalized method representation.
    fn from_source(method: &TraitMethodDeclaration<'a>) -> Self {
        Self {
            name: method.name.name.clone(),
            signature: TraitMethodSignature {
                has_receiver: method
                    .parameters
                    .first()
                    .is_some_and(|parameter| parameter.name.name == "self"),
                parameter_types: method
                    .parameters
                    .iter()
                    .map(|parameter| annotation_members(parameter.type_annotation.as_ref()))
                    .collect(),
                return_types: annotation_members(method.return_type.as_ref()),
            },
            default_implementation: method.default_implementation(),
            declaration_span: Some(method.name.span),
            display_signature: None,
        }
    }
}

impl TraitMethodSignature {
    /// Returns whether one implementation function satisfies this normalized signature.
    pub(crate) fn matches(
        &self,
        implementation: &FunctionDeclaration<'_>,
        self_type: &str,
    ) -> bool {
        self.has_receiver
            == implementation
                .parameters
                .first()
                .is_some_and(|parameter| parameter.name.name == "self")
            && self.parameter_types.len() == implementation.parameters.len()
            && self
                .parameter_types
                .iter()
                .zip(&implementation.parameters)
                .all(|(required, supplied)| {
                    annotations_match(
                        required.as_deref(),
                        annotation_members(supplied.type_annotation.as_ref()).as_deref(),
                        self_type,
                    )
                })
            && annotations_match(
                self.return_types.as_deref(),
                annotation_members(implementation.return_type.as_ref()).as_deref(),
                self_type,
            )
    }
}

impl From<StandardOperator> for TraitOperator {
    /// Converts one compiler-owned operator declaration into its registry binding.
    fn from(operator: StandardOperator) -> Self {
        match operator {
            StandardOperator::Add => Self::Add,
        }
    }
}

/// Collects the ordered type-member names in one optional source annotation.
fn annotation_members(annotation: Option<&TypeAnnotation<'_>>) -> Option<Vec<String>> {
    annotation.map(|annotation| {
        annotation
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect()
    })
}

/// Returns whether two optional annotation member lists match for one implementation target.
fn annotations_match(
    required: Option<&[String]>,
    supplied: Option<&[String]>,
    self_type: &str,
) -> bool {
    match (required, supplied) {
        (None, None) => true,
        (Some(required), Some(supplied)) => {
            required.len() == supplied.len()
                && required.iter().zip(supplied).all(|(required, supplied)| {
                    annotation_name(required, self_type) == annotation_name(supplied, self_type)
                })
        }
        _ => false,
    }
}

/// Resolves contextual `Self` and standard-library qualification for signature comparison.
fn annotation_name<'a>(name: &'a str, self_type: &'a str) -> &'a str {
    if name == "Self" {
        self_type
    } else {
        name.strip_prefix("std::").unwrap_or(name)
    }
}

#[cfg(test)]
mod tests {
    use super::{TraitOperator, TraitRegistry};
    use crate::SourceInput;
    use crate::ast::Module;

    /// Parses one source fixture for direct trait-registry inspection.
    fn parse_module(source: &str) -> Module<'_> {
        let lexed = crate::lexer::lex(SourceInput {
            source_id: "trait-registry-test.exs",
            text: source,
        });
        assert!(lexed.diagnostics.is_empty());
        match crate::parser::parse("trait-registry-test.exs", lexed.tokens, true) {
            Ok(module) => module,
            Err(diagnostics) => panic!("source did not parse: {diagnostics}"),
        }
    }

    /// Normalizes standard and source traits before method-signature validation.
    #[test]
    fn shares_method_signatures_and_operator_bindings() {
        let module = parse_module(
            "type Value {} trait Combine { fn combine(self, other: Any) -> Any; } impl std::Add for Value { fn add(self, value: Any) -> Any { ret self; } } impl Combine for Value { fn combine(self, value: Any) -> Any { ret self; } } fn main() { ret Value {}; }",
        );
        let registry = TraitRegistry::build(&module);
        let add = match registry.definition("std::Add") {
            Some(definition) => definition,
            None => panic!("missing normalized Add definition"),
        };
        let combine = match registry.definition("Combine") {
            Some(definition) => definition,
            None => panic!("missing normalized Combine definition"),
        };
        let add_implementation = &module.implementations[0].methods[0];
        let combine_implementation = &module.implementations[1].methods[0];

        assert!(
            add.methods[0]
                .signature
                .matches(add_implementation, "Value")
        );
        assert!(
            combine.methods[0]
                .signature
                .matches(combine_implementation, "Value")
        );
        assert!(registry.binds_operator("std::Add", "add", TraitOperator::Add));
        assert!(!registry.binds_operator("Combine", "combine", TraitOperator::Add));
    }
}
