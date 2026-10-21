//! Runs programs the runtime v2 prototype supports with both backends and compares them.

use std::time::Duration;

use exs_compiler::{Backend, CompileOptions, SourceInput, compile_with_backend};
use exs_runner::{ExecutionCancellation, ExsValue, ProtectionLevel, ServerRunner};

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
    let runner = ServerRunner::new(
        64 << 20,
        1_000_000_000,
        Duration::from_secs(60),
        ProtectionLevel::Standard,
    );
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

/// Runs `source` with both backends, asserts equal results and returns both fuel values.
fn compare(source: &str, inputs: &[ExsValue]) -> (u64, u64) {
    let (v1, v1_fuel) = run(source, Backend::V1, inputs);
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

/// Traps instead of silently bypassing operator and `ToString` implementations, which v2 does
/// not dispatch yet.
#[test]
fn traps_on_undispatched_trait_implementations() {
    let programs = [
        "type Number { value: Int } impl Add for Number { fn add(self, other: Any) -> Any { ret Number { value: self.value + other.value }; } } fn main() { let one = Number { value: 1 }; ret one + one; }",
        "type Number { value: Int } impl ToString for Number { fn to_string(self) -> String { ret \"n\"; } } fn main() { ret f\"{Number { value: 1 }}\"; }",
    ];
    for program in programs {
        let compiled = compile_with_backend(
            SourceInput {
                source_id: "test.exs",
                text: program,
            },
            CompileOptions::default(),
            Backend::V2,
        )
        .expect("v2 compiles the program");
        let runner = ServerRunner::default();
        let outcome =
            block_on(runner.execute(&compiled.wasm, "main", &[], &ExecutionCancellation::new()));
        assert!(outcome.is_err(), "v2 did not trap on:\n{program}");
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
