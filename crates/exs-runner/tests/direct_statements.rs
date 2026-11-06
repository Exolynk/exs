//! Statements of resumable functions that cannot suspend run as synchronous code; these
//! tests check that they behave exactly like the same statements in synchronous functions.

mod support;

use exs_abi::ExsValue;
use exs_runner::ServerRunner;
use support::{block_on, compile_source};

/// Runs `main` of `source` with `inputs`, once as written and once with `{HOST}` replaced by
/// a host call, which makes every function containing it resumable, and returns both
/// results.
fn run_both(source: &str, inputs: &[ExsValue]) -> (ExsValue, ExsValue) {
    let run = |source: &str| {
        let compiled = compile_source(source);
        let mut runner = ServerRunner::default();
        assert!(
            runner
                .registry_mut()
                .fn_sync_raw("echo", |arguments: Vec<ExsValue>| {
                    arguments.into_iter().next().unwrap_or(ExsValue::None)
                })
                .is_ok()
        );
        match block_on(runner.execute(&compiled.wasm, "main", inputs)) {
            Ok(result) => result,
            Err(error) => panic!("execution failed: {error}"),
        }
    };
    (
        run(&source.replace("{HOST}", "")),
        run(&source.replace("{HOST}", "Host::call(\"echo\", 0);")),
    )
}

/// Asserts that `main` of `source` returns `expected` with and without the host call.
fn assert_both(source: &str, inputs: &[ExsValue], expected: &ExsValue) {
    let (synchronous, resumable) = run_both(source, inputs);
    assert_eq!(&synchronous, expected);
    assert_eq!(&resumable, expected);
}

/// Runs `while` loops with `break`, `continue` and nested loops.
#[test]
fn runs_while_loops_with_branches() {
    assert_both(
        r#"
        fn main(limit: Int) -> List {
            {HOST}
            let total = 0;
            let index = 0;
            let found = -1;
            while index < limit {
                index = index + 1;
                if index % 3 == 0 { continue; }
                total = total + index * 2;
                if total > 500 && found < 0 { found = index; }
                if index == 91 { break; }
            }
            let nested = 0;
            let outer = 0;
            while outer < 5 {
                let inner = 0;
                while inner < outer {
                    nested = nested + 1;
                    inner = inner + 1;
                }
                outer = outer + 1;
            }
            ret [total, found, index, nested];
        }
        "#,
        &[ExsValue::Int(1000)],
        &ExsValue::List(vec![
            ExsValue::Int(5582),
            ExsValue::Int(28),
            ExsValue::Int(91),
            ExsValue::Int(10),
        ]),
    );
}

/// Runs `for` loops over Ranges and Lists and `if` statements with `else` branches.
#[test]
fn runs_for_loops_and_conditionals() {
    assert_both(
        r#"
        fn main() -> List {
            {HOST}
            let squares = [];
            for value in 0..10 {
                if value % 2 == 0 { squares.push(value * value); }
            }
            let letters = 0;
            for word in ["a", "bb", "ccc"] { letters = letters + word.length(); }
            let label = "none";
            if letters > 10 { label = "many"; } else if letters > 5 { label = "some"; } else { label = "few"; }
            ret [squares, letters, label];
        }
        "#,
        &[],
        &ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::Int(0),
                ExsValue::Int(4),
                ExsValue::Int(16),
                ExsValue::Int(36),
                ExsValue::Int(64),
            ]),
            ExsValue::Int(6),
            ExsValue::String("some".to_owned()),
        ]),
    );
}

/// Keeps statements calling a list helper that runs as a frame, such as `contains` when an
/// `Eq` implementation calls the host, in the graph instead of calling it synchronously.
#[test]
fn keeps_statements_with_frame_list_helpers_in_the_graph() {
    assert_both(
        r#"
        type Tag { name: String }

        impl Eq for Tag {
            fn eq(self, other: Any) -> Bool {
                Host::call("echo", 0);
                ret other is Tag && self.name == other.name;
            }
        }

        fn main() -> List {
            {HOST}
            let names = ["a"];
            if !names.contains("b") { names.push("b"); }
            if !names.contains("a") { names.push("a"); }
            let found = -1;
            for name in ["x", "b"] {
                if names.index_of(name) != None { found = names.index_of(name); }
            }
            ret [names, found];
        }
        "#,
        &[],
        &ExsValue::List(vec![
            ExsValue::List(vec![
                ExsValue::String("a".to_owned()),
                ExsValue::String("b".to_owned()),
            ]),
            ExsValue::Int(1),
        ]),
    );
}

/// Returns from inside a loop and keeps assignments to captured bindings.
#[test]
fn returns_from_loops_and_shares_captured_bindings() {
    assert_both(
        r#"
        fn first_square_over(limit: Int) -> Int {
            {HOST}
            let index = 0;
            while true {
                index = index + 1;
                if index * index > limit { ret index; }
            }
            ret -1;
        }
        fn main() -> List {
            {HOST}
            let count = 0;
            let read = () => { ret count; };
            while count < 7 { count = count + 1; }
            ret [first_square_over(50), read(), count];
        }
        "#,
        &[],
        &ExsValue::List(vec![ExsValue::Int(8), ExsValue::Int(7), ExsValue::Int(7)]),
    );
}

