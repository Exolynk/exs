//! Markdown language and source-API documentation generation.

use std::path::Path;

use crate::ast::{
    EnumDeclaration, FunctionDeclaration, Module, Parameter, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration,
};
use crate::codegen::standard::{
    self as codegen_standard, StandardEnumDescriptor, StandardTraitDescriptor,
};
use crate::{Documentation, DocumentationPage, ModuleResolver, SourceInput, SourceSpan};

mod shared;
mod source;
mod standard;

pub(crate) use source::generate;
pub use standard::standard_library_types;

/// One source-visible standard-library type and its documentation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardType {
    /// Source-visible built-in type name.
    pub name: &'static str,
    /// Overview rendered before the type declaration.
    pub description: &'static str,
    /// Short valid ExS example that introduces the type.
    pub usage: &'static str,
    /// Runtime-owned methods exposed by this type.
    pub methods: &'static [StandardMethod],
}

/// One documented runtime-owned standard-library method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardMethod {
    /// Source-level method signature.
    pub signature: &'static str,
    /// Detailed observable behavior and error conditions.
    pub description: &'static str,
    /// Short valid ExS example of a call to the method.
    pub example: &'static str,
}

/// One globally callable standard-library function and its documentation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardFunction {
    /// Source-visible function name.
    pub name: &'static str,
    /// Source-level call signature.
    pub signature: &'static str,
    /// Detailed observable behavior and error conditions.
    pub description: &'static str,
    /// Short valid ExS example of a call to the function.
    pub example: &'static str,
}

/// One source-visible standard-library namespace and its static operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardNamespace {
    /// Source-visible namespace name.
    pub name: &'static str,
    /// Overview rendered before the namespace operations.
    pub description: &'static str,
    /// Static operations available through the namespace separator.
    pub functions: &'static [StandardFunction],
}

/// One source-visible standard-library trait.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardTrait {
    /// Source-visible trait name.
    pub name: &'static str,
}

/// One source-visible standard-library enum and its variants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardEnum {
    /// Source-visible enum name.
    pub name: &'static str,
    /// Source-visible variants in declaration order.
    pub variants: &'static [&'static str],
}

/// Returns every globally callable standard-library function.
#[must_use]
pub fn standard_library_functions() -> &'static [StandardFunction] {
    &[StandardFunction {
        name: "Error",
        signature: "Error(kind, message, data)",
        description: "Constructs a recoverable Error with a stable category, a human-readable message, and any related language value. `kind` and `message` must be Strings. The constructor does not assign a cause; `cause()` consequently returns None for directly constructed Errors.",
        example: "let error = Error(\"InvalidInput\", \"age must be positive\", -1);\nret error.message();",
    }]
}

