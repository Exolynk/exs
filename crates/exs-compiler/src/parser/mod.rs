//! Recursive-descent parser for the `ExS` grammar.

use std::collections::HashSet;

use crate::ast::{
    AssignmentTarget, BinaryOperator, Block, ConstDeclaration, ElseBranch, EnumDeclaration,
    EnumVariant, Expression, FormattedStringKind, FormattedStringPart, FunctionDeclaration,
    Identifier, ImplDeclaration, ImportDeclaration, MatchArm, MatchArmBody, MatchPattern, Module,
    ObjectProperty, Parameter, Statement, TestDeclaration, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration, TypeField, TypeName, UnaryOperator,
    UseDeclaration, UseItem, Visibility,
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

/// Parses only the `import` declarations at the start of a token stream, skipping `use`
/// declarations between them and stopping at the first other declaration, since imports must
/// precede every declaration. Errors in the rest of the module are left to the full parse.
pub fn parse_imports(
    tokens: Vec<Token<'_>>,
) -> Result<Vec<ImportDeclaration<'_>>, CompileDiagnostics<'_>> {
    let mut parser = Parser {
        tokens,
        current: 0,
        type_names: HashSet::new(),
        diagnostics: CompileDiagnostics::new(),
        depth: 0,
        max_nesting: 0,
    };
    let mut imports = Vec::new();
    loop {
        let declaration = match &parser.peek().kind {
            TokenKind::Import => parser.import_declaration().map(Some),
            TokenKind::Use => parser.use_declaration().map(|_| None),
            _ => break,
        };
        match declaration {
            Ok(Some(import)) => imports.push(import),
            Ok(None) => {}
            Err(diagnostic) => return Err(CompileDiagnostics::from(diagnostic)),
        }
    }
    Ok(imports)
}

/// Parses a token stream into one ExS module within the default nesting limit.
pub fn parse<'a>(
    source_id: &'a str,
    tokens: Vec<Token<'a>>,
    require_main: bool,
) -> Result<Module<'a>, CompileDiagnostics<'a>> {
    parse_with_nesting(
        source_id,
        tokens,
        require_main,
        crate::CompileLimits::default().max_nesting,
    )
}

/// Parses a token stream into one ExS module; expressions, blocks and type annotations may
/// nest at most `max_nesting` levels deep.
pub fn parse_with_nesting<'a>(
    source_id: &'a str,
    tokens: Vec<Token<'a>>,
    require_main: bool,
    max_nesting: usize,
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
        depth: 0,
        max_nesting,
    };
    let mut imports = Vec::new();
    let mut uses = Vec::new();
    let mut types = Vec::new();
    let mut enums = Vec::new();
    let mut traits = Vec::new();
    let mut implementations = Vec::new();
    let mut functions = Vec::new();
    let mut constants = Vec::new();
    let mut tests = Vec::new();
    while !parser.at_end() {
        let visibility = if parser.matches(&TokenKind::Private) {
            if !matches!(
                parser.peek().kind,
                TokenKind::Fn
                    | TokenKind::Type
                    | TokenKind::Enum
                    | TokenKind::Trait
                    | TokenKind::Const
            ) {
                parser.diagnostics.push(parser.error(
                    parser.previous().span,
                    "E0115",
                    "`private` must precede `fn`, `type`, `enum`, `trait`, or `const`",
                ));
                parser.synchronize_declaration();
                continue;
            }
            Visibility::Private
        } else {
            Visibility::Public
        };
        match &parser.peek().kind {
            TokenKind::Import => match parser.import_declaration() {
                Ok(declaration)
                    if types.is_empty()
                        && traits.is_empty()
                        && enums.is_empty()
                        && implementations.is_empty()
                        && functions.is_empty()
                        && constants.is_empty()
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
                        && constants.is_empty()
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
            TokenKind::Type => match parser.type_declaration(visibility) {
                Ok(declaration) => types.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Enum => match parser.enum_declaration(visibility) {
                Ok(declaration) => enums.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Trait => match parser.trait_declaration(visibility) {
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
            TokenKind::Const => match parser.const_declaration(visibility) {
                Ok(declaration) => constants.push(declaration),
                Err(diagnostic) => {
                    parser.diagnostics.push(diagnostic);
                    parser.synchronize_declaration();
                }
            },
            TokenKind::Fn => match parser.function(visibility, visibility == Visibility::Public) {
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
                    "expected `type`, `enum`, `trait`, `impl`, `fn`, `const`, or `test` at module level",
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
        constants,
        tests,
    })
}

/// The state of one parse.
struct Parser<'a> {
    /// The tokens being parsed.
    tokens: Vec<Token<'a>>,
    /// Index of the next token.
    current: usize,
    /// Names declared as types, which make `Name {` a typed Object literal.
    type_names: HashSet<String>,
    /// Diagnostics of declarations and statements the parser recovered from.
    diagnostics: CompileDiagnostics<'a>,
    /// Current nesting depth of expressions, blocks and type annotations.
    depth: usize,
    /// Deepest nesting the parser accepts.
    max_nesting: usize,
}

impl<'a> Parser<'a> {
    /// Runs `parse` one nesting level deeper, rejecting source that nests beyond the limit
    /// before recursion could exhaust the stack.
    fn nested<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, CompileDiagnostic<'a>>,
    ) -> Result<T, CompileDiagnostic<'a>> {
        if self.depth >= self.max_nesting {
            return Err(self.error(
                self.peek().span,
                "E0114",
                format!(
                    "source nests deeper than the limit of {} levels",
                    self.max_nesting
                ),
            ));
        }
        self.depth += 1;
        let parsed = parse(self);
        self.depth -= 1;
        parsed
    }
}

pub(crate) fn expression_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
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
        | Expression::IsType { span, .. }
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
        MatchPattern::Literal(literal) => expression_span(literal),
    }
}
