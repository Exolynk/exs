//! Integration tests for executable ExS scripts in the workspace root.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Executes every root test script successfully through the public CLI.
#[test]
fn executes_root_test_scripts() {
    let scripts = root_test_scripts();
    assert!(
        !scripts.is_empty(),
        "the workspace root tests directory must contain at least one .exs script"
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
        assert!(
            output.status.success(),
            "root test script {} failed\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// Returns all `.exs` scripts in the workspace root test directory in a stable order.
fn root_test_scripts() -> Vec<PathBuf> {
    let tests_directory = workspace_root().join("tests");
    let entries = match fs::read_dir(&tests_directory) {
        Ok(entries) => entries,
        Err(error) => panic!(
            "could not read root test directory {}: {error}",
            tests_directory.display()
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
