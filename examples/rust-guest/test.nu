#!/usr/bin/env nu

# Builds the Rust guest and verifies its Host stream through a native runner smoke test.
const example_dir = (path self | path dirname)

cd $example_dir
cargo build --target wasm32-unknown-unknown
cargo run --features smoke --bin smoke
