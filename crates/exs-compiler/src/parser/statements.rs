use super::*;

impl<'a> Parser<'a> {
    pub(super) fn block(&mut self) -> Result<Block<'a>, CompileDiagnostic<'a>> {
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

    pub(super) fn statement(&mut self) -> Result<Statement<'a>, CompileDiagnostic<'a>> {
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
            let type_annotation = if self.matches(&TokenKind::Colon) {
                Some(self.type_annotation()?)
            } else {
                None
            };
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
                type_annotation,
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
            let type_annotation = if self.matches(&TokenKind::Colon) {
                Some(self.type_annotation()?)
            } else {
                None
            };
            self.expect_simple(TokenKind::In, "expected in after for-loop binding")?;
            let iterable = self.expression()?;
            let body = self.block()?;
            return Ok(Statement::For {
                binding,
                type_annotation,
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
}
