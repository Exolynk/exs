//! The programs both runners execute, and the host functions they call.

use exs_abi::ExsValue;

/// One program with its inputs.
pub struct Case {
    /// Name of the case in the results.
    pub name: &'static str,
    /// The `ExS` source.
    pub source: &'static str,
    /// Returns the inputs of `main`.
    pub inputs: fn() -> Vec<ExsValue>,
}

/// Returns no inputs.
fn none() -> Vec<ExsValue> {
    Vec::new()
}

/// Returns inputs of every kind the boundary passes.
fn mixed() -> Vec<ExsValue> {
    vec![
        ExsValue::Int(-7),
        ExsValue::List(vec![
            ExsValue::String("äb".to_owned()),
            ExsValue::Bytes(vec![0, 255]),
            ExsValue::Float(0.5),
        ]),
        ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("Ada".to_owned())),
            ("tags".to_owned(), ExsValue::List(Vec::new())),
        ]),
    ]
}

/// The programs both runners execute.
pub const CASES: &[Case] = &[
    Case {
        name: "scalars",
        source: r#"fn main() { ret [1, -2, 2.5, true, None, "äb €", 9223372036854775807]; }"#,
        inputs: none,
    },
    Case {
        name: "inputs",
        source: "fn main(a, b, c) { ret [a, b, c, c.name]; }",
        inputs: mixed,
    },
    Case {
        name: "typed_objects",
        source: "type Point { x: Int, y: Int } type Line { from: Point, to: Point } fn main() { let line = Line { from: Point { x: 0, y: 0 }, to: Point { x: 3, y: 4 } }; line.to.x = line.to.x + 1; line.label = \"l\"; ret line; }",
        inputs: none,
    },
    Case {
        name: "enums",
        source: "enum Color { Rgb(red: Int, green: Int, blue: Int), Gray(level: Int), Transparent, } fn main() { ret [Color::Rgb(1, 2, 3), Color::Gray(4), Color::Transparent]; }",
        inputs: none,
    },
    Case {
        name: "errors",
        source: r#"fn fail(value) { ret Error("Custom", "failed", [value]); } fn main() { ret [fail(1), 1 + "a"]; }"#,
        inputs: none,
    },
    Case {
        name: "large_string",
        source: r#"fn main() { let text = "abcdefgh"; let index = 0; while index < 14 { text = text + text; index = index + 1; } ret [text.length(), text]; }"#,
        inputs: none,
    },
    Case {
        name: "host_calls",
        source: r#"fn main() { let a = Host::call("echo", 5); let b = Host::call("echo", a * 2); ret [a, b, Host::call("add", a, b)]; }"#,
        inputs: none,
    },
    Case {
        name: "host_values",
        source: r#"enum Color { Gray(level: Int), } fn main() { ret Host::call("echo", { list: [1, "x", None], color: Color::Gray(3), error: Error("K", "m", 2) }); }"#,
        inputs: none,
    },
    Case {
        name: "host_unsupported",
        source: r#"fn main() { let f = (value) => { ret value; }; ret Host::call("echo", f); }"#,
        inputs: none,
    },
    Case {
        name: "result_unsupported",
        source: "fn main() { ret [1, (value) => { ret value; }]; }",
        inputs: none,
    },
];

/// One entry of the Rust guest with its inputs.
pub struct GuestCase {
    /// Name of the case and the guest entry.
    pub entry: &'static str,
    /// Returns the inputs of the entry.
    pub inputs: fn() -> Vec<ExsValue>,
}

/// Returns inputs of every kind, including an enum and an Error.
fn guest_values() -> Vec<ExsValue> {
    let mut values = mixed();
    values.push(ExsValue::Enum {
        type_id: "test::Color".to_owned(),
        variant: "Gray".to_owned(),
        fields: vec![ExsValue::Int(3)],
    });
    values.push(ExsValue::Error(exs_abi::ExsError {
        severity: exs_abi::ErrorSeverity::Recoverable,
        kind: "K".to_owned(),
        message: "m".to_owned(),
        data: Box::new(ExsValue::None),
        origin: Some(exs_abi::SourcePositionId(4)),
        trace: Vec::new(),
        cause: Some(Box::new(ExsValue::String("cause".to_owned()))),
    }));
    values
}

/// Returns the input of the guest's `add`.
fn guest_add() -> Vec<ExsValue> {
    vec![ExsValue::Int(34)]
}

/// The entries of the example Rust guest both runners execute.
pub const GUEST_CASES: &[GuestCase] = &[
    GuestCase {
        entry: "add",
        inputs: guest_add,
    },
    GuestCase {
        entry: "main",
        inputs: guest_add,
    },
    GuestCase {
        entry: "round_trip",
        inputs: guest_values,
    },
    GuestCase {
        entry: "sum_stream",
        inputs: none,
    },
];

/// Returns the values the host stream `counter` yields for its `arguments`: 1 up to the
/// first Int argument.
#[must_use]
pub fn counter(arguments: &[ExsValue]) -> Vec<ExsValue> {
    let end = match arguments {
        [ExsValue::Int(end)] => *end,
        _ => 0,
    };
    (1..=end).map(ExsValue::Int).collect()
}

/// The host function `add`: sums its Int arguments.
#[must_use]
pub fn add(arguments: &[ExsValue]) -> ExsValue {
    ExsValue::Int(
        arguments
            .iter()
            .map(|argument| match argument {
                ExsValue::Int(value) => *value,
                _ => 0,
            })
            .sum(),
    )
}

/// The host function `echo`: returns its first argument.
#[must_use]
pub fn echo(arguments: Vec<ExsValue>) -> ExsValue {
    arguments.into_iter().next().unwrap_or(ExsValue::None)
}

/// Formats the result of one case for the comparison.
#[must_use]
pub fn line(case: &str, backend: &str, result: &Result<ExsValue, String>) -> String {
    format!("{case} {backend} {result:?}")
}
