use super::*;

/// Source prefix that forces `main` through the resumable continuation lowering.
const SUSPEND: &str = "Host::sleep(Duration::milliseconds(0));";

/// Executes `body` as a parameterless `main` after `declarations`, in both lowerings.
fn execute_both_with(declarations: &str, signature: &str, body: &str) -> [ExsValue; 2] {
    [
        execute_source_with_inputs(
            &format!("{declarations} fn main() {signature} {{ {body} }}"),
            &[],
        ),
        execute_source_with_inputs(
            &format!("{declarations} fn main() {signature} {{ {SUSPEND} {body} }}"),
            &[],
        ),
    ]
}

/// Executes `body` as a parameterless `main` in both the direct and resumable lowering.
fn execute_both(signature: &str, body: &str) -> [ExsValue; 2] {
    execute_both_with("", signature, body)
}

/// Asserts that both lowerings of one `main` body produce the expected value.
fn assert_both_with(declarations: &str, signature: &str, body: &str, expected: &ExsValue) {
    let values = execute_both_with(declarations, signature, body);
    for (lowering, value) in ["direct", "resumable"].iter().zip(values) {
        assert_eq!(&value, expected, "{lowering} lowering of `{body}`");
    }
}

/// Asserts that both lowerings of one `main` body produce the expected value.
fn assert_both(signature: &str, body: &str, expected: &ExsValue) {
    assert_both_with("", signature, body, expected);
}

/// Builds an expected List of Ints.
fn ints(values: &[i64]) -> ExsValue {
    ExsValue::List(values.iter().copied().map(ExsValue::Int).collect())
}

/// Builds an expected List of Bools.
fn bools(values: &[bool]) -> ExsValue {
    ExsValue::List(values.iter().copied().map(ExsValue::Bool).collect())
}

/// Int literals at and just beyond the `i32` range keep their exact values.
#[test]
fn preserves_ints_at_the_immediate_boundaries() {
    assert_both(
        "-> List",
        "ret [2147483647, -2147483648, 2147483648, -2147483649];",
        &ints(&[2_147_483_647, -2_147_483_648, 2_147_483_648, -2_147_483_649]),
    );
    for value in [
        i64::from(i32::MAX),
        i64::from(i32::MIN),
        i64::from(i32::MAX) + 1,
        i64::from(i32::MIN) - 1,
    ] {
        assert_eq!(
            execute_source(
                "fn main(input: Int) -> Int { ret input; }",
                ExsValue::Int(value)
            ),
            ExsValue::Int(value)
        );
    }
}

/// Inline arithmetic overflows into boxed Ints and boxed results return to immediates.
#[test]
fn moves_between_immediate_and_boxed_ints() {
    assert_both(
        "-> List",
        r"
        let max = 2147483647;
        let min = -2147483648;
        let above = max + 1;
        let below = min - 1;
        ret [above, below, above - 1, below + 1, min * -1, max * max, 65536 * 65536, above - above];
        ",
        &ints(&[
            2_147_483_648,
            -2_147_483_649,
            2_147_483_647,
            -2_147_483_648,
            2_147_483_648,
            4_611_686_014_132_420_609,
            4_294_967_296,
            0,
        ]),
    );
}

/// Boxed and immediate Ints compare and test equal by value in every combination.
#[test]
fn compares_boxed_and_immediate_ints_by_value() {
    assert_both(
        "-> List",
        r"
        let max = 2147483647;
        let above = max + 1;
        let back = above - 1;
        ret [
            back == max, back != max, above == 2147483648, above > max, max < above,
            -2147483649 < -2147483648, 3000000000 <= 2, -3000000000 >= 2, above >= above,
            max == 2147483647.0, above == 2147483648.0, 1 == true, 2 < 2.5
        ];
        ",
        &bools(&[
            true, false, true, true, true, true, false, false, true, true, true, true, true,
        ]),
    );
}

/// Comparisons of boundary values that differ only above the `i32` payload stay exact.
#[test]
fn orders_ints_across_the_immediate_boundary() {
    assert_both(
        "-> List",
        r"
        let values = [-2147483649, -2147483648, -1, 0, 1, 2147483647, 2147483648];
        let ordered = [];
        let index = 0;
        while index < 6 {
            ordered.push(values[index] < values[index + 1]);
            ordered.push(values[index + 1] > values[index]);
            ordered.push(values[index] == values[index + 1]);
            index = index + 1;
        }
        ret ordered;
        ",
        &bools(&[true, true, false].repeat(6)),
    );
}

