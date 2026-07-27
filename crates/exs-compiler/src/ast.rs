//! Phase-1 `ExS` abstract syntax tree.

use crate::diagnostic::SourceSpan;

/// A parsed `ExS` source unit.
#[derive(Debug)]
pub struct Module<'a> {
    /// Top-level Phase-1 function items.
    pub functions: Vec<FunctionDeclaration<'a>>,
}

/// A named function declaration.
#[derive(Debug)]
pub struct FunctionDeclaration<'a> {
    /// Function name.
    pub name: Identifier<'a>,
    /// Positional parameter names.
    pub parameters: Vec<Identifier<'a>>,
    /// Function body.
    pub body: Block<'a>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// A source identifier.
#[derive(Debug, Clone)]
pub struct Identifier<'a> {
    /// Identifier spelling.
    pub name: String,
    /// Identifier source span.
    pub span: SourceSpan<'a>,
}

/// A statement block.
#[derive(Debug)]
pub struct Block<'a> {
    /// Statements in source order.
    pub statements: Vec<Statement<'a>>,
    /// Full block span.
    pub span: SourceSpan<'a>,
}

/// A Phase-1 statement.
#[allow(dead_code)] // Every statement keeps its span for later source-map emission.
#[derive(Debug)]
pub enum Statement<'a> {
    /// A local declaration.
    Let {
        /// Declared name.
        name: Identifier<'a>,
        /// Initializer expression.
        value: Expression<'a>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// A local binding assignment.
    Assign {
        /// Assigned name.
        name: Identifier<'a>,
        /// Assigned expression.
        value: Expression<'a>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// A function return.
    Return {
        /// Returned expression, if present.
        value: Option<Expression<'a>>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// A conditional statement.
    If {
        /// Boolean condition.
        condition: Expression<'a>,
        /// Selected when true.
        then_block: Block<'a>,
        /// Selected when false.
        else_block: Option<Block<'a>>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// An expression evaluated for side effects.
    Expression {
        /// Expression to evaluate.
        expression: Expression<'a>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
}

/// A Phase-1 expression.
#[derive(Debug)]
pub enum Expression<'a> {
    /// An integer literal.
    Integer(i64, SourceSpan<'a>),
    /// A boolean literal.
    Bool(bool, SourceSpan<'a>),
    /// A local variable lookup.
    Variable(Identifier<'a>),
    /// A unary operation.
    Unary {
        /// Operation kind.
        operator: UnaryOperator,
        /// Operand expression.
        operand: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A binary operation.
    Binary {
        /// Operation kind.
        operator: BinaryOperator,
        /// Left operand.
        left: Box<Expression<'a>>,
        /// Right operand.
        right: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A direct named function call.
    Call {
        /// Called function name.
        callee: Identifier<'a>,
        /// Positional arguments.
        arguments: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
}

/// A unary operator supported in Phase 1.
#[derive(Debug, Clone, Copy)]
pub enum UnaryOperator {
    /// Integer negation.
    Negate,
    /// Boolean negation.
    Not,
}

/// A binary operator supported in Phase 1.
#[derive(Debug, Clone, Copy)]
pub enum BinaryOperator {
    /// Integer addition.
    Add,
    /// Integer subtraction.
    Subtract,
    /// Integer multiplication.
    Multiply,
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Integer less-than comparison.
    LessThan,
    /// Integer less-or-equal comparison.
    LessOrEqual,
    /// Integer greater-than comparison.
    GreaterThan,
    /// Integer greater-or-equal comparison.
    GreaterOrEqual,
    /// Short-circuiting boolean conjunction.
    And,
    /// Short-circuiting boolean disjunction.
    Or,
}
