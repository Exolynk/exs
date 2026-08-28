//! Phase-1 `ExS` abstract syntax tree.

use crate::diagnostic::SourceSpan;

/// A parsed `ExS` source unit.
#[derive(Debug, Clone)]
pub struct Module<'a> {
    /// Compile-time source-file imports in declaration order.
    pub imports: Vec<ImportDeclaration<'a>>,
    /// Compile-time aliases for imported declarations.
    pub uses: Vec<UseDeclaration<'a>>,
    /// Named nominal Object type declarations.
    pub types: Vec<TypeDeclaration<'a>>,
    /// Named nominal enum declarations.
    pub enums: Vec<EnumDeclaration<'a>>,
    /// Named trait declarations.
    pub traits: Vec<TraitDeclaration<'a>>,
    /// Type-specific direct method declarations.
    pub implementations: Vec<ImplDeclaration<'a>>,
    /// Top-level direct function declarations.
    pub functions: Vec<FunctionDeclaration<'a>>,
    /// Source test declarations omitted from normal executable modules.
    pub tests: Vec<TestDeclaration<'a>>,
}

/// One relative source-file import.
#[derive(Debug, Clone)]
pub struct ImportDeclaration<'a> {
    /// Source-spelled relative import path.
    pub path: String,
    /// Optional explicit namespace replacing the file-stem default.
    pub alias: Option<Identifier<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// One declaration introduced by a `use` declaration.
#[derive(Debug, Clone)]
pub struct UseItem<'a> {
    /// Imported declaration name.
    pub name: Identifier<'a>,
    /// Optional local alias.
    pub alias: Option<Identifier<'a>>,
}

/// One compile-time `use` declaration.
#[derive(Debug, Clone)]
pub struct UseDeclaration<'a> {
    /// Imported namespace being referenced.
    pub namespace: Identifier<'a>,
    /// Declarations introduced into the local module scope.
    pub items: Vec<UseItem<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// A nominal Object type with declared field contracts.
#[derive(Debug, Clone)]
pub struct TypeDeclaration<'a> {
    /// The source-visible type name.
    pub name: Identifier<'a>,
    /// Fields in source order.
    pub fields: Vec<TypeField<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// One named field of a nominal Object type.
#[derive(Debug, Clone)]
pub struct TypeField<'a> {
    /// The source-visible field name.
    pub name: Identifier<'a>,
    /// Optional accepted type union. An omitted annotation means `Any`.
    pub type_annotation: Option<TypeAnnotation<'a>>,
    /// Full field declaration span.
    pub span: SourceSpan<'a>,
}

/// A nominal tagged union with named variants.
#[derive(Debug, Clone)]
pub struct EnumDeclaration<'a> {
    /// The source-visible enum name.
    pub name: Identifier<'a>,
    /// Variants in source order.
    pub variants: Vec<EnumVariant<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// One enum variant with zero or more ordered payload fields.
#[derive(Debug, Clone)]
pub struct EnumVariant<'a> {
    /// The source-visible variant name.
    pub name: Identifier<'a>,
    /// Constructor payload fields in source order.
    pub fields: Vec<TypeField<'a>>,
    /// Full variant declaration span.
    pub span: SourceSpan<'a>,
}

/// The inherent or trait-provided methods associated with one nominal type.
#[derive(Debug, Clone)]
pub struct ImplDeclaration<'a> {
    /// Implemented trait name, or None for an inherent implementation block.
    pub trait_name: Option<Identifier<'a>>,
    /// The type receiving these methods.
    pub type_name: Identifier<'a>,
    /// Methods in source order.
    pub methods: Vec<FunctionDeclaration<'a>>,
    /// Full implementation span.
    pub span: SourceSpan<'a>,
}

/// One named trait with required and default methods.
#[derive(Debug, Clone)]
pub struct TraitDeclaration<'a> {
    /// The source-visible trait name.
    pub name: Identifier<'a>,
    /// Required and default trait methods in source order.
    pub methods: Vec<TraitMethodDeclaration<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

/// One required signature or default implementation declared by a trait.
#[derive(Debug, Clone)]
pub struct TraitMethodDeclaration<'a> {
    /// Source-visible method name.
    pub name: Identifier<'a>,
    /// Positional parameters with optional type annotations.
    pub parameters: Vec<Parameter<'a>>,
    /// Optional union type annotation for the returned value.
    pub return_type: Option<TypeAnnotation<'a>>,
    /// Default body, or None when every implementation must provide this method.
    pub body: Option<Block<'a>>,
    /// Full declaration span.
    pub span: SourceSpan<'a>,
}

