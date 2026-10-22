//! Descriptors of the runtime-owned standard library: types, functions, namespaces, and the
//! methods traits provide automatically.

use crate::codegen::standard as codegen_standard;

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
                    description: "Invokes a runner-registered host function selected by a runtime String name. Arguments are collected into a List and passed through the runner boundary. A call may complete immediately or suspend; it returns the host result or a recoverable Error such as `HostFunctionNotFound`.",
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
            StandardFunction {
                name: "from_hex",
                signature: "Bytes::from_hex(value: String) -> Bytes | Error",
                description: "Decodes pairs of lowercase or uppercase hexadecimal digits. Odd-length or malformed input returns ParseError.",
                example: "let payload = Bytes::from_hex(\"4869\");",
            },
            StandardFunction {
                name: "from_base64",
                signature: "Bytes::from_base64(value: String) -> Bytes | Error",
                description: "Decodes canonical padded RFC 4648 base64 text. Malformed input returns ParseError.",
                example: "let payload = Bytes::from_base64(\"SGk=\");",
            },
        ],
        "Int" => &[StandardFunction {
            name: "parse",
            signature: "Int::parse(value: String) -> Int | Error",
            description: "Parses a base-10 signed 64-bit integer without trimming or coercion. Invalid or out-of-range input returns ParseError.",
            example: "let count = Int::parse(\"42\");",
        }],
        "Float" => &[StandardFunction {
            name: "parse",
            signature: "Float::parse(value: String) -> Float | Error",
            description: "Parses one IEEE-754 binary64 decimal spelling without trimming or coercion. Invalid input returns ParseError.",
            example: "let ratio = Float::parse(\"0.75\");",
        }],
        "Math" => &[
            StandardFunction {
                name: "pi",
                signature: "Math::pi() -> Float",
                description: "Returns the mathematical constant pi.",
                example: "let angle = Math::pi();",
            },
            StandardFunction {
                name: "tau",
                signature: "Math::tau() -> Float",
                description: "Returns the mathematical constant tau, equal to two pi.",
                example: "let turn = Math::tau();",
            },
            StandardFunction {
                name: "e",
                signature: "Math::e() -> Float",
                description: "Returns Euler's number.",
                example: "let base = Math::e();",
            },
            StandardFunction {
                name: "sin",
                signature: "Math::sin(value: Float) -> Float | Error",
                description: "Returns the sine of a radian Float. Non-Float input returns TypeError.",
                example: "let height = Math::sin(angle);",
            },
            StandardFunction {
                name: "cos",
                signature: "Math::cos(value: Float) -> Float | Error",
                description: "Returns the cosine of a radian Float. Non-Float input returns TypeError.",
                example: "let width = Math::cos(angle);",
            },
            StandardFunction {
                name: "tan",
                signature: "Math::tan(value: Float) -> Float | Error",
                description: "Returns the tangent of a radian Float. Non-Float input returns TypeError.",
                example: "let slope = Math::tan(angle);",
            },
            StandardFunction {
                name: "asin",
                signature: "Math::asin(value: Float) -> Float | Error",
                description: "Returns the inverse sine in radians. Non-Float input returns TypeError.",
                example: "let angle = Math::asin(0.5);",
            },
            StandardFunction {
                name: "acos",
                signature: "Math::acos(value: Float) -> Float | Error",
                description: "Returns the inverse cosine in radians. Non-Float input returns TypeError.",
                example: "let angle = Math::acos(0.5);",
            },
            StandardFunction {
                name: "atan",
                signature: "Math::atan(value: Float) -> Float | Error",
                description: "Returns the inverse tangent in radians. Non-Float input returns TypeError.",
                example: "let angle = Math::atan(1.0);",
            },
            StandardFunction {
                name: "atan2",
                signature: "Math::atan2(y: Float, x: Float) -> Float | Error",
                description: "Returns the quadrant-aware inverse tangent in radians. Non-Float input returns TypeError.",
                example: "let angle = Math::atan2(y, x);",
            },
            StandardFunction {
                name: "exp",
                signature: "Math::exp(value: Float) -> Float | Error",
                description: "Returns e raised to a Float power. Non-Float input returns TypeError.",
                example: "let value = Math::exp(1.0);",
            },
            StandardFunction {
                name: "ln",
                signature: "Math::ln(value: Float) -> Float | Error",
                description: "Returns the natural logarithm. Non-Float input returns TypeError.",
                example: "let power = Math::ln(value);",
            },
            StandardFunction {
                name: "log2",
                signature: "Math::log2(value: Float) -> Float | Error",
                description: "Returns the base-two logarithm. Non-Float input returns TypeError.",
                example: "let bits = Math::log2(value);",
            },
            StandardFunction {
                name: "log10",
                signature: "Math::log10(value: Float) -> Float | Error",
                description: "Returns the base-ten logarithm. Non-Float input returns TypeError.",
                example: "let digits = Math::log10(value);",
            },
            StandardFunction {
                name: "hypot",
                signature: "Math::hypot(x: Float, y: Float) -> Float | Error",
                description: "Returns the Euclidean length of two Floats. Non-Float input returns TypeError.",
                example: "let distance = Math::hypot(x, y);",
            },
        ],
        "Object" => &[StandardFunction {
            name: "from_entries",
            signature: "Object::from_entries(entries: List) -> Object | Error",
            description: "Builds an Object from two-item [String, value] Lists. Repeated keys replace their earlier value without changing their insertion position.",
            example: "let user = Object::from_entries([[\"name\", \"Ada\"]]);",
        }],
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

