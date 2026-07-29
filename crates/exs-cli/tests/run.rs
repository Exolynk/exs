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
