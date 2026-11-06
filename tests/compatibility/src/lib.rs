//! The behaviour every release of ExS has to keep: scripts and their expected results, which run
//! both as compiled modules frozen by earlier releases and through the current compiler.

pub mod template;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use exs::{
    CompileOptions, ErrorSeverity, ExecutionCancellation, ExecutionOptions, ExsError,
    ExsStackFrame, ExsValue, HostStream, HostStreamFuture, HostStreamItem, Runner, RunnerError,
    Source, SourcePositionId, Sources,
};

/// The name of the frozen module of the Rust guest in `examples/rust-guest`.
pub const RUST_GUEST: &str = "rust_guest";

/// The scripts of the corpus, by module name.
pub const SCRIPTS: &[(&str, &str)] = &[
    ("values", include_str!("../scripts/values.exs")),
    ("types", include_str!("../scripts/types.exs")),
    ("errors", include_str!("../scripts/errors.exs")),
    ("tasks", include_str!("../scripts/tasks.exs")),
    ("datetime", include_str!("../scripts/datetime.exs")),
    ("project", include_str!("../scripts/project.exs")),
];

/// The library the `project` script names.
const APP_LIBRARY: &str = "type Record { id: Int } fn answer() -> Int { ret 42; }";

/// What one entry point has to produce.
pub enum Expected {
    /// Exactly this value.
    Value(ExsValue),
    /// An Error of this severity and kind. Its message and source positions may change.
    Error(ErrorSeverity, &'static str),
    /// An Int of at least this size.
    AtLeast(i64),
    /// An Error of this kind that records where it came from.
    Traced(&'static str),
}

/// One execution of an entry point of a module.
pub struct Case {
    /// The name of the module.
    pub module: &'static str,
    /// The entry point.
    pub function: &'static str,
    /// The inputs of the entry point.
    pub inputs: Vec<ExsValue>,
    /// The result it has to produce.
    pub expected: Expected,
}

/// Returns an Int value.
fn int(value: i64) -> ExsValue {
    ExsValue::Int(value)
}

/// Returns a String value.
fn text(value: &str) -> ExsValue {
    ExsValue::String(value.to_owned())
}

/// Returns a List value.
fn list(items: impl IntoIterator<Item = ExsValue>) -> ExsValue {
    ExsValue::List(items.into_iter().collect())
}

/// Returns an Object value with the fields in this order.
fn object<const N: usize>(fields: [(&str, ExsValue); N]) -> ExsValue {
    ExsValue::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// Returns a value of the nominal type `Point` of the `types` script.
fn point(x: i64, y: i64) -> ExsValue {
    ExsValue::Typed {
        type_id: "types.exs::Point".to_owned(),
        fields: vec![("x".to_owned(), int(x)), ("y".to_owned(), int(y))],
    }
}

/// Returns an Error with data, origin, trace and a cause.
fn full_error() -> ExsValue {
    let cause = ExsError::fatal("Cause", "the cause");
    ExsValue::Error(
        ExsError::recoverable("Custom", "it failed")
            .with_data(list([int(7)]))
            .with_origin(Some(SourcePositionId(3)))
            .with_trace(vec![ExsStackFrame {
                function_id: 1,
                call_site: SourcePositionId(2),
            }])
            .with_cause(ExsValue::Error(cause)),
    )
}

/// Returns one value of every kind that crosses the boundary, at the edges of each kind, with a
/// type and an enum this corpus does not declare.
fn every_kind() -> ExsValue {
    list([
        ExsValue::None,
        ExsValue::Unit,
        ExsValue::Bool(true),
        ExsValue::Bool(false),
        int(-7),
        int(i64::from(i32::MAX) + 1),
        int(i64::MIN),
        int(i64::MAX),
        ExsValue::Float(2.5),
        ExsValue::Float(f64::INFINITY),
        text(""),
        text("héllo \u{1F600}"),
        ExsValue::Bytes(vec![]),
        ExsValue::Bytes(vec![0, 255]),
        list([int(1), list([])]),
        object([("a", int(1)), ("b", object([]))]),
        ExsValue::Typed {
            type_id: "future.exs::Decimal".to_owned(),
            fields: vec![("v".to_owned(), int(1))],
        },
        ExsValue::Enum {
            type_id: "future.exs::Kind".to_owned(),
            variant: "New".to_owned(),
            fields: vec![int(2)],
        },
        full_error(),
    ])
}

/// Returns a case that expects exactly `expected`.
fn value(
    module: &'static str,
    function: &'static str,
    inputs: Vec<ExsValue>,
    expected: ExsValue,
) -> Case {
    Case {
        module,
        function,
        inputs,
        expected: Expected::Value(expected),
    }
}

/// Returns every case of the corpus.
#[must_use]
pub fn cases() -> Vec<Case> {
    let mut cases = script_cases();
    cases.extend(rust_guest_cases());
    cases
}

/// Returns the cases of the scripts.
fn script_cases() -> Vec<Case> {
    vec![
        value(
            "values",
            "main",
            vec![],
            list([
                int(1),
                int(23),
                ExsValue::Float(3.5),
                ExsValue::Float(3.5),
                int(6_000),
                text("a1"),
                text("2 {x} Ada"),
                text("ADA"),
                list([text("a"), text("b")]),
                ExsValue::Bytes(b"hi".to_vec()),
                list([int(1), int(2), int(3), int(4)]),
                object([("name", text("Ada")), ("age", int(36))]),
                int(7),
                list([text("h"), text("é"), text("y")]),
                int(3),
                ExsValue::Bool(true),
                ExsValue::Bool(true),
                ExsValue::Bool(true),
                text("0..3"),
                text("[1, \"a\"]"),
                ExsValue::Bool(true),
                ExsValue::None,
            ]),
        ),
        value("values", "identity", vec![every_kind()], every_kind()),
        value("values", "nothing", vec![], ExsValue::Unit),
        value("values", "through_host", vec![every_kind()], every_kind()),
        value(
            "values",
            "field_of",
            vec![ExsValue::Typed {
                type_id: "future.exs::Decimal".to_owned(),
                fields: vec![("v".to_owned(), int(1))],
            }],
            int(1),
        ),
        value(
            "types",
            "main",
            vec![],
            list([
                int(25),
                int(24),
                int(9),
                int(0),
                text("Point { x: 1, y: 2 }"),
                text("Shape::Circle(2)"),
                int(0),
                int(6),
                int(2),
                int(42),
                ExsValue::Bool(true),
                ExsValue::Bool(true),
            ]),
        ),
        value("types", "point", vec![], point(3, 4)),
        value(
            "types",
            "circle",
            vec![],
            ExsValue::Enum {
                type_id: "types.exs::Shape".to_owned(),
                variant: "Circle".to_owned(),
                fields: vec![int(2)],
            },
        ),
        value("types", "moved", vec![point(1, 2)], point(2, 3)),
        value(
            "types",
            "area_of",
            vec![ExsValue::Enum {
                type_id: "types.exs::Shape".to_owned(),
                variant: "Square".to_owned(),
                fields: vec![int(3)],
            }],
            int(9),
        ),
        value(
            "errors",
            "main",
            vec![],
            list([
                ExsValue::Bool(true),
                text("Custom"),
                text("it failed"),
                int(7),
                ExsValue::None,
                int(2),
                text("DivisionByZeroError"),
                text("IndexError"),
                text("IntOverflowError"),
                text("TypeError"),
            ]),
        ),
        Case {
            module: "errors",
            function: "recoverable",
            inputs: vec![],
            expected: Expected::Error(ErrorSeverity::Recoverable, "Custom"),
        },
        Case {
            module: "errors",
            function: "fatal",
            inputs: vec![],
            expected: Expected::Error(ErrorSeverity::Fatal, "TypeError"),
        },
        Case {
            module: "errors",
            function: "traced",
            inputs: vec![],
            expected: Expected::Traced("Custom"),
        },
        value(
            "errors",
            "from_host",
            vec![],
            list([
                text("Custom"),
                text("it failed"),
                list([int(7)]),
                text("Cause"),
            ]),
        ),
        value(
            "tasks",
            "main",
            vec![],
            list([
                list([int(2), int(4), int(4)]),
                list([int(10), int(10)]),
                int(6),
                object([
                    ("name", text("Ada")),
                    (
                        "values",
                        list([
                            int(1),
                            ExsValue::Float(2.5),
                            ExsValue::Bytes(b"x".to_vec()),
                            ExsValue::None,
                        ]),
                    ),
                ]),
                text("HostFunctionNotFound"),
            ]),
        ),
        value(
            "datetime",
            "main",
            vec![],
            list([
                text("2026-03-04T04:06:07.500000000Z"),
                int(2026),
                int(3),
                int(63),
                int(366),
                int(1_772_597_167),
                text("2026-03-05T06:06:07.500000000+01:00"),
                text("2026-03-04T13:06:07.500000000+09:00"),
                int(7_200),
                int(1_500),
                ExsValue::Bool(true),
            ]),
        ),
        value(
            "project",
            "main",
            vec![],
            list([
                int(8),
                int(10),
                int(10),
                int(42),
                int(42),
                int(1),
                text("{\"name\":\"Ada\",\"values\":[1,null]}"),
                object([(
                    "a",
                    list([
                        int(1),
                        ExsValue::Float(2.5),
                        ExsValue::None,
                        ExsValue::Bool(true),
                    ]),
                )]),
                text("a%20b%26c"),
                text("a b"),
            ]),
        ),
    ]
}

/// Returns the cases of the frozen Rust guest, which `examples/rust-guest` builds.
fn rust_guest_cases() -> Vec<Case> {
    let line = |price, quantity| object([("price", int(price)), ("quantity", int(quantity))]);
    vec![
        value(RUST_GUEST, "add", vec![int(34)], int(42)),
        value(
            RUST_GUEST,
            "round_trip",
            vec![every_kind()],
            list([list([every_kind()]), list([every_kind()])]),
        ),
        value(RUST_GUEST, "sum_stream", vec![], int(6)),
        value(
            RUST_GUEST,
            "clock",
            vec![],
            list([int(7_200), int(32_400), ExsValue::Bool(true)]),
        ),
        value(
            RUST_GUEST,
            "total",
            vec![list([line(3, 2), line(5, 4)])],
            object([("amount", int(26))]),
        ),
        Case {
            module: RUST_GUEST,
            function: "pause",
            inputs: vec![int(2)],
            expected: Expected::AtLeast(2),
        },
    ]
}

/// A finite stream of the Ints from 1 up to `end`.
struct Counter {
    /// The next value.
    next: i64,
    /// The last value.
    end: i64,
}

impl HostStream for Counter {
    /// Yields the next Int, or ends after the last one.
    fn next(&mut self) -> HostStreamFuture {
        let item = if self.next > self.end {
            HostStreamItem::End
        } else {
            self.next += 1;
            HostStreamItem::Item(int(self.next - 1))
        };
        Box::pin(std::future::ready(item))
    }
}

/// Returns a runner with the libraries and the host functions the corpus calls.
///
/// # Errors
///
/// Returns the failure of a registration.
pub fn runner() -> Result<Runner, String> {
    let mut runner = Runner::default();
    exs::libs::register(runner.registry_mut()).map_err(|error| error.to_string())?;
    let registry = runner.registry_mut();
    registry
        .fn_sync_raw("echo", |arguments: Vec<ExsValue>| {
            arguments.into_iter().next().unwrap_or(ExsValue::None)
        })
        .map_err(|error| error.to_string())?;
    registry
        .fn_sync_raw("fail", |_arguments: Vec<ExsValue>| full_error())
        .map_err(|error| error.to_string())?;
    registry
        .fn_async_raw("later", |arguments: Vec<ExsValue>| {
            Box::pin(async move {
                match arguments.as_slice() {
                    [ExsValue::Int(value)] => int(value + 1),
                    _ => ExsValue::None,
                }
            })
        })
        .map_err(|error| error.to_string())?;
    registry
        .stream_raw("counter", |arguments: Vec<ExsValue>| {
            let end = match arguments.as_slice() {
                [ExsValue::Int(end)] => *end,
                _ => 0,
            };
            Ok(Counter { next: 1, end })
        })
        .map_err(|error| error.to_string())?;
    Ok(runner)
}

/// Compiles the script `name` of the corpus with the current compiler.
///
/// # Errors
///
/// Returns the report of a failed compilation.
pub fn compile_script(name: &str, source: &str) -> Result<Vec<u8>, String> {
    let mut sources = Sources::new();
    exs::libs::add_to(&mut sources);
    sources.add("./helper.exs", include_str!("../scripts/helper.exs"));
    sources.add_library("app", APP_LIBRARY);
    let module = exs::compile(
        Source::new(format!("{name}.exs"), source),
        CompileOptions::default().resolver(&sources),
    )
    .map_err(|error| error.to_string())?;
    Ok(module.wasm)
}

/// Executes `case` on the module `wasm` and compares its result.
///
/// # Errors
///
/// Returns a description of a failed execution or a different result.
pub fn check(runner: &Runner, wasm: &[u8], case: &Case) -> Result<(), String> {
    let label = format!("{}::{}", case.module, case.function);
    let result = block_on(runner.execute(wasm, case.function, &case.inputs))
        .map_err(|error| format!("{label} failed to execute: {error}"))?;
    let matches = match (&case.expected, &result) {
        (Expected::Value(expected), result) => expected == result,
        (Expected::Error(severity, kind), ExsValue::Error(error)) => {
            error.severity == *severity && error.kind == *kind
        }
        (Expected::AtLeast(minimum), ExsValue::Int(value)) => value >= minimum,
        (Expected::Traced(kind), ExsValue::Error(error)) => {
            error.kind == *kind && error.origin.is_some() && !error.trace.is_empty()
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(format!("{label} returned {result:?}"))
    }
}

/// Runs the entry point `function` of the module `wasm`, whose host call cancels the execution,
/// and requires the module to stop through its cancel export.
///
/// # Errors
///
/// Returns a description of any other outcome.
pub fn check_cancellation(wasm: &[u8], function: &str) -> Result<(), String> {
    let cancellation = ExecutionCancellation::new();
    let cancel = cancellation.clone();
    let mut runner = Runner::default();
    runner
        .registry_mut()
        .fn_async_raw("echo", move |_arguments: Vec<ExsValue>| {
            cancel.cancel();
            Box::pin(std::future::ready(ExsValue::None))
        })
        .map_err(|error| error.to_string())?;
    let options = ExecutionOptions::with_cancellation(&cancellation);
    let inputs = [every_kind()];
    match block_on(runner.execute_with(wasm, function, &inputs, &options)) {
        Err(RunnerError::Cancelled) => Ok(()),
        other => Err(format!("{function} was not cancelled: {other:?}")),
    }
}

/// The entry points that suspend on the host call [`check_cancellation`] answers.
pub const CANCELLABLE: &[(&str, &str)] = &[("tasks", "waits"), (RUST_GUEST, "round_trip")];

/// Polls `future` to completion.
fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}
