#!/usr/bin/env nu

# Builds the Rust fuel baseline guest and compares its fuel with every ExS fuel scenario.
const example_dir = (path self | path dirname)

cd $example_dir
cargo build --release --target wasm32-unknown-unknown
cargo run --release --features compare --bin compare
