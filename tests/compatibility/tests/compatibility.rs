//! Runs the corpus on the modules frozen by every release and through the current compiler.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use exs_compatibility::{
    CANCELLABLE, RUST_GUEST, SCRIPTS, cases, check, check_cancellation, compile_script, runner,
};

/// Returns the directory that holds one directory of frozen modules per release.
fn frozen() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("frozen")
}

/// Returns the entries of `directory`.
fn entries(directory: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()));
    entries
        .map(|entry| entry.expect("a directory entry").path())
        .collect()
}

/// Requires the modules of the current version, so that a version bump freezes its modules.
#[test]
fn the_current_version_is_frozen() {
    let directory = frozen().join(env!("CARGO_PKG_VERSION"));
    let names = SCRIPTS.iter().map(|(name, _)| *name);
    for name in names.chain([RUST_GUEST]) {
        let path = directory.join(format!("{name}.wasm"));
        assert!(path.exists(), "{} is missing", path.display());
    }
}

/// Runs the modules of every release. A failure means that a change broke stored modules: keep the
/// old behaviour, and never replace the module.
#[test]
fn frozen_modules_keep_their_behaviour() {
    let runner = runner().expect("the host functions register");
    let cases = cases();
    let mut failures = Vec::new();
    for release in entries(&frozen()) {
        for path in entries(&release) {
            let name = path.file_stem().and_then(|name| name.to_str());
            let wasm = fs::read(&path).expect("a frozen module");
            let cases = cases.iter().filter(|case| Some(case.module) == name);
            let mut count = 0;
            for case in cases {
                count += 1;
                if let Err(failure) = check(&runner, &wasm, case) {
                    failures.push(format!("{}: {failure}", release.display()));
                }
            }
            assert!(count > 0, "{} has no cases", path.display());
            for (_, function) in CANCELLABLE
                .iter()
                .filter(|(module, _)| Some(*module) == name)
            {
                if let Err(failure) = check_cancellation(&wasm, function) {
                    failures.push(format!("{}: {failure}", release.display()));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Compiles the scripts with the current compiler. A failure means that a change broke existing
/// source code.
#[test]
fn scripts_keep_their_behaviour() {
    let runner = runner().expect("the host functions register");
    let modules = SCRIPTS
        .iter()
        .map(|(name, source)| {
            let wasm = compile_script(name, source).unwrap_or_else(|error| panic!("{error}"));
            (*name, wasm)
        })
        .collect::<HashMap<_, _>>();
    let mut failures = Vec::new();
    for case in cases() {
        if let Some(wasm) = modules.get(case.module)
            && let Err(failure) = check(&runner, wasm, &case)
        {
            failures.push(failure);
        }
    }
    for (module, function) in CANCELLABLE {
        if let Some(wasm) = modules.get(module)
            && let Err(failure) = check_cancellation(wasm, function)
        {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
