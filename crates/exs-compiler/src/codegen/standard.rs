//! Compiler-owned declarations for globally available standard protocols.

use exs_abi::{TYPE_ANY, TYPE_BOOL, TYPE_FLOAT, TYPE_INT, TYPE_LIST, TYPE_STRING};

/// Canonical source name for the protocol invoked by the `+` operator.
pub(crate) const ADD_TRAIT: &str = "Add";

/// Required instance-method name for the `Add` protocol.
pub(crate) const ADD_METHOD: &str = "add";

/// Canonical source name for the protocol invoked by the `-` operator.
pub(crate) const SUB_TRAIT: &str = "Sub";

/// Required instance-method name for the `Sub` protocol.
pub(crate) const SUB_METHOD: &str = "sub";

/// Canonical source name for the protocol invoked by the `*` operator.
pub(crate) const MUL_TRAIT: &str = "Mul";

/// Required instance-method name for the `Mul` protocol.
pub(crate) const MUL_METHOD: &str = "mul";

/// Canonical source name for the protocol invoked by the `/` operator.
pub(crate) const DIV_TRAIT: &str = "Div";

/// Required instance-method name for the `Div` protocol.
pub(crate) const DIV_METHOD: &str = "div";

/// Canonical source name for the protocol invoked by equality and ordering operators.
pub(crate) const COMPARE_TRAIT: &str = "Compare";

/// Required instance-method name for the `Compare` protocol.
pub(crate) const COMPARE_METHOD: &str = "compare";

/// Canonical source name for the standard comparison-result enum.
pub(crate) const ORDERING_ENUM: &str = "Ordering";

/// One source operator whose implementation is selected through a standard trait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StandardOperator {
    /// The binary `+` operator.
    Add,
    /// The binary `-` operator.
    Subtract,
    /// The binary `*` operator.
    Multiply,
    /// The binary `/` operator.
    Divide,
    /// The binary `==` operator.
    Equal,
    /// The binary `!=` operator.
    NotEqual,
    /// The binary `<` operator.
    LessThan,
    /// The binary `<=` operator.
    LessOrEqual,
    /// The binary `>` operator.
    GreaterThan,
    /// The binary `>=` operator.
    GreaterOrEqual,
}

/// Describes one compiler-owned trait alongside its built-in implementations.
pub(crate) struct StandardTraitDescriptor {
    /// Canonical source-visible trait name.
    pub(crate) name: &'static str,
    /// Built-in runtime categories that satisfy this trait contract.
    pub(crate) builtin_mask: u32,
    /// Required methods supplied by every nominal implementation.
    pub(crate) methods: &'static [StandardMethodDescriptor],
    /// Source operators supplied by one of this trait's methods.
    pub(crate) operators: &'static [StandardOperator],
    /// Overview rendered on the standard trait API page.
    pub(crate) description: &'static str,
    /// Usage example rendered on the standard trait API page.
    pub(crate) usage: &'static str,
    /// Built-in type names rendered as standard trait implementations.
    pub(crate) implemented_by: &'static [&'static str],
}

/// Describes one compiler-owned enum and its zero-payload variants.
pub(crate) struct StandardEnumDescriptor {
    /// Canonical source-visible enum name.
    pub(crate) name: &'static str,
    /// Source-visible zero-payload variant names.
    pub(crate) variants: &'static [&'static str],
    /// Overview rendered on the standard enum API page.
    pub(crate) description: &'static str,
    /// Usage example rendered on the standard enum API page.
    pub(crate) usage: &'static str,
}

/// Describes one method required by a compiler-owned standard trait.
pub(crate) struct StandardMethodDescriptor {
    /// Source-visible method name.
    pub(crate) name: &'static str,
    /// Whether the first parameter is the bare `self` receiver.
    pub(crate) has_receiver: bool,
    /// Required parameter annotations, including the receiver when present.
    pub(crate) parameter_types: &'static [Option<&'static str>],
    /// Required ordered return union members.
    pub(crate) return_types: &'static [&'static str],
    /// Display signature rendered in documentation and diagnostics.
    pub(crate) signature: &'static str,
    /// Behavior inherited by an implementation method without its own documentation.
    pub(crate) description: &'static str,
}

/// Parameter annotations required by the standard `Add::add` method.
const ADD_PARAMETER_TYPES: [Option<&str>; 2] = [None, Some("Any")];

/// Return annotation required by the standard `Add::add` method.
const ADD_RETURN_TYPES: [&str; 1] = ["Any"];

/// Parameter annotations required by standard binary arithmetic trait methods.
const ARITHMETIC_PARAMETER_TYPES: [Option<&str>; 2] = [None, Some("Any")];

/// Return annotation required by standard binary arithmetic trait methods.
const ARITHMETIC_RETURN_TYPES: [&str; 1] = ["Any"];

/// Parameter annotations required by the standard `Compare::compare` method.
const COMPARE_PARAMETER_TYPES: [Option<&str>; 2] = [None, Some("Any")];

/// Return annotation required by the standard `Compare::compare` method.
const COMPARE_RETURN_TYPES: [&str; 1] = [ORDERING_ENUM];

