//! Compiler-owned declarations for globally available standard protocols.

use exs_abi::{TYPE_BOOL, TYPE_FLOAT, TYPE_INT, TYPE_LIST, TYPE_STRING};

/// Canonical source name for the protocol invoked by the `+` operator.
pub(crate) const ADD_TRAIT: &str = "Add";

/// Required instance-method name for the `Add` protocol.
pub(crate) const ADD_METHOD: &str = "add";

/// One source operator whose implementation is selected through a standard trait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StandardOperator {
    /// The binary `+` operator.
    Add,
}

/// Describes one compiler-owned trait alongside its built-in implementations.
pub(crate) struct StandardTraitDescriptor {
    /// Canonical source-visible trait name.
    pub(crate) name: &'static str,
    /// Built-in runtime categories that satisfy this trait contract.
    pub(crate) builtin_mask: u32,
    /// Required methods supplied by every nominal implementation.
    pub(crate) methods: &'static [StandardMethodDescriptor],
    /// Optional source operator supplied by one of this trait's methods.
    pub(crate) operator: Option<StandardOperator>,
    /// Overview rendered on the standard trait API page.
    pub(crate) description: &'static str,
    /// Usage example rendered on the standard trait API page.
    pub(crate) usage: &'static str,
    /// Built-in type names rendered as standard trait implementations.
    pub(crate) implemented_by: &'static [&'static str],
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

/// The required `Add::add` method declaration.
const ADD_METHODS: [StandardMethodDescriptor; 1] = [StandardMethodDescriptor {
    name: ADD_METHOD,
    has_receiver: true,
    parameter_types: &ADD_PARAMETER_TYPES,
    return_types: &ADD_RETURN_TYPES,
    signature: "fn add(self, other: Any) -> Any;",
    description: "Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.",
}];

/// The standard `Add` trait declaration shared by validation, contracts, and documentation.
const STANDARD_TRAITS: [StandardTraitDescriptor; 1] = [StandardTraitDescriptor {
    name: ADD_TRAIT,
    builtin_mask: TYPE_BOOL | TYPE_INT | TYPE_FLOAT | TYPE_STRING | TYPE_LIST,
    methods: &ADD_METHODS,
    operator: Some(StandardOperator::Add),
    description: "`Add` is the protocol selected by `left + right` when the left operand is a nominal type or enum with an `impl Add` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, Float, String, and List values implement the same protocol, so `value.add(other)` and `value + other` have identical behavior. String receivers concatenate String, Bool, Int, and Float operands using their normal source spelling.",
    usage: "type Vector { value: Int }\n\nimpl Add for Vector {\n    fn add(self, other: Any) -> Any {\n        ret Vector { value: self.value + other.value };\n    }\n}\n\nfn main() -> Int {\n    ret (Vector { value: 20 } + Vector { value: 22 }).value;\n}",
    implemented_by: &["Bool", "Int", "Float", "String", "List"],
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
