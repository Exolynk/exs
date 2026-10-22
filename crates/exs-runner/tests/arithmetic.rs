//! The `%` operator, the `Rem` trait and Bools outside arithmetic.

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
