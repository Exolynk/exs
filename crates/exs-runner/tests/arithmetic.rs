//! The `%` operator, the `Rem` trait, Bools outside arithmetic, and the operators of
//! Duration and DateTime.

mod support;

use exs_abi::ExsValue;
use support::execute_source_with_inputs;

/// Computes Euclidean remainders of Ints and Floats.
#[test]
fn computes_euclidean_remainders() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    ret [7 % 3, -1 % 24, 7 % -3, 7.5 % 2, -7.5 % 2, 10.rem(4), 2 * 3 % 4];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Int(1),
            ExsValue::Int(23),
            ExsValue::Int(1),
            ExsValue::Float(1.5),
            ExsValue::Float(0.5),
            ExsValue::Int(2),
            ExsValue::Int(2),
        ])
    );
}

/// Returns DivisionByZeroError for an Int divisor of zero and NaN for a Float one.
#[test]
fn handles_zero_divisors() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    let error = 7 % 0;
                    ret [error.kind(), (1.0 % 0.0).is_nan()];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::String("DivisionByZeroError".to_owned()),
            ExsValue::Bool(true),
        ])
    );
}

/// Dispatches `%` to `Rem` implementations.
#[test]
fn dispatches_rem_implementations() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                type Angle { degrees: Int }

                impl Rem for Angle {
                    fn rem(self, other: Any) -> Any {
                        ret Angle { degrees: self.degrees % other };
                    }
                }

                fn main() -> Int {
                    ret (Angle { degrees: 450 } % 360).degrees;
                }
            "#,
            &[],
        ),
        ExsValue::Int(90)
    );
}

/// Keeps Bools out of arithmetic, ordering and numeric equality.
#[test]
fn keeps_bools_out_of_arithmetic() {
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List {
                    ret [
                        (true + 1) is Error,
                        (-true) is Error,
                        (true * 2.5) is Error,
                        (true < 1) is Error,
                        true == 1,
                        "a" + true,
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(false),
            ExsValue::String("atrue".to_owned()),
        ])
    );
}

/// Orders and combines Durations and DateTimes.
#[test]
fn orders_and_combines_durations_and_datetimes() {
    let duration = |seconds: i64, nanoseconds: i64| {
        ExsValue::Object(vec![
            ("seconds".to_owned(), ExsValue::Int(seconds)),
            ("nanoseconds".to_owned(), ExsValue::Int(nanoseconds)),
        ])
    };
    assert_eq!(
        execute_source_with_inputs(
            r#"
                fn main() -> List | Error {
                    let a = DateTime::from_unix(100, 0, 0, None)?;
                    let b = DateTime::from_unix(160, 500, 3600, None)?;
                    ret [
                        Duration::seconds(1) < Duration::seconds(2),
                        Duration::milliseconds(1500)? + Duration::milliseconds(700)?,
                        Duration::seconds(3) - Duration::milliseconds(500)?,
                        (Duration::seconds(1) - Duration::seconds(2)) is Error,
                        a < b,
                        a == DateTime::from_unix(100, 0, 7200, None)?,
                        b - a,
                        (a - b) is Error,
                        a + Duration::seconds(60) == DateTime::from_unix(160, 0, 0, None)?,
                    ];
                }
            "#,
            &[],
        ),
        ExsValue::List(vec![
            ExsValue::Bool(true),
            duration(2, 200_000_000),
            duration(2, 500_000_000),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
            duration(60, 500),
            ExsValue::Bool(true),
            ExsValue::Bool(true),
        ])
    );
}