/// The arithmetic source operator bound to `Add::add`.
const ADD_OPERATORS: [StandardOperator; 1] = [StandardOperator::Add];
/// The arithmetic source operator bound to `Sub::sub`.
const SUB_OPERATORS: [StandardOperator; 1] = [StandardOperator::Subtract];
/// The arithmetic source operator bound to `Mul::mul`.
const MUL_OPERATORS: [StandardOperator; 1] = [StandardOperator::Multiply];
/// The arithmetic source operator bound to `Div::div`.
const DIV_OPERATORS: [StandardOperator; 1] = [StandardOperator::Divide];
/// Source operators bound to `Compare::compare`.
const COMPARE_OPERATORS: [StandardOperator; 6] = [
    StandardOperator::Equal,
    StandardOperator::NotEqual,
    StandardOperator::LessThan,
    StandardOperator::LessOrEqual,
    StandardOperator::GreaterThan,
    StandardOperator::GreaterOrEqual,
];

/// The required `Add::add` method declaration.
const ADD_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: ADD_METHOD,
    has_receiver: true,
    parameter_types: &ADD_PARAMETER_TYPES,
    return_types: &ADD_RETURN_TYPES,
    signature: "fn add(self, other: Any) -> Any;",
    description: "Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.",
}];

/// The required `Sub::sub` method declaration.
const SUB_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: SUB_METHOD,
    has_receiver: true,
    parameter_types: &ARITHMETIC_PARAMETER_TYPES,
    return_types: &ARITHMETIC_RETURN_TYPES,
    signature: "fn sub(self, other: Any) -> Any;",
    description: "Subtracts the evaluated `other` operand from the receiver. Implementations may return any ExS value, including a recoverable Error. The `-` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.sub(other)`.",
}];

/// The required `Mul::mul` method declaration.
const MUL_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: MUL_METHOD,
    has_receiver: true,
    parameter_types: &ARITHMETIC_PARAMETER_TYPES,
    return_types: &ARITHMETIC_RETURN_TYPES,
    signature: "fn mul(self, other: Any) -> Any;",
    description: "Multiplies the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `*` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.mul(other)`.",
}];

/// The required `Div::div` method declaration.
const DIV_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: DIV_METHOD,
    has_receiver: true,
    parameter_types: &ARITHMETIC_PARAMETER_TYPES,
    return_types: &ARITHMETIC_RETURN_TYPES,
    signature: "fn div(self, other: Any) -> Any;",
    description: "Divides the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `/` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.div(other)`. Built-in division always returns Float and follows IEEE 754 behavior for zero divisors.",
}];

/// The required `Compare::compare` method declaration.
const COMPARE_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: COMPARE_METHOD,
    has_receiver: true,
    parameter_types: &COMPARE_PARAMETER_TYPES,
    return_types: &COMPARE_RETURN_TYPES,
    signature: "fn compare(self, other: Any) -> Ordering;",
    description: "Compares the receiver with the evaluated `other` operand and returns `Ordering::Less`, `Ordering::Equal`, `Ordering::Greater`, or `Ordering::Unordered`. Equality uses only the Equal result. Ordering operators reject Unordered with TypeError.",
}];

