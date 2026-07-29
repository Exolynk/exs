//! Integration tests for the `exs` command-line interface.

use std::fs;
use std::process::Command;

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
