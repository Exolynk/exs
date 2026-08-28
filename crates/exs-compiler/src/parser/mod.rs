//! Recursive-descent parser for the Phase-1 `ExS` grammar.

use std::collections::HashSet;

use crate::ast::{
    AssignmentTarget, BinaryOperator, Block, ElseBranch, EnumDeclaration, EnumVariant, Expression,
    FormattedStringKind, FormattedStringPart, FunctionDeclaration, FunctionVisibility, Identifier,
    ImplDeclaration, ImportDeclaration, MatchArm, MatchArmBody, MatchPattern, Module,
    ObjectProperty, Parameter, Statement, TestDeclaration, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration, TypeField, TypeName, UnaryOperator,
    UseDeclaration, UseItem,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::lexer::{
    FormattedStringKind as LexerFormattedStringKind,
    FormattedStringPart as LexerFormattedStringPart, Token, TokenKind,
};

mod declarations;
mod expressions;
mod statements;
mod utilities;

/// Parses a token stream into one ExS module.
pub fn parse<'a>(
    source_id: &'a str,
    tokens: Vec<Token<'a>>,
    require_main: bool,
) -> Result<Module<'a>, CompileDiagnostics<'a>> {
    let mut type_names: HashSet<_> = tokens
        .windows(2)
        .filter_map(|pair| match (&pair[0].kind, &pair[1].kind) {
            (TokenKind::Type, TokenKind::Identifier(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    type_names.extend(crate::prelude::type_names().map(str::to_owned));
    let mut parser = Parser {
        tokens,
        current: 0,
        type_names,
        diagnostics: CompileDiagnostics::new(),
    };
    let mut imports = Vec::new();
    let mut uses = Vec::new();
    let mut types = Vec::new();
    let mut enums = Vec::new();
    let mut traits = Vec::new();
    let mut implementations = Vec::new();
    let mut functions = Vec::new();
    let mut tests = Vec::new();
    while !parser.at_end() {
        match &parser.peek().kind {
            TokenKind::Import => match parser.import_declaration() {
                Ok(declaration)
                    if types.is_empty()
                        && traits.is_empty()
                        && enums.is_empty()
                        && implementations.is_empty()
                        && functions.is_empty()
                        && tests.is_empty() =>
                {
                    imports.push(declaration)
                }
                Ok(declaration) => parser.diagnostics.push(parser.error(
                    declaration.span,
                    "E0112",
                    "imports must precede module declarations",
                )),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Use => match parser.use_declaration() {
                Ok(declaration)
                    if types.is_empty()
                        && traits.is_empty()
                        && enums.is_empty()
                        && implementations.is_empty()
                        && functions.is_empty()
                        && tests.is_empty() =>
                {
                    uses.push(declaration)
                }
                Ok(declaration) => parser.diagnostics.push(parser.error(
                    declaration.span,
                    "E0112",
                    "use declarations must precede module declarations",
                )),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Type => match parser.type_declaration() {
                Ok(declaration) => types.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Enum => match parser.enum_declaration() {
                Ok(declaration) => enums.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Trait => match parser.trait_declaration() {
                Ok(declaration) => traits.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Impl => match parser.implementation() {
                Ok(declaration) => implementations.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Fn => match parser.function(FunctionVisibility::Public) {
                Ok(declaration) => functions.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Identifier(name) if name == "test" => match parser.test_declaration() {
                Ok(declaration) => tests.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            _ => {
                parser.diagnostics.push(parser.error(
                    parser.peek().span,
                    "E0100",
                    "expected `type`, `enum`, `trait`, `impl`, `fn`, or `test` at module level",
                ));
                parser.synchronize_declaration();
            }
        }
    }
    if require_main && functions.is_empty() {
        parser.diagnostics.push(CompileDiagnostic::new(
            "E0200",
            SourceSpan::empty(source_id),
            "a module must declare fn main()",
        ));
    }
    if !parser.diagnostics.is_empty() {
        return Err(parser.diagnostics);
    }
    Ok(Module {
        imports,
        uses,
        types,
        enums,
        traits,
        implementations,
        functions,
        tests,
    })
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    current: usize,
    type_names: HashSet<String>,
    diagnostics: CompileDiagnostics<'a>,
}

fn expression_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
        | Expression::Bytes(_, span)
        | Expression::Bool(_, span)
        | Expression::None(span) => *span,
        Expression::FormattedString { span, .. } => *span,
        Expression::List { span, .. } => *span,
        Expression::Object { span, .. } => *span,
        Expression::TypedObject { span, .. } => *span,
        Expression::Match { span, .. } => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Closure { span, .. } => *span,
        Expression::Unary { span, .. }
        | Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::HostCall { span, .. }
        | Expression::HostStream { span, .. }
        | Expression::HostTime { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
        Expression::ParallelStatic { span, .. } | Expression::ParallelDynamic { span, .. } => *span,
    }
}

/// Returns the full span of one parsed match pattern.
fn match_pattern_span<'a>(pattern: &MatchPattern<'a>) -> SourceSpan<'a> {
    match pattern {
        MatchPattern::Variant { span, .. } | MatchPattern::Wildcard(span) => *span,
    }
}
