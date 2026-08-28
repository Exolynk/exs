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

impl<'a> Parser<'a> {
    /// Parses one relative source-file import declaration.
    fn import_declaration(&mut self) -> Result<ImportDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Import, "expected `import`")?
            .span;
        let token = self.advance().clone();
        let path = match token.kind {
            TokenKind::String(path) => path,
            _ => {
                return Err(self.error(token.span, "E0113", "expected string path after `import`"));
            }
        };
        let alias = if self.matches(&TokenKind::As) {
            Some(self.identifier("expected namespace after `as`")?)
        } else {
            None
        };
        let end = self
            .expect_simple(TokenKind::Semicolon, "expected `;` after import")?
            .span;
        Ok(ImportDeclaration {
            path,
            alias,
            span: start.through(end),
        })
    }

    /// Parses one nominal enum declaration and its zero-or-more-field variants.
    fn enum_declaration(&mut self) -> Result<EnumDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Enum, "expected `enum` at module level")?
            .span;
        let name = self.identifier("expected enum name after `enum`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after enum name")?;
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let variant_name = self.identifier("expected enum variant name")?;
            let mut fields = Vec::new();
            let mut end = variant_name.span;
            if self.matches(&TokenKind::LeftParen) {
                while !self.check(&TokenKind::RightParen) && !self.at_end() {
                    let field_start = self.peek().span;
                    let field_name = self.identifier("expected variant payload field name")?;
                    let type_annotation = if self.matches(&TokenKind::Colon) {
                        Some(self.type_annotation()?)
                    } else {
                        None
                    };
                    let field_end = type_annotation
                        .as_ref()
                        .map_or(field_name.span, |annotation| annotation.span);
                    fields.push(TypeField {
                        name: field_name,
                        type_annotation,
                        span: field_start.through(field_end),
                    });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
                end = self
                    .expect_simple(TokenKind::RightParen, "expected `)` after variant fields")?
                    .span;
            }
            variants.push(EnumVariant {
                span: variant_name.span.through(end),
                name: variant_name,
                fields,
            });
            if !self.matches(&TokenKind::Comma) {
                if !self.check(&TokenKind::RightBrace) {
                    return Err(self.error(
                        self.peek().span,
                        "E0103",
                        "expected `,` or `}` after enum variant",
                    ));
                }
                break;
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after enum variants")?
            .span;
        Ok(EnumDeclaration {
            name,
            variants,
            span: start.through(end),
        })
    }

    /// Parses one `use namespace::{name as alias}` declaration.
    fn use_declaration(&mut self) -> Result<UseDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self.expect_simple(TokenKind::Use, "expected `use`")?.span;
        let namespace = self.identifier("expected namespace after `use`")?;
        self.expect_simple(TokenKind::DoubleColon, "expected `::` after use namespace")?;
        let mut items = Vec::new();
        if self.matches(&TokenKind::LeftBrace) {
            loop {
                let name = self.identifier("expected imported declaration name")?;
                let alias = if self.matches(&TokenKind::As) {
                    Some(self.identifier("expected alias after `as`")?)
                } else {
                    None
                };
                self.type_names.insert(
                    alias
                        .as_ref()
                        .map_or_else(|| name.name.clone(), |alias| alias.name.clone()),
                );
                items.push(UseItem { name, alias });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
            }
            self.expect_simple(TokenKind::RightBrace, "expected `}` after use list")?;
        } else {
            let name = self.identifier("expected imported declaration name")?;
            let alias = if self.matches(&TokenKind::As) {
                Some(self.identifier("expected alias after `as`")?)
            } else {
                None
            };
            self.type_names.insert(
                alias
                    .as_ref()
                    .map_or_else(|| name.name.clone(), |alias| alias.name.clone()),
            );
            items.push(UseItem { name, alias });
        }
        let end = self
            .expect_simple(TokenKind::Semicolon, "expected `;` after use declaration")?
            .span;
        Ok(UseDeclaration {
            namespace,
            items,
            span: start.through(end),
        })
    }

    fn function(
        &mut self,
        visibility: FunctionVisibility,
    ) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Fn, "expected `fn` at module level")?
            .span;
        self.function_from_start(start, visibility)
    }

    /// Parses one named source test declaration.
    fn test_declaration(&mut self) -> Result<TestDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self.advance().span;
        let token = self.advance().clone();
        let description = match token.kind {
            TokenKind::String(description) => description,
            _ => {
                return Err(self.error(
                    token.span,
                    "E0113",
                    "expected string test description after `test`",
                ));
            }
        };
        let body = self.block()?;
        Ok(TestDeclaration {
            description,
            span: start.through(body.span),
            body,
        })
    }

    /// Parses one function after its `fn` keyword was consumed.
    fn function_from_start(
        &mut self,
        start: SourceSpan<'a>,
        visibility: FunctionVisibility,
    ) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let (name, parameters, return_type) = self.function_header_from_start()?;
        let body = self.block()?;
        Ok(FunctionDeclaration {
            visibility,
            name,
            parameters,
            return_type,
            span: start.through(body.span),
            body,
        })
    }

    /// Parses the shared signature portion of a function or trait method.
    fn function_header_from_start(
        &mut self,
    ) -> Result<
        (
            Identifier<'a>,
            Vec<Parameter<'a>>,
            Option<TypeAnnotation<'a>>,
        ),
        CompileDiagnostic<'a>,
    > {
        let name = self.identifier("expected function name")?;
        self.expect_simple(TokenKind::LeftParen, "expected `(` after function name")?;
        let mut parameters = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                let name = self.identifier("expected parameter name")?;
                let type_annotation = if self.matches(&TokenKind::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                let variadic = self.matches(&TokenKind::Ellipsis);
                parameters.push(Parameter {
                    name,
                    type_annotation,
                    variadic,
                });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        self.expect_simple(TokenKind::RightParen, "expected `)` after parameters")?;
        let return_type = if self.matches(&TokenKind::Arrow) {
            Some(self.type_annotation()?)
        } else {
            None
        };
        Ok((name, parameters, return_type))
    }

    /// Parses one nominal Object type declaration.
    fn type_declaration(&mut self) -> Result<TypeDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Type, "expected `type` at module level")?
            .span;
        let name = self.identifier("expected type name after `type`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after type name")?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let field_start = self.peek().span;
            let name = self.identifier("expected type field name")?;
            let type_annotation = if self.matches(&TokenKind::Colon) {
                Some(self.type_annotation()?)
            } else {
                None
            };
            let end = type_annotation
                .as_ref()
                .map_or(name.span, |annotation| annotation.span);
            fields.push(TypeField {
                name,
                type_annotation,
                span: field_start.through(end),
            });
            if !self.matches(&TokenKind::Comma) {
                if !self.check(&TokenKind::RightBrace) {
                    return Err(self.error(
                        self.peek().span,
                        "E0103",
                        "expected `,` or `}` after type field",
                    ));
                }
                break;
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after type fields")?
            .span;
        Ok(TypeDeclaration {
            name,
            fields,
            span: start.through(end),
        })
    }

    /// Parses one trait declaration with required signatures and default method bodies.
    fn trait_declaration(&mut self) -> Result<TraitDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Trait, "expected `trait` at module level")?
            .span;
        let name = self.identifier("expected trait name after `trait`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after trait name")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let method_start = self
                .expect_simple(TokenKind::Fn, "expected `fn` inside trait block")?
                .span;
            let (name, parameters, return_type) = self.function_header_from_start()?;
            let (body, end) = if self.matches(&TokenKind::Semicolon) {
                (None, self.previous().span)
            } else {
                let body = self.block()?;
                let end = body.span;
                (Some(body), end)
            };
            methods.push(TraitMethodDeclaration {
                name,
                parameters,
                return_type,
                body,
                span: method_start.through(end),
            });
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after trait methods")?
            .span;
        Ok(TraitDeclaration {
            name,
            methods,
            span: start.through(end),
        })
    }

    /// Parses one inherent or trait implementation block for a nominal type.
    fn implementation(&mut self) -> Result<ImplDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Impl, "expected `impl` at module level")?
            .span;
        let first_name = self.qualified_identifier("expected trait or type name after `impl`")?;
        let (trait_name, type_name) = if self.matches(&TokenKind::For) {
            (
                Some(first_name),
                self.qualified_identifier("expected type name after `for")?,
            )
        } else {
            (None, first_name)
        };
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after impl type name")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let method_start = self
                .expect_simple(TokenKind::Fn, "expected `fn` inside impl block")?
                .span;
            methods.push(self.function_from_start(method_start, FunctionVisibility::Private)?);
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after impl methods")?
            .span;
        Ok(ImplDeclaration {
            trait_name,
            type_name,
            methods,
            span: start.through(end),
        })
    }

    /// Parses one non-empty `Type | Type` annotation in a function boundary position.
    fn type_annotation(&mut self) -> Result<TypeAnnotation<'a>, CompileDiagnostic<'a>> {
        let first = self.type_name()?;
        let mut members = vec![first];
        while self.matches(&TokenKind::Pipe) {
            members.push(self.type_name()?);
        }
        let span = members
            .first()
            .map_or_else(|| SourceSpan::empty("<unknown>"), |member| member.span)
            .through(
                members
                    .last()
                    .map_or_else(|| SourceSpan::empty("<unknown>"), |member| member.span),
            );
        Ok(TypeAnnotation { members, span })
    }

    /// Parses one source-visible type name.
    fn type_name(&mut self) -> Result<TypeName<'a>, CompileDiagnostic<'a>> {
        let (mut name, mut span) = self.type_name_component("expected type name")?;
        if self.matches(&TokenKind::DoubleColon) {
            let (member, member_span) =
                self.type_name_component("expected type name after `::`")?;
            name.push_str("::");
            name.push_str(&member);
            span = span.through(member_span);
        }
        Ok(TypeName { name, span })
    }

    /// Parses one identifier or reserved built-in type-name token.
    fn type_name_component(
        &mut self,
        message: &str,
    ) -> Result<(String, SourceSpan<'a>), CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        let name = match token.kind {
            TokenKind::Identifier(name) => name,
            TokenKind::None => "None".to_owned(),
            TokenKind::Error => "Error".to_owned(),
            _ => return Err(self.error(token.span, "E0111", message)),
        };
        Ok((name, token.span))
    }

    fn block(&mut self) -> Result<Block<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::LeftBrace, "expected `{`")?
            .span;
        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            match self.statement() {
                Ok(statement) => statements.push(statement),
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic);
                    self.synchronize_statement();
                }
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after block")?
            .span;
        Ok(Block {
            statements,
            span: start.through(end),
        })
    }

    fn statement(&mut self) -> Result<Statement<'a>, CompileDiagnostic<'a>> {
        if self.check(&TokenKind::LeftBrace) {
            let block = self.block()?;
            return Ok(Statement::Block {
                span: block.span,
                block,
            });
        }
        if self.matches(&TokenKind::Let) {
            let start = self.previous().span;
            let name = self.identifier("expected binding name after `let`")?;
            let value = if self.matches(&TokenKind::Equal) {
                self.expression()?
            } else {
                Expression::None(name.span)
            };
            let end = self.expect_statement_semicolon(
                expression_span(&value),
                "expected `;` after let declaration",
            )?;
            return Ok(Statement::Let {
                name,
                value,
                span: start.through(end),
            });
        }
        if self.matches(&TokenKind::Ret) {
            let start = self.previous().span;
            let value = if self.check(&TokenKind::Semicolon) {
                None
            } else {
                Some(self.expression()?)
            };
            let end = self.expect_statement_semicolon(
                value.as_ref().map_or(start, expression_span),
                "expected `;` after return",
            )?;
            return Ok(Statement::Return {
                value,
                span: start.through(end),
            });
        }
        if self.matches(&TokenKind::If) {
            let start = self.previous().span;
            let condition = self.expression()?;
            let then_block = self.block()?;
            let else_branch = if self.matches(&TokenKind::Else) {
                if self.check(&TokenKind::If) {
                    Some(ElseBranch::If(Box::new(self.statement()?)))
                } else {
                    Some(ElseBranch::Block(self.block()?))
                }
            } else {
                None
            };
            let end = if else_branch.is_some() {
                self.previous().span
            } else {
                then_block.span
            };
            return Ok(Statement::If {
                condition,
                then_block,
                else_branch,
                span: start.through(end),
            });
        }
        if self.matches(&TokenKind::While) {
            let start = self.previous().span;
            let condition = self.expression()?;
            let body = self.block()?;
            return Ok(Statement::While {
                condition,
                span: start.through(body.span),
                body,
            });
        }
        if self.matches(&TokenKind::For) {
            let start = self.previous().span;
            let binding = self.identifier("expected binding name after for")?;
            self.expect_simple(TokenKind::In, "expected in after for-loop binding")?;
            let iterable = self.expression()?;
            let body = self.block()?;
            return Ok(Statement::For {
                binding,
                iterable,
                span: start.through(body.span),
                body,
            });
        }
        if self.matches(&TokenKind::Break) {
            let start = self.previous().span;
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected semicolon after break")?
                .span;
            return Ok(Statement::Break {
                span: start.through(end),
            });
        }
        if self.matches(&TokenKind::Continue) {
            let start = self.previous().span;
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected semicolon after continue")?
                .span;
            return Ok(Statement::Continue {
                span: start.through(end),
            });
        }
        let expression = self.expression()?;
        if self.matches(&TokenKind::Equal) {
            let start = expression_span(&expression);
            let target = self.assignment_target(expression)?;
            let value = self.expression()?;
            let end = self.expect_statement_semicolon(
                expression_span(&value),
                "expected `;` after assignment",
            )?;
            return Ok(Statement::Assign {
                target,
                value,
                span: start.through(end),
            });
        }
        let start = expression_span(&expression);
        let end = self.expect_statement_semicolon(start, "expected `;` after expression")?;
        Ok(Statement::Expression {
            expression,
            span: start.through(end),
        })
    }

    fn expression(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.or()
    }

    fn or(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(Self::and, &[(TokenKind::OrOr, BinaryOperator::Or)])
    }

    fn and(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(Self::equality, &[(TokenKind::AndAnd, BinaryOperator::And)])
    }

    fn equality(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::comparison,
            &[
                (TokenKind::EqualEqual, BinaryOperator::Equal),
                (TokenKind::BangEqual, BinaryOperator::NotEqual),
            ],
        )
    }

    fn comparison(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let mut expression = self.binary(
            Self::term,
            &[
                (TokenKind::Less, BinaryOperator::LessThan),
                (TokenKind::LessEqual, BinaryOperator::LessOrEqual),
                (TokenKind::Greater, BinaryOperator::GreaterThan),
                (TokenKind::GreaterEqual, BinaryOperator::GreaterOrEqual),
            ],
        )?;
        if self.matches(&TokenKind::Is) {
            self.expect_simple(TokenKind::Error, "expected Error after is")?;
            let span = expression_span(&expression).through(self.previous().span);
            expression = Expression::IsError {
                value: Box::new(expression),
                span,
            };
        }
        Ok(expression)
    }

    fn term(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::factor,
            &[
                (TokenKind::Plus, BinaryOperator::Add),
                (TokenKind::Minus, BinaryOperator::Subtract),
            ],
        )
    }

    fn factor(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::unary,
            &[
                (TokenKind::Star, BinaryOperator::Multiply),
                (TokenKind::Slash, BinaryOperator::Divide),
            ],
        )
    }

    fn binary(
        &mut self,
        next: fn(&mut Self) -> Result<Expression<'a>, CompileDiagnostic<'a>>,
        operators: &[(TokenKind, BinaryOperator)],
    ) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let mut expression = next(self)?;
        while let Some(operator) = operators
            .iter()
            .find_map(|(token, operator)| self.matches(token).then_some(*operator))
        {
            let right = next(self)?;
            let span = expression_span(&expression).through(expression_span(&right));
            expression = Expression::Binary {
                operator,
                left: Box::new(expression),
                right: Box::new(right),
                span,
            };
        }
        Ok(expression)
    }

    fn unary(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let operator = if self.matches(&TokenKind::Minus) {
            Some(UnaryOperator::Negate)
        } else if self.matches(&TokenKind::Bang) {
            Some(UnaryOperator::Not)
        } else {
            None
        };
        if let Some(operator) = operator {
            let start = self.previous().span;
            let operand = self.unary()?;
            let span = start.through(expression_span(&operand));
            return Ok(Expression::Unary {
                operator,
                operand: Box::new(operand),
                span,
            });
        }
        self.call()
    }

    fn call(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let mut expression = self.primary()?;
        loop {
            if self.matches(&TokenKind::DoubleColon) {
                let type_name = match expression {
                    Expression::Variable(identifier) => identifier,
                    _ => {
                        return Err(self.error(
                            self.previous().span,
                            "E0110",
                            "static calls require a type name",
                        ));
                    }
                };
                let method = self.identifier("expected method name after `::`")?;
                if self.check(&TokenKind::DoubleColon) {
                    expression = Expression::Variable(Identifier {
                        name: format!("{}::{}", type_name.name, method.name),
                        span: type_name.span.through(method.span),
                    });
                    continue;
                }
                if self.check(&TokenKind::LeftBrace) {
                    expression = Expression::Variable(Identifier {
                        name: format!("{}::{}", type_name.name, method.name),
                        span: type_name.span.through(method.span),
                    });
                    continue;
                }
                if self.check(&TokenKind::LeftParen) && !self.type_names.contains(&type_name.name) {
                    expression = Expression::Variable(Identifier {
                        name: format!("{}::{}", type_name.name, method.name),
                        span: type_name.span.through(method.span),
                    });
                    continue;
                }
                if !self.check(&TokenKind::LeftParen) {
                    expression = Expression::Variable(Identifier {
                        name: format!("{}::{}", type_name.name, method.name),
                        span: type_name.span.through(method.span),
                    });
                    continue;
                }
                self.expect_simple(
                    TokenKind::LeftParen,
                    "expected `(` after static method name",
                )?;
                let arguments = self.arguments(TokenKind::RightParen)?;
                let close = self
                    .expect_simple(TokenKind::RightParen, "expected `)` after arguments")?
                    .span;
                expression = Expression::StaticMethodCall {
                    type_name: type_name.clone(),
                    method,
                    arguments,
                    span: type_name.span.through(close),
                };
            } else if self.check(&TokenKind::LeftBrace)
                && matches!(&expression, Expression::Variable(identifier) if self.type_names.contains(&identifier.name) || identifier.name.contains("::"))
            {
                self.advance();
                let type_name = match expression {
                    Expression::Variable(identifier) => identifier,
                    _ => {
                        return Err(self.error(
                            self.previous().span,
                            "E0110",
                            "typed object construction requires a type name",
                        ));
                    }
                };
                let properties = self.object_properties()?;
                let end = self
                    .expect_simple(
                        TokenKind::RightBrace,
                        "expected `}` after typed Object properties",
                    )?
                    .span;
                expression = Expression::TypedObject {
                    type_name: type_name.clone(),
                    properties,
                    span: type_name.span.through(end),
                };
            } else if self.matches(&TokenKind::LeftParen) {
                let open = self.previous().span;
                let callee = match expression {
                    Expression::Variable(identifier) => identifier,
                    _ => {
                        return Err(self.error(
                            open,
                            "E0110",
                            "only named function calls are supported in Phase 1",
                        ));
                    }
                };
                let arguments = self.arguments(TokenKind::RightParen)?;
                let close = self
                    .expect_simple(TokenKind::RightParen, "expected `)` after arguments")?
                    .span;
                expression = Expression::Call {
                    callee,
                    arguments,
                    span: open.through(close),
                };
            } else if self.matches(&TokenKind::LeftBracket) {
                let index = self.expression()?;
                let close = self
                    .expect_simple(TokenKind::RightBracket, "expected `]` after index")?
                    .span;
                let span = expression_span(&expression).through(close);
                expression = Expression::Index {
                    receiver: Box::new(expression),
                    index: Box::new(index),
                    span,
                };
            } else if self.matches(&TokenKind::Dot) {
                let property = self.identifier("expected property name after `.`")?;
                if self.matches(&TokenKind::LeftParen) {
                    let arguments = self.arguments(TokenKind::RightParen)?;
                    let close = self
                        .expect_simple(TokenKind::RightParen, "expected `)` after arguments")?
                        .span;
                    let span = expression_span(&expression).through(close);
                    expression = Expression::MethodCall {
                        receiver: Box::new(expression),
                        method: property,
                        arguments,
                        span,
                    };
                } else {
                    let span = expression_span(&expression).through(property.span);
                    expression = Expression::Property {
                        receiver: Box::new(expression),
                        property,
                        span,
                    };
                }
            } else if self.matches(&TokenKind::Question) {
                let span = expression_span(&expression).through(self.previous().span);
                expression = Expression::Propagate {
                    value: Box::new(expression),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expression)
    }

    fn primary(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Integer(value) => Ok(Expression::Integer(value, token.span)),
            TokenKind::Float(value) => Ok(Expression::Float(value, token.span)),
            TokenKind::String(value) => Ok(Expression::String(value, token.span)),
            TokenKind::FormattedString(value) => self.formatted_string(value, token.span),
            TokenKind::True => Ok(Expression::Bool(true, token.span)),
            TokenKind::False => Ok(Expression::Bool(false, token.span)),
            TokenKind::None => Ok(Expression::None(token.span)),
            TokenKind::Error => {
                self.expect_simple(TokenKind::LeftParen, "expected ( after Error")?;
                let arguments = self.arguments(TokenKind::RightParen)?;
                let end = self
                    .expect_simple(TokenKind::RightParen, "expected ) after Error arguments")?
                    .span;
                Ok(Expression::Call {
                    callee: Identifier {
                        name: "Error".to_owned(),
                        span: token.span,
                    },
                    arguments,
                    span: token.span.through(end),
                })
            }
            TokenKind::Host => self.host_call(token.span),
            TokenKind::Par => self.parallel(token.span),
            TokenKind::Match => self.match_expression(token.span),
            TokenKind::LeftBracket => {
                let elements = self.arguments(TokenKind::RightBracket)?;
                let end = self
                    .expect_simple(TokenKind::RightBracket, "expected `]` after list elements")?
                    .span;
                Ok(Expression::List {
                    elements,
                    span: token.span.through(end),
                })
            }
            TokenKind::LeftBrace => {
                let properties = self.object_properties()?;
                let end = self
                    .expect_simple(
                        TokenKind::RightBrace,
                        "expected `}` after object properties",
                    )?
                    .span;
                Ok(Expression::Object {
                    properties,
                    span: token.span.through(end),
                })
            }
            TokenKind::Identifier(name) => Ok(Expression::Variable(Identifier {
                name,
                span: token.span,
            })),
            TokenKind::LeftParen => {
                if let Some(parameters) = self.closure_parameters() {
                    self.expect_simple(
                        TokenKind::FatArrow,
                        "expected `=>` after closure parameters",
                    )?;
                    let body = self.block()?;
                    return Ok(Expression::Closure {
                        parameters,
                        span: token.span.through(body.span),
                        body,
                    });
                }
                let expression = self.expression()?;
                self.expect_simple(TokenKind::RightParen, "expected `)` after expression")?;
                Ok(expression)
            }
            _ => Err(self.error(token.span, "E0101", "expected expression")),
        }
    }

    /// Parses the independently lexed expression fragments of one formatted string token.
    fn formatted_string(
        &mut self,
        value: crate::lexer::FormattedString,
        span: SourceSpan<'a>,
    ) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let kind = match value.kind {
            LexerFormattedStringKind::Standard => FormattedStringKind::Standard,
            LexerFormattedStringKind::Raw => FormattedStringKind::Raw,
            LexerFormattedStringKind::Dedented => FormattedStringKind::Dedented,
        };
        let mut parts = Vec::with_capacity(value.parts.len());
        for part in value.parts {
            match part {
                LexerFormattedStringPart::Text(value) => {
                    parts.push(FormattedStringPart::Text(value));
                }
                LexerFormattedStringPart::Expression { source, start_byte } => {
                    let mut lexed = crate::lexer::lex_fragment(span.source_id, &source, start_byte);
                    if let Some(diagnostic) = lexed.diagnostics.diagnostics.first_mut() {
                        return Err(diagnostic.clone());
                    }
                    let mut parser = Self {
                        tokens: lexed.tokens,
                        current: 0,
                        type_names: self.type_names.clone(),
                        diagnostics: CompileDiagnostics::new(),
                    };
                    let expression = parser.expression()?;
                    if !parser.at_end() {
                        return Err(parser.error(
                            parser.peek().span,
                            "E0103",
                            "expected the end of a formatted string interpolation",
                        ));
                    }
                    parts.push(FormattedStringPart::Expression(expression));
                }
            }
        }
        Ok(Expression::FormattedString { kind, parts, span })
    }

    /// Parses one enum-pattern `match` expression with expression-valued arms.
    fn match_expression(
        &mut self,
        start: SourceSpan<'a>,
    ) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let value = self.expression()?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after match value")?;
        let mut arms = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let pattern = self.match_pattern()?;
            self.expect_simple(TokenKind::FatArrow, "expected `=>` after match pattern")?;
            let body = if self.check(&TokenKind::LeftBrace) {
                MatchArmBody::Block(self.block()?)
            } else {
                MatchArmBody::Expression(self.expression()?)
            };
            let body_span = match &body {
                MatchArmBody::Expression(value) => expression_span(value),
                MatchArmBody::Block(block) => block.span,
            };
            let arm_span = match_pattern_span(&pattern).through(body_span);
            arms.push(MatchArm {
                pattern,
                body,
                span: arm_span,
            });
            if !self.matches(&TokenKind::Comma) && !self.check(&TokenKind::RightBrace) {
                return Err(self.error(
                    self.peek().span,
                    "E0110",
                    "expected `,` or `}` after match arm",
                ));
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after match arms")?
            .span;
        Ok(Expression::Match {
            value: Box::new(value),
            arms,
            span: start.through(end),
        })
    }

    /// Parses one qualified variant or wildcard match pattern.
    fn match_pattern(&mut self) -> Result<MatchPattern<'a>, CompileDiagnostic<'a>> {
        let first = self.identifier("expected match pattern")?;
        if first.name == "_" {
            return Ok(MatchPattern::Wildcard(first.span));
        }
        self.expect_simple(
            TokenKind::DoubleColon,
            "expected `::` in enum match pattern",
        )?;
        let pattern_start = first.span;
        let mut type_name = first;
        let mut component = self.identifier("expected enum type or variant name")?;
        while self.matches(&TokenKind::DoubleColon) {
            type_name.name.push_str("::");
            type_name.name.push_str(&component.name);
            type_name.span = type_name.span.through(component.span);
            component = self.identifier("expected enum variant name after `::`")?;
        }
        let mut bindings = Vec::new();
        let mut end = component.span;
        if self.matches(&TokenKind::LeftParen) {
            while !self.check(&TokenKind::RightParen) && !self.at_end() {
                bindings.push(self.identifier("expected enum payload binding")?);
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
            end = self
                .expect_simple(TokenKind::RightParen, "expected `)` after match bindings")?
                .span;
        }
        Ok(MatchPattern::Variant {
            type_name,
            variant: component,
            bindings,
            span: pattern_start.through(end),
        })
    }

    /// Parses static parallel expressions or one dynamic callable List expression.
    fn parallel(&mut self, start: SourceSpan<'a>) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        if self.matches(&TokenKind::LeftBrace) {
            let mut tasks = Vec::new();
            while !self.check(&TokenKind::RightBrace) && !self.at_end() {
                let value = self.expression()?;
                let value_span = expression_span(&value);
                let end = self.expect_statement_semicolon(
                    value_span,
                    "expected semicolon after static par task",
                )?;
                let body = Block {
                    statements: vec![Statement::Return {
                        value: Some(value),
                        span: value_span.through(end),
                    }],
                    span: value_span.through(end),
                };
                tasks.push(Expression::Closure {
                    parameters: Vec::new(),
                    body,
                    span: value_span,
                });
            }
            let end = self
                .expect_simple(TokenKind::RightBrace, "expected `}` after static par tasks")?
                .span;
            return Ok(Expression::ParallelStatic {
                tasks,
                span: start.through(end),
            });
        }
        self.expect_simple(TokenKind::LeftParen, "expected `{` or `(` after par")?;
        let functions = self.expression()?;
        let end = self
            .expect_simple(
                TokenKind::RightParen,
                "expected `)` after par callable List",
            )?
            .span;
        Ok(Expression::ParallelDynamic {
            functions: Box::new(functions),
            span: start.through(end),
        })
    }

    /// Parses a simple closure parameter list only when it is followed by `=>`.
    fn closure_parameters(&mut self) -> Option<Vec<Parameter<'a>>> {
        let start = self.current;
        let mut parameters = Vec::new();
        if self.check(&TokenKind::RightParen) {
            self.advance();
        } else {
            loop {
                let TokenKind::Identifier(name) = self.peek().kind.clone() else {
                    self.current = start;
                    return None;
                };
                let span = self.advance().span;
                let variadic = self.matches(&TokenKind::Ellipsis);
                parameters.push(Parameter {
                    name: Identifier { name, span },
                    type_annotation: None,
                    variadic,
                });
                if self.check(&TokenKind::RightParen) {
                    self.advance();
                    break;
                }
                if !self.matches(&TokenKind::Comma) {
                    self.current = start;
                    return None;
                }
            }
        }
        if self.check(&TokenKind::FatArrow) {
            Some(parameters)
        } else {
            self.current = start;
            None
        }
    }

    /// Parses the built-in dynamic Host boundary operations.
    fn host_call(
        &mut self,
        start: SourceSpan<'a>,
    ) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.expect_simple(TokenKind::DoubleColon, "expected `::` after Host")?;
        let method = self.identifier("expected Host operation after `Host::")?;
        self.expect_simple(TokenKind::LeftParen, "expected `(` after Host operation")?;
        let mut values = self.arguments(TokenKind::RightParen)?;
        let end = self
            .expect_simple(
                TokenKind::RightParen,
                "expected `)` after Host operation arguments",
            )?
            .span;
        let name = match method.name.as_str() {
            "call" => {
                if values.is_empty() {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        "Host::call expects a host-function name as its first argument",
                    ));
                }
                Box::new(values.remove(0))
            }
            "sleep" => {
                if values.len() != 1 {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        format!(
                            "Host::sleep expects 1 argument but received {}",
                            values.len()
                        ),
                    ));
                }
                Box::new(Expression::String(
                    exs_abi::HOST_SLEEP_HOST_NAME.to_owned(),
                    start.through(method.span),
                ))
            }
            "now" => {
                if !values.is_empty() {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        format!(
                            "Host::now expects 0 arguments but received {}",
                            values.len()
                        ),
                    ));
                }
                return Ok(Expression::HostTime {
                    operation: crate::ast::HostTimeOperation::Now,
                    arguments: values,
                    span: start.through(end),
                });
            }
            "elapsed" => {
                if !values.is_empty() {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        format!(
                            "Host::elapsed expects 0 arguments but received {}",
                            values.len()
                        ),
                    ));
                }
                return Ok(Expression::HostTime {
                    operation: crate::ast::HostTimeOperation::Elapsed,
                    arguments: values,
                    span: start.through(end),
                });
            }
            "date_time_in_timezone" => {
                if values.len() != 2 {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        format!(
                            "Host::date_time_in_timezone expects 2 arguments but received {}",
                            values.len()
                        ),
                    ));
                }
                return Ok(Expression::HostTime {
                    operation: crate::ast::HostTimeOperation::InTimezone,
                    arguments: values,
                    span: start.through(end),
                });
            }
            "date_time_from_components" => {
                if values.len() != 8 {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        format!(
                            "Host::date_time_from_components expects 8 arguments but received {}",
                            values.len()
                        ),
                    ));
                }
                return Ok(Expression::HostTime {
                    operation: crate::ast::HostTimeOperation::FromComponents,
                    arguments: values,
                    span: start.through(end),
                });
            }
            "stream" => {
                if values.is_empty() {
                    return Err(self.error(
                        start.through(end),
                        "E0208",
                        "Host::stream expects a host-stream name as its first argument",
                    ));
                }
                return Ok(Expression::HostStream {
                    arguments: values,
                    span: start.through(end),
                });
            }
            _ => {
                return Err(self.error(
                    method.span,
                    "E0113",
                    "supported Host operations are `call`, `sleep`, `now`, `elapsed`, `date_time_in_timezone`, `date_time_from_components`, and `stream`",
                ));
            }
        };
        Ok(Expression::HostCall {
            name,
            arguments: values,
            span: start.through(end),
        })
    }

    /// Parses a comma-separated expression sequence ending at `terminator`.
    fn arguments(
        &mut self,
        terminator: TokenKind,
    ) -> Result<Vec<Expression<'a>>, CompileDiagnostic<'a>> {
        let mut arguments = Vec::new();
        if !self.check(&terminator) {
            loop {
                arguments.push(self.expression()?);
                if !self.matches(&TokenKind::Comma) || self.check(&terminator) {
                    break;
                }
            }
        }
        Ok(arguments)
    }

    /// Parses comma-separated statically named Object properties after `{`.
    fn object_properties(&mut self) -> Result<Vec<ObjectProperty<'a>>, CompileDiagnostic<'a>> {
        let mut properties = Vec::new();
        if !self.check(&TokenKind::RightBrace) {
            loop {
                let (key, key_span) = self.object_key()?;
                self.expect_simple(TokenKind::Colon, "expected `:` after object property")?;
                let value = self.expression()?;
                let property_span = key_span.through(expression_span(&value));
                properties.push(ObjectProperty {
                    key,
                    key_span,
                    value,
                    span: property_span,
                });
                if self.matches(&TokenKind::Comma) || self.check(&TokenKind::RightBrace) {
                    if self.check(&TokenKind::RightBrace) {
                        break;
                    }
                    continue;
                }
                let diagnostic = self.error(
                    self.peek().span,
                    "E0103",
                    "expected `,` or `}` after object property",
                );
                self.synchronize_delimited(TokenKind::RightBrace);
                return Err(diagnostic);
            }
        }
        Ok(properties)
    }

    /// Converts a parsed expression into a permitted statement assignment location.
    fn assignment_target(
        &self,
        expression: Expression<'a>,
    ) -> Result<AssignmentTarget<'a>, CompileDiagnostic<'a>> {
        match expression {
            Expression::Variable(identifier) => Ok(AssignmentTarget::Variable(identifier)),
            Expression::Index {
                receiver,
                index,
                span,
            } => Ok(AssignmentTarget::Index {
                receiver,
                index,
                span,
            }),
            Expression::Property {
                receiver,
                property,
                span,
            } => Ok(AssignmentTarget::Property {
                receiver,
                property,
                span,
            }),
            expression => Err(self.error(
                expression_span(&expression),
                "E0111",
                "assignment target must be a binding or index access",
            )),
        }
    }

    fn identifier(&mut self, message: &str) -> Result<Identifier<'a>, CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        if let TokenKind::Identifier(name) = token.kind {
            Ok(Identifier {
                name,
                span: token.span,
            })
        } else {
            Err(self.error(token.span, "E0102", message))
        }
    }

    /// Parses one optionally namespace-qualified identifier.
    fn qualified_identifier(
        &mut self,
        message: &str,
    ) -> Result<Identifier<'a>, CompileDiagnostic<'a>> {
        let mut identifier = self.identifier(message)?;
        if self.matches(&TokenKind::DoubleColon) {
            let member = self.identifier("expected identifier after `::`")?;
            identifier.name.push_str("::");
            identifier.name.push_str(&member.name);
            identifier.span = identifier.span.through(member.span);
        }
        Ok(identifier)
    }

    /// Parses an identifier or string object property key.
    fn object_key(&mut self) -> Result<(String, SourceSpan<'a>), CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Identifier(key) | TokenKind::String(key) => Ok((key, token.span)),
            _ => Err(self.error(
                token.span,
                "E0112",
                "expected identifier or string object property key",
            )),
        }
    }

    fn expect_simple(
        &mut self,
        expected: TokenKind,
        message: &str,
    ) -> Result<Token<'a>, CompileDiagnostic<'a>> {
        if self.check(&expected) {
            Ok(self.advance().clone())
        } else {
            Err(self.error(self.peek().span, "E0103", message))
        }
    }

    /// Requires one statement terminator and anchors a missing terminator at the statement end.
    fn expect_statement_semicolon(
        &mut self,
        statement_end: SourceSpan<'a>,
        message: &str,
    ) -> Result<SourceSpan<'a>, CompileDiagnostic<'a>> {
        if self.check(&TokenKind::Semicolon) {
            return Ok(self.advance().span);
        }
        let span = SourceSpan {
            source_id: statement_end.source_id,
            start_byte: statement_end.end_byte,
            end_byte: statement_end.end_byte,
        };
        Err(self.error(span, "E0103", message))
    }

    fn matches(&mut self, expected: &TokenKind) -> bool {
        if self.check(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check(&self, expected: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(expected)
    }

    fn advance(&mut self) -> &Token<'a> {
        if !self.at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Skips tokens until the next top-level declaration can be parsed independently.
    fn synchronize_declaration(&mut self) {
        while !self.at_end()
            && !matches!(
                self.peek().kind,
                TokenKind::Type | TokenKind::Trait | TokenKind::Impl | TokenKind::Fn
            )
        {
            self.advance();
        }
    }

    /// Skips a malformed statement while preserving its enclosing block terminator.
    fn synchronize_statement(&mut self) {
        while !self.at_end() {
            if self.matches(&TokenKind::Semicolon) {
                return;
            }
            if self.check(&TokenKind::RightBrace)
                || matches!(
                    self.peek().kind,
                    TokenKind::Let
                        | TokenKind::Ret
                        | TokenKind::If
                        | TokenKind::While
                        | TokenKind::For
                        | TokenKind::Break
                        | TokenKind::Continue
                )
            {
                return;
            }
            self.advance();
        }
    }

    /// Skips malformed delimited content and consumes its closing delimiter when present.
    fn synchronize_delimited(&mut self, terminator: TokenKind) {
        while !self.at_end() && !self.check(&terminator) {
            self.advance();
        }
        if self.check(&terminator) {
            self.advance();
        }
    }

    fn peek(&self) -> &Token<'a> {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token<'a> {
        &self.tokens[self.current - 1]
    }

    fn error(
        &self,
        span: SourceSpan<'a>,
        code: &'static str,
        message: impl Into<String>,
    ) -> CompileDiagnostic<'a> {
        CompileDiagnostic::new(code, span, message)
    }
}

fn expression_span<'a>(expression: &Expression<'a>) -> SourceSpan<'a> {
    match expression {
        Expression::Integer(_, span)
        | Expression::Float(_, span)
        | Expression::String(_, span)
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