impl<'a> TraitMethodDeclaration<'a> {
    /// Creates a concrete inherited method from this default declaration.
    #[must_use]
    pub fn default_implementation(&self) -> Option<FunctionDeclaration<'a>> {
        self.body.clone().map(|body| FunctionDeclaration {
            visibility: FunctionVisibility::Private,
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            return_type: self.return_type.clone(),
            body,
            span: self.span,
        })
    }
}

/// External visibility of one top-level function declaration.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FunctionVisibility {
    /// The function is available only to ExS code in the linked module.
    Private,
    /// The function receives a runner-callable entry wrapper when it is in the root module.
    Public,
}

/// A named function declaration.
#[derive(Debug, Clone)]
pub struct FunctionDeclaration<'a> {
    /// Whether this top-level function is callable by an external runner.
    pub visibility: FunctionVisibility,
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

/// One named zero-argument source test.
#[derive(Debug, Clone)]
pub struct TestDeclaration<'a> {
    /// Human-readable test description.
    pub description: String,
    /// Test body executed by the test command.
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
#[derive(Debug, Clone)]
pub struct Parameter<'a> {
    /// Parameter binding name.
    pub name: Identifier<'a>,
    /// Optional declared accepted value types.
    pub type_annotation: Option<TypeAnnotation<'a>>,
    /// Whether this trailing parameter receives every remaining call argument as a List.
    pub variadic: bool,
}

/// One optional function-boundary union type annotation.
#[derive(Debug, Clone)]
pub struct TypeAnnotation<'a> {
    /// Source-spelled type members in union order.
    pub members: Vec<TypeName<'a>>,
    /// Full source span of the annotation excluding `:` and `->`.
    pub span: SourceSpan<'a>,
}

/// One source-spelled member of a union type annotation.
#[derive(Debug, Clone)]
pub struct TypeName<'a> {
    /// Type name spelling.
    pub name: String,
    /// Optional recursive generic argument, currently supported by `List<T>`.
    pub argument: Option<Box<TypeAnnotation<'a>>>,
    /// Source span of this type name.
    pub span: SourceSpan<'a>,
}

/// A statement block.
#[derive(Debug, Clone)]
pub struct Block<'a> {
    /// Statements in source order.
    pub statements: Vec<Statement<'a>>,
    /// Full block span.
    pub span: SourceSpan<'a>,
}

