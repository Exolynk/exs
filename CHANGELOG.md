# Changelog

Notable changes to ExS. The [compatibility promise](COMPATIBILITY.md) starts with 0.2.0.

## 0.2.0 - Unreleased

The first public release. From here on, scripts and compiled modules keep working across releases. Modules built before this version must be rebuilt from source.

### Added

- The `exs` crate is the single entry point for applications, with features for the compiler, the server and browser runners, serde, tooling, and each library.
- `exs::compile(source, options)` compiles every kind of project. `Sources` holds sources known up front, and `ModuleResolver` loads them on demand.
- `CompileError` lists each error's stable code, message, source identity, and byte range, also for imported files and libraries. Lexical and syntax errors are reported together.
- `ExecutionOptions` with `ExecutionCancellation`, and `Runner::execute_with`, which returns an `ExecutionReport` with the consumed fuel and times.
- `RunnerError::AbiMismatch` names the module's version and the versions the runner supports.
- In `exs-guest`: `host::sleep`, `Result<ExsValue, ExsError>` as an entry point's result so that `?` works, `DateTimeComponents`, and a `serde` feature with `decode`, `encode`, and `Bytes`.
- `exs --version`.
- `ABI.md`, which describes the contract between a module and a runner.
- `tests/compatibility`, which runs a script corpus through the current compiler and through modules frozen by each release.
- CI checks for the committed core template, the public API, the browser runner, the Rust guest, and the fuel comparison.

### Changed

- **Boundary:** every value crosses as an image, for all modules, and runners no longer read a module's heap. A module grows by about 10 KB, returning a large list costs more fuel and time, and normal scripts are unaffected.
- **Language:** a script's own type, enum, or trait takes the place of a prelude type of the same name, except for `Range`, `IteratorStep`, `HostStream`, and `Iterator`.
- **Language:** `async`, `await`, `loop`, `return`, `struct`, and `yield` are reserved for future use (`E0011`). A script using one of them as a name has to rename it.
- **Runner:** `BrowserRunner` has the same methods as the server runner, and `exs::Runner` names either one.
- **`exs-guest`:** host operations return `Result<_, ExsError>`, `in_timezone` borrows its `DateTime`, and `from_components_in_timezone` takes `DateTimeComponents`. `HostStream` is no longer `Copy`, and `DateTime` is `#[non_exhaustive]`.
- **`exs-libs`:** `library` returns a `Source`, and `add_to` adds the libraries to `Sources`.
- `ExsValue`, `ExsError`, `RunnerError`, and other public types are `#[non_exhaustive]`. Build `ExsError` values with `ExsError::new`, `recoverable`, or `fatal`, and configure from `Default` values.

### Fixed

- `DateTime::ordinal` returned one too few for every date from March onward. Recompile a script to get the fix; see [what a bug fix means for stored modules](COMPATIBILITY.md#what-a-bug-fix-means-for-stored-modules).
- A compiler produced different module bytes for the same input, depending on hash order.

### Removed

- `exs-guest`: the `no-std` feature. Turning off the default `std` feature gives `no_std`. The re-export `exs_guest::Vec` and the public macro internals are gone too.
- `exs-autocomplete`: the `birei` feature. The adapter moved to the playground example.
- The ABI constants for reading a module's heap directly.