/// Returns every documented source-visible standard-library type.
#[must_use]
pub fn standard_library_types() -> Vec<StandardType> {
    vec![
        StandardType {
            name: "Any",
            description: "`Any` accepts every ExS value. It is the implicit contract when a parameter or return annotation is omitted, and is useful when a function deliberately forwards values without narrowing their type.",
            usage: "fn main(value: Any) -> Any {\n    ret value;\n}",
            methods: &[],
        },
        StandardType {
            name: "None",
            description: "`None` is the single absence value. It is used for missing optional results, empty mutations, and Object reads whose key does not exist; ExS has no `null` source literal.",
            usage: "fn main() -> None {\n    let absent = None;\n    ret absent;\n}",
            methods: &[],
        },
        StandardType {
            name: "Duration",
            description: "`Duration` is a normal prelude type representing a non-negative interval as normalized `seconds` and `nanoseconds` Int fields. Create values through its static factories before passing them to `Host::sleep`.",
            usage: "fn main() -> None {\n    let pause = Duration::milliseconds(250);\n    Host::sleep(pause);\n    ret None;\n}",
            methods: &[
                StandardMethod {
                    signature: "as_seconds() -> Int",
                    description: "Returns the normalized whole-second component. Any fractional second remains available through the `nanoseconds` field.",
                    example: "let seconds = Duration::milliseconds(1500).as_seconds(); // 1",
                },
                StandardMethod {
                    signature: "as_milliseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole milliseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let milliseconds = Duration::nanoseconds(1_500_000).as_milliseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_microseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole microseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let microseconds = Duration::nanoseconds(1_500).as_microseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_nanoseconds() -> Int | Error",
                    description: "Returns the total duration as exact nanoseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let nanoseconds = Duration::milliseconds(1).as_nanoseconds(); // 1_000_000",
                },
            ],
        },
        StandardType {
            name: "Error",
            description: "`Error` is a structured language failure value. Operations return it instead of throwing, and functions that may return an Error should include `Error` in their return contract or use the implicit `Any` contract.",
            usage: "fn main() -> Int | Error {\n    ret Error(\"DivisionByZeroError\", \"cannot divide by zero\", None);\n}",
            methods: &[
                StandardMethod {
                    signature: "kind() -> String",
                    description: "Returns the stable machine-readable category assigned when the Error was created. Use it to distinguish expected failures without parsing a human-facing message.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet category = error.kind();",
                },
                StandardMethod {
                    signature: "message() -> String",
                    description: "Returns the human-readable explanation stored in the Error. The message is intended for diagnostics and user-facing reporting, not control-flow classification.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet text = error.message();",
                },
                StandardMethod {
                    signature: "data() -> Any",
                    description: "Returns the language value attached to the Error. Runtime operations use this to retain the invalid input, index, or other relevant context that caused the failure.",
                    example: "let error = Error(\"InvalidInput\", \"age must be positive\", -1);\nlet invalid_age = error.data();",
                },
                StandardMethod {
                    signature: "cause() -> Error | None",
                    description: "Returns a related prior Error or value when one is present. Errors created directly with `Error(...)` have no cause and therefore return None.",
                    example: "let error = Error(\"Example\", \"no prior failure\", None);\nlet previous = error.cause();",
                },
            ],
        },
        StandardType {
            name: "Bool",
            description: "`Bool` has exactly the values `true` and `false`. Conditions require Bool explicitly; ExS does not apply implicit truthiness to numbers, strings, collections, or None. Bools are not numbers: arithmetic and ordering with a Bool return TypeError.",
            usage: "fn main() {\n    let ready = true;\n    if ready {\n        Host::call(\"println\", \"ready\");\n    }\n}",
            methods: &[],
        },
        StandardType {
            name: "Int",
            description: "`Int` is a signed 64-bit exact integer. It supports numeric operators and reports `IntOverflowError` when an operation overflows the ExS integer range.",
            usage: "fn main() -> Int {\n    let quantity = 42;\n    ret quantity + 8;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Int",
                    description: "Returns the non-negative magnitude of the receiver. The smallest representable Int has no representable positive counterpart, so calling `abs()` on it returns `IntOverflowError`.",
                    example: "let change = -42;\nlet magnitude = change.abs(); // 42",
                },
                StandardMethod {
                    signature: "div_euclid(other: Int) -> Int | Error",
                    description: "Returns the Euclidean quotient of two Int values. The result satisfies `self == other * quotient + self.rem_euclid(other)`. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let seconds = 1001.div_euclid(1000); // 1",
                },
                StandardMethod {
                    signature: "rem_euclid(other: Int) -> Int | Error",
                    description: "Returns the non-negative Euclidean remainder of two Int values. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let milliseconds = 1001.rem_euclid(1000); // 1",
                },
                StandardMethod {
                    signature: "signum() -> Int",
                    description: "Returns -1, 0, or 1 according to the receiver sign.",
                    example: "let direction = change.signum();",
                },
                StandardMethod {
                    signature: "is_even() -> Bool",
                    description: "Returns whether the receiver is evenly divisible by two.",
                    example: "let even = count.is_even();",
                },
                StandardMethod {
                    signature: "is_odd() -> Bool",
                    description: "Returns whether the receiver is not evenly divisible by two.",
                    example: "let odd = count.is_odd();",
                },
                StandardMethod {
                    signature: "min(other: Int) -> Int",
                    description: "Returns the smaller exact integer.",
                    example: "let lower = start.min(end);",
                },
                StandardMethod {
                    signature: "max(other: Int) -> Int",
                    description: "Returns the larger exact integer.",
                    example: "let upper = start.max(end);",
                },
                StandardMethod {
                    signature: "clamp(minimum: Int, maximum: Int) -> Int | Error",
                    description: "Restricts the receiver to an inclusive range. An inverted range returns ValueError.",
                    example: "let page = requested.clamp(1, 100);",
                },
                StandardMethod {
                    signature: "pow(exponent: Int) -> Int | Error",
                    description: "Raises the receiver to a non-negative Int exponent. Negative exponents return ValueError and overflow returns IntOverflowError.",
                    example: "let value = 12;\nlet square = value.pow(2);",
                },
                StandardMethod {
                    signature: "gcd(other: Int) -> Int | Error",
                    description: "Returns the non-negative greatest common divisor, or IntOverflowError when it is not representable.",
                    example: "let value = 12;\nlet divisor = value.gcd(18);",
                },
                StandardMethod {
                    signature: "lcm(other: Int) -> Int | Error",
                    description: "Returns the non-negative least common multiple, or IntOverflowError when it is not representable.",
                    example: "let value = 12;\nlet period = value.lcm(18);",
                },
                StandardMethod {
                    signature: "to_float() -> Float",
                    description: "Converts the receiver to binary64 Float using standard IEEE-754 conversion.",
                    example: "let ratio = count.to_float();",
                },
            ],
        },
        StandardType {
            name: "Float",
            description: "`Float` uses IEEE-754 binary64 values, including infinities, signed zero, and NaN. Mixed arithmetic promotes the other numeric operand to Float.",
            usage: "fn main() -> Float {\n    let price = 19.95;\n    ret price * 1.19;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Float",
                    description: "Returns the non-negative floating-point magnitude. It preserves Float semantics for signed zero, infinities, and NaN.",
                    example: "let delta = -1.5;\nlet magnitude = delta.abs(); // 1.5",
                },
                StandardMethod {
                    signature: "floor() -> Float",
                    description: "Rounds down to the greatest integral Float that is not greater than the receiver. The result remains Float so it composes with floating-point arithmetic.",
                    example: "let page = 3.8;\nlet first_index = page.floor(); // 3.0",
                },
                StandardMethod {
                    signature: "ceil() -> Float",
                    description: "Rounds up to the least integral Float that is not less than the receiver. The result remains Float.",
                    example: "let pages = 3.2;\nlet required = pages.ceil(); // 4.0",
                },
                StandardMethod {
                    signature: "round() -> Float",
                    description: "Rounds to the nearest integral Float. Exact halfway values are rounded away from zero, so `1.5` becomes `2.0` and `-1.5` becomes `-2.0`.",
                    example: "let rating = 4.5;\nlet displayed = rating.round(); // 5.0",
                },
                StandardMethod {
                    signature: "trunc() -> Float",
                    description: "Removes the fractional component while preserving the Float result type.",
                    example: "let whole = value.trunc();",
                },
                StandardMethod {
                    signature: "fract() -> Float",
                    description: "Returns the signed fractional component of the receiver.",
                    example: "let fraction = value.fract();",
                },
                StandardMethod {
                    signature: "min(other: Float) -> Float",
                    description: "Returns the IEEE-754 minimum of two Float values.",
                    example: "let lower = start.min(end);",
                },
                StandardMethod {
                    signature: "max(other: Float) -> Float",
                    description: "Returns the IEEE-754 maximum of two Float values.",
                    example: "let upper = start.max(end);",
                },
                StandardMethod {
                    signature: "clamp(minimum: Float, maximum: Float) -> Float | Error",
                    description: "Restricts the receiver to an inclusive Float range. An inverted range returns ValueError.",
                    example: "let opacity = value.clamp(0.0, 1.0);",
                },
                StandardMethod {
                    signature: "pow(exponent: Float) -> Float",
                    description: "Raises the receiver to a Float exponent with IEEE-754 semantics.",
                    example: "let area = radius.pow(2.0);",
                },
                StandardMethod {
                    signature: "sqrt() -> Float",
                    description: "Returns the IEEE-754 square root; a negative input produces NaN.",
                    example: "let root = value.sqrt();",
                },
                StandardMethod {
                    signature: "to_int() -> Int | Error",
                    description: "Truncates a finite Float to Int. Out-of-range values return IntOverflowError and non-finite values return ValueError.",
                    example: "let count = value.to_int();",
                },
                StandardMethod {
                    signature: "is_nan() -> Bool",
                    description: "Returns whether the receiver is NaN.",
                    example: "let invalid = value.is_nan();",
                },
                StandardMethod {
                    signature: "is_finite() -> Bool",
                    description: "Returns whether the receiver is neither infinite nor NaN.",
                    example: "let usable = value.is_finite();",
                },
                StandardMethod {
                    signature: "is_infinite() -> Bool",
                    description: "Returns whether the receiver is positive or negative infinity.",
                    example: "let unbounded = value.is_infinite();",
                },
            ],
        },
        StandardType {
            name: "Math",
            description: "`Math` is the static namespace for strict-Float mathematical constants and functions. Trigonometric inputs and outputs use radians.",
            usage: "fn main() -> Float {\n    ret Math::sin(Math::pi() / 2.0);\n}",
            methods: &[],
        },
        StandardType {
            name: "String",
            description: "`String` is an immutable UTF-8 sequence. `length()` operates on Unicode scalar values rather than UTF-8 byte positions; use `for` to iterate scalar Strings.",
            usage: "fn main() -> String {\n    let greeting = \"Hello\";\n    ret greeting;\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of Unicode scalar values in the String. This is not the UTF-8 byte length, so a single emoji scalar counts as one.",
                    example: "let symbol = \"🙂\";\nlet count = symbol.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the String contains no Unicode scalar values and false otherwise. It does not trim or normalize the String.",
                    example: "let input = \"\";\nif input.is_empty() {\n    Host::call(\"println\", \"missing input\");\n}",
                },
                StandardMethod {
                    signature: "slice(start: Int, end: Int) -> String | Error",
                    description: "Returns the half-open Unicode-scalar range. Invalid scalar offsets return IndexError.",
                    example: "let middle = text.slice(1, 3);",
                },
                StandardMethod {
                    signature: "contains(needle: String) -> Bool | Error",
                    description: "Returns whether the receiver contains the exact String needle.",
                    example: "let found = text.contains(\"ExS\");",
                },
                StandardMethod {
                    signature: "starts_with(prefix: String) -> Bool | Error",
                    description: "Returns whether the receiver starts with the exact String prefix.",
                    example: "let header = text.starts_with(\"ExS\");",
                },
                StandardMethod {
                    signature: "ends_with(suffix: String) -> Bool | Error",
                    description: "Returns whether the receiver ends with the exact String suffix.",
                    example: "let footer = text.ends_with(\"!\");",
                },
                StandardMethod {
                    signature: "split(delimiter: String) -> List | Error",
                    description: "Returns new String values separated by the exact delimiter, including empty fields.",
                    example: "let fields = text.split(\",\");",
                },
                StandardMethod {
                    signature: "trim() -> String",
                    description: "Removes Unicode whitespace from both ends.",
                    example: "let clean = text.trim();",
                },
                StandardMethod {
                    signature: "trim_start() -> String",
                    description: "Removes Unicode whitespace from the start.",
                    example: "let clean = text.trim_start();",
                },
                StandardMethod {
                    signature: "trim_end() -> String",
                    description: "Removes Unicode whitespace from the end.",
                    example: "let clean = text.trim_end();",
                },
                StandardMethod {
                    signature: "replace(from: String, to: String) -> String | Error",
                    description: "Replaces every non-overlapping exact String match.",
                    example: "let updated = text.replace(\"old\", \"new\");",
                },
                StandardMethod {
                    signature: "to_lowercase() -> String",
                    description: "Applies Unicode lowercase mapping.",
                    example: "let lower = text.to_lowercase();",
                },
                StandardMethod {
                    signature: "to_uppercase() -> String",
                    description: "Applies Unicode uppercase mapping.",
                    example: "let upper = text.to_uppercase();",
                },
                StandardMethod {
                    signature: "repeat(count: Int) -> String | Error",
                    description: "Returns a new String repeated a non-negative number of times.",
                    example: "let divider = \"-\".repeat(3);",
                },
                StandardMethod {
                    signature: "encode_utf8() -> Bytes | Error",
                    description: "Encodes the immutable UTF-8 contents as raw Bytes.",
                    example: "let bytes = text.encode_utf8();",
                },
            ],
        },
        StandardType {
            name: "Bytes",
            description: "`Bytes` is an immutable sequence of raw octets. It is the binary-safe value used for arbitrary host data; indexing and iteration yield Int values from 0 through 255.",
            usage: "fn main() -> Bytes | Error {\n    let header = b\"EXS\";\n    ret header.concat(Bytes::from_list([0, 1])?);\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of octets in the Bytes value.",
                    example: "let payload = b\"abc\";\nlet count = payload.length(); // 3",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the Bytes value contains no octets. It does not mutate the receiver.",
                    example: "let payload = b\"\";\nlet empty = payload.is_empty(); // true",
                },
                StandardMethod {
                    signature: "to_list() -> List",
                    description: "Returns a new List whose elements are the receiver octets as Int values in source order.",
                    example: "let values = b\"AB\".to_list(); // [65, 66]",
                },
                StandardMethod {
                    signature: "slice(start: Int, end: Int) -> Bytes | Error",
                    description: "Returns a new Bytes value covering the half-open range from start through end. Both indexes must be non-negative and the range must lie inside the receiver, otherwise IndexError is returned.",
                    example: "let middle = b\"abcd\".slice(1, 3); // b\"bc\"",
                },
                StandardMethod {
                    signature: "concat(other: Bytes) -> Bytes | Error",
                    description: "Returns a new Bytes value with other appended. A non-Bytes argument returns TypeError.",
                    example: "let message = b\"hello, \".concat(b\"world\");",
                },
                StandardMethod {
                    signature: "decode_utf8() -> String | Error",
                    description: "Decodes the receiver as UTF-8. Invalid byte sequences return EncodingError rather than replacing or dropping octets.",
                    example: "let text = b\"hello\".decode_utf8(); // \"hello\"",
                },
                StandardMethod {
                    signature: "starts_with(prefix: Bytes) -> Bool | Error",
                    description: "Returns whether the receiver starts with the exact byte prefix.",
                    example: "let header = payload.starts_with(b\"EX\");",
                },
                StandardMethod {
                    signature: "ends_with(suffix: Bytes) -> Bool | Error",
                    description: "Returns whether the receiver ends with the exact byte suffix.",
                    example: "let trailer = payload.ends_with(b\"!\");",
                },
                StandardMethod {
                    signature: "contains(needle: Bytes) -> Bool | Error",
                    description: "Returns whether the receiver contains the exact byte sequence.",
                    example: "let found = payload.contains(b\"EX\");",
                },
                StandardMethod {
                    signature: "repeat(count: Int) -> Bytes | Error",
                    description: "Returns new Bytes repeated a non-negative number of times.",
                    example: "let padded = b\"0\".repeat(4);",
                },
                StandardMethod {
                    signature: "to_hex() -> String",
                    description: "Encodes every byte as two lowercase hexadecimal digits.",
                    example: "let hex = payload.to_hex();",
                },
                StandardMethod {
                    signature: "to_base64() -> String",
                    description: "Encodes the Bytes as padded RFC 4648 base64 text.",
                    example: "let encoded = payload.to_base64();",
                },
            ],
        },
        StandardType {
            name: "List",
            description: "`List` is a mutable ordered collection. Variables and closure captures preserve List identity, so mutations through one alias are visible through every alias of the same List. Its shared eager iterable methods are documented by the [`Iterator`](../traits/iterator.md) trait.",
            usage: "fn main() -> Int {\n    let items = [\"Ada\", \"Lin\"];\n    ret items.push(\"Mia\");\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the current number of elements. The count changes immediately after List mutations such as `push`, `pop`, `insert`, `remove`, and `clear`.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet count = items.length(); // 2",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the List has no elements. It does not mutate the List.",
                    example: "let queue = [];\nif queue.is_empty() {\n    Host::call(\"println\", \"queue is empty\");\n}",
                },
                StandardMethod {
                    signature: "push(value) -> Int",
                    description: "Appends one value to the end of the List and returns the new element count. The operation mutates the existing List rather than allocating a replacement.",
                    example: "let items = [\"Ada\"];\nlet count = items.push(\"Lin\"); // 2",
                },
                StandardMethod {
                    signature: "pop() -> Any | None",
                    description: "Removes and returns the final element. Calling `pop()` on an empty List leaves it unchanged and returns None.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet last = items.pop(); // \"Lin\"",
                },
                StandardMethod {
                    signature: "insert(index: Int, value) -> None | Error",
                    description: "Inserts one value before the zero-based index and returns None. The index may equal the current length to append; invalid indexes return `IndexError`.",
                    example: "let items = [\"Ada\", \"Mia\"];\nitems.insert(1, \"Lin\"); // [\"Ada\", \"Lin\", \"Mia\"]",
                },
                StandardMethod {
                    signature: "remove(index: Int) -> Any | Error",
                    description: "Removes and returns the element at a zero-based index. Invalid indexes return `IndexError` and leave the List unchanged.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet removed = items.remove(0); // \"Ada\"",
                },
                StandardMethod {
                    signature: "clear() -> None",
                    description: "Removes every element from the existing List and returns None. Aliases continue to refer to the now-empty same List.",
                    example: "let items = [1, 2, 3];\nitems.clear();\nlet empty = items.is_empty(); // true",
                },
                StandardMethod {
                    signature: "get(index: Int) -> Any | None | Error",
                    description: "Returns the zero-based item, or None when the index is beyond the List. Negative or unsupported indexes return IndexError.",
                    example: "let value = items.get(3);",
                },
                StandardMethod {
                    signature: "first() -> Any | None",
                    description: "Returns the first item, or None when the List is empty.",
                    example: "let first = items.first();",
                },
                StandardMethod {
                    signature: "last() -> Any | None",
                    description: "Returns the final item, or None when the List is empty.",
                    example: "let last = items.last();",
                },
                StandardMethod {
                    signature: "slice(start: Int, end: Int) -> List | Error",
                    description: "Returns a new shallow List for the validated half-open range. Invalid bounds return IndexError.",
                    example: "let middle = items.slice(1, 3);",
                },
                StandardMethod {
                    signature: "extend(other: List) -> Int | Error",
                    description: "Mutates this List by appending a shallow copy of other and returns the new length.",
                    example: "items.extend(more_items);",
                },
                StandardMethod {
                    signature: "reverse() -> None",
                    description: "Reverses this List in place and returns None.",
                    example: "items.reverse();",
                },
                StandardMethod {
                    signature: "reversed() -> List",
                    description: "Returns a new shallow List in reverse order without mutating the receiver.",
                    example: "let latest_first = items.reversed();",
                },
                StandardMethod {
                    signature: "contains(value) -> Bool",
                    description: "Tests for an item equal to `value` under `==`, which compares composites part by part and uses `Eq` implementations.",
                    example: "let found = items.contains(target);",
                },
                StandardMethod {
                    signature: "index_of(value) -> Int | None",
                    description: "Returns the index of the first item equal to `value` under `==`, or None when absent.",
                    example: "let index = items.index_of(target);",
                },
                StandardMethod {
                    signature: "last_index_of(value) -> Int | None",
                    description: "Returns the index of the final item equal to `value` under `==`, or None when absent.",
                    example: "let index = items.last_index_of(target);",
                },
                StandardMethod {
                    signature: "sort() -> None | Error",
                    description: "Sorts the List in place in ascending order under `<`, which uses `Compare` implementations; equal items keep their order. Items without an order return the TypeError of `<`.",
                    example: "let items = [3, 1, 2];\nitems.sort(); // [1, 2, 3]",
                },
                StandardMethod {
                    signature: "sort(compare: Fn) -> None | Error",
                    description: "Sorts the List in place by the Ordering `compare(a, b)` returns: `Ordering::Less` puts `a` first. Equal items keep their order; a comparison Error is returned.",
                    example: "let items = [1, 3, 2];\nitems.sort((a, b) => b.compare(a)); // [3, 2, 1]",
                },
                StandardMethod {
                    signature: "sorted() -> List | Error",
                    description: "Returns a new List of the items in ascending order under `<`, like `sort()`, without changing the receiver.",
                    example: "let ordered = [\"b\", \"a\"].sorted(); // [\"a\", \"b\"]",
                },
                StandardMethod {
                    signature: "sorted(compare: Fn) -> List | Error",
                    description: "Returns a new List of the items ordered by `compare`, like `sort(compare)`, without changing the receiver.",
                    example: "let newest = [1, 3, 2].sorted((a, b) => b.compare(a));",
                },
                StandardMethod {
                    signature: "sum() -> Any | Error",
                    description: "Returns the sum of the items under `+`, which uses `Add` implementations; an empty List sums to 0. The first Error of `+` is returned.",
                    example: "let total = [1, 2, 3.5].sum(); // 6.5",
                },
                StandardMethod {
                    signature: "min() -> Any | None | Error",
                    description: "Returns the smallest item under `<`, the first of equal ones, or None for an empty List.",
                    example: "let smallest = [3, 1, 2].min(); // 1",
                },
                StandardMethod {
                    signature: "max() -> Any | None | Error",
                    description: "Returns the largest item under `>`, the first of equal ones, or None for an empty List.",
                    example: "let largest = [3, 1, 2].max(); // 3",
                },
                StandardMethod {
                    signature: "flatten() -> List",
                    description: "Returns a new List in which nested Lists are replaced by their items, one level deep.",
                    example: "let flat = [[1, 2], 3, [[4]]].flatten(); // [1, 2, 3, [4]]",
                },
                StandardMethod {
                    signature: "unique() -> List | Error",
                    description: "Returns a new List without items equal under `==` to an earlier item, keeping first occurrences in order.",
                    example: "let distinct = [1, 2, 1].unique(); // [1, 2]",
                },
                StandardMethod {
                    signature: "join(separator: String) -> String | Error",
                    description: "Joins String items using a String separator. Non-String items return TypeError.",
                    example: "let csv = names.join(\",\");",
                },
            ],
        },
        StandardType {
            name: "Object",
            description: "`Object` is a mutable insertion-ordered mapping from String keys to values. Dot properties and bracket access operate on the same ordered collection, and `for` iterates its `[key, value]` entries.",
            usage: "fn main() -> Object {\n    let user = { name: \"Ada\" };\n    user.role = \"admin\";\n    ret user;\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of present keys. Replacing an existing key does not increase the count; creating or deleting a key does.",
                    example: "let user = { name: \"Ada\" };\nlet count = user.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the Object has no keys. It does not mutate the Object.",
                    example: "let options = {};\nif options.is_empty() {\n    Host::call(\"println\", \"using defaults\");\n}",
                },
                StandardMethod {
                    signature: "has(key: String) -> Bool | Error",
                    description: "Returns whether a String key is present. A non-String key returns `TypeError` rather than coercing the key.",
                    example: "let user = { name: \"Ada\" };\nlet has_name = user.has(\"name\"); // true",
                },
                StandardMethod {
                    signature: "get(key: String) -> Any | None | Error",
                    description: "Returns the value for a String key, or None when the key is absent. A non-String key returns `TypeError`.",
                    example: "let user = { name: \"Ada\" };\nlet name = user.get(\"name\"); // \"Ada\"",
                },
                StandardMethod {
                    signature: "delete(key: String) -> Any | None | Error",
                    description: "Removes a String key and returns its previous value. When the key is absent, it returns None; a non-String key returns `TypeError`.",
                    example: "let user = { name: \"Ada\" };\nlet name = user.delete(\"name\"); // \"Ada\"",
                },
                StandardMethod {
                    signature: "keys() -> List",
                    description: "Returns a new List of String keys in insertion order. Changing the returned List does not change the Object.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet keys = user.keys(); // [\"name\", \"role\"]",
                },
                StandardMethod {
                    signature: "values() -> List",
                    description: "Returns a new shallow List of values in the same insertion order as `keys()`. The values themselves retain their original identity.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet values = user.values(); // [\"Ada\", \"admin\"]",
                },
                StandardMethod {
                    signature: "get_or(key: String, default) -> Any | Error",
                    description: "Returns the key value or the supplied default when the key is absent.",
                    example: "let role = user.get_or(\"role\", \"guest\");",
                },
                StandardMethod {
                    signature: "clear() -> None",
                    description: "Removes all keys from this Object while preserving its identity.",
                    example: "user.clear();",
                },
                StandardMethod {
                    signature: "entries() -> List",
                    description: "Returns new two-item [key, value] Lists in insertion order.",
                    example: "let entries = user.entries();",
                },
                StandardMethod {
                    signature: "merge(other: Object) -> Object | Error",
                    description: "Returns a new plain Object; matching right-hand keys replace left-hand values while retaining their original position.",
                    example: "let merged = defaults.merge(overrides);",
                },
            ],
        },
        StandardType {
            name: "Fn",
            description: "`Fn` is the callable closure contract used in annotations. A closure captures lexical bindings and can be called with its declared parameter count.",
            usage: "fn apply(function: Fn, value: Int) -> Int {\n    ret function(value);\n}\n\nfn main() -> Int {\n    let increment = (value) => { ret value + 1; };\n    ret apply(increment, 1);\n}",
            methods: &[],
        },
    ]
}