/// The full 64-bit overflow check still reports a recoverable error after boxing.
#[test]
fn reports_overflow_beyond_the_boxed_int_range() {
    for value in execute_both("-> Error", "let big = 9223372036854775807; ret big + 1;") {
        let ExsValue::Error(error) = value else {
            panic!("expected an IntOverflowError, got {value:?}");
        };
        assert_eq!(error.kind, "IntOverflowError");
    }
}

/// Bool and None results are canonical immediates that compare and type-check by identity.
#[test]
fn treats_bool_and_none_as_canonical_immediates() {
    assert_both(
        "-> List",
        r"
        let less = 1 < 2;
        let none = None;
        ret [
            less == true, (1 > 2) == false, !less == false, none == None,
            less is Bool, none is None, 5 is Bool, none is Int, 2147483648 is Int, 5 is Int,
            true && less, false || less
        ];
        ",
        &bools(&[
            true, true, true, true, true, true, false, false, true, true, true, true,
        ]),
    );
}

/// A non-Bool condition still produces the runtime TypeError instead of an inline result.
#[test]
fn rejects_non_bool_conditions() {
    for value in execute_both("", "if 1 { ret 1; } ret 2;") {
        let ExsValue::Error(error) = value else {
            panic!("expected a TypeError, got {value:?}");
        };
        assert_eq!(error.kind, "TypeError");
    }
}

/// Operators on non-Int operands still use the runtime and trait fallbacks.
#[test]
fn falls_back_for_non_int_operands() {
    assert_both_with(
        r"
        type Money { cents: Int }
        impl Add for Money { fn add(self, other: Any) -> Any { ret Money { cents: self.cents + other.cents }; } }
        ",
        "-> List",
        r#"
        let total = Money { cents: 2147483647 } + Money { cents: 1 };
        ret ["a" + 1, 1.5 + 1, [1] + [2], total.cents, "b" == "b", 2 * 0.5];
        "#,
        &ExsValue::List(vec![
            ExsValue::String("a1".into()),
            ExsValue::Float(2.5),
            ints(&[1, 2]),
            ExsValue::Int(2_147_483_648),
            ExsValue::Bool(true),
            ExsValue::Float(1.0),
        ]),
    );
}

/// Mutating methods on immediate receivers report a TypeError instead of trapping.
#[test]
fn rejects_mutation_of_immediate_receivers() {
    for input in [ExsValue::Int(5), ExsValue::Bool(true), ExsValue::None] {
        let value = execute_source("fn main(input) -> Error { ret input.pop(); }", input);
        let ExsValue::Error(error) = value else {
            panic!("expected a TypeError, got {value:?}");
        };
        assert_eq!(error.kind, "TypeError");
    }
}

/// Number of declared nominal types, enough to push ids past the inline extra-bit range.
const MANY_TYPES: usize = 65_600;

/// Nominal tests and dispatch agree for type ids inside and beyond the inline extra bits.
#[test]
fn distinguishes_nominal_types_beyond_the_inline_id_range() {
    let mut declarations = String::new();
    for index in 0..MANY_TYPES {
        declarations.push_str(&format!("type T{index} {{ v: Int }}\n"));
    }
    let last = MANY_TYPES - 1;
    declarations.push_str(&format!(
        "impl T{last} {{ fn get(self) -> Int {{ ret self.v; }} }}\n"
    ));
    // Ids are assigned in declaration order, so this window spans the inline id bound.
    let window = (MANY_TYPES - 80)..MANY_TYPES;
    let mut checks = Vec::new();
    for index in window.clone() {
        checks.push(format!("T{index} {{ v: 1 }} is T{index}"));
        checks.push(format!("T{index} {{ v: 1 }} is T{}", index - 1));
    }
    let body = format!(
        "let value = T{last} {{ v: 7 }}; ret [value.get(), {}];",
        checks.join(", ")
    );
    let mut expected = vec![ExsValue::Int(7)];
    for _ in window {
        expected.extend([ExsValue::Bool(true), ExsValue::Bool(false)]);
    }
    assert_both_with(&declarations, "-> List", &body, &ExsValue::List(expected));
}
