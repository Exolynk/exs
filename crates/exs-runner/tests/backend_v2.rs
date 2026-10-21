//! Runs programs the runtime v2 prototype supports with both backends and compares them.

use std::time::Duration;

use exs_compiler::{Backend, CompileOptions, SourceInput, compile_with_backend};
use exs_runner::{
    ErrorSeverity, ExecutionCancellation, ExsError, ExsValue, ProtectionLevel, ServerRunner,
    SourcePositionId,
};

/// Polls one runner future to completion.
fn block_on<Output>(future: impl std::future::Future<Output = Output>) -> Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// Compiles and runs `source` with one backend and returns its result and fuel.
fn run(source: &str, backend: Backend, inputs: &[ExsValue]) -> (ExsValue, u64) {
    let compiled = match compile_with_backend(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        backend,
    ) {
        Ok(module) => module,
        Err(error) => panic!("{backend:?} compilation failed: {error}"),
    };
    let mut runner = ServerRunner::new(
        64 << 20,
        1_000_000_000,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    );
    register_hosts(&mut runner);
    match block_on(runner.execute_with_report(
        &compiled.wasm,
        "main",
        inputs,
        &ExecutionCancellation::new(),
    )) {
        Ok(report) => (report.value, report.fuel_consumed),
        Err(error) => panic!("{backend:?} execution failed: {error}"),
    }
}

/// Registers the host functions the programs call: `add` sums its Int arguments at once,
/// `echo` returns its first argument after suspending the task.
fn register_hosts(runner: &mut ServerRunner) {
    let add = runner
        .registry_mut()
        .fn_sync_raw("add", |arguments: Vec<ExsValue>| {
            ExsValue::Int(
                arguments
                    .iter()
                    .map(|argument| match argument {
                        ExsValue::Int(value) => *value,
                        _ => 0,
                    })
                    .sum(),
            )
        });
    assert!(add.is_ok());
    let echo = runner
        .registry_mut()
        .fn_async_raw("echo", |arguments: Vec<ExsValue>| async move {
            arguments.into_iter().next().unwrap_or(ExsValue::None)
        });
    assert!(echo.is_ok());
}

/// An unused closure, which makes v1 lower every function of a program as resumable. As the
/// last method of an existing type it shifts no function, type or source position id.
const RESUMABLE_PROBE: &str =
    " impl Range { fn resumable_probe() { let probe = () => { ret 1; }; } }";

/// Runs `source` with both backends, asserts equal results and returns both fuel values.
///
/// v1 is also run with every function lowered as resumable, which must not change the result:
/// synchronous and resumable lowering share one semantics.
fn compare(source: &str, inputs: &[ExsValue]) -> (u64, u64) {
    let (v1, v1_fuel) = run(source, Backend::V1, inputs);
    // A program with a closure is resumable already, and the probe would shift closure ids.
    if !source.contains("=>") {
        let (resumable, _) = run(&format!("{source}{RESUMABLE_PROBE}"), Backend::V1, inputs);
        assert_eq!(resumable, v1, "v1 lowering modes disagree on:\n{source}");
    }
    let (v2, v2_fuel) = run(source, Backend::V2, inputs);
    assert_eq!(v2, v1, "backends disagree on:\n{source}");
    (v1_fuel, v2_fuel)
}