/// The standard `Add` trait declaration shared by validation, contracts, and documentation.
const STANDARD_TRAITS: [StandardTraitDescriptor; 5] = [
    StandardTraitDescriptor {
        name: ADD_TRAIT,
        builtin_mask: TYPE_BOOL | TYPE_INT | TYPE_FLOAT | TYPE_STRING | TYPE_LIST,
        methods: &ADD_METHODS,
        operators: &ADD_OPERATORS,
        description: "`Add` is the protocol selected by `left + right` when the left operand is a nominal type or enum with an `impl Add` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, Float, String, and List values implement the same protocol, so `value.add(other)` and `value + other` have identical behavior. String receivers concatenate String, Bool, Int, and Float operands using their normal source spelling.",
        usage: "type Vector { value: Int }\n\nimpl Add for Vector {\n    fn add(self, other: Any) -> Any {\n        ret Vector { value: self.value + other.value };\n    }\n}\n\nfn main() -> Int {\n    ret (Vector { value: 20 } + Vector { value: 22 }).value;\n}",
        implemented_by: &["Bool", "Int", "Float", "String", "List"],
    },
    StandardTraitDescriptor {
        name: SUB_TRAIT,
        builtin_mask: TYPE_BOOL | TYPE_INT | TYPE_FLOAT,
        methods: &SUB_METHODS,
        operators: &SUB_OPERATORS,
        description: "`Sub` is the protocol selected by `left - right` when the left operand is a nominal type or enum with an `impl Sub` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.sub(other)` and `value - other` have identical behavior.",
        usage: "type Temperature { value: Float }\n\nimpl Sub for Temperature {\n    fn sub(self, other: Any) -> Any {\n        ret Temperature { value: self.value - other.value };\n    }\n}\n\nfn main() -> Float {\n    ret (Temperature { value: 22.5 } - Temperature { value: 2.5 }).value;\n}",
        implemented_by: &["Bool", "Int", "Float"],
    },
    StandardTraitDescriptor {
        name: MUL_TRAIT,
        builtin_mask: TYPE_BOOL | TYPE_INT | TYPE_FLOAT,
        methods: &MUL_METHODS,
        operators: &MUL_OPERATORS,
        description: "`Mul` is the protocol selected by `left * right` when the left operand is a nominal type or enum with an `impl Mul` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.mul(other)` and `value * other` have identical behavior.",
        usage: "type Scale { value: Int }\n\nimpl Mul for Scale {\n    fn mul(self, other: Any) -> Any {\n        ret Scale { value: self.value * other.value };\n    }\n}\n\nfn main() -> Int {\n    ret (Scale { value: 6 } * Scale { value: 7 }).value;\n}",
        implemented_by: &["Bool", "Int", "Float"],
    },
    StandardTraitDescriptor {
        name: DIV_TRAIT,
        builtin_mask: TYPE_BOOL | TYPE_INT | TYPE_FLOAT,
        methods: &DIV_METHODS,
        operators: &DIV_OPERATORS,
        description: "`Div` is the protocol selected by `left / right` when the left operand is a nominal type or enum with an `impl Div` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.div(other)` and `value / other` have identical behavior. Built-in division always returns Float and follows IEEE 754 behavior for zero divisors.",
        usage: "type Ratio { value: Float }\n\nimpl Div for Ratio {\n    fn div(self, other: Any) -> Any {\n        ret Ratio { value: self.value / other.value };\n    }\n}\n\nfn main() -> Float {\n    ret (Ratio { value: 84.0 } / Ratio { value: 2.0 }).value;\n}",
        implemented_by: &["Bool", "Int", "Float"],
    },
    StandardTraitDescriptor {
        name: COMPARE_TRAIT,
        builtin_mask: TYPE_ANY,
        methods: &COMPARE_METHODS,
        operators: &COMPARE_OPERATORS,
        description: "`Compare` is the protocol selected by equality and ordering operators. Its fixed Ordering result lets one implementation define equal, less-than, and greater-than behavior coherently. Built-in values preserve ExS equality semantics: numeric and String values are ordered, while identity-only values return Unordered unless they are the same value.",
        usage: "type Version { major: Int, minor: Int }\n\nimpl Compare for Version {\n    fn compare(self, other: Any) -> Ordering {\n        if self.major < other.major { ret Ordering::Less; }\n        if self.major > other.major { ret Ordering::Greater; }\n        if self.minor < other.minor { ret Ordering::Less; }\n        if self.minor > other.minor { ret Ordering::Greater; }\n        ret Ordering::Equal;\n    }\n}\n\nfn main() -> Bool {\n    ret Version { major: 1, minor: 2 } < Version { major: 2, minor: 0 };\n}",
        implemented_by: &[
            "None", "Error", "Bool", "Int", "Float", "String", "List", "Object", "Fn", "Ordering",
        ],
    },
];

/// The compiler-owned `Ordering` variants in declaration order.
const ORDERING_VARIANTS: [&str; 4] = ["Less", "Equal", "Greater", "Unordered"];

/// Every compiler-owned standard enum declaration.
const STANDARD_ENUMS: [StandardEnumDescriptor; 1] = [StandardEnumDescriptor {
    name: ORDERING_ENUM,
    variants: &ORDERING_VARIANTS,
    description: "`Ordering` is the explicit comparison result returned by `Compare::compare`. It distinguishes equal values from values that have no defined ordering relationship.",
    usage: "fn main() -> Bool {\n    let result = 20.compare(22);\n    ret match result {\n        Ordering::Less => true,\n        Ordering::Equal => false,\n        Ordering::Greater => false,\n        Ordering::Unordered => false,\n    };\n}",
}];

/// Returns the canonical name for one compiler-owned standard trait spelling.
pub(crate) fn canonical_trait_name(name: &str) -> Option<&'static str> {
    trait_descriptor(name).map(|descriptor| descriptor.name)
}

/// Returns one standard trait descriptor for a source spelling.
pub(crate) fn trait_descriptor(name: &str) -> Option<&'static StandardTraitDescriptor> {
    let canonical = name.strip_prefix("std::").unwrap_or(name);
    STANDARD_TRAITS
        .iter()
        .find(|descriptor| descriptor.name == canonical)
}

/// Returns every compiler-owned standard trait descriptor.
pub(crate) fn traits() -> &'static [StandardTraitDescriptor] {
    &STANDARD_TRAITS
}

/// Returns the canonical name for one compiler-owned standard enum spelling.
pub(crate) fn canonical_enum_name(name: &str) -> Option<&'static str> {
    enum_descriptor(name).map(|descriptor| descriptor.name)
}

/// Returns one standard enum descriptor for a source spelling.
pub(crate) fn enum_descriptor(name: &str) -> Option<&'static StandardEnumDescriptor> {
    let canonical = name.strip_prefix("std::").unwrap_or(name);
    STANDARD_ENUMS
        .iter()
        .find(|descriptor| descriptor.name == canonical)
}

/// Returns every compiler-owned standard enum descriptor.
pub(crate) fn enums() -> &'static [StandardEnumDescriptor] {
    &STANDARD_ENUMS
}
