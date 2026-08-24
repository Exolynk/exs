//! Integration tests for executable ExS example scripts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Executes example scripts through the public CLI, including the expected Error fixture.
#[test]
fn executes_example_scripts() {
    let scripts = example_scripts();
    assert!(
        !scripts.is_empty(),
        "the examples/scripts directory must contain at least one .exs script"
    );
    for script in scripts {
        let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
            .arg("run")
            .arg(&script)
            .output()
        {
            Ok(output) => output,
            Err(error) => panic!(
                "could not execute root test script {}: {error}",
                script.display()
            ),
        };
        let expected_error = script.file_stem().is_some_and(|stem| stem == "error");
        if expected_error {
            assert!(
                !output.status.success(),
                "example Error script {} unexpectedly succeeded",
                script.display(),
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("exs:"),
                "example Error script {} did not print its language Error",
                script.display(),
            );
        } else {
            assert!(
                output.status.success(),
                "example script {} failed\nstdout:\n{}\nstderr:\n{}",
                script.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
}

/// Executes standalone test examples through the public test command.
#[test]
fn executes_example_tests() {
    let tests_directory = workspace_root().join("examples/scripts");
    let output = match Command::new(env!("CARGO_BIN_EXE_exs"))
        .arg("test")
        .arg(&tests_directory)
        .output()
    {
        Ok(output) => output,
        Err(error) => panic!(
            "could not execute example tests directory {}: {error}",
            tests_directory.display()
        ),
    };
    assert!(
        output.status.success(),
        "example tests failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("test result: 3 passed; 0 failed"),
        "example test command did not report every passing test\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}

/// Returns all `.exs` scripts in the workspace examples directory in a stable order.
fn example_scripts() -> Vec<PathBuf> {
    let scripts_directory = workspace_root().join("examples/scripts");
    let entries = match fs::read_dir(&scripts_directory) {
        Ok(entries) => entries,
        Err(error) => panic!(
            "could not read example scripts directory {}: {error}",
            scripts_directory.display()
        ),
    };
    let mut scripts = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "exs"))
        .collect::<Vec<_>>();
    scripts.sort();
    scripts
}

/// Finds the workspace root from this crate's manifest directory.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("exs-cli manifest is not nested below the workspace root"))
}