/// Returns scalar results identical to v1 for literals, arithmetic, comparisons and control flow.
#[test]
fn matches_v1_for_scalar_programs() {
    let programs = [
        "fn main() { ret 42; }",
        "fn main() {}",
        "fn main() { ret true; }",
        "fn main() { ret 1.5 + 2; }",
        "fn main() { ret 7 / 2; }",
        "fn main() { ret 3000000000 * 3; }",
        "fn main() { ret -(2147483647 + 1); }",
        "fn main() { ret 2147483647 + 1 == 2147483648; }",
        "fn main() { ret 1 == 1.0 && true != false; }",
        "fn main() { ret !(2 < 1) || false; }",
        "fn main() { let total = 0; let index = 0; while index < 10 { if index == 3 { index = index + 1; continue; } if index == 8 { break; } total = total + index; index = index + 1; } ret total; }",
        "fn main() { let total = 0; for value in 0..10 { total = total + value; } for value in 5..=1 { total = total * 2 + value; } ret total; }",
        "fn square(value: Int) -> Int { ret value * value; } fn main() { ret square(square(3)) + square(-2); }",
        "fn fib(n: Int) -> Int { if n < 2 { ret n; } ret fib(n - 1) + fib(n - 2); } fn main() { ret fib(15); }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Decodes scalar arguments and encodes scalar results like v1.
#[test]
fn matches_v1_for_scalar_arguments() {
    let source = "fn main(a, b, c, d) { if c { ret a * 2 + b; } ret d; }";
    for inputs in [
        [
            ExsValue::Int(20),
            ExsValue::Int(2),
            ExsValue::Bool(true),
            ExsValue::None,
        ],
        [
            ExsValue::Int(i64::MAX / 4),
            ExsValue::Float(0.25),
            ExsValue::Bool(true),
            ExsValue::None,
        ],
        [
            ExsValue::Int(0),
            ExsValue::Int(0),
            ExsValue::Bool(false),
            ExsValue::Float(-1.5),
        ],
    ] {
        compare(source, &inputs);
    }
    compare(
        "fn main(limit: Int) -> Int { ret limit; }",
        &[ExsValue::Int(-3_000_000_000)],
    );
}

/// Keeps heap Ints and Floats alive across collections triggered by many allocations.
#[test]
fn matches_v1_while_collecting() {
    let source = "fn main(limit: Int) -> Float { let total = 0.0; let index = 0; while index < limit { total = total + 0.5 + (index + 3000000000) * 0; index = index + 1; } ret total; }";
    compare(source, &[ExsValue::Int(200_000)]);
}

/// Prints the fuel of both backends for scalar loops and calls.
#[test]
fn reports_scalar_fuel() {
    let int_loop = "fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { total = total + index; index = index + 1; } ret total; }";
    let for_range = "fn main(limit: Int) -> Int { let total = 0; for index in 0..limit { total = total + index; } ret total; }";
    let calls = "fn bump(value: Int) -> Int { ret value + 1; } fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { total = bump(total); index = index + 1; } ret total; }";
    let floats = "fn main(limit: Int) -> Float { let total = 0.0; let index = 0; while index < limit { total = total + 1.5; index = index + 1; } ret total; }";
    for (name, source) in [
        ("int_loop", int_loop),
        ("for_range", for_range),
        ("sync_calls", calls),
        ("float_loop", floats),
    ] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
}

/// Builds, compares and returns Strings exactly like v1.
#[test]
fn matches_v1_for_strings() {
    let programs = [
        r#"fn main() { ret "hello"; }"#,
        r#"fn main() { ret ""; }"#,
        r#"fn main() { ret "äöü " + "€ " + "𝄞"; }"#,
        r#"fn main() { ret "n=" + 7 + ", big=" + 3000000000 + ", f=" + 0.5 + ", whole=" + 2.0 + ", b=" + true; }"#,
        r#"fn main() { let name = "ExS"; let count = 3; ret f"Hallo {name}, {count} mal, {count * 1.5}!"; }"#,
        r#"fn main() { ret 1 + 2 + 0.5; }"#,
        r#"fn main() { ret "a" == "a" && "a" != "b" && "apple" < "banana" && "b" >= "b"; }"#,
        r#"fn main() { let text = "x"; let index = 0; while index < 5 { text = text + index; index = index + 1; } ret text; }"#,
        r#"fn pad(value: Int) -> String { if value < 10 { ret "0" + value; } ret "" + value; } fn main() { ret pad(4) + ":" + pad(12); }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
    compare(
        r#"fn main(name: String, suffix) { ret name + suffix + name; }"#,
        &[ExsValue::String("Grüße".to_owned()), ExsValue::Int(-5)],
    );
}

/// Keeps Strings alive across collections while building many temporary ones.
#[test]
fn matches_v1_for_strings_while_collecting() {
    let source = r#"fn main(limit: Int) -> String { let kept = "start"; let index = 0; while index < limit { let temporary = "row " + index + ": " + (index * 2) + ";"; if index == 1000 || index == 20000 { kept = kept + temporary; } index = index + 1; } ret kept; }"#;
    compare(source, &[ExsValue::Int(30_000)]);
}

/// Prints the fuel of both backends for String building.
#[test]
fn reports_string_fuel() {
    let format_loop = r#"fn main(limit: Int) -> Int { let length = 0; let index = 0; while index < limit { let line = f"row {index}: {index * 2};"; length = length + 1; index = index + 1; } ret length; }"#;
    let concat_loop = r#"fn main(limit: Int) -> Int { let count = 0; let index = 0; while index < limit { let line = "row " + index + ": " + (index * 2) + ";"; if line != "" { count = count + 1; } index = index + 1; } ret count; }"#;
    for (name, source) in [("format_loop", format_loop), ("concat_loop", concat_loop)] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
}

/// Builds, indexes, adds and iterates Lists like v1.
#[test]
fn matches_v1_for_lists() {
    let programs = [
        "fn main() { ret []; }",
        "fn main() { ret [1, 2.5, \"drei\", [true, None], []]; }",
        "fn main() { let items = [1, 2, 3]; items[1] = items[0] + items[2]; ret items; }",
        "fn main() { let items = [1, 2]; ret items + [3, 4] + 5 + [[6]]; }",
        "fn main() { let items = [1, 2]; let more = items + 3; more[0] = 9; ret [items, more]; }",
        "fn main() { let total = 0; for item in [1, 2, 3, 4] { if item == 2 { continue; } if item == 4 { break; } total = total + item; } ret total; }",
        "fn main() { let items = [1, 2, 3]; let seen = []; for item in items { items[0] = 7; seen = seen + item; } ret [items, seen]; }",
        "fn main() { let outer = [[1, 2], [3]]; let total = 0; for inner in outer { for item in inner { total = total + item; } } ret total; }",
        "fn main() { let items = [\"a\", \"b\"]; ret [items[0] + items[1] == \"ab\", items == items, items == [\"a\", \"b\"]]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
    let nested = ExsValue::List(vec![
        ExsValue::Int(1),
        ExsValue::List(vec![ExsValue::String("zwei".into()), ExsValue::None]),
        ExsValue::List(vec![]),
    ]);
    compare(
        "fn main(items) { items[1][0] = items[1][0] + \"!\"; ret items + [items[0]]; }",
        &[nested],
    );
}

/// Constructs typed Objects, reads and writes their fields and encodes them like v1.
#[test]
fn matches_v1_for_typed_objects() {
    let programs = [
        "type Point { x: Int, y: Int } fn main() { ret Point { x: 1, y: 2 }; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.y = point.x + point.y * 10; ret [point.x, point.y]; }",
        "type User { name: String, nickname: String | None, } fn main() { let user = User { name: \"Ada\" }; ret [user, user.nickname]; }",
        "type Point { x: Int, y: Int } type Size { y: Int, z: Int } fn main() { let items = [Point { x: 1, y: 2 }, Size { y: 3, z: 4 }]; let total = 0; for item in items { total = total + item.y; } ret total; }",
        "type Point { x: Int, y: Int } type Line { from: Point, to: Point } fn main() { let line = Line { from: Point { x: 0, y: 0 }, to: Point { x: 3, y: 4 } }; line.to.x = line.to.x + 1; ret line; }",
        "type Point { x: Int, y: Int } fn shift(point: Point) -> Point { point.x = point.x + 1; ret point; } fn main() { ret shift(shift(Point { x: 1, y: 2 })); }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; ret [point[\"x\"], point == point, point == Point { x: 1, y: 2 }]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Keeps Lists and typed Objects alive across collections triggered by many allocations.
#[test]
fn matches_v1_for_collections_while_collecting() {
    let source = "type Cell { value: Int, next: Cell | None } fn main(limit: Int) -> Int { let head = None; let items = []; let index = 0; let countdown = 0; while index < limit { head = Cell { value: index, next: head }; if countdown == 0 { items = items + [head]; countdown = 100; } countdown = countdown - 1; index = index + 1; } let total = 0; for item in items { total = total + item.value; } let cursor = head; while cursor != None { total = total + cursor.value; cursor = cursor.next; } ret total; }";
    compare(source, &[ExsValue::Int(30_000)]);
}

/// Prints the fuel of both backends for List and typed Object workloads.
#[test]
fn reports_collection_fuel() {
    let property = "type Point { x: Int, y: Int } fn main(limit: Int) -> Int { let point = Point { x: 1, y: 2 }; let total = 0; let index = 0; while index < limit { total = total + point.x + point.y; point.x = index; index = index + 1; } ret total; }";
    let literals = "type Point { x: Int, y: Int } fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { let point = Point { x: index, y: 1 }; let items = [point, index]; total = total + items[0].y; index = index + 1; } ret total; }";
    let index = "fn main(limit: Int) -> Int { let items = [1, 2, 3, 4, 5, 6, 7, 8]; let total = 0; let index = 0; let slot = 0; while index < limit { total = total + items[slot]; items[slot] = index; slot = slot + 1; if slot == 8 { slot = 0; } index = index + 1; } ret total; }";
    let iterate = "fn main(limit: Int) -> Int { let items = []; let index = 0; while index < 100 { items = items + index; index = index + 1; } let total = 0; let round = 0; while round < limit / 100 { for item in items { total = total + item; } round = round + 1; } ret total; }";
    for (name, source) in [
        ("property_access", property),
        ("object_literals", literals),
        ("list_index", index),
        ("list_iteration", iterate),
    ] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
}

/// Returns an Object value of `entries`.
fn object(entries: &[(&str, ExsValue)]) -> ExsValue {
    ExsValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).into(), value.clone()))
            .collect(),
    )
}

/// Builds, reads, writes and extends untyped Objects like v1.
#[test]
fn matches_v1_for_untyped_objects() {
    let programs = [
        "fn main() { ret {}; }",
        "fn main() { ret { name: \"Ada\", age: 36, tags: [1, 2] }; }",
        "fn main() { ret { a: 1, b: 2, a: 3 }; }",
        "fn main() { let item = { a: 1 }; item.b = item.a + 1; item[\"c\"] = item.b * 2; item.a = 0; ret [item, item.missing, item[\"b\"]]; }",
        "fn main() { let items = [{ x: 1, y: 2 }, { y: 3, x: 4 }, { x: 5 }]; let total = 0; for item in items { total = total + item.x; if item.y != None { total = total + item.y; } } ret total; }",
        "type Point { x: Int, y: Int } fn main() { let items = [Point { x: 1, y: 2 }, { y: 10 }, { z: 0, y: 20 }]; let total = 0; for item in items { total = total + item.y; } ret total; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.label = \"p\"; point[\"z\"] = point.x; ret [point, point.label]; }",
        "fn main() { let item = { a: 1 }; let other = { a: 1 }; ret [item == item, item == other]; }",
        "fn main() { let item = {}; let index = 0; while index < 20 { item[\"k\" + index] = index; index = index + 1; } ret item; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
    let input = object(&[
        ("name", ExsValue::String("Ada".into())),
        (
            "items",
            ExsValue::List(vec![object(&[("x", ExsValue::Int(1))]), object(&[])]),
        ),
    ]);
    compare(
        "fn main(input) { input.count = 2; input.items[0].x = input.items[0].x + 1; ret input; }",
        &[input],
    );
}

/// Keeps untyped Objects and shapes alive across collections triggered by many allocations.
#[test]
fn matches_v1_for_untyped_objects_while_collecting() {
    let source = "fn main(limit: Int) -> Int { let items = []; let index = 0; let countdown = 0; let last = { value: 0 }; while index < limit { last = { value: index, previous: last, label: \"n\" + index }; if countdown == 0 { items = items + [last]; countdown = 500; } countdown = countdown - 1; index = index + 1; } let total = 0; for item in items { total = total + item.value + item.previous.value; } ret total; }";
    compare(source, &[ExsValue::Int(20_000)]);
}

/// Prints the fuel of both backends for untyped Object workloads.
#[test]
fn reports_untyped_object_fuel() {
    let property = "fn main(limit: Int) -> Int { let point = { x: 1, y: 2 }; let total = 0; let index = 0; while index < limit { total = total + point.x + point.y; point.x = index; index = index + 1; } ret total; }";
    let literals = "fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit { let point = { x: index, y: 1, label: \"p\" }; total = total + point.y; index = index + 1; } ret total; }";
    let concat = "fn main(items) -> String { let output = \"\"; for item in items { output = output + item.name + \";\"; } ret output; }";
    for (name, source) in [("object_property", property), ("object_literals", literals)] {
        let (v1, v2) = compare(source, &[ExsValue::Int(1000)]);
        println!("v2 fuel {name}: v1 {v1} v2 {v2}");
    }
    for count in [10, 100] {
        let items = (0..count)
            .map(|index| {
                object(&[
                    ("id", ExsValue::Int(index)),
                    ("name", ExsValue::String(format!("name {index}"))),
                    ("active", ExsValue::Bool(index % 2 == 0)),
                ])
            })
            .collect();
        let (v1, v2) = compare(concat, &[ExsValue::List(items)]);
        println!("v2 fuel concat_{count}_objects: v1 {v1} v2 {v2}");
    }
}

/// Produces the same Error values as v1, including kinds, messages, data, origins and traces.
#[test]
fn matches_v1_for_errors() {
    let programs = [
        "fn main() { ret 1 + \"a\"; }",
        "fn main() { ret \"a\" + None; }",
        "fn main() { let text = \"x\"; ret text + 1 + None + 2; }",
        "fn main() { ret 9223372036854775807 + 1; }",
        "fn main() { let big = 9223372036854775807; ret [big * 2, big - -1, -(-big - 1)]; }",
        "fn main() { ret [!5, 1.5 / None, -\"x\"]; }",
        "fn main() { ret -9223372036854775808; }",
        "fn main() { let e = 1 + \"a\"; ret e + 1; }",
        "fn main() { if 1 { ret 2; } ret 3; }",
        "fn main() -> Int { if None { ret 2; } ret 3; }",
        "fn main() { let count = 0; while count { count = count + 1; } ret count; }",
        "fn main() { ret 1 && true; }",
        "fn main() { ret true && 2; }",
        "fn main() { ret false || None; }",
        "fn main() { let items = [1]; ret [items[5], items[-1], items[\"x\"], items[true], items[4294967296]]; }",
        "fn main() { let value = 1; ret [value[0], value.name, { a: 1 }[1], None.x]; }",
        "fn main() { let items = [1]; items[3] = 2; items.x = 1; let number = 5; number.y = 2; number[0] = 1; ret items; }",
        "fn inner() { ret 1 + \"a\"; } fn middle() { ret inner(); } fn main() { ret middle(); }",
        "fn inner() -> Int { ret \"x\"; } fn middle() { let value = inner(); ret value + 1; } fn main() { ret [middle(), 2]; }",
        "fn take(a: Int) { ret a; } fn main() { ret take(\"x\"); }",
        "fn take(a: Int) -> Int { ret a; } fn main() { ret take(\"x\"); }",
        "type Point { x: Int } fn main() { ret Point { y: 1 }; }",
        "type Point { x: Int } fn main() { ret Point { x: 1, x: 2 }; }",
        "type Point { x: Int } fn main() { ret Point { x: \"a\" }; }",
        "type Point { x: Int } fn main() { ret Point {}; }",
        "fn main() { let x: Int = \"a\"; ret x; }",
        "fn main() { let x: Int = 1 + \"a\"; ret x; }",
        "fn main() -> Int { let x: Int = \"a\"; ret x; }",
        "fn main() { for item in 5 { ret item; } ret 0; }",
        "fn main() { ret Error(\"Custom\", \"went wrong\", [1, { a: 2 }]); }",
        "fn main() { ret [Error(1, \"m\", None), Error(\"k\", 2, None)]; }",
        "fn fail() -> Int | Error { ret Error(\"Custom\", \"m\", 1); } fn main() -> Any { let value = fail()?; ret value + 1; }",
        "fn missing() { ret None; } fn main() -> Any { let value = missing()?; ret value; }",
        "fn fine() { ret 4; } fn main() -> Any { ret fine()? + 1; }",
        "fn main() -> Int { ret \"x\"; }",
        "fn main() -> Int { }",
    ];
    for program in programs {
        compare(program, &[]);
    }
    compare(
        "fn main(a) { ret a; }",
        &[ExsValue::Int(1), ExsValue::Int(2)],
    );
}

/// Keeps Errors, their data and traces alive across collections triggered by many Errors.
#[test]
fn matches_v1_for_errors_while_collecting() {
    let source = "fn fail(index) { ret \"n\" + index + None; } fn main(limit: Int) { let kept = []; let index = 0; let countdown = 0; while index < limit { let error = fail(index); if countdown == 0 { kept = kept + [error]; countdown = 2000; } countdown = countdown - 1; index = index + 1; } ret kept; }";
    compare(source, &[ExsValue::Int(20_000)]);
}

/// Renders every value in formatted strings like v1, including None, Lists, Objects and Errors.
#[test]
fn matches_v1_for_formatted_strings() {
    let programs = [
        r#"fn main() { ret f"none {None}, list {[1, 2]}, object {{ a: 1 }}, flag {true}"; }"#,
        r#"type Point { x: Int } fn main() { let error = 1 + "a"; ret f"{Point { x: 1 }} {error} {-2.5} {9223372036854775807}"; }"#,
        r#"fn main() { ret f"{""}{"ä"}{1}"; }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Dispatches operator and `ToString` implementations, which 3a and 3b guarded with traps.
#[test]
fn matches_v1_for_trait_implementations() {
    let programs = [
        "type Number { value: Int } impl Add for Number { fn add(self, other: Any) -> Any { ret Number { value: self.value + other.value }; } } fn main() { let one = Number { value: 1 }; ret one + one; }",
        "type Number { value: Int } impl ToString for Number { fn to_string(self) -> String { ret \"n\"; } } fn main() { ret f\"{Number { value: 1 }}\"; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates built-in numeric, value and Error methods like v1, including their Errors.
#[test]
fn matches_v1_for_builtin_methods() {
    let programs = [
        "fn main() { ret [(7).add(5), (7).sub(5), (7).mul(5), (7).div(2), \"a\".add(1), [1].add(2)]; }",
        "fn main() { ret [(-7).div_euclid(2), (-7).rem_euclid(2), (7).div_euclid(0), true.div_euclid(1), (-9223372036854775807 - 1).div_euclid(-1)]; }",
        "fn main() { ret [(-3).abs(), (-2.5).abs(), (-9223372036854775807 - 1).abs(), \"x\".abs(), true.abs()]; }",
        "fn main() { let value = 2.5; ret [value.floor(), value.ceil(), value.round(), (-2.5).round(), value.trunc(), value.fract(), 2.0.sqrt(), (2).sqrt()]; }",
        "fn main() { ret [(-4).signum(), (0).signum(), (4).is_even(), (3).is_odd(), 4.0.is_even()]; }",
        "fn main() { ret [(12).gcd(-18), (0).gcd(0), (4).lcm(6), (0).lcm(5), (3037000500).lcm(3037000501), (-9223372036854775807 - 1).gcd(0), (2).gcd(1.0)]; }",
        "fn main() { ret [(2).pow(10), (2).pow(-1), (2).pow(1.5), (2).pow(63), 2.0.pow(0.5), 2.0.pow(2), \"x\".pow(2)]; }",
        "fn main() { ret [(3).min(5), (3).max(5), 3.0.min(5.0), 3.0.max(5), (1).min(true), None.max(1)]; }",
        "fn main() { ret [(5).clamp(1, 3), (5).clamp(3, 1), (5).clamp(1.0, 3), 2.5.clamp(1.0, 2.0), 2.5.clamp(2.0, 1.0), \"x\".clamp(1, 2)]; }",
        "fn main() { ret [(3).to_float(), 3.9.to_int(), (-3.9).to_int(), (1.0 / 0.0).to_int(), 1e300.to_int(), (3).to_int(), 3.0.to_float()]; }",
        "fn main() { let nan = 0.0 / 0.0; ret [nan.is_nan(), (1.0 / 0.0).is_infinite(), 1.0.is_finite(), (1).is_nan()]; }",
        "fn main() { ret [(1).to_string(), 2.5.debug(), None.to_string(), [1].to_string(), { a: 1 }.debug(), true.to_string(), \"s\".to_string()]; }",
        "type Point { x: Int } fn main() { let point = Point { x: 1 }; point.extra = 2; ret [\"äb\".length(), [1, 2].length(), { a: 1, b: 2 }.length(), point.length(), \"\".is_empty(), [].is_empty(), (1).length()]; }",
        "fn main() { let error = Error(\"K\", \"m\", [1]); ret [error.kind(), error.message(), error.data(), error.cause(), (1).kind()]; }",
        "fn main() { ret [(1).unknown(), (1).abs(2), (1).pow(), (1).clamp(1)]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates `Int::parse`, `Float::parse` and the `Math` functions like v1.
#[test]
fn matches_v1_for_static_builtins() {
    let programs = [
        "fn main() { ret [Int::parse(\"-42\"), Int::parse(\"+7\"), Int::parse(\"4.2\"), Int::parse(\"99999999999999999999\"), Int::parse(1)]; }",
        "fn main() { ret [Float::parse(\"2.5\"), Float::parse(\"1e3\"), Float::parse(\"inf\"), Float::parse(\"x\"), Float::parse(None)]; }",
        "fn main() { ret [Math::pi(), Math::tau(), Math::e(), Math::sin(1.0), Math::cos(1.0), Math::tan(1.0), Math::asin(0.5), Math::acos(0.5), Math::atan(1.0)]; }",
        "fn main() { ret [Math::exp(1.0), Math::ln(2.0), Math::log2(8.0), Math::log10(1000.0), Math::atan2(1.0, 2.0), Math::hypot(3.0, 4.0), Math::sin(1), Math::atan2(1, 2.0), Math::hypot(1.0, 2)]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates List methods like v1, including their Errors and order of checks.
#[test]
fn matches_v1_for_list_methods() {
    let programs = [
        "fn main() { let items = [1]; let lengths = [items.push(2), items.push([3])]; ret [items, lengths, items.pop(), items.pop(), items.pop(), items.pop(), items]; }",
        "fn main() { let items = [1, 2, 3]; ret [items.get(1), items.get(5), items.get(-1), items.first(), items.last(), [].first(), [].last(), (1).first()]; }",
        "fn main() { let items = [1, 2, 3, 4]; ret [items.slice(1, 3), items.slice(0, 0), items.slice(3, 1), items.slice(0, 9), items.slice(-1, 2), \"x\".slice(0, 1), (1).slice(0, 1)]; }",
        "fn main() { let items = [1, 2]; let more = [3]; ret [items.extend(more), items, items.extend(items), items, items.extend(1)]; }",
        "fn main() { let items = [1, 2, 3]; let copy = items.reversed(); ret [items.reverse(), items, copy, [].reversed()]; }",
        "fn main() { let items = [1, 2.0, \"a\", None, 2]; ret [items.contains(2), items.contains(\"a\"), items.contains(None), items.contains(5), items.index_of(2), items.last_index_of(2), items.index_of(7), (1).contains(1)]; }",
        "fn main() { ret [[\"a\", \"b\", \"c\"].join(\", \"), [].join(\"-\"), [\"ä\"].join(\"\"), [\"a\", 1].join(\"-\"), [\"a\"].join(1), (1).join(\"x\")]; }",
        "fn main() { let items = [1, 3]; ret [items.insert(1, 2), items, items.insert(3, 4), items, items.insert(9, 0), items.remove(0), items, items.remove(5), items.remove(true)]; }",
        "fn main() { let items = [1, 2]; let object = { a: 1 }; ret [items.clear(), items, object.clear(), object, (1).clear()]; }",
        "fn main() { let items = []; let index = 0; while index < 100 { items.push(index); index = index + 1; } let total = 0; while items.length() > 0 { total = total + items.pop(); } ret [total, items]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates Object methods like v1, on untyped and typed Objects.
#[test]
fn matches_v1_for_object_methods() {
    let programs = [
        "fn main() { let object = { a: 1, b: 2 }; ret [object.has(\"a\"), object.has(\"z\"), object.has(1), object.get(\"b\"), object.get(\"z\"), object.get_or(\"z\", 9), object.get_or(\"a\", 9), (1).has(\"a\")]; }",
        "fn main() { let object = { a: 1, b: 2, c: 3 }; ret [object.delete(\"b\"), object, object.delete(\"z\"), object.keys(), object.values(), object.entries(), object.length()]; }",
        "fn main() { let object = { a: 1, b: 2 }; object.delete(\"a\"); object.a = 5; object.d = 4; ret [object, object.keys()]; }",
        "fn main() { let left = { a: 1, b: 2 }; let right = { b: 3, c: 4 }; ret [left.merge(right), left, right, left.merge(1), (1).merge(left)]; }",
        "fn main() { ret [Object::from_entries([[\"a\", 1], [\"b\", 2], [\"a\", 3]]), Object::from_entries([]), Object::from_entries([[1, 2]]), Object::from_entries([[\"a\"]]), Object::from_entries(1)]; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.label = \"p\"; ret [point.keys(), point.values(), point.entries(), point.length(), point.has(\"label\"), point.get(\"y\"), point.merge({ z: 3 })]; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.extra = 3; ret [point.delete(\"x\"), point.delete(\"extra\"), point, point.x, point.length(), point.keys(), point.has(\"x\")]; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.extra = 3; point.clear(); ret [point, point.length(), point.y]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates String methods like v1, by characters and with Unicode whitespace.
#[test]
fn matches_v1_for_string_methods() {
    let programs = [
        "fn main() { let text = \"häll😀o\"; ret [text.slice(1, 4), text.slice(0, 7), text.slice(3, 1), text.slice(0, 8), text.slice(-1, 2), text.slice(0, 1.0)]; }",
        "fn main() { let text = \"hello world\"; ret [text.contains(\"lo w\"), text.contains(\"\"), text.contains(\"x\"), text.starts_with(\"he\"), text.ends_with(\"ld\"), text.ends_with(\"\"), text.contains(1), (1).starts_with(\"a\")]; }",
        "fn main() { ret [\"ab\".repeat(3), \"ä\".repeat(0), \"x\".repeat(-1), \"x\".repeat(1.5), (1).repeat(2)]; }",
        "fn main() { let text = \" \\t\\u{3000}hi \\n\"; ret [text.trim(), text.trim_start(), text.trim_end(), \"\".trim(), (1).trim()]; }",
        "fn main() { ret [\"a-b-c\".replace(\"-\", \"+\"), \"aaaa\".replace(\"aa\", \"b\"), \"ab\".replace(\"\", \"-\"), \"äöü\".replace(\"ö\", \"oe\"), \"x\".replace(\"y\", \"z\"), \"x\".replace(1, \"z\"), \"x\".replace(\"x\", 2)]; }",
        "fn main() { ret [\"a,b,,c\".split(\",\"), \"abc\".split(\"\"), \"\".split(\",\"), \"äxöxü\".split(\"x\"), \"aaa\".split(\"aa\"), \"a\".split(1), (1).split(\",\")]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Clones values deeply like v1, keeping aliases and cycles.
#[test]
fn matches_v1_for_clone() {
    let programs = [
        "fn main() { let inner = [1, 2]; let outer = [inner, inner, { list: inner }]; let copy = outer.clone(); copy[0].push(3); ret [outer, copy]; }",
        "fn main() { let items = [1]; items.push(items); let copy = items.clone(); copy.push(2); ret [items.length(), copy.length(), copy[1].length()]; }",
        "type Point { x: Int, y: Int } fn main() { let point = Point { x: 1, y: 2 }; point.tag = [1]; let copy = point.clone(); copy.x = 5; copy.tag.push(2); ret [point, copy]; }",
        "fn main() { let error = Error(\"K\", \"m\", [1]); let copy = error.clone(); ret [copy, copy == error, \"s\".clone(), (1).clone(), None.clone()]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Evaluates Bytes literals, methods and constructors like v1, including their Errors.
#[test]
fn matches_v1_for_bytes() {
    let programs = [
        r#"fn main() { let data = b"häll"; ret [data, data.length(), data.is_empty(), b"".is_empty(), data[0], data[9], data[-1], data == b"häll", data == b"x", data.to_string(), f"{data}"]; }"#,
        r#"fn main() { let data = b"abcdef"; ret [data.slice(1, 3), data.slice(3, 1), data.slice(0, 9), data.slice(-1, 2), data.concat(b"gh"), data.concat("gh"), "x".concat(data)]; }"#,
        r#"fn main() { let data = b"abcabc"; ret [data.contains(b"ca"), data.contains(b""), data.contains(b"x"), data.starts_with(b"ab"), data.ends_with(b"bc"), data.contains("ca"), data.repeat(2), data.repeat(-1), data.repeat(0)]; }"#,
        r#"fn main() { ret [Bytes::from_list([255, 0]).to_hex(), b"hi!".to_base64(), b"h".to_base64(), b"hi".to_base64(), b"".to_base64(), b"ok".decode_utf8(), "äb".encode_utf8()]; }"#,
        r#"fn main() { ret [Bytes::from_list([0, 255, 7]), Bytes::from_list([256]), Bytes::from_list([1.0]), Bytes::from_list(1), Bytes::from_utf8("ä"), Bytes::from_utf8(1)]; }"#,
        r#"fn main() { ret [Bytes::from_hex("00ffA0"), Bytes::from_hex("abc"), Bytes::from_hex("zz"), Bytes::from_base64("aGkh"), Bytes::from_base64("aA=="), Bytes::from_base64("aB=="), Bytes::from_base64("aGk"), Bytes::from_base64("a=bc"), Bytes::from_base64("")]; }"#,
        r#"fn main() { let data = Bytes::from_list([255]); ret [data.decode_utf8(), data.clone() == data, [data].contains(Bytes::from_list([255]))]; }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
    compare(
        "fn main(data) { ret [data, data.length(), data.to_hex()]; }",
        &[ExsValue::Bytes(vec![1, 2, 250])],
    );
}

/// Changes the case of Strings like v1, including final sigma and multi-character mappings.
#[test]
fn matches_v1_for_case_mapping() {
    let programs = [
        r#"fn main() { ret ["Hello Wörld".to_lowercase(), "Hello Wörld".to_uppercase(), "straße ﬁ ŉ".to_uppercase(), "İ".to_lowercase(), "ΟΔΟΣ ΣΑΣ".to_lowercase(), "Α'Σ.".to_lowercase(), "".to_uppercase(), 1.to_lowercase()]; }"#,
        r#"fn main() { let text = "ΣΑΣ"; ret [text.to_lowercase().to_uppercase(), text.to_lowercase().length()]; }"#,
    ];
    for program in programs {
        compare(
            &program.replace("1.to_lowercase()", "(1).to_lowercase()"),
            &[],
        );
    }
    let without = run("fn main() { ret \"Hi\".trim(); }", Backend::V2, &[]).1;
    let with = run("fn main() { ret \"Hi\".to_lowercase(); }", Backend::V2, &[]).1;
    println!("v2 fuel without case tables {without}, with {with}");
}

/// Constructs, matches, tests, renders and orders enums like v1, including their Errors.
#[test]
fn matches_v1_for_enums() {
    let color =
        "enum Color { Rgb(red: Int, green: Int, blue: Int), Gray(level: Int), Transparent, }";
    let programs = [
        "fn main() { ret [Color::Rgb(1, 2, 3), Color::Transparent, Color::Gray(7)]; }",
        "fn main() { let color = Color::Rgb(1, 2, 3); ret [color == Color::Rgb(1, 2, 3), color == Color::Rgb(1, 2, 4), Color::Transparent == Color::Transparent, color.clone(), f\"{color} {Color::Transparent}\", color.to_string()]; }",
        "fn name(color: Color) -> String { ret match color { Color::Rgb(r, g, b) => f\"{r}-{g}-{b}\", Color::Gray(level) => f\"gray {level}\", Color::Transparent => \"none\", }; } fn main() { ret [name(Color::Rgb(1, 2, 3)), name(Color::Gray(4)), name(Color::Transparent)]; }",
        "fn main() { let total = 0; for color in [Color::Gray(2), Color::Transparent, Color::Gray(5)] { total = total + match color { Color::Gray(level) => level, _ => 100, }; } ret total; }",
        "fn kind(value) { ret match value { Color::Transparent => 0, Color::Gray(level) => { let x = level; }, Color::Rgb(r, g, b) => r + g + b, }; } fn main() { ret [kind(Color::Transparent), kind(Color::Gray(1)), kind(Color::Rgb(1, 2, 3))]; }",
        "fn kind(value) -> Int { ret match value { Color::Transparent => 0, Color::Gray(level) => level, Color::Rgb(r, g, b) => r, }; } fn main() { ret kind(1); }",
        "fn kind(value) { ret match value { Color::Transparent => 0, Color::Gray(level) => level, Color::Rgb(r, g, b) => r, }; } fn main() { ret [kind(\"x\")]; }",
        "fn main() { let c = Color::Gray(1); ret [c is Color, 1 is Color, c is Int, c is Int | Color, Color::Transparent is Color, None is Color]; }",
        "fn main() { ret Color::Gray(\"x\"); }",
        "fn check(value: Color) -> Int { ret 1; } fn main() { ret [check(Color::Transparent), check(1)]; }",
        "fn main() { let o = Color::Gray(1).compare(Color::Gray(2)); ret [o, (1).compare(2), (2).compare(1), \"b\".compare(\"a\"), (1).compare(1.0), [1].compare([2]), f\"{(1).compare(2)}\"]; }",
        "fn main() { ret [match (1).compare(2) { Ordering::Less => \"lt\", Ordering::Equal => \"eq\", Ordering::Greater => \"gt\", Ordering::Unordered => \"un\", }, std::Ordering::Less == Ordering::Less]; }",
        "fn main() { ret [1 < \"a\", [1] <= [1], [1] < [2], 0.0 / 0.0 > 1]; }",
        "fn main() { let ordering = Ordering::Greater; ret [ordering, ordering is Ordering, Ordering::Unordered]; }",
    ];
    for program in programs {
        compare(&format!("{color} {program}"), &[]);
    }
    let input = ExsValue::Enum {
        type_id: "test.exs::Color".to_owned(),
        variant: "Rgb".to_owned(),
        fields: vec![ExsValue::Int(1), ExsValue::Int(2), ExsValue::Int(3)],
    };
    compare(
        &format!(
            "{color} fn main(value: Color) {{ ret [value, match value {{ Color::Rgb(r, g, b) => r + g + b, _ => 0, }}]; }}"
        ),
        &[input],
    );
}

/// Checks `List<T>` contracts like v1, which only tests the List itself.
#[test]
fn matches_v1_for_list_contracts() {
    let programs = [
        "fn first(items: List<Int>) -> Int { ret items[0]; } fn main() { ret [first([1, 2]), first([\"a\"])]; }",
        "fn wrap(value) -> List<String> { ret [value]; } fn main() { ret [wrap(1), wrap(\"a\")]; }",
        "fn main() { ret [[1] is List<Int>, [\"a\"] is List<Int>, 1 is List<Int>]; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Lists the fields of typed Objects in the order of the literal that built them, like v1.
#[test]
fn matches_v1_for_typed_object_field_order() {
    let point = "type Point { x: Int, y: Int | None, z: Int | None }";
    let programs = [
        "fn main() { ret [Point { z: 3, x: 1 }, Point { x: 1, z: 3 }, Point { y: 2, x: 1 }, Point { x: 1 }]; }",
        "fn main() { let point = Point { z: 3, x: 1 }; point.y = 2; point.extra = 4; ret [point.keys(), point.values(), point.entries(), point.y, point.extra]; }",
        "fn main() { let point = Point { z: 3, x: 1 }; ret [point.clone(), point.merge({ a: 1 }), { a: 1 }.merge(point), point.clone().keys(), point.length()]; }",
        "fn main() { let point = Point { z: 3, x: 1, y: 2 }; point.delete(\"x\"); ret [point, point.keys(), point.get(\"z\")]; }",
        "fn make(value) { ret Point { z: value, x: 1 }; } fn main() { ret [make(1), make(2), Point { z: 5, x: 6 }.keys()]; }",
    ];
    for program in programs {
        compare(&format!("{point} {program}"), &[]);
    }
}

/// Decodes annotated `let` and `for` bindings like v1's wire decoding.
#[test]
fn matches_v1_for_wire_decoding() {
    let types =
        "type Point { x: Int, y: Int | None } enum Color { Gray(level: Int), Transparent, }";
    let programs = [
        "fn main() { let c: Color = Color::Gray(1); let t: Color = Color::Transparent; ret [c, t]; }",
        "fn main() { let p: Point = { y: 2, x: 1, extra: 3 }; ret [p, p is Point, p.keys()]; }",
        "fn main() { let p: Point = Point { y: 2, x: 1 }; ret p.keys(); }",
        r#"fn main() { let xs: List<Int> = [1, "a"]; ret xs; }"#,
        "fn main() { let xs: List<Point> = [{ x: 1 }]; ret [xs[0] is Point, xs]; }",
        r#"fn main() { let c: Color = { "$variant": "Gray", level: 3 }; ret [c, c is Color]; }"#,
        r#"fn main() { let c: Color = { "$variant": "Gray", "$values": [4] }; ret c; }"#,
        r#"fn main() { let c: Color = { "$variant": "Transparent" }; ret c; }"#,
        r#"fn main() { let c: Color = { "$variant": 1 }; ret c; }"#,
        "fn main() { let o: Ordering = (1).compare(2); ret o; }",
        r#"fn main() { let total = 0; for x: Int in [1, "a"] { total = total + 1; } ret total; }"#,
        "fn main() { let total = 0; for p: Point in [{ x: 1 }, { x: 2, y: 3 }] { total = total + p.x; } ret total; }",
        "fn main() { let total = 0; for x: Int in 0..3 { total = total + x; } ret total; }",
        "fn main() { let total = 0; for x: String in 0..3 { total = total + 1; } ret total; }",
        "fn main() { let a: Int | Point = { x: 1 }; ret [a, a is Point]; }",
        "fn main() { let l = [1]; let m: List<Int> = l; ret [m == l, m]; }",
        r#"fn main() { let p: Point = { y: "s" }; ret p; }"#,
        r#"fn main() { let n: Int = "x"; ret n; }"#,
        "fn main() { let c: Color = 5; ret c; }",
        r#"fn main() { let c: Color | Int = { "$variant": "Nope" }; ret c; }"#,
        "fn main() { let p: Point = Color::Gray(1); ret p; }",
        r#"fn main() { let e: List<Int> = [1, Error("K", "m", None)]; ret 1; }"#,
        r#"fn main() { let e: Int = Error("K", "m", None); ret 1; }"#,
        "fn main() { let nested: List<List<Point>> = [[{ x: 1 }], []]; ret nested; }",
        "fn read() -> Int { let p: Point = { y: 1 }; ret 1; } fn main() { ret read(); }",
    ];
    for program in programs {
        compare(&format!("{types} {program}"), &[]);
    }
}

/// Dispatches implementation methods, static methods, operator traits and `ToString` like v1.
#[test]
fn matches_v1_for_implementations() {
    let types = r#"
        type Money { cents: Int }
        type Version { major: Int, minor: Int }
        type User { name: String }
        enum Shape { Circle(radius: Int), Square(side: Int), }
        trait Area { fn area(self) -> Int; fn double(self) -> Int { ret self.area() * 2; } }
        impl Money {
            fn new(cents: Int) -> Money { ret Money { cents: cents }; }
            fn euros(self) -> Float { ret self.cents / 100; }
            fn add_cents(self, cents: Int) -> Money { ret Money { cents: self.cents + cents }; }
        }
        impl Add for Money { fn add(self, other: Any) -> Any { ret Money { cents: self.cents + other.cents }; } }
        impl Sub for Money { fn sub(self, other: Any) -> Any { ret Money { cents: self.cents - other.cents }; } }
        impl Mul for Money { fn mul(self, other: Any) -> Any { ret Money { cents: self.cents * other }; } }
        impl Div for Money { fn div(self, other: Any) -> Any { ret Money { cents: self.cents / other }; } }
        impl Compare for Version {
            fn compare(self, other: Any) -> Ordering {
                if !(other is Version) { ret Ordering::Unordered; }
                if self.major != other.major { ret self.major.compare(other.major); }
                ret self.minor.compare(other.minor);
            }
        }
        impl ToString for User { fn to_string(self) -> String { ret f"User({self.name})"; } }
        impl Area for Shape {
            fn area(self) -> Int { ret match self { Shape::Circle(radius) => radius * radius * 3, Shape::Square(side) => side * side, }; }
        }
        fn total(values: List) -> Int { let sum = 0; for value in values { sum = sum + value.area(); } ret sum; }
        fn measure(value: Area) -> Int { ret value.double(); }
    "#;
    let programs = [
        "fn main() { let m = Money::new(150); ret [m, m.euros(), m.add_cents(5).cents, (m + Money::new(50)).cents, (m - Money { cents: 50 }).cents, (m * 3).cents, (m / 2).cents, 1 + 2]; }",
        "fn main() { let a = Version { major: 1, minor: 2 }; let b = Version { major: 2, minor: 0 }; ret [a < b, a <= b, a > b, a >= b, a == b, a != b, a == Version { major: 1, minor: 2 }, a.compare(b), b.compare(a), a == 1, a < 1]; }",
        r#"fn main() { let user = User { name: "Ada" }; ret [f"hi {user}!", user.to_string(), f"{[user]}", user.debug(), "x" + 1]; }"#,
        "fn main() { ret [Shape::Circle(2).area(), Shape::Square(3).double(), total([Shape::Circle(1), Shape::Square(2)]), measure(Shape::Square(4)), measure(5)]; }",
        "fn main() { ret [Money::new(1).euros(2), Money::new(1).missing(), (1).euros()]; }",
        "fn main() { let m = Money::new(1); ret [m.cents, m.clone() == m, m.keys()]; }",
        "fn main() { ret [Range::until(1, 4), Range::step(0, 10, 3), Range::step(1, 2, 0), Duration::seconds(3)]; }",
        r#"fn main() { let parsed = DateTime::parse_rfc3339("2024-02-29T22:05:07.12Z")?; ret [parsed, DateTime::parse_rfc3339("nope"), parsed.to_rfc3339()]; }"#,
    ];
    for program in programs {
        compare(&format!("{types} {program}"), &[]);
    }
}

/// Creates, calls and clones closures, shares captured bindings and packs variadic
/// arguments like v1.
#[test]
fn matches_v1_for_closures_and_variadics() {
    let programs = [
        "fn main() { let base = 10; let add = (value) => { ret value + base; }; base = 20; ret [add(1), add(2)]; }",
        "fn main() { let count = 0; let bump = () => { count = count + 1; ret count; }; bump(); bump(); ret [count, bump()]; }",
        "fn main() { let makers = []; for index in 0..3 { makers.push(() => { ret index * 10; }); } let results = []; for make in makers { results.push(make()); } ret results; }",
        "fn adder(base) { ret (value) => { ret (inner) => { ret base + value + inner; }; }; } fn main() { let first = adder(1); let second = first(2); let third = adder(10); let fourth = third(20); ret [second(3), fourth(30)]; }",
        "fn main() { let f = (a, b) => { ret a + b; }; ret [f(1), f(1, 2, 3), f(1, 2)]; }",
        "fn main() { let value = 5; ret value(1); }",
        "fn main() { let sum = (values...) => { let total = 0; for value in values { total = total + value; } ret total; }; ret [sum(), sum(1, 2, 3)]; }",
        "fn total(first: Int, rest: Int...) -> Int { let sum = first; for value in rest { sum = sum + value; } ret sum; } fn main() { ret [total(1), total(1, 2, 3)]; }",
        "fn select(values: Int...) -> Int | Error { ret values[1]; } fn main() { ret [select(1, 2), select(1, \"x\")]; }",
        "type Total {} impl Total { fn sum(values: Int...) -> Int { let total = 0; for value in values { total = total + value; } ret total; } fn add(self, values: Int...) -> Int { ret Total::sum(values[0], 1); } } fn main() { ret [Total::sum(1, 2, 3), Total {}.add(5), Total {}.add()]; }",
        "fn main() { let items = [1, 2, 3, 4]; ret [items.map((x) => { ret x * 2; }), items.filter((x) => { ret x > 2; }), items.find((x) => { ret x == 3; }), items.any((x) => { ret x > 3; }), items.all((x) => { ret x > 0; }), items.reduce(0, (sum, x) => { ret sum + x; })]; }",
        "fn main() { let seen = []; [1, 2].each((x) => { seen.push(x); }); ret [seen, [1].map((x) => { ret Error(\"K\", \"m\", x); }), [1].map(5)]; }",
        "fn main() { let count = 0; let f = () => { count = count + 1; ret count; }; let g = f.clone(); g(); ret [f(), g(), f == f, f == g, f\"{f}\"]; }",
        "fn main() { ret [() => { ret 1; }]; }",
        "fn main() { let cycle = []; cycle.push(cycle); ret cycle; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
    compare(
        "fn main(first, rest...) { ret [first, rest]; }",
        &[ExsValue::Int(1), ExsValue::Int(2), ExsValue::Int(3)],
    );
    compare("fn main(first, rest...) { ret [first, rest]; }", &[]);
}

/// Iterates Strings, Bytes, Range objects and Iterator implementations like v1.
#[test]
fn matches_v1_for_iterators() {
    let counter = "type Countdown { left: Int } impl Iterator for Countdown { fn next(self) -> IteratorStep | Error { if self.left == 0 { ret IteratorStep::Done; } self.left = self.left - 1; ret IteratorStep::Item(self.left); } } type Broken {} impl Iterator for Broken { fn next(self) -> IteratorStep | Error { ret 5; } } type Failing {} impl Iterator for Failing { fn next(self) -> IteratorStep | Error { ret Error(\"K\", \"m\", None); } }";
    let programs = [
        r#"fn main() { let parts = []; for scalar in "aé𝄞" { parts.push(scalar); } for octet in b"hi" { parts.push(octet); } ret parts; }"#,
        "fn main() { let values = []; for value in Range::step(0, 10, 3) { values.push(value); } for value in Range::step_through(10, 0, -5) { values.push(value); } ret values; }",
        "fn main() { let range = 0..5; let seen = []; for value in range { seen.push(value); if value == 2 { break; } } for value in range { seen.push(value); } ret [seen, range.current, range.done]; }",
        "fn main() { let range = 0..=3; let seen = []; for value in range { seen.push([value, range.current]); } ret seen; }",
        "fn main() { let range = 0..4; for value in range { range.end = 2; } ret [range.current, range.done]; }",
        "fn main() { let values = []; for value in Countdown { left: 3 } { values.push(value); } ret values; }",
        "fn main() { let total = 0; for value in Countdown { left: 5 } { if value == 3 { continue; } if value == 1 { break; } total = total + value; } ret total; }",
        "fn main() { for value in Broken {} { } ret 1; }",
        "fn main() { for value in Failing {} { } ret 1; }",
        "fn main() { for value in { a: 1 } { } ret 1; }",
        "fn main() { for value in 5 { } ret 1; }",
        "fn main() { let values = []; for value in -9223372036854775807..=-9223372036854775808 { values.push(value); } for value in 2147483646..2147483649 { values.push(value); } ret values; }",
        "fn main() { for value in 0..\"x\" { } ret 1; }",
        "fn main() { ret [(0..4).collect(), (0..4).count(), Range::step(0, 9, 4).to_list(), [3, 4].count(), Countdown { left: 2 }.collect(), (1..4).map((x) => { ret x * x; })]; }",
        "fn main() { let values = []; for value: Int in Countdown { left: 2 } { values.push(value); } ret values; }",
        "fn main() { let make = []; for value in Countdown { left: 3 } { make.push(() => { ret value; }); } let out = []; for f in make { out.push(f()); } ret out; }",
    ];
    for program in programs {
        compare(&format!("{counter} {program}"), &[]);
    }
}

/// Gives v1's synchronous and resumable lowering one semantics, which `compare` checks for
/// every program: static-call and comparison Errors are values, and checks report the spans
/// of synchronous code.
#[test]
fn matches_v1_across_lowering_modes() {
    let programs = [
        r#"fn main() { ret [Int::parse("x"), Float::parse("y"), Object::from_entries(1), Math::sin("a"), 1]; }"#,
        r#"fn main() { ret [1 < "a", [1] < [2], 1]; }"#,
        "type V { x: Int } impl Compare for V { fn compare(self, other: Any) -> Ordering { ret Ordering::Unordered; } } fn main() { ret [V { x: 1 } < V { x: 2 }, V { x: 1 } == V { x: 1 }, 1]; }",
        "fn main() -> Int { if None { ret 2; } ret 3; }",
        "fn main() { ret [true && 2]; }",
        "fn take(a: Int, b: String) { ret a; } fn main() { ret [take(1, 2), take(\"x\", \"y\")]; }",
        "enum E { A(value: Int), } fn main() { ret E::A(\"x\"); }",
        "fn main() { let n: Int = \"x\"; ret n; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Runs resumable functions like v1: host calls that finish at once or suspend the task,
/// child frames of resumable functions, methods and iterators, and their Errors and traces.
#[test]
fn matches_v1_for_host_calls() {
    let counter = "type Countdown { left: Int } impl Iterator for Countdown { fn next(self) -> IteratorStep | Error { if self.left == 0 { ret IteratorStep::Done; } self.left = self.left - 1; ret IteratorStep::Item(Host::call(\"echo\", self.left)); } }";
    let programs = [
        r#"fn main() { ret Host::call("add", 2, 3) + 1; }"#,
        r#"fn main() { let a = Host::call("echo", 5); let b = Host::call("echo", a * 2); ret [a, b, Host::call("add", a, b)]; }"#,
        r#"fn twice(value) { ret Host::call("echo", value) * 2; } fn main() { let total = 0; for index in 0..4 { total = total + twice(index); } ret total; }"#,
        r#"fn sum(values...) { let total = 0; for value in values { total = total + Host::call("echo", value); } ret total; } fn main() { ret [sum(), sum(1, 2, 3)]; }"#,
        r#"type P { x: Int } impl P { fn get(self) { ret Host::call("echo", self.x); } } impl Add for P { fn add(self, other: Any) -> Any { ret Host::call("echo", self.x) + other.x; } } fn main() { let p = P { x: 4 }; ret [p.get() + p.get(), p + p, p.x]; }"#,
        r#"type V { x: Int } impl Compare for V { fn compare(self, other: Any) -> Ordering { if Host::call("echo", self.x) < other.x { ret Ordering::Less; } ret Ordering::Greater; } } fn main() { ret [V { x: 1 } < V { x: 2 }, V { x: 1 } >= V { x: 1 }, 1 < 2]; }"#,
        r#"fn inner() -> Int { ret Host::call("echo", "x"); } fn main() { ret inner(); }"#,
        r#"fn inner() -> Int | Error { ret Host::call("missing"); } fn main() { ret [inner(), 1]; }"#,
        r#"fn main() { ret [Host::call(1), Host::call("echo", 1, 2)]; }"#,
        r#"fn main() -> Int | Error { let value = Host::call("echo", None)?; ret value; }"#,
        r#"enum C { A, B(Int), } fn main() { let c = C::B(Host::call("echo", 3)); ret match c { C::A => "a", C::B(v) => f"b{v} {Host::call("echo", [1, { k: 2 }])}", }; }"#,
        r#"fn main() { let values = []; for value in Countdown { left: 3 } { values.push(value); } ret values; }"#,
        r#"fn main() { let seen = []; let index = 0; while index < 3 { if Host::call("echo", index) == 1 { index = index + 1; continue; } seen.push(index); index = index + 1; } ret seen; }"#,
        r#"fn main(limit: Int) { Host::call("echo", 1); ret limit; }"#,
        r#"fn main() { let f = () => { ret 1; }; ret [Host::call("echo", f), Host::call("echo", [1, { g: f }]), 2]; }"#,
        r#"fn main() { let list = [1]; list.push(list); ret [Host::call("echo", list), Host::call("add", 1, 2)]; }"#,
        r#"fn main() { ret Host::call("echo", { a: [1.5, b"x", None, Error("K", "m", 1)], e: Ordering::Greater }); }"#,
    ];
    for program in programs {
        compare(&format!("{counter} {program}"), &[]);
    }
    compare(
        r#"fn main(limit: Int, rest...) { ret [Host::call("echo", limit), rest]; }"#,
        &[ExsValue::Int(7), ExsValue::Int(8)],
    );
    compare(
        r#"fn main(limit: Int) { ret Host::call("echo", limit); }"#,
        &[ExsValue::Int(1), ExsValue::Int(2)],
    );
}

/// Runs closures that suspend like v1: as child frames with their captured Cells, through
/// binding calls, the prelude's eager helpers and nested closures, with v1's ArityErrors.
#[test]
fn matches_v1_for_suspending_closures() {
    let programs = [
        r#"fn main() { let f = (x) => { ret Host::call("echo", x) + 1; }; ret [f(1), f(2)]; }"#,
        r#"fn main() { let count = 0; let bump = () => { count = count + Host::call("echo", 2); ret count; }; bump(); bump(); ret [count, bump()]; }"#,
        r#"fn main() { let f = (first, rest...) => { ret [Host::call("echo", first), rest]; }; ret [f(1), f(1, 2, 3)]; }"#,
        r#"fn main() { let f = (x) => { ret Host::call("echo", x); }; ret [f(), f(1, 2), f(3)]; }"#,
        r#"fn main() { let base = 10; ret [1, 2, 3].map((x) => { ret Host::call("echo", x) + base; }); }"#,
        r#"fn main() { let fs = [(x) => { ret x * 2; }, (x) => { ret Host::call("echo", x) * 3; }]; let out = []; for f in fs { out.push(f(4)); } ret out; }"#,
        r#"fn make(n) { ret () => { ret Host::call("echo", n); }; } fn main() { let a = make(1); let b = make(2); let five = 5; ret [b(), a(), five()]; }"#,
        r#"fn apply(f, x) { ret f(x); } fn main() { let outer = (x) => { let inner = (y) => { ret Host::call("echo", y) + x; }; ret apply(inner, x * 10); }; ret apply(outer, 2); }"#,
        r#"fn main() -> Int { let f = (x) => { ret Host::call("echo", "s"); }; let value: Int = f(1); ret value; }"#,
        r#"fn double(values) { ret values.map((x) => { ret x * 2; }); } fn main() { let f = () => { ret Host::call("echo", 1); }; ret [double([1, 2]), f(), [3].filter((x) => { ret x > 1; })]; }"#,
        r#"fn check(value: Int) -> Int { ret value; } fn main() { let f = (x) => { ret check(Host::call("echo", x)); }; ret [f(1), f("s")]; }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Reads results of every kind from module memory like v1 encodes them, and reports results
/// that cannot cross the boundary as v1's fatal SerializationError.
#[test]
fn matches_v1_for_boundary_results() {
    let programs = [
        r#"fn main() { ret [None, true, -7, 3000000000, 1.25, "äö𝄞", b"hi", { a: [1, { b: 2 }] }]; }"#,
        "type P { x: Int, y: Any } fn main() { let p = P { y: 2, x: 1 }; p.extra = 3; ret p; }",
        "enum E { A, B(value: Int), } fn main() { ret [E::A, E::B(2), Ordering::Less]; }",
        r#"fn main() { ret Error("K", "m", [1, Error("C", "d", None)]); }"#,
        "fn main() { ret () => { ret 1; }; }",
        "fn main() { ret [1, { f: (x) => { ret x; } }]; }",
        "fn main() { let list = [1]; list.push(list); ret list; }",
        "fn main() { let o = { a: 1 }; o.self = o; ret o; }",
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Adopts inputs and host responses of every kind like v1 decodes them: echoed back as the
/// result, rendered, and passed through a host call that suspends.
#[test]
fn matches_v1_for_boundary_inputs() {
    let error = ExsValue::Error(ExsError {
        severity: ErrorSeverity::Recoverable,
        kind: "Kind".to_owned(),
        message: "message ü".to_owned(),
        data: Box::new(ExsValue::List(vec![ExsValue::Int(1)])),
        origin: Some(SourcePositionId(3)),
        trace: vec![exs_abi::ExsStackFrame {
            function_id: 2,
            call_site: SourcePositionId(7),
        }],
        cause: Some(Box::new(ExsValue::String("cause".to_owned()))),
    });
    let values = [
        ExsValue::None,
        ExsValue::Bool(true),
        ExsValue::Int(-5),
        ExsValue::Int(i64::MIN),
        ExsValue::Int(3_000_000_000),
        ExsValue::Float(-0.5),
        ExsValue::String("aé𝄞".to_owned()),
        ExsValue::String(String::new()),
        ExsValue::Bytes(vec![0, 255, 7]),
        ExsValue::List(vec![ExsValue::Int(1), ExsValue::List(Vec::new())]),
        ExsValue::Object(vec![
            ("b".to_owned(), ExsValue::Int(1)),
            ("a".to_owned(), ExsValue::Object(Vec::new())),
        ]),
        ExsValue::Enum {
            type_id: "std::Ordering".to_owned(),
            variant: "Greater".to_owned(),
            fields: Vec::new(),
        },
        error,
    ];
    for value in values {
        let inputs = [value];
        compare("fn main(value) { ret value; }", &inputs);
        compare(
            "fn main(value) { ret [f\"{value}\", value == value]; }",
            &inputs,
        );
        compare(
            r#"fn main(value) { ret Host::call("echo", value); }"#,
            &inputs,
        );
    }
    let rest = [
        ExsValue::Int(1),
        ExsValue::String("x".to_owned()),
        ExsValue::None,
    ];
    compare("fn main(first, rest...) { ret [first, rest]; }", &rest);
    compare("fn main(first) { ret first; }", &rest);
    compare("fn main(a, b, c, d) { ret [a, b, c, d]; }", &rest);
}

/// Prints the fuel and median execution time of both backends for programs dominated by the
/// host boundary: large inputs, results and host-call values.
#[test]
fn reports_boundary_fuel_and_time() {
    let record = |index: i64| {
        ExsValue::Object(vec![
            ("id".to_owned(), ExsValue::Int(index)),
            (
                "name".to_owned(),
                ExsValue::String(format!("Record ü {index}")),
            ),
            (
                "tags".to_owned(),
                ExsValue::List(vec![
                    ExsValue::String("alpha".to_owned()),
                    ExsValue::String("beta".to_owned()),
                ]),
            ),
            ("score".to_owned(), ExsValue::Float(0.5 * index as f64)),
            (
                "nested".to_owned(),
                ExsValue::Object(vec![("flag".to_owned(), ExsValue::Bool(index % 2 == 0))]),
            ),
        ])
    };
    let records = [ExsValue::List((0..200).map(record).collect())];
    let scenarios = [
        ("echo_records_200", "fn main(records) { ret records; }"),
        (
            "count_records_200",
            "fn main(records) { let total = 0; for record in records { total = total + record.id; } ret total; }",
        ),
        (
            "host_echo_records_200",
            r#"fn main(records) { ret Host::call("echo", records); }"#,
        ),
    ];
    for (name, source) in scenarios {
        compare(source, &records);
        let (v1_fuel, v1_time) = measure(source, Backend::V1, &records, 21);
        let (v2_fuel, v2_time) = measure(source, Backend::V2, &records, 21);
        println!(
            "boundary {name}: fuel v1 {v1_fuel} v2 {v2_fuel}, time v1 {v1_time:?} v2 {v2_time:?}"
        );
    }
}

/// Tests, dispatches, reads and encodes values of nominal types whose ids are too large to
/// inline like v1, through the type id in their heap header.
#[test]
fn matches_v1_beyond_the_inline_type_ids() {
    let mut declarations = String::new();
    for index in 0..65_540 {
        declarations.push_str(&format!("type T{index} {{ v: Int }}\n"));
    }
    declarations.push_str("enum Big { A, B(value: Int), }\n");
    declarations.push_str("impl T65539 { fn get(self) -> Int { ret self.v; } }\n");
    let program = format!(
        "{declarations} fn main(input) {{ let t = T65539 {{ v: 7 }}; t.v = t.v + 1; let b = Big::B(3); \
         ret [t, t.get(), t.v, t is T65539, t is T65538, T65533 {{ v: 1 }} is T65533, b, b is Big, \
         match b {{ Big::A => 0, Big::B(value) => value, }}, input is Big, f\"{{b}}\", {{ x: 1 }}.x]; }}"
    );
    compare(
        &program,
        &[ExsValue::Enum {
            type_id: "test.exs::Big".to_owned(),
            variant: "A".to_owned(),
            fields: Vec::new(),
        }],
    );
}

/// Renders Floats like v1, which formats them with Rust's `{}`.
#[test]
fn matches_v1_for_float_text() {
    let source = r#"fn main(values) { let out = []; for value in values { out.push(f"{value}"); out.push("" + value); out.push(value.to_string()); } ret out; }"#;
    let values = [
        0.0,
        -0.0,
        0.1,
        0.2,
        0.3,
        1.0,
        -1.5,
        1e15,
        1e16,
        1e21,
        1e22,
        1e-7,
        5e-324,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::EPSILON,
        123_456.789,
        1.0 / 3.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        2.0_f64.powi(-1022),
        9_007_199_254_740_993.0,
    ];
    let list = ExsValue::List(values.iter().copied().map(ExsValue::Float).collect());
    compare(source, &[list]);
}

/// Runs `par` like v1: static and dynamic tasks, their results in source order, host calls
/// that suspend single tasks, nested `par`, v1's Errors for invalid dynamic operands, and a
/// fatal Error that ends every task.
#[test]
fn matches_v1_for_parallel_tasks() {
    let programs = [
        "fn main() { ret par { 6 * 7; 20 + 1; }; }",
        r#"fn main() { ret par { Host::call("echo", 1); Host::call("add", 2, 3); Host::call("echo", "x"); }; }"#,
        r#"fn main() { let base = 10; let tasks = [() => { ret Host::call("echo", base); }, () => { ret base + 1; }]; ret [par(tasks), par([]), base]; }"#,
        r#"fn main() { let log = []; let result = par { log.push(Host::call("echo", 1)); log.push(2); }; log.push(3); ret [result, log]; }"#,
        r#"fn inner(value) { ret par { Host::call("echo", value); value * 2; }; } fn main() { ret par { inner(1); inner(Host::call("echo", 5)); }; }"#,
        r#"fn main() { ret par { Error("K", "m", None); 42; }; }"#,
        r#"fn helper() { ret Error("K", "m", None); } fn main() { let parts = par { helper(); 1; }; ret [parts, Error("A", "b", None), helper()]; }"#,
        r#"fn helper() { Host::call("echo", 1); ret Error("K", "m", None); } fn main() { let parts = par { helper(); 1; }; ret [parts, Error("A", "b", None), helper()]; }"#,
        "fn wrong() -> Int { ret \"invalid\"; } fn main() { ret par { wrong(); 1; }; }",
        r#"fn wrong() -> Int { ret Host::call("echo", "invalid"); } fn main() { ret par { Host::call("echo", 1); wrong(); }; }"#,
        "fn main() { ret par(1); }",
        "fn main() { ret par([1]); }",
        "fn main() { ret par([(value) => { ret value; }]); }",
        "fn main() { ret par([(rest...) => { ret rest; }]); }",
        r#"fn main() -> Int { let total = 0; let count = 0; while count < 300 { let parts = par { count; Host::call("echo", 1); }; total = total + parts[0] + parts[1]; count = count + 1; } ret total; }"#,
        r#"fn main() { let f = () => { ret par { Host::call("echo", 1); 2; }; }; ret [f(), par { f(); }]; }"#,
    ];
    for program in programs {
        compare(program, &[]);
    }
}

/// Compiles `source` with `backend` and returns its fuel and the median execution time of
/// `runs` executions, which exclude compiling and instantiating the module.
fn measure(source: &str, backend: Backend, inputs: &[ExsValue], runs: usize) -> (u64, Duration) {
    let compiled = match compile_with_backend(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        backend,
    ) {
        Ok(module) => module,
        Err(error) => panic!("{backend:?} compilation failed: {error}"),
    };
    let mut runner = ServerRunner::new(
        64 << 20,
        1_000_000_000,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    );
    register_hosts(&mut runner);
    let mut fuel = 0;
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        match block_on(runner.execute_with_report(
            &compiled.wasm,
            "main",
            inputs,
            &ExecutionCancellation::new(),
        )) {
            Ok(report) => {
                fuel = report.fuel_consumed;
                times.push(report.execution_time);
            }
            Err(error) => panic!("{backend:?} execution failed: {error}"),
        }
    }
    times.sort();
    (fuel, times.get(runs / 2).copied().unwrap_or_default())
}

/// Prints the fuel and median execution time of both backends for programs that suspend.
#[test]
fn reports_resumable_fuel_and_time() {
    let scenarios = [
        (
            "host_then_loop_1000",
            r#"fn main(limit: Int) -> Int { Host::call("echo", 1); let total = 0; let index = 0; while index < limit { total = total + index * 3 + 1; index = index + 1; } ret total; }"#,
        ),
        (
            "pending_calls_100",
            r#"fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit / 10 { total = total + Host::call("echo", index); index = index + 1; } ret total; }"#,
        ),
        (
            "ready_calls_100",
            r#"fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit / 10 { total = Host::call("add", total, index); index = index + 1; } ret total; }"#,
        ),
        (
            "resumable_calls_100",
            r#"fn step(value) { if value == 0 { Host::call("echo", 0); } ret value + 1; } fn main(limit: Int) -> Int { let total = 0; let index = 0; while index < limit / 10 { total = step(total); index = index + 1; } ret total; }"#,
        ),
        (
            "closure_calls_100",
            r#"fn main(limit: Int) -> Int { let total = 0; let bump = (value) => { if value == 0 { Host::call("echo", 0); } ret value + 1; }; let index = 0; while index < limit / 10 { total = bump(total); index = index + 1; } ret total; }"#,
        ),
        (
            "par_fan_out_100",
            r#"fn main(limit: Int) -> Int { let tasks = []; let index = 0; while index < limit / 10 { let value = index; tasks.push(() => { ret Host::call("echo", value); }); index = index + 1; } let total = 0; for value in par(tasks) { total = total + value; } ret total; }"#,
        ),
    ];
    let inputs = [ExsValue::Int(1000)];
    for (name, source) in scenarios {
        compare(source, &inputs);
        let (v1_fuel, v1_time) = measure(source, Backend::V1, &inputs, 21);
        let (v2_fuel, v2_time) = measure(source, Backend::V2, &inputs, 21);
        println!(
            "resumable {name}: fuel v1 {v1_fuel} v2 {v2_fuel}, time v1 {v1_time:?} v2 {v2_time:?}"
        );
    }
}

/// Represents host enums, enums of identities or variants the program does not declare, like
/// v1: they cross the boundary unchanged, render with their names, fail every match arm and
/// pass contracts of the declared enum of their identity.
#[test]
fn matches_v1_for_host_enums() {
    let foreign = ExsValue::Enum {
        type_id: "host::Status".to_owned(),
        variant: "Ready".to_owned(),
        fields: vec![ExsValue::Int(1), ExsValue::String("x".to_owned())],
    };
    let unknown_variant = ExsValue::Enum {
        type_id: "test.exs::Color".to_owned(),
        variant: "Green".to_owned(),
        fields: Vec::new(),
    };
    let declarations = "enum Color { Red, Blue(value: Int), }";
    let programs = [
        "fn main(value) { ret value; }",
        "fn main(value) { ret [value, [value], { v: value }]; }",
        r#"fn main(value) { ret [f"{value}", "" + value, value == value, value != None]; }"#,
        "fn main(value) { ret [value.debug(), value.to_string(), value.clone(), value.keys()]; }",
        "fn main(value) { ret match value { Color::Red => 1, Color::Blue(v) => v, }; }",
        "fn main(value) -> Int | Error { ret match value { Color::Red => 1, Color::Blue(v) => v, }; }",
        "fn main(value) { ret [value is Color, value is Ordering, value is Object]; }",
        "fn main(value: Color) { ret 1; }",
        "fn main(value) { let c: Color = value; ret c; }",
        r#"fn main(value) { ret Host::call("echo", value); }"#,
    ];
    for program in programs {
        for value in [&foreign, &unknown_variant] {
            compare(
                &format!("{declarations} {program}"),
                std::slice::from_ref(value),
            );
        }
    }
}