/// A Phase-1 statement.
#[allow(dead_code)] // Every statement keeps its span for later source-map emission.
#[derive(Debug, Clone)]
pub enum Statement<'a> {
    /// A local declaration.
    Let {
        /// Declared name.
        name: Identifier<'a>,
        /// Optional type used to decode a host-boundary value at this binding.
        type_annotation: Option<TypeAnnotation<'a>>,
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
    /// A standalone lexical scope.
    Block {
        /// Statements evaluated within the scope.
        block: Block<'a>,
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
        else_branch: Option<ElseBranch<'a>>,
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
        /// Optional type used to decode each host-boundary iteration value.
        type_annotation: Option<TypeAnnotation<'a>>,
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

/// A conditional statement's false path.
#[derive(Debug, Clone)]
pub enum ElseBranch<'a> {
    /// A final block selected when every preceding condition is false.
    Block(Block<'a>),
    /// A nested conditional selected when the preceding condition is false.
    If(Box<Statement<'a>>),
}

/// A source location that can receive an assignment.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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

/// One arm of a `match` expression.
#[derive(Debug, Clone)]
pub struct MatchArm<'a> {
    /// Variant or fallback pattern selecting this arm.
    pub pattern: MatchPattern<'a>,
    /// Expression or statement block evaluated when the pattern matches.
    pub body: MatchArmBody<'a>,
    /// Full arm span.
    pub span: SourceSpan<'a>,
}

/// The executable body selected by one `match` arm.
#[derive(Debug, Clone)]
pub enum MatchArmBody<'a> {
    /// A value-producing expression.
    Expression(Expression<'a>),
    /// A normal statement block whose ordinary completion yields None.
    Block(Block<'a>),
}

/// One enum-variant or wildcard `match` pattern.
#[derive(Debug, Clone)]
pub enum MatchPattern<'a> {
    /// A qualified enum variant and its ordered payload bindings.
    Variant {
        /// Qualified enum type name.
        type_name: Identifier<'a>,
        /// Selected enum variant name.
        variant: Identifier<'a>,
        /// Payload bindings in declaration order.
        bindings: Vec<Identifier<'a>>,
        /// Full pattern span.
        span: SourceSpan<'a>,
    },
    /// The fallback pattern matching every remaining value.
    Wildcard(SourceSpan<'a>),
}

/// A Phase-1 expression.
#[derive(Debug, Clone)]
pub enum Expression<'a> {
    /// An integer literal magnitude, validated as an i64 during lowering.
    Integer(i128, SourceSpan<'a>),
    /// A binary64 floating-point literal.
    Float(f64, SourceSpan<'a>),
    /// A decoded UTF-8 string literal.
    String(String, SourceSpan<'a>),
    /// A raw-octet literal encoded from UTF-8 source bytes.
    Bytes(String, SourceSpan<'a>),
    /// A string assembled from literal and runtime expression fragments.
    FormattedString {
        /// Delimiter rules preserved for canonical source formatting.
        kind: FormattedStringKind,
        /// Source-order literal and expression fragments.
        parts: Vec<FormattedStringPart<'a>>,
        /// Full formatted literal span.
        span: SourceSpan<'a>,
    },
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
    /// Selects one expression arm by an enum variant or fallback pattern.
    Match {
        /// Value evaluated exactly once before arm selection.
        value: Box<Expression<'a>>,
        /// Source-order variant and fallback arms.
        arms: Vec<MatchArm<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A local variable lookup.
    Variable(Identifier<'a>),
    /// An anonymous callable value with a lexical body.
    Closure {
        /// Positional parameters introduced for the closure body.
        parameters: Vec<Parameter<'a>>,
        /// Closure body evaluated when the callable is invoked.
        body: Block<'a>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A fixed source-order set of zero-argument expressions evaluated as parallel tasks.
    ParallelStatic {
        /// Compiler-synthesized zero-argument closure task expressions.
        tasks: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// A runtime List of zero-argument callable values evaluated as parallel tasks.
    ParallelDynamic {
        /// Expression expected to evaluate to a List of callable values.
        functions: Box<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
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
    /// A dynamically resolved host invocation.
    ///
    /// The name expression must evaluate to String at runtime. Every argument is transported in
    /// source order across the host CBOR boundary.
    HostCall {
        /// Runtime expression that supplies the registered host-function name.
        name: Box<Expression<'a>>,
        /// Positional arguments passed to the resolved host function.
        arguments: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// Opens one runner-owned Host stream through the compiler-reserved Host ABI operation.
    HostStream {
        /// Stream-factory name followed by ordered factory arguments.
        arguments: Vec<Expression<'a>>,
        /// Full expression span.
        span: SourceSpan<'a>,
    },
    /// Invokes one compiler-owned runner time operation that constructs a nominal prelude value.
    HostTime {
        /// Built-in time operation selected by the parsed Host member name.
        operation: HostTimeOperation,
        /// Ordered arguments passed to the built-in runner operation.
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

/// Runner-owned Host time operations that construct nominal prelude values after resumption.
#[derive(Debug, Clone, Copy)]
pub enum HostTimeOperation {
    /// Captures the current wall-clock DateTime snapshot.
    Now,
    /// Returns monotonic Duration elapsed since root execution start.
    Elapsed,
    /// Renders one instant in a named IANA time zone.
    InTimezone,
    /// Resolves one civil DateTime in a named IANA time zone with compatible DST handling.
    FromComponents,
}

/// The source delimiter form selected for a formatted string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormattedStringKind {
    /// `f"..."` with ordinary escape decoding.
    Standard,
    /// `f#"..."#` with raw hash delimiters.
    Raw,
    /// `fd#"..."#` with raw hash delimiters and common indentation removal.
    Dedented,
}

/// One fragment in a formatted string expression.
#[derive(Debug, Clone)]
pub enum FormattedStringPart<'a> {
    /// A decoded static text fragment.
    Text(String),
    /// A runtime expression interpolated with String-add conversion rules.
    Expression(Expression<'a>),
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
    /// Numeric division.
    Divide,
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
