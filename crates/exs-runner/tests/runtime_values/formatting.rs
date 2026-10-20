use super::*;

/// Evaluates ordinary, raw, and dedented formatted string interpolation.
#[test]
fn evaluates_formatted_strings() {
    let source = r##"
        fn main(name: String) -> String {
            let ordinary = f"Hello {name}: {20 + 1}";
            let raw = f#"{ordinary}\n{{literal}}"#;
            ret fd#"
                {raw}
                done
            "#;
        }
    "##;
    assert_eq!(
        execute_source(source, ExsValue::String("Ada".to_owned())),
        ExsValue::String("Hello Ada: 21\\n{literal}\ndone".to_owned()),
    );
}

/// Uses default renderers and nominal `ToString` and `Debug` implementations.
#[test]
fn renders_values_through_to_string_and_debug() {
    let source = r#"
        type User { name: String }
        type Plain {}
        enum State { Ready }

        impl ToString for User {
            fn to_string(self) -> String {
                ret f"User({self.name})";
            }
        }

        impl Debug for User {
            fn debug(self) -> String {
                ret f"User {{ name: {self.name} }}";
            }
        }

        fn default_rendering(value: ToString, diagnostic: Debug) -> List {
            ret [f"{value}", diagnostic.debug()];
        }

        fn main() -> List {
            let user = User { name: "Ada" };
            let text = "text";
            let object = {};
            let callback = () => { ret None; };
            let defaults = default_rendering(Plain {}, State::Ready);
            ret [
                f"{user}",
                user.debug(),
                None.to_string(),
                Error("Example", "message", None).debug(),
                true.debug(),
                (42).to_string(),
                (1.5).debug(),
                text.to_string(),
                [].debug(),
                object.to_string(),
                State::Ready.debug(),
                callback.to_string(),
                defaults[0],
                defaults[1],
            ];
        }
    "#;
    assert_eq!(
        execute_source_with_inputs(source, &[]),
        ExsValue::List(vec![
            ExsValue::String("User(Ada)".to_owned()),
            ExsValue::String("User { name: Ada }".to_owned()),
            ExsValue::String("None".to_owned()),
            ExsValue::String("Error".to_owned()),
            ExsValue::String("true".to_owned()),
            ExsValue::String("42".to_owned()),
            ExsValue::String("1.5".to_owned()),
            ExsValue::String("text".to_owned()),
            ExsValue::String("[]".to_owned()),
            ExsValue::String("{}".to_owned()),
            ExsValue::String("test.exs::State::Ready".to_owned()),
            ExsValue::String("fn main()".to_owned()),
            ExsValue::String("{}".to_owned()),
            ExsValue::String("test.exs::State::Ready".to_owned()),
        ]),
    );
}

/// Executes calls, assignments, conditionals, and boolean operators.
#[test]
fn executes_calls_assignments_conditionals_and_booleans() {
    assert_eq!(
        execute_source(
            r#"
            fn double(value) { ret value * 2; }
            fn main(input) {
                let value = 20;
                value = double(value);
                if value == 40 && true {
                    ret value + 2;
                } else {
                    ret 0;
                }
            }
        "#,
            ExsValue::None,
        ),
        ExsValue::Int(42)
    );
}

/// Renders every built-in value kind, user `ToString` impls and nested formatted strings alike
/// in synchronous and resumable functions.
#[test]
fn formats_all_value_kinds_in_sync_and_resumable_functions() {
    let common = r##"
        type User { name: String }
        type Count { value: Int }
        type Plain {}
        enum State { Ready, Busy(level: Int) }
        impl ToString for User { fn to_string(self) -> String { ret f"User({self.name})"; } }
        impl ToString for Count { fn to_string(self) -> String { ret f"#{self.value}"; } }
    "##;
    let render = r##"
        fn render(input) -> List {
            let user = User { name: "Ada" };
            ret [
                f"",
                f"plain",
                f"{1}|{-42}|{1.5}|{true}|{None}|{"s"}|{[1]}|{{}}|{Plain {}}|{State::Ready}|{State::Busy(2)}|{b"ab"}",
                f"{user} and {Count { value: 7 }}",
                f"a{f"b{1}c"}d",
            ];
        }
    "##;
    let expected = ExsValue::List(
        [
            "",
            "plain",
            "1|-42|1.5|true|None|s|[]|{}|{}|test.exs::State::Ready|test.exs::State::Busy|Bytes(2)",
            "User(Ada) and #7",
            "ab1cd",
        ]
        .into_iter()
        .map(|text| ExsValue::String(text.to_owned()))
        .collect(),
    );
    let sync = format!("{common}{render} fn main(input) -> List {{ ret render(input); }}");
    // A for loop selects resumable lowering for both functions.
    let resumable = format!(
        "{common}{} fn main(input) -> List {{ for item in [1] {{ }} ret render(input); }}",
        render.replace(
            "fn render(input) -> List {",
            "fn render(input) -> List { for item in [1] { }"
        )
    );
    assert_eq!(execute_source(&sync, ExsValue::None), expected);
    assert_eq!(execute_source(&resumable, ExsValue::None), expected);
}
