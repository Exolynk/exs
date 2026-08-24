//! Integration tests for the `exs` command-line interface.

use std::fs;
use std::process::Command;

/// Formats a source file in place through the compiler library formatter.
#[test]
fn formats_source_files_in_place() {
    let path = std::env::temp_dir().join(format!("exs-cli-format-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main(){ret 1+2;}") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("fmt")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    assert!(output.status.success());
    let formatted = match fs::read_to_string(&path) {
        Ok(formatted) => formatted,
        Err(error) => panic!("could not read formatted source: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert_eq!(formatted, "fn main() {\n    ret 1 + 2;\n}\n");
}

/// Formats errors with the standard source excerpt diagnostics.
#[test]
fn reports_format_errors_with_source_context() {
    let path =
        std::env::temp_dir().join(format!("exs-cli-format-error-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main( {") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("fmt")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(!output.status.success());
    let stderr = match String::from_utf8(output.stderr) {
        Ok(stderr) => stderr,
        Err(error) => panic!("CLI error output was not UTF-8: {error}"),
    };
    assert!(stderr.contains("error: E0102 (compile syntax)"));
    assert!(stderr.contains("fn main( {"));
}

/// Writes an index and root-module API page through the documentation CLI command.
#[test]
fn generates_markdown_documentation() {
    let directory = std::env::temp_dir().join(format!("exs-cli-docs-{}", std::process::id()));
    let source_path = directory.join("main.exs");
    let output_path = directory.join("docs");
    if let Err(error) = fs::create_dir_all(&directory) {
        panic!("could not create documentation fixture directory: {error}");
    }
    if let Err(error) = fs::write(
        &source_path,
        "/// Runs the program.\nfn main(value: Int) -> Int { ret value + 1; }",
    ) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("docs")
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    assert!(output.status.success());
    let index = match fs::read_to_string(output_path.join("index.md")) {
        Ok(index) => index,
        Err(error) => panic!("could not read documentation index: {error}"),
    };
    let module = match fs::read_to_string(output_path.join("modules/00-main/index.md")) {
        Ok(module) => module,
        Err(error) => panic!("could not read module documentation: {error}"),
    };
    let function = match fs::read_to_string(output_path.join("modules/00-main/fn/main.md")) {
        Ok(function) => function,
        Err(error) => panic!("could not read function documentation: {error}"),
    };
    if let Err(error) = fs::remove_dir_all(&directory) {
        panic!("could not remove documentation fixture directory: {error}");
    }
    assert!(index.contains("# ExS API Documentation"));
    assert!(index.contains("[`std`](modules/std/index.md)"));
    assert!(index.contains("[`./main.exs`](modules/00-main/index.md)"));
    assert!(module.starts_with("# Module `./main.exs`"));
    assert!(module.contains("[`main`](fn/main.md)"));
    assert!(function.contains("Runs the program."));
    assert!(function.contains("fn main(value: Int) -> Int"));
}

/// Prints the completed floating-point `main` result for a source program.
#[test]
fn prints_the_main_result() {
    let path = std::env::temp_dir().join(format!("exs-cli-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main(input) { ret 1.0; }") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "1.0\n");
}

/// Makes the CLI-owned print and println host functions available to source programs.
#[test]
fn executes_cli_print_and_println_host_functions() {
    let path = std::env::temp_dir().join(format!("exs-cli-output-{}.exs", std::process::id()));
    let source = r#"
fn main() {
    Host::call("print", "Ada", 7);
    Host::call("println", true);
    Host::call("println");
    ret None;
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "Ada 7true\n\nNone\n");
}

/// Omits inline tests from normal execution and reports them through the test command.
#[test]
fn runs_inline_source_tests_without_affecting_normal_execution() {
    let directory = std::env::temp_dir().join(format!("exs-cli-tests-{}", std::process::id()));
    let path = directory.join("checks.exs");
    if let Err(error) = fs::create_dir_all(&directory) {
        panic!("could not create test fixture directory: {error}");
    }
    let source = r#"
fn main() -> String {
    ret "program";
}

test "adds values" {
    std::test::assert_eq(20 + 22, 42, "addition must preserve both operands");
}

test "reports default failed equality" {
    assert_eq(false, true);
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let run = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs run: {error}"),
    };
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "\"program\"\n");
    let test = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("test")
        .arg(&directory)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs test: {error}"),
    };
    if let Err(error) = fs::remove_dir_all(&directory) {
        panic!("could not remove test fixture directory: {error}");
    }
    assert!(!test.status.success());
    let stdout = String::from_utf8_lossy(&test.stdout);
    let stderr = String::from_utf8_lossy(&test.stderr);
    assert!(stdout.contains("PASS"));
    assert!(stdout.contains("test result: 1 passed; 1 failed"));
    assert!(stderr.contains("FAIL"));
    assert!(stderr.contains("AssertionFailed"));
    assert!(stderr.contains("assert_eq failed"));
}

/// Executes a standalone test module that intentionally has no production main function.
#[test]
fn executes_test_only_source_files() {
    let path = std::env::temp_dir().join(format!("exs-cli-test-only-{}.exs", std::process::id()));
    let source = r#"
fn add(left: Int, right: Int) -> Int {
    ret left + right;
}

type Version {
    value: Int,
}

impl std::Compare for Version {
    fn compare(self, other: Any) -> Ordering {
        if self.value < other.value {
            ret Ordering::Less;
        }
        if self.value > other.value {
            ret Ordering::Greater;
        }
        ret Ordering::Equal;
    }
}

test "adds values" {
    assert_eq(add(20, 22), 42, "addition must preserve both operands");
}

test "uses custom equality" {
    assert_eq(
        Version { value: 1 },
        Version { value: 1 },
        "versions with the same value must compare equal",
    );
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("test")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs test: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("test result: 2 passed; 0 failed"));
}

/// Terminates normal programs when an assertion fails inside a nested direct call.
#[test]
fn assertion_failures_abort_normal_execution() {
    let path = std::env::temp_dir().join(format!("exs-cli-assert-{}.exs", std::process::id()));
    let source = r#"
fn require_enabled(enabled: Bool) {
    assert(enabled, "the account must be enabled");
}

fn main() {
    require_enabled(false);
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("AssertionFailed"));
    assert!(stderr.contains("the account must be enabled"));
    assert!(stderr.contains("require_enabled"));
}

/// Parses multiple CLI values and passes them to a typed main declaration.
#[test]
fn passes_multiple_cli_values_to_main() {
    let path = std::env::temp_dir().join(format!("exs-cli-inputs-{}.exs", std::process::id()));
    let source = r#"
fn main(number: Int, name: String, values: List, profile: Object) -> String {
    ret profile.role;
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .arg("--")
        .arg("1")
        .arg("Ada")
        .arg("[3, 'four']")
        .arg("{role: admin}")
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "\"admin\"\n");
}

/// Supplies None when no CLI value is present for a declared main parameter.
#[test]
fn supplies_none_for_missing_cli_values() {
    let path = std::env::temp_dir().join(format!("exs-cli-missing-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main(value: None) -> None { ret value; }") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "None\n");
}

/// Prints a completed string result in ExS source notation.
#[test]
fn prints_the_main_string_result() {
    let path = std::env::temp_dir().join(format!("exs-cli-string-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, r#"fn main(input) { ret "Ada\n"; }"#) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "\"Ada\\n\"\n");
}

/// Prints a completed nested list result in ExS source notation.
#[test]
fn prints_the_main_list_result() {
    let path = std::env::temp_dir().join(format!("exs-cli-list-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main(input) { ret [1, [true, \"Ada\"]]; }") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "[1, [true, \"Ada\"]]\n");
}

/// Prints a completed object result in insertion order using source-like notation.
#[test]
fn prints_the_main_object_result() {
    let path = std::env::temp_dir().join(format!("exs-cli-object-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main(input) { ret { name: \"Ada\", age: 42 }; }") {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(output.status.success());
    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(error) => panic!("CLI output was not UTF-8: {error}"),
    };
    assert_eq!(stdout, "{\"name\": \"Ada\", \"age\": 42}\n");
}

/// Prints a complete source-resolved language Error and exits unsuccessfully.
#[test]
fn prints_runtime_errors_with_origin_and_named_trace() {
    let path = std::env::temp_dir().join(format!("exs-cli-error-{}.exs", std::process::id()));
    let source = r#"
fn inner(value) {
    if 1 {
        ret value;
    }
    ret value;
}
fn main(input) {
    ret inner(input);
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(!output.status.success());
    let stderr = match String::from_utf8(output.stderr) {
        Ok(stderr) => stderr,
        Err(error) => panic!("CLI error output was not UTF-8: {error}"),
    };
    assert!(stderr.contains("error: TypeError (recoverable)"));
    assert!(stderr.contains("origin: "));
    assert!(stderr.contains("if 1 {"));
    assert!(stderr.contains("trace:"));
    assert!(stderr.contains("inner called at"));
    assert!(stderr.contains("main"));
}

/// Prints a fatal type contract Error without exposing a Wasmtime backtrace.
#[test]
fn prints_fatal_type_contract_errors_with_source_context() {
    let path = std::env::temp_dir().join(format!("exs-cli-fatal-{}.exs", std::process::id()));
    let source = r#"
fn add(value: Int, offset: Float) -> Float {
    ret value + offset;
}
fn main(input) {
    ret add(3.3, 2.3)?;
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(!output.status.success());
    let stderr = match String::from_utf8(output.stderr) {
        Ok(stderr) => stderr,
        Err(error) => panic!("CLI error output was not UTF-8: {error}"),
    };
    assert!(stderr.contains("error: TypeError (fatal)"));
    assert!(stderr.contains("value does not satisfy the declared function type"));
    assert!(stderr.contains("data: 3.3"));
    assert!(stderr.contains("origin: "));
    assert!(stderr.contains("trace:"));
    assert!(stderr.contains("add called at"));
    assert!(stderr.contains("main"));
    assert!(!stderr.contains("WebAssembly error"));
}

/// Renders all recovered compiler diagnostics with source excerpts.
#[test]
fn prints_structured_compile_diagnostics() {
    let path =
        std::env::temp_dir().join(format!("exs-cli-compile-errors-{}.exs", std::process::id()));
    let source = r#"
fn main() {
    let value = { name: "Ada"; };
    ret value
}
"#;
    if let Err(error) = fs::write(&path, source) {
        panic!("could not create source fixture: {error}");
    }
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("run")
        .arg(&path)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!("could not execute exs: {error}"),
    };
    if let Err(error) = fs::remove_file(&path) {
        panic!("could not remove source fixture: {error}");
    }
    assert!(!output.status.success());
    let stderr = match String::from_utf8(output.stderr) {
        Ok(stderr) => stderr,
        Err(error) => panic!("CLI error output was not UTF-8: {error}"),
    };
    assert_eq!(stderr.matches("error: E0103 (compile syntax)").count(), 2);
    assert!(stderr.contains("message: expected `,` or `}` after object property"));
    assert!(stderr.contains("origin: "));
    assert!(stderr.contains("ret value"));
}
