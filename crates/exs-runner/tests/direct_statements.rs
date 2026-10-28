//! Statements of resumable functions that cannot suspend run as synchronous code; these
//! tests check that they behave exactly like the same statements in synchronous functions.

mod support;

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, ServerRunner};
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
        match block_on(runner.execute(
            &compiled.wasm,
            "main",
            inputs,
            &ExecutionCancellation::new(),
        )) {
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
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
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
