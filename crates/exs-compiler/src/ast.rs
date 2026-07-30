//! Phase-1 `ExS` abstract syntax tree.

use crate::diagnostic::SourceSpan;

/// A parsed `ExS` source unit.
#[derive(Debug)]
pub struct Module<'a> {
    /// Named nominal Object type declarations.
    pub types: Vec<TypeDeclaration<'a>>,
    /// Type-specific direct method declarations.
    pub implementations: Vec<ImplDeclaration<'a>>,
    /// Top-level direct function declarations.
    pub functions: Vec<FunctionDeclaration<'a>>,
}

/// A nominal Object type with declared field contracts.
#[derive(Debug)]
pub struct TypeDeclaration<'a> {
    /// The source-visible type name.
    pub name: Identifier<'a>,
    /// Fields in source order.
    pub fields: Vec<TypeField<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// One named field of a nominal Object type.
#[derive(Debug)]
pub struct TypeField<'a> {
    /// The source-visible field name.
    pub name: Identifier<'a>,
    /// Optional accepted type union. An omitted annotation means `Any`.
    pub type_annotation: Option<TypeAnnotation<'a>>,
    /// Full field declaration span.
    pub span: SourceSpan<'a>,
}

/// The methods associated with one nominal Object type.
#[derive(Debug)]
pub struct ImplDeclaration<'a> {
    /// The type receiving these methods.
    pub type_name: Identifier<'a>,
    /// Methods in source order.
    pub methods: Vec<FunctionDeclaration<'a>>,
    /// Full implementation span.
    pub span: SourceSpan<'a>,
}

/// A named function declaration.
#[derive(Debug)]
pub struct FunctionDeclaration<'a> {
    /// Function name.
    pub name: Identifier<'a>,
    /// Positional parameters with optional type annotations.
    pub parameters: Vec<Parameter<'a>>,
    /// Optional union type annotation for the returned value.
    pub return_type: Option<TypeAnnotation<'a>>,
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

/// One named function parameter with an optional type annotation.
#[derive(Debug)]
pub struct Parameter<'a> {
    /// Parameter binding name.
    pub name: Identifier<'a>,
    /// Optional declared accepted value types.
    pub type_annotation: Option<TypeAnnotation<'a>>,
}

/// One optional function-boundary union type annotation.
#[derive(Debug)]
pub struct TypeAnnotation<'a> {
    /// Source-spelled type members in union order.
    pub members: Vec<TypeName<'a>>,
    /// Full source span of the annotation excluding `:` and `->`.
    pub span: SourceSpan<'a>,
}

/// One source-spelled member of a union type annotation.
#[derive(Debug)]
pub struct TypeName<'a> {
    /// Type name spelling.
    pub name: String,
    /// Source span of this type name.
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
        /// Assigned storage location.
        target: AssignmentTarget<'a>,
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
    /// A conditionally repeated statement block.
    While {
        /// Expression evaluated before every iteration.
        condition: Expression<'a>,
        /// Repeated while the condition evaluates to true.
        body: Block<'a>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// A repeated statement block over one runtime iterable snapshot.
    For {
        /// Binding introduced for the current iteration value.
        binding: Identifier<'a>,
        /// Expression evaluated once before iteration begins.
        iterable: Expression<'a>,
        /// Repeated once for each snapshot value.
        body: Block<'a>,
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// Exits the nearest enclosing loop.
    Break {
        /// Full statement span.
        span: SourceSpan<'a>,
    },
    /// Advances the nearest enclosing loop.
    Continue {
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

/// A source location that can receive an assignment.
#[derive(Debug)]
pub enum AssignmentTarget<'a> {
    /// A local binding.
    Variable(Identifier<'a>),
    /// A dynamically indexed runtime value.
    Index {
        /// The value to mutate.
        receiver: Box<Expression<'a>>,
        /// The runtime index or key.
        index: Box<Expression<'a>>,
        /// Full target span.
        span: SourceSpan<'a>,
    },
    /// A statically named runtime property.
    Property {
        /// The value to mutate.
        receiver: Box<Expression<'a>>,
        /// The property name.
        property: Identifier<'a>,
        /// Full target span.
        span: SourceSpan<'a>,
    },
}

/// One statically named property in an object literal.
#[derive(Debug)]
pub struct ObjectProperty<'a> {
    /// Decoded property key.
    pub key: String,
    /// Property-key source span.
    pub key_span: SourceSpan<'a>,
    /// Property value expression.
    pub value: Expression<'a>,
    /// Full property span.
    pub span: SourceSpan<'a>,
}

/// A Phase-1 expression.
#[derive(Debug)]
pub enum Expression<'a> {
    /// An integer literal.
    Integer(i64, SourceSpan<'a>),
    /// A binary64 floating-point literal.
    Float(f64, SourceSpan<'a>),
    /// A decoded UTF-8 string literal.
    String(String, SourceSpan<'a>),
    /// A boolean literal.
    Bool(bool, SourceSpan<'a>),
    /// The absence value shared by Options and empty operations.
    None(SourceSpan<'a>),
    /// Tests whether one value is a language Error.
    IsError {
        /// Value being tested.
        value: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// Propagates an Error or converts None into MissingValue.
    Propagate {
        /// Option or Result value being propagated.
        value: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A mutable list literal.
    List {
        /// Elements evaluated from left to right.
        elements: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A mutable object literal with statically named properties.
    Object {
        /// Properties evaluated from left to right.
        properties: Vec<ObjectProperty<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A nominal Object construction with statically named properties.
    TypedObject {
        /// The constructed nominal type.
        type_name: Identifier<'a>,
        /// Properties evaluated from left to right.
        properties: Vec<ObjectProperty<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
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
    /// A dynamically dispatched member call.
    MethodCall {
        /// Receiver evaluated before arguments.
        receiver: Box<Expression<'a>>,
        /// Statically written method name.
        method: Identifier<'a>,
        /// Positional arguments evaluated from left to right.
        arguments: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A direct static method call on one nominal Object type.
    StaticMethodCall {
        /// The type owning the static method.
        type_name: Identifier<'a>,
        /// Statically written method name.
        method: Identifier<'a>,
        /// Positional arguments.
        arguments: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A dynamically dispatched index lookup.
    Index {
        /// Indexed runtime value.
        receiver: Box<Expression<'a>>,
        /// Runtime index or key.
        index: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A statically named runtime property lookup.
    Property {
        /// Value that owns the property.
        receiver: Box<Expression<'a>>,
        /// Property name.
        property: Identifier<'a>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
}

/// A unary operator supported in Phase 1.
#[derive(Debug, Clone, Copy)]
pub enum UnaryOperator {
    /// Numeric negation.
    Negate,
    /// Boolean negation.
    Not,
}

/// A binary operator supported in Phase 1.
#[derive(Debug, Clone, Copy)]
pub enum BinaryOperator {
    /// Numeric addition.
    Add,
    /// Numeric subtraction.
    Subtract,
    /// Numeric multiplication.
    Multiply,
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Numeric less-than comparison.
    LessThan,
    /// Numeric less-or-equal comparison.
    LessOrEqual,
    /// Numeric greater-than comparison.
    GreaterThan,
    /// Numeric greater-or-equal comparison.
    GreaterOrEqual,
    /// Short-circuiting boolean conjunction.
    And,
    /// Short-circuiting boolean disjunction.
    Or,
}