/// Reports Errors raised inside a loop like synchronous code.
#[test]
fn reports_errors_raised_in_loops() {
    let (synchronous, resumable) = run_both(
        r#"
        fn main() -> Int | Error {
            {HOST}
            let index = 0;
            let total = 0;
            while index < 5 {
                if index == 2 { total = total - "text"; }
                index = index + 1;
            }
            ret total;
        }
        "#,
        &[],
    );
    // Source position ids differ between the two modules, so only the Errors' contents are
    // compared.
    let (ExsValue::Error(synchronous), ExsValue::Error(resumable)) = (synchronous, resumable)
    else {
        panic!("expected two Errors");
    };
    assert_eq!(synchronous.kind, "TypeError");
    assert_eq!(
        (synchronous.kind, synchronous.message, synchronous.data),
        (resumable.kind, resumable.message, resumable.data)
    );
}

/// Keeps a loop whose body suspends in the graph, so its host calls still run in order.
#[test]
fn suspends_inside_loops_with_host_calls() {
    assert_both(
        r#"
        fn main() -> Int {
            {HOST}
            let total = 0;
            for value in 0..4 { total = total + Host::call("echo", value); }
            ret total;
        }
        "#,
        &[],
        &ExsValue::Int(6),
    );
}

/// Lets long loops of parallel tasks yield to their siblings at scheduler checkpoints.
#[test]
fn interleaves_parallel_tasks_with_long_loops() {
    let compiled = compile_source(
        r#"
        fn main() -> List {
            let order = [];
            let tasks = [
                () => { let index = 0; while index < 200 { order.push(1); index = index + 1; } ret 1; },
                () => { let index = 0; while index < 200 { order.push(2); index = index + 1; } ret 2; },
            ];
            par(tasks);
            ret order;
        }
        "#,
    );
    let runner = ServerRunner::default();
    let result = match block_on(runner.execute(&compiled.wasm, "main", &[])) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    let ExsValue::List(order) = result else {
        panic!("expected a List");
    };
    assert_eq!(order.len(), 400);
    let first_second = order.iter().position(|value| *value == ExsValue::Int(2));
    assert!(
        first_second.is_some_and(|position| position < 200),
        "the second task only ran after the first finished"
    );
}

/// Types whose protocol implementations, methods and iterator call the host, so every
/// statement using them reaches a unit that runs as a frame.
const SUSPENDING_TYPES: &str = r#"
type Num { value: Int }

impl Eq for Num {
    fn eq(self, other: Any) -> Bool {
        Host::call("echo", 0);
        ret other is Num && self.value == other.value;
    }
}

impl Compare for Num {
    fn compare(self, other: Any) -> Ordering {
        Host::call("echo", 0);
        ret self.value.compare(other.value);
    }
}

impl Add for Num {
    fn add(self, other: Any) -> Any {
        Host::call("echo", 0);
        ret Num { value: self.value + other.value };
    }
}

impl ToString for Num {
    fn to_string(self) -> String {
        Host::call("echo", 0);
        ret f"n{self.value}";
    }
}

impl Debug for Num {
    fn debug(self) -> String {
        Host::call("echo", 0);
        ret f"dbg{self.value}";
    }
}

impl Num {
    fn ping(self) -> Int {
        Host::call("echo", 0);
        ret self.value;
    }

    fn make(value: Int) -> Num {
        Host::call("echo", 0);
        ret Num { value: value };
    }
}

type Count { left: Int }

impl Iterator for Count {
    fn next(self) -> IteratorStep | Error {
        Host::call("echo", 0);
        if self.left == 0 { ret IteratorStep::Done; }
        self.left = self.left - 1;
        ret IteratorStep::Item(self.left);
    }
}

fn remote(value: Int) -> Int {
    Host::call("echo", 0);
    ret value;
}
"#;

/// Runs each `(name, body, expected)` case as the body of `main` after
/// [`SUSPENDING_TYPES`], with and without a host call in `main`.
fn assert_suspending_cases(cases: &[(&str, &str, ExsValue)]) {
    for (name, body, expected) in cases {
        let source = format!("{SUSPENDING_TYPES}\nfn main() -> Any {{ {{HOST}} {body} }}");
        let (synchronous, resumable) = run_both(&source, &[]);
        assert_eq!(&synchronous, expected, "{name} without a host call in main");
        assert_eq!(&resumable, expected, "{name} with a host call in main");
    }
}

