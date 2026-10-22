//! Arm validation shared by the synchronous and the continuation `match` lowering.
//!
//! Both lowerings validate each arm right before they lower it, in source order, so they report
//! the same first diagnostic for the same source.

use std::collections::HashSet;

use crate::ast::{Identifier, MatchArm, MatchPattern};
use crate::codegen::diagnostics;
use crate::codegen::types::TypeRegistry;
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};

/// What one validated arm tests before its body runs.
pub(super) enum ArmTest {
    /// The wildcard fallback, which always matches.
    Wildcard,
    /// A literal, which matches values equal to it.
    Literal,
    /// One enum variant, whose payload fields bind in declaration order.
    Variant {
        /// Stable host-boundary identity of the enum type.
        type_identity: String,
        /// Source-visible variant name.
        variant: String,
    },
}

/// Validation state of one `match` expression while its arms are lowered.
pub(super) struct MatchArms<'registry> {
    /// Registry that resolves enum variants.
    types: &'registry TypeRegistry,
    /// Fully qualified variant keys already covered by an arm.
    matched: HashSet<String>,
    /// Type identity shared by every variant arm, once one was seen.
    enum_identity: Option<String>,
    /// Source spelling of the enum type of the first variant arm.
    enum_type_name: Option<String>,
    /// Whether a wildcard arm was seen.
    fallback: bool,
    /// Source spellings of the literals already covered by an arm.
    literals: HashSet<String>,
}

impl<'registry> MatchArms<'registry> {
    /// Starts validating the arms of one `match` expression.
    pub(super) fn new(types: &'registry TypeRegistry) -> Self {
        Self {
            types,
            matched: HashSet::new(),
            enum_identity: None,
            enum_type_name: None,
            fallback: false,
            literals: HashSet::new(),
        }
    }

    /// Validates the pattern of the next arm and returns what it tests.
    pub(super) fn arm<'a>(
        &mut self,
        arm: &MatchArm<'a>,
    ) -> Result<ArmTest, CompileDiagnostics<'a>> {
        if self.fallback {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0231",
                arm.span,
                "match arm follows wildcard fallback",
            )));
        }
        let (type_name, variant, bindings, pattern_span) = match &arm.pattern {
            MatchPattern::Wildcard(_) => {
                self.fallback = true;
                return Ok(ArmTest::Wildcard);
            }
            MatchPattern::Literal(literal) => {
                let span = crate::parser::expression_span(literal);
                if self.enum_identity.is_some() {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0231",
                        span,
                        "match arms must all be enum variants or all literals",
                    )));
                }
                if !self
                    .literals
                    .insert(crate::formatter::render_expression(literal))
                {
                    return Err(diagnostics(CompileDiagnostic::new(
                        "E0231",
                        span,
                        "duplicate match literal arm",
                    )));
                }
                return Ok(ArmTest::Literal);
            }
            MatchPattern::Variant {
                type_name,
                variant,
                bindings,
                span,
            } => (type_name, variant, bindings, *span),
        };
        let key = format!("{}::{}", type_name.name, variant.name);
        let variant_data = self.types.enum_variant(&key).ok_or_else(|| {
            diagnostics(CompileDiagnostic::new(
                "E0216",
                pattern_span,
                crate::diagnostic::unknown_name("enum variant", &key),
            ))
        })?;
        if bindings.len() != variant_data.fields.len() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0208",
                pattern_span,
                format!(
                    "enum pattern `{key}` expects {} bindings",
                    variant_data.fields.len()
                ),
            )));
        }
        if !self.literals.is_empty() {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0231",
                pattern_span,
                "match arms must all be enum variants or all literals",
            )));
        }
        let type_identity = variant_data.type_identity.clone();
        let variant = variant_data.name.clone();
        if !self.matched.insert(key) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0231",
                pattern_span,
                "duplicate match variant arm",
            )));
        }
        if let Some(identity) = &self.enum_identity {
            if identity != &type_identity {
                return Err(diagnostics(CompileDiagnostic::new(
                    "E0231",
                    pattern_span,
                    "match arms must belong to one enum",
                )));
            }
        } else {
            self.enum_identity = Some(type_identity.clone());
            self.enum_type_name = Some(type_name.name.clone());
        }
        Ok(ArmTest::Variant {
            type_identity,
            variant,
        })
    }

    /// Checks exhaustiveness after the last arm.
    ///
    /// Returns whether no wildcard covers unmatched values, which then raise a match error.
    pub(super) fn finish<'a>(self, span: SourceSpan<'a>) -> Result<bool, CompileDiagnostics<'a>> {
        if self.fallback {
            return Ok(false);
        }
        let Some(type_name) = self.enum_type_name else {
            // Values no literal arm matches produce the MatchError.
            if !self.literals.is_empty() {
                return Ok(true);
            }
            return Err(diagnostics(CompileDiagnostic::new(
                "E0232",
                span,
                "match requires at least one enum variant, literal, or wildcard arm",
            )));
        };
        let missing = self
            .types
            .enum_variant_names(&type_name)
            .into_iter()
            .find(|variant| !self.matched.contains(&format!("{type_name}::{variant}")));
        if let Some(variant) = missing {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0232",
                span,
                format!("non-exhaustive match: missing `{type_name}::{variant}`"),
            )));
        }
        Ok(true)
    }
}

/// Rejects an arm that binds the same payload name twice.
pub(super) fn check_bindings<'a>(
    bindings: &[Identifier<'a>],
) -> Result<(), CompileDiagnostics<'a>> {
    let mut declared = HashSet::new();
    for binding in bindings {
        if !declared.insert(&binding.name) {
            return Err(diagnostics(CompileDiagnostic::new(
                "E0204",
                binding.span,
                format!("duplicate match binding `{}`", binding.name),
            )));
        }
    }
    Ok(())
}
