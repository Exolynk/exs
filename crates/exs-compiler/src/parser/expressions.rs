use super::*;

impl<'a> Parser<'a> {
    pub(super) fn expression(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.or()
    }

    pub(super) fn or(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(Self::and, &[(TokenKind::OrOr, BinaryOperator::Or)])
    }

    pub(super) fn and(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(Self::equality, &[(TokenKind::AndAnd, BinaryOperator::And)])
    }

    pub(super) fn equality(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::comparison,
            &[
                (TokenKind::EqualEqual, BinaryOperator::Equal),
                (TokenKind::BangEqual, BinaryOperator::NotEqual),
            ],
        )
    }

    pub(super) fn comparison(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
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

    pub(super) fn term(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::factor,
            &[
                (TokenKind::Plus, BinaryOperator::Add),
                (TokenKind::Minus, BinaryOperator::Subtract),
            ],
        )
    }

    pub(super) fn factor(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        self.binary(
            Self::unary,
            &[
                (TokenKind::Star, BinaryOperator::Multiply),
                (TokenKind::Slash, BinaryOperator::Divide),
            ],
        )
    }

    pub(super) fn binary(
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

    pub(super) fn unary(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
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

    pub(super) fn call(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
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

    pub(super) fn primary(&mut self) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Integer(value) => Ok(Expression::Integer(value, token.span)),
            TokenKind::Float(value) => Ok(Expression::Float(value, token.span)),
            TokenKind::String(value) => Ok(Expression::String(value, token.span)),
            TokenKind::Bytes(value) => Ok(Expression::Bytes(value, token.span)),
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
    pub(super) fn formatted_string(
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
    pub(super) fn match_expression(
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
    pub(super) fn match_pattern(&mut self) -> Result<MatchPattern<'a>, CompileDiagnostic<'a>> {
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
    pub(super) fn parallel(
        &mut self,
        start: SourceSpan<'a>,
    ) -> Result<Expression<'a>, CompileDiagnostic<'a>> {
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
    pub(super) fn closure_parameters(&mut self) -> Option<Vec<Parameter<'a>>> {
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
    pub(super) fn host_call(
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
}