/// Runs statements whose operators, renderings, calls and iterators suspend.
#[test]
fn runs_statements_reaching_suspending_units() {
    let string = |text: &str| ExsValue::String(text.to_owned());
    assert_suspending_cases(&[
        (
            "==",
            "let r = 0; if Num{value:1} == Num{value:1} { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "!=",
            "let r = 0; if Num{value:1} != Num{value:2} { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "<",
            "let r = 0; if Num{value:1} < Num{value:2} { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "+",
            "let r = 0; if (Num{value:1} + Num{value:2}).value == 3 { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "List ==",
            "let r = 0; if [Num{value:1}] == [Num{value:1}] { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "while with !=",
            "let r = 0; let n = Num{value:0}; while n != Num{value:3} { n = Num{value: n.value + 1}; r = r + 1; } ret r;",
            ExsValue::Int(3),
        ),
        (
            "for with ==",
            "let r = 0; for n in [Num{value:1}, Num{value:2}] { if n == Num{value:2} { r = 1; } } ret r;",
            ExsValue::Int(1),
        ),
        (
            "interpolation",
            "let r = \"\"; if true { r = f\"{Num{value:4}}\"; } ret r;",
            string("n4"),
        ),
        (
            "List interpolation",
            "let r = \"\"; if true { r = f\"{[Num{value:4}]}\"; } ret r;",
            string("[dbg4]"),
        ),
        (
            "to_string",
            "let r = \"\"; if true { r = Num{value:5}.to_string(); } ret r;",
            string("n5"),
        ),
        (
            "List to_string",
            "let r = \"\"; if true { r = [Num{value:5}].to_string(); } ret r;",
            string("[dbg5]"),
        ),
        (
            "debug",
            "let r = \"\"; if true { r = Num{value:5}.debug(); } ret r;",
            string("dbg5"),
        ),
        (
            "List debug",
            "let r = \"\"; if true { r = [Num{value:5}].debug(); } ret r;",
            string("[dbg5]"),
        ),
        (
            "instance method",
            "let r = 0; if true { r = Num{value:6}.ping(); } ret r;",
            ExsValue::Int(6),
        ),
        (
            "static method",
            "let r = 0; if true { r = Num::make(7).value; } ret r;",
            ExsValue::Int(7),
        ),
        (
            "function",
            "let r = 0; if true { r = remote(8); } ret r;",
            ExsValue::Int(8),
        ),
        (
            "custom iterator",
            "let r = 0; for x in Count{left:3} { r = r + x; } ret r;",
            ExsValue::Int(3),
        ),
        (
            "assert_eq",
            "let r = 0; if true { assert_eq(Num{value:1}, Num{value:1}, \"same\"); r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "literal match",
            "let r = 0; let v = Num{value:1}; if true { r = match v { 1 => 1, _ => 2 }; } ret r;",
            ExsValue::Int(2),
        ),
    ]);
}

/// Runs statements calling list helpers that compare items with suspending protocols.
#[test]
fn runs_statements_with_suspending_list_helpers() {
    assert_suspending_cases(&[
        (
            "contains",
            "let r = 0; if [Num{value:1}].contains(Num{value:1}) { r = 1; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "index_of",
            "let r = -1; if true { r = [Num{value:1}].index_of(Num{value:1}); } ret r;",
            ExsValue::Int(0),
        ),
        (
            "sort",
            "let l = [Num{value:2}, Num{value:1}]; if true { l.sort(); } ret l[0].value;",
            ExsValue::Int(1),
        ),
        (
            "sorted",
            "let r = 0; if true { r = [Num{value:2}, Num{value:1}].sorted()[0].value; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "min",
            "let r = 0; if true { r = [Num{value:2}, Num{value:1}].min().value; } ret r;",
            ExsValue::Int(1),
        ),
        (
            "unique",
            "let r = 0; if true { r = [Num{value:1}, Num{value:1}].unique().length(); } ret r;",
            ExsValue::Int(1),
        ),
    ]);
}

/// Runs loops of `par` tasks, which restart at checkpoints, reaching suspending units.
#[test]
fn runs_parallel_loops_reaching_suspending_units() {
    let ints = |values: &[i64]| ExsValue::List(values.iter().copied().map(ExsValue::Int).collect());
    assert_suspending_cases(&[
        (
            "while with == and contains",
            "let out = []; let tasks = [() => { let n = 0; while n < 50 { if Num{value:n} == Num{value:7} { out.push(n); } n = n + 1; } ret 1; }, () => { let n = 0; while n < 50 { if [n].contains(9) { out.push(n); } n = n + 1; } ret 2; }]; par(tasks); ret out.sorted();",
            ints(&[7, 9]),
        ),
        (
            "for with ==",
            "let out = []; let tasks = [() => { for n in 0..20 { if Num{value:n} == Num{value:3} { out.push(n); } } ret 1; }]; par(tasks); ret out;",
            ints(&[3]),
        ),
        (
            "custom iterator in while",
            "let out = []; let tasks = [() => { let n = 0; while n < 2 { for x in Count{left:2} { out.push(x); } n = n + 1; } ret 1; }]; par(tasks); ret out;",
            ints(&[1, 0, 1, 0]),
        ),
    ]);
}
