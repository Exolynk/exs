use super::*;

impl<'a> Parser<'a> {
    /// Parses a comma-separated expression sequence ending at `terminator`.
    pub(super) fn arguments(
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
    pub(super) fn object_properties(
        &mut self,
    ) -> Result<Vec<ObjectProperty<'a>>, CompileDiagnostic<'a>> {
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
    pub(super) fn assignment_target(
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

    pub(super) fn identifier(
        &mut self,
        message: &str,
    ) -> Result<Identifier<'a>, CompileDiagnostic<'a>> {
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
    pub(super) fn qualified_identifier(
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
    pub(super) fn object_key(&mut self) -> Result<(String, SourceSpan<'a>), CompileDiagnostic<'a>> {
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

    pub(super) fn expect_simple(
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
    pub(super) fn expect_statement_semicolon(
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

    pub(super) fn matches(&mut self, expected: &TokenKind) -> bool {
        if self.check(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn check(&self, expected: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(expected)
    }

    pub(super) fn advance(&mut self) -> &Token<'a> {
        if !self.at_end() {
            self.current += 1;
        }
        self.previous()
    }

    pub(super) fn at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Skips tokens until the next top-level declaration can be parsed independently.
    pub(super) fn synchronize_declaration(&mut self) {
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
    pub(super) fn synchronize_statement(&mut self) {
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
    pub(super) fn synchronize_delimited(&mut self, terminator: TokenKind) {
        while !self.at_end() && !self.check(&terminator) {
            self.advance();
        }
        if self.check(&terminator) {
            self.advance();
        }
    }

    pub(super) fn peek(&self) -> &Token<'a> {
        &self.tokens[self.current]
    }

    pub(super) fn previous(&self) -> &Token<'a> {
        &self.tokens[self.current - 1]
    }

    pub(super) fn error(
        &self,
        span: SourceSpan<'a>,
        code: &'static str,
        message: impl Into<String>,
    ) -> CompileDiagnostic<'a> {
        CompileDiagnostic::new(code, span, message)
    }
}