/// Returns every source-visible standard-library namespace.
#[must_use]
pub fn standard_library_namespaces() -> &'static [StandardNamespace] {
    &[
        StandardNamespace {
            name: "test",
            description: "Test assertions available directly in every module and through the std::test namespace.",
            functions: &[
                StandardFunction {
                    name: "assert",
                    signature: "std::test::assert(condition: Bool[, description: String]) -> None",
                    description: "Returns None when condition is true. When condition is false, it creates a fatal AssertionFailed Error with the supplied description, or the default message \"assert failed\" when omitted. The direct assert spelling is equivalent.",
                    example: "assert(total > 0, \"total must be positive\");",
                },
                StandardFunction {
                    name: "assert_eq",
                    signature: "std::test::assert_eq(actual: Any, expected: Any[, description: String]) -> None",
                    description: "Returns None when actual and expected compare equal with ExS equality semantics. Otherwise it creates a fatal AssertionFailed Error whose data contains actual and expected values and uses the supplied description, or the default message \"assert_eq failed\" when omitted. The direct assert_eq spelling is equivalent.",
                    example: "assert_eq(total, 42, \"total must include every line item\");",
                },
            ],
        },
        StandardNamespace {
            name: "Host",
            description: "The runner-provided boundary for application-specific operations.",
            functions: &[
                StandardFunction {
                    name: "call",
                    signature: "Host::call(name, arguments...)",
                    description: "Invokes a runner-registered host function selected by a runtime String name. Arguments are collected into a List and transported through the runner CBOR boundary. A call may complete immediately or suspend; it returns the host result or a recoverable Error such as `HostFunctionNotFound`.",
                    example: "let greeting = Host::call(\"greet\", \"Ada\");\nret greeting;",
                },
                StandardFunction {
                    name: "sleep",
                    signature: "Host::sleep(duration: Duration) -> None",
                    description: "Suspends the current ExS task until the supplied normalized Duration elapses, then returns None. This capability is built into every runner and does not use the application host-function registry.",
                    example: "Host::sleep(Duration::milliseconds(250));\nret None;",
                },
                StandardFunction {
                    name: "now",
                    signature: "Host::now() -> DateTime",
                    description: "Returns a snapshot of the runner wall clock. The result always carries the Unix instant and observed UTC offset; `timezone` contains the runner-resolved IANA name when available.",
                    example: "let now = Host::now();\nret now.unix_seconds;",
                },
                StandardFunction {
                    name: "elapsed",
                    signature: "Host::elapsed() -> Duration",
                    description: "Returns monotonic time elapsed since this root execution began. It is unaffected by wall-clock changes and is suitable for measuring work inside one execution.",
                    example: "Host::sleep(Duration::milliseconds(250));\nret Host::elapsed();",
                },
                StandardFunction {
                    name: "stream",
                    signature: "Host::stream(name, arguments...) -> HostStream | Error",
                    description: "Opens a runner-registered pull stream selected by a runtime String name. Arguments are collected into a List and passed to the stream factory without the name. HostStream implements Iterator; each advance may suspend, and the runner drops the stream after IteratorStep::Done or execution cleanup.",
                    example: "let events = Host::stream(\"events.subscribe\", user_id)?;\nfor event in events {\n    Host::call(\"events.record\", event);\n}",
                },
            ],
        },
    ]
}

/// Returns one standard namespace by its source-visible name.
#[must_use]
pub fn standard_library_namespace(name: &str) -> Option<&'static StandardNamespace> {
    standard_library_namespaces()
        .iter()
        .find(|namespace| namespace.name == name)
}

/// Returns static functions declared by one documented standard type.
#[must_use]
pub fn standard_library_type_static_functions(name: &str) -> &'static [StandardFunction] {
    match name {
        "Bytes" => &[
            StandardFunction {
                name: "from_list",
                signature: "Bytes::from_list(values: List) -> Bytes | Error",
                description: "Creates immutable Bytes from a List of Int octets. Every value must be between 0 and 255; non-Int entries return TypeError and out-of-range entries return ValueError.",
                example: "let payload = Bytes::from_list([0, 127, 255]);",
            },
            StandardFunction {
                name: "from_utf8",
                signature: "Bytes::from_utf8(value: String) -> Bytes | Error",
                description: "Encodes the UTF-8 contents of a String into immutable Bytes.",
                example: "let encoded = Bytes::from_utf8(\"hello\");",
            },
        ],
        "Duration" => &[
            StandardFunction {
                name: "nanoseconds",
                signature: "Duration::nanoseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact nanosecond count. Negative input returns ValueError.",
                example: "let pause = Duration::nanoseconds(500);",
            },
            StandardFunction {
                name: "microseconds",
                signature: "Duration::microseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact microsecond count. Negative input returns ValueError and conversion overflow returns IntOverflowError.",
                example: "let pause = Duration::microseconds(500);",
            },
            StandardFunction {
                name: "milliseconds",
                signature: "Duration::milliseconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact millisecond count. Negative input returns ValueError and non-Int input returns TypeError.",
                example: "let timeout = Duration::milliseconds(500);",
            },
            StandardFunction {
                name: "seconds",
                signature: "Duration::seconds(value: Int) -> Duration | Error",
                description: "Constructs a Duration from a non-negative exact second count. Negative input returns ValueError.",
                example: "let interval = Duration::seconds(2);",
            },
        ],
        _ => &[],
    }
}

/// Returns every documented source-visible standard-library trait.
#[must_use]
pub fn standard_library_traits() -> Vec<StandardTrait> {
    codegen_standard::traits()
        .iter()
        .map(|descriptor| StandardTrait {
            name: descriptor.name,
        })
        .collect()
}

/// Returns every documented source-visible standard-library enum.
#[must_use]
pub fn standard_library_enums() -> Vec<StandardEnum> {
    codegen_standard::enums()
        .iter()
        .map(|descriptor| StandardEnum {
            name: descriptor.name,
            variants: descriptor.variants,
        })
        .collect()
}
