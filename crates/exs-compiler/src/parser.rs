//! Recursive-descent parser for the Phase-1 `ExS` grammar.

use crate::ast::{
    BinaryOperator, Block, Expression, FunctionDeclaration, Identifier, Module, Statement,
    UnaryOperator,
};
use crate::diagnostic::{CompileDiagnostic, CompileDiagnostics, SourceSpan};
use crate::lexer::{Token, TokenKind};

/// Parses a token stream into a function-only `ExS` module.
pub fn parse<'a>(
    source_id: &'a str,
    tokens: Vec<Token<'a>>,
) -> Result<Module<'a>, CompileDiagnostics<'a>> {
    let mut parser = Parser { tokens, current: 0 };
    let mut functions = Vec::new();
    while !parser.at_end() {
        functions.push(parser.function().map_err(CompileDiagnostics::from)?);
    }
    if functions.is_empty() {
        return Err(CompileDiagnostics::from(CompileDiagnostic::new(
            "E0100",
            SourceSpan::empty(source_id),
            "a module must declare fn main()",
        )));
    }
    Ok(Module { functions })
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    current: usize,
}

impl<'a> Parser<'a> {
    fn function(&mut self) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Fn, "expected `fn` at module level")?
            .span;
        let name = self.identifier("expected function name")?;
        self.expect_simple(TokenKind::LeftParen, "expected `(` after function name")?;
        let mut parameters = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                parameters.push(self.identifier("expected parameter name")?);
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        self.expect_simple(TokenKind::RightParen, "expected `)` after parameters")?;
        let body = self.block()?;
        Ok(FunctionDeclaration {
            name,
            parameters,
            span: start.through(body.span),
            body,
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
        if matches!(self.peek().kind, TokenKind::Identifier(_))
            && matches!(self.peek_next().kind, TokenKind::Equal)
        {
            let name = self.identifier("expected assignment target")?;
            let start = name.span;
            self.advance();
            let value = self.expression()?;
            let end = self
                .expect_simple(TokenKind::Semicolon, "expected `;` after assignment")?
                .span;
            return Ok(Statement::Assign {
                name,
                value,
                span: start.through(end),
            });
        }
        let expression = self.expression()?;
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
        self.binary(
            Self::term,
            &[
                (TokenKind::Less, BinaryOperator::LessThan),
                (TokenKind::LessEqual, BinaryOperator::LessOrEqual),
                (TokenKind::Greater, BinaryOperator::GreaterThan),
                (TokenKind::GreaterEqual, BinaryOperator::GreaterOrEqual),
            ],
        )
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
        while self.matches(&TokenKind::LeftParen) {
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
            let mut arguments = Vec::new();
            if !self.check(&TokenKind::RightParen) {
                loop {
                    arguments.push(self.expression()?);
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                    if self.check(&TokenKind::RightParen) {
                        break;
                    }
                }
            }
            let close = self
                .expect_simple(TokenKind::RightParen, "expected `)` after arguments")?
                .span;
            expression = Expression::Call {
                callee,
                arguments,
                span: open.through(close),
            };
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

    fn peek_next(&self) -> &Token<'a> {
        self.tokens.get(self.current + 1).unwrap_or(self.peek())
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
        | Expression::Bool(_, span) => *span,
        Expression::Variable(identifier) => identifier.span,
        Expression::Unary { span, .. }
        | Expression::Binary { span, .. }
        | Expression::Call { span, .. } => *span,
    }
}
