# Runs the browser runner under Node and compares its results with the server runner, for v1
# and runtime v2 modules and the example Rust guest.
#
# Run from the workspace root: `nu examples/browser-check/check.nu`. The wasm-bindgen CLI must
# match the locked wasm-bindgen crate; a matching one is installed under target/tools.

def main [] {
    let version = (open --raw Cargo.lock | from toml | get package | where name == "wasm-bindgen" | first | get version)
    let tools = ("target/tools" | path expand)
    let bindgen = ($tools | path join "bin" "wasm-bindgen")
    let installed = if ($bindgen | path exists) { ^$bindgen --version | str trim } else { "" }
    if $installed != $"wasm-bindgen ($version)" {
        cargo install -q wasm-bindgen-cli --version $version --locked --root $tools
    }
    cargo build -q --release -p exs-browser-check -p exs-rust-guest --target wasm32-unknown-unknown
    let guest = ("target/wasm32-unknown-unknown/release/exs_rust_guest.wasm" | path expand)
    let out = ("target/browser-check" | path expand)
    ^$bindgen --target nodejs --out-dir $out target/wasm32-unknown-unknown/release/exs_browser_check.wasm
    let results = ($out | path join "results.txt")
    node examples/browser-check/run.cjs ($out | path join "exs_browser_check.js") $guest $results
    with-env { EXS_BROWSER_RESULTS: $results, EXS_RUST_GUEST_WASM: $guest } {
        cargo test -q --release -p exs-browser-check
    }
}
