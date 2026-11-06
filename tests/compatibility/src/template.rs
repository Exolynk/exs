//! The hash that ties the committed core template to the sources it was built from. Building the
//! template can give different bytes on different systems, so a test compares sources rather than
//! rebuilding it.

use std::fs;
use std::path::{Path, PathBuf};

/// The file beside the template that holds the hash of its sources, relative to the workspace.
pub const STAMP: &str = "crates/exs-core-template/exs-core-template.sources";

/// The crates the template is built from, relative to the workspace.
const CRATES: &[&str] = &[
    "crates/exs-abi",
    "crates/exs-core",
    "crates/exs-core-template",
    "crates/exs-layout",
];

/// The files outside those crates that affect the build, relative to the workspace.
const FILES: &[&str] = &[
    "crates/exs-core-template/build-template.nu",
    "rust-toolchain.toml",
];

/// Returns the workspace directory.
#[must_use]
pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Returns the hash of the sources of the template in the workspace at `root`. It is the same on
/// every system, and it does not protect against deliberate tampering.
///
/// # Errors
///
/// Returns a description of a file that cannot be read.
pub fn sources_hash(root: &Path) -> Result<String, String> {
    let mut files = FILES
        .iter()
        .map(|file| (*file).to_owned())
        .collect::<Vec<_>>();
    for name in CRATES {
        files.push(format!("{name}/Cargo.toml"));
        collect_sources(root, &format!("{name}/src"), &mut files)?;
    }
    files.sort();
    let mut hash = FNV_OFFSET;
    for file in &files {
        let path = root.join(file);
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        hash = fnv(hash, file.as_bytes());
        hash = fnv(hash, &[0]);
        // Git may check text out with CRLF line endings.
        let text = bytes
            .iter()
            .copied()
            .filter(|byte| *byte != b'\r')
            .collect::<Vec<_>>();
        hash = fnv(hash, &text);
        hash = fnv(hash, &[0]);
    }
    Ok(format!("{hash:016x}"))
}

/// Adds the Rust files below `directory` to `files`, without the tests, which are not part of the
/// template.
fn collect_sources(root: &Path, directory: &str, files: &mut Vec<String>) -> Result<(), String> {
    let path = root.join(directory);
    let entries = fs::read_dir(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = format!("{directory}/{name}");
        if entry.path().is_dir() {
            if name != "tests" {
                collect_sources(root, &relative, files)?;
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            files.push(relative);
        }
    }
    Ok(())
}

/// The FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// The FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Continues the FNV-1a hash `hash` over `bytes`.
fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}
