//! Built-in equality, the `Eq` trait and the ordered built-in values.

mod support;

use exs_abi::ExsValue;
use exs_runner::{ExecutionCancellation, ServerRunner};
use support::{block_on, compile_source, execute_source_with_inputs};

/// Returns the Bools of a List result.
fn bools(value: ExsValue) -> Vec<bool> {
    match value {
        ExsValue::List(items) => items
            .into_iter()
            .map(|item| match item {
                ExsValue::Bool(flag) => flag,
                other => panic!("expected a Bool, got {other:?}"),
            })
            .collect(),
        other => panic!("expected a List, got {other:?}"),
    }
}

/// Compares enums, nominal values, Lists, Objects and Errors part by part.
#[test]
fn compares_composites_part_by_part() {
    let result = execute_source_with_inputs(
        r#"
            type Point { x: Int, tags: List }
            enum Color { Red, Rgb(r: Int) }

            fn main() -> List {
                ret [
                    Ordering::Less == Ordering::Less,
                    Color::Red == Color::Red,
                    Color::Rgb(1) == Color::Rgb(1),
                    Color::Rgb(1) != Color::Rgb(2),
                    Color::Red != Color::Rgb(1),
                    Point { x: 1, tags: [1] } == Point { x: 1, tags: [1] },
                    Point { x: 1, tags: [1] } != Point { x: 1, tags: [2] },
                    [1, [2, 3]] == [1, [2, 3]],
                    [1, 2] != [1, 2, 3],
                    { a: 1, b: 2 } == { b: 2, a: 1 },
                    { a: 1 } != { a: 2 },
                    { a: 1 } != { b: 1 },
                    Error("Kind", "message", [1]) == Error("Kind", "message", [1]),
                    Duration::seconds(1) == Duration::seconds(1),
                    (1..3) == (1..3),
                    1 == 1.0,
                    [1] != { a: 1 },
                ];
            }
        "#,
        &[],
    );
    assert!(bools(result).into_iter().all(|flag| flag));
}

/// Treats cyclic composites as equal where both sides repeat the same pair.
#[test]
fn compares_cyclic_composites() {
    let result = execute_source_with_inputs(
        r#"
            fn main() -> List {
                let left = [1];
                left.push(left);
                let right = [1];
                right.push(right);
                let other = [2];
                other.push(other);
                ret [left == right, left == left, left != other];
            }
        "#,
        &[],
    );
    assert_eq!(bools(result), [true, true, true]);
}

/// Uses `Eq` implementations at the top level, inside composites and in List searches.
#[test]
fn uses_eq_implementations_everywhere() {
    let result = execute_source_with_inputs(
        r#"
            type Money { cents: Int, label: String }

            impl Eq for Money {
                fn eq(self, other: Any) -> Bool {
                    ret other is Money && self.cents == other.cents;
                }
            }

            fn main() -> List {
                let a = Money { cents: 100, label: "a" };
                let b = Money { cents: 100, label: "b" };
                ret [
                    a == b,
                    [a] == [b],
                    { value: a } == { value: b },
                    [a].contains(b),
                    [a, b].index_of(b) == 0,
                    [a, b].last_index_of(a) == 1,
                    a != Money { cents: 1, label: "a" },
                ];
            }
        "#,
        &[],
    );
    assert!(bools(result).into_iter().all(|flag| flag));
}

/// Prefers `Eq` over `Compare` for equality and keeps `Compare` for ordering.
#[test]
fn prefers_eq_over_compare() {
    let result = execute_source_with_inputs(
        r#"
            type Version { major: Int, build: Int }

            impl Compare for Version {
                fn compare(self, other: Any) -> Ordering {
                    ret self.major.compare(other.major);
                }
            }

            impl Eq for Version {
                fn eq(self, other: Any) -> Bool {
                    ret self.major == other.major && self.build == other.build;
                }
            }

            fn main() -> List {
                let one = Version { major: 1, build: 1 };
                let two = Version { major: 1, build: 2 };
                ret [one != two, one <= two, one >= two];
            }
        "#,
        &[],
    );
    assert_eq!(bools(result), [true, true, true]);
}

/// Orders only numbers and Strings.
#[test]
fn orders_only_numbers_and_strings() {
    let result = execute_source_with_inputs(
        r#"
            fn main() -> List {
                ret [
                    (None < None) is Error,
                    ([1] <= [1]) is Error,
                    [1].compare([1]) is Error,
                    1 < 2.5,
                    "a" < "b",
                    (2).compare(1) == Ordering::Greater,
                ];
            }
        "#,
        &[],
    );
    assert!(bools(result).into_iter().all(|flag| flag));
}

/// Resumes an `Eq` implementation that suspends while a composite is compared.
#[test]
fn resumes_suspending_eq_implementations() {
    let compiled = compile_source(
        r#"
            type Tag { name: String }

            impl Eq for Tag {
                fn eq(self, other: Any) -> Bool {
                    ret Host::call("echo", self.name) == other.name;
                }
            }

            fn main() -> List {
                ret [[Tag { name: "a" }] == [Tag { name: "a" }], [Tag { name: "a" }].contains(Tag { name: "b" })];
            }
        "#,
    );
    let mut runner = ServerRunner::default();
    assert!(
        runner
            .registry_mut()
            .fn_async_raw("echo", |arguments: Vec<ExsValue>| async move {
                arguments.into_iter().next().unwrap_or(ExsValue::None)
            })
            .is_ok()
    );
    let result = match block_on(runner.execute(
        &compiled.wasm,
        "main",
        &[],
        &ExecutionCancellation::new(),
    )) {
        Ok(result) => result,
        Err(error) => panic!("execution failed: {error}"),
    };
    assert_eq!(bools(result), [true, false]);
}