/// Returns standard-library methods automatically available through one trait.
pub(super) fn automatic_trait_methods(trait_name: &str) -> &'static [StandardMethod] {
    match trait_name {
        "Iterator" => &[
            StandardMethod {
                signature: "map(callback: Fn) -> List | Error",
                description: "Maps every yielded item and returns the first Error returned by the callback. Error values already yielded remain normal callback inputs.",
                example: "let doubled = (0..3).map((value) => { ret value * 2; });",
            },
            StandardMethod {
                signature: "filter(callback: Fn) -> List | Error",
                description: "Collects items whose callback result is true. The first callback Error is returned.",
                example: "let even = (0..6).filter((value) => { ret value % 2 == 0; });",
            },
            StandardMethod {
                signature: "find(callback: Fn) -> Any | None | Error",
                description: "Returns the first item whose callback result is true, or None when no item matches. The first callback Error is returned.",
                example: "let first = (0..6).find((value) => { ret value > 3; });",
            },
            StandardMethod {
                signature: "any(callback: Fn) -> Bool | Error",
                description: "Returns whether any yielded item produces true. The first callback Error is returned.",
                example: "let present = (0..6).any((value) => { ret value == 4; });",
            },
            StandardMethod {
                signature: "all(callback: Fn) -> Bool | Error",
                description: "Returns whether every yielded item produces true. The first callback Error is returned.",
                example: "let valid = (1..4).all((value) => { ret value > 0; });",
            },
            StandardMethod {
                signature: "each(callback: Fn) -> None | Error",
                description: "Invokes the callback once for every yielded item. The first callback Error is returned.",
                example: "(0..3).each((value) => { Host::call(\"record\", value); ret None; });",
            },
            StandardMethod {
                signature: "reduce(initial, callback: Fn) -> Any | Error",
                description: "Folds yielded items from an explicit initial accumulator. The first callback Error is returned.",
                example: "let total = (1..4).reduce(0, (total, value) => { ret total + value; });",
            },
            StandardMethod {
                signature: "collect() -> List | Error",
                description: "Returns a new List containing every yielded item.",
                example: "let values = (0..3).collect(); // [0, 1, 2]",
            },
            StandardMethod {
                signature: "count() -> Int | Error",
                description: "Returns the number of yielded items.",
                example: "let count = \"Ada\".count(); // 3",
            },
            StandardMethod {
                signature: "to_list() -> List | Error",
                description: "Returns a new List containing every yielded item. This is an alias for `collect()`.",
                example: "let values = (0..3).to_list(); // [0, 1, 2]",
            },
        ],
        _ => &[],
    }
}

/// Returns traits whose automatic methods are available to one runtime-owned built-in type.
pub(super) fn automatic_traits_for_standard_type(type_name: &str) -> &'static [&'static str] {
    match type_name {
        "String" | "Bytes" | "List" => &["Iterator"],
        _ => &[],
    }
}
