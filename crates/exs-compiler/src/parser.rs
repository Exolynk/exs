//! Recursive-descent parser for the Phase-1 `ExS` grammar.

use std::collections::HashSet;

use crate::ast::{
    AssignmentTarget, BinaryOperator, Block, Expression, FunctionDeclaration, Identifier,
    ImplDeclaration, Module, ObjectProperty, Parameter, Statement, TypeAnnotation, TypeDeclaration,
    TypeField, TypeName, UnaryOperator,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::lexer::{Token, TokenKind};

/// Parses a token stream into one ExS module.
pub fn parse<'a>(
    source_id: &'a str,
    tokens: Vec<Token<'a>>,
) -> Result<Module<'a>, CompileDiagnostics<'a>> {
    let type_names = tokens
        .windows(2)
        .filter_map(|pair| match (&pair[0].kind, &pair[1].kind) {
            (TokenKind::Type, TokenKind::Identifier(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    let mut parser = Parser {
        tokens,
        current: 0,
        type_names,
    };
    let mut types = Vec::new();
    let mut implementations = Vec::new();
    let mut functions = Vec::new();
    while !parser.at_end() {
        match &parser.peek().kind {
            TokenKind::Type => types.push(
                parser
                    .type_declaration()
                    .map_err(CompileDiagnostics::from)?,
            ),
            TokenKind::Impl => {
                implementations.push(parser.implementation().map_err(CompileDiagnostics::from)?)
            }
            TokenKind::Fn => functions.push(parser.function().map_err(CompileDiagnostics::from)?),
            _ => {
                return Err(CompileDiagnostics::from(parser.error(
                    parser.peek().span,
                    "E0100",
                    "expected `type`, `impl`, or `fn` at module level",
                )));
            }
        }
    }
    if functions.is_empty() {
        return Err(CompileDiagnostics::from(CompileDiagnostic::new(
            "E0100",
            SourceSpan::empty(source_id),
            "a module must declare fn main()",
        )));
    }
    Ok(Module {
        types,
        implementations,
        functions,
    })
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    current: usize,
    type_names: HashSet<String>,
}

impl<'a> Parser<'a> {
    fn function(&mut self) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Fn, "expected `fn` at module level")?
            .span;
        self.function_from_start(start)
    }

    /// Parses one function after its `fn` keyword was consumed.
    fn function_from_start(
        &mut self,
        start: SourceSpan<'a>,
    ) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
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
                parameters.push(Parameter {
                    name,
                    type_annotation,
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
        let body = self.block()?;
        Ok(FunctionDeclaration {
            name,
            parameters,
            return_type,
            span: start.through(body.span),
            body,
        })
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

    /// Parses one implementation block for a nominal Object type.
    fn implementation(&mut self) -> Result<ImplDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Impl, "expected `impl` at module level")?
            .span;
        let type_name = self.identifier("expected type name after `impl`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after impl type name")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let method_start = self
                .expect_simple(TokenKind::Fn, "expected `fn` inside impl block")?
                .span;
            methods.push(self.function_from_start(method_start)?);
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after impl methods")?
            .span;
        Ok(ImplDeclaration {
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
        let token = self.advance().clone();
        let name = match token.kind {
            TokenKind::Identifier(name) => name,
            TokenKind::None => "None".to_owned(),
            TokenKind::Error => "Error".to_owned(),
            _ => return Err(self.error(token.span, "E0111", "expected type name")),
        };
        Ok(TypeName {
            name,
            span: token.span,
        })
    }

    fn block(&mut self) -> Result<Block<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::LeftBrace, "expected `{`")?
            .span;
        let mut statements = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            statements.push(self.statement()?);
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
        if self.matches(&TokenKind::Let) {
            let start = self.previous().span;
            let name = self.identifier("expected binding name after `let`")?;
            self.expect_simple(TokenKind::Equal, "Phase 1 requires a `let` initializer")?;
            let value = self.expression()?;
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected `;` after let declaration")?
                .span;
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
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected `;` after return")?
                .span;
            return Ok(Statement::Return {
                value,
                span: start.through(end),
            });
        }
        if self.matches(&TokenKind::If) {
            let start = self.previous().span;
            let condition = self.expression()?;
            let then_block = self.block()?;
            let else_block = if self.matches(&TokenKind::Else) {
                Some(self.block()?)
            } else {
                None
            };
            let end = else_block
                .as_ref()
                .map_or(then_block.span, |block| block.span);
            return Ok(Statement::If {
                condition,
                then_block,
                else_block,
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
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected `;` after assignment")?
                .span;
            return Ok(Statement::Assign {
                target,
                value,
                span: start.through(end),
            });
        }
        let start = expression_span(&expression);
        let end = self
            .expect_simple(TokenKind::Semicolon, "expected `;` after expression")?
            .span;
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
        self.binary(Self::unary, &[(TokenKind::Star, BinaryOperator::Multiply)])
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
                && matches!(&expression, Expression::Variable(identifier) if self.type_names.contains(&identifier.name))
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
                let expression = self.expression()?;
                self.expect_simple(TokenKind::RightParen, "expected `)` after expression")?;
                Ok(expression)
            }
            _ => Err(self.error(token.span, "E0101", "expected expression")),
        }
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
                if !self.matches(&TokenKind::Comma) || self.check(&TokenKind::RightBrace) {
                    break;
                }
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
        Expression::List { span, .. } => *span,
        Expression::Object { span, .. } => *span,
        Expression::TypedObject { span, .. } => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Unary { span, .. }
        | Expression::IsError { span, .. }
        | Expression::Propagate { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. }
        | Expression::MethodCall { span, .. }
        | Expression::StaticMethodCall { span, .. }
        | Expression::Index { span, .. }
        | Expression::Property { span, .. } => *span,
    }
}
