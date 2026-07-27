//! Integration tests for the `exs` command-line interface.

use std::fs;
use std::process::Command;

/// Prints the completed `main` result for a source program.
#[test]
fn prints_the_main_result() {
    let path = std::env::temp_dir().join(format!("exs-cli-{}.exs", std::process::id()));
    if let Err(error) = fs::write(&path, "fn main() { ret 42; }") {
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
    assert_eq!(stdout, "42\n");
}
