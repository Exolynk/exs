#!/usr/bin/env nu

# Builds the Rust guest and executes it through the ExS CLI runner.
const example_dir = (path self | path dirname)
const workspace_dir = ($example_dir | path dirname | path dirname)
const wasm = ($workspace_dir | path join "target" "wasm32-unknown-unknown" "debug" "exs_rust_guest.wasm")

cd $example_dir
cargo build --target wasm32-unknown-unknown

cd $workspace_dir
cargo run -p exs-cli -- run $wasm -- 7
