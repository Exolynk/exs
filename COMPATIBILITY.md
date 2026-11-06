# Compatibility

Applications store scripts and compiled modules for years. This document says what keeps working across releases of ExS, starting with 0.2.0, and how that is enforced.

## What is promised

**Compiled modules keep running.** A module built by this or a later compiler, or built with `exs-guest`, runs unchanged on every later runner. It needs no source code and no recompilation. ExS modules and Wasm from other languages are treated the same: both rely only on the ABI.

**Scripts keep compiling and keep their meaning.** A script that compiles today compiles with later compilers, and behaves the same unless it relied on a bug.

**Host integrations keep working.** The names and value shapes of built-in host operations and of the functions behind the `exs` libraries do not change. A value's type identity, such as `app::dom::Div`, keeps its form.

**The Rust API follows semantic versioning.** This covers the public API of `exs` and `exs-guest`. A breaking change needs a new minor version while ExS is at 0.x, and is listed in the [changelog](CHANGELOG.md).

## What is not promised

- **The `exs-*` crates behind `exs`.** `exs-abi`, `exs-compiler`, `exs-runner`, and the others can change in any release. They are pinned to exact versions, so depend on `exs` and `exs-guest` only.
- **Items marked `#[doc(hidden)]`**, such as the internals of `exs_guest::export!`.
- **Error messages and rendered reports.** Error kinds such as `TypeError`, and compiler codes such as `E0205`, are stable. Their wording is not.
- **Fuel consumption and speed.** A later compiler may spend more or less fuel on the same script, so leave headroom in `max_fuel`. A stored module's own code does not change.
- **The bytes of a compiled module.** One compiler produces the same bytes for the same input, but another version may not.
- **New modules on old runners.** A module needs a runner that supports its ABI version. An older runner reports `RunnerError::AbiMismatch`, so update runners before compilers.
- **The modules of a version before 0.2.0.** They must be rebuilt from source.
- **Exhaustive matches on `#[non_exhaustive]` enums.** These enums, such as `ExsValue` and `RunnerError`, can gain variants without a breaking change, so a `match` on them needs a fallback arm.

## What a bug fix means for stored modules

A compiled module contains its own copy of the runtime and standard library. A bug fix therefore reaches a script when it is compiled again, and a stored module keeps its old behaviour until then. That is deliberate: nothing changes under a module that is already running.

A fix can still change the results of a script that relied on the bug, so the [changelog](CHANGELOG.md) lists fixes that change behaviour.

## Rules for changing ExS

- **ABI:** [ABI.md](ABI.md) describes the contract. `ABI_VERSION` may rise. `MIN_SUPPORTED_ABI_VERSION` stays where it is, because raising it stops stored modules from running. A runner keeps supporting every version in between, and it treats each module according to the version the module reports.
- **New kinds of values:** a value that the existing image records cannot express, such as a cyclic value, a shared reference, a host resource, or a new primitive type, needs a new record kind and so a new ABI version. It only reaches modules built for that version. If a host passes such a value to an older module, the runner reports a clear error rather than sending bytes the module cannot read. New `type` and `enum` declarations are not new kinds: they cross as `Typed` and `Enum` records, and older modules handle them as well.
- **Reserved names:** the lists are closed, because adding to one can break a script that uses the name. They are the keywords, including those reserved for future use (`async await loop return struct yield`), the functions `assert` and `assert_eq`, and the types `Range`, `IteratorStep`, `HostStream`, and `Iterator`.
- **New language features** use the reserved words or syntax that does not compile today.
- **New standard library items** must leave a script's own declarations in place. A script's own type, enum, trait, or function of the same name wins, and so does a method on its own type.
- **Removals:** language features, standard library items, and built-in host operations are not removed.

A first version change has not happened yet: ABI 29 is the only version so far, so `MIN_SUPPORTED_ABI_VERSION` and `ABI_VERSION` are equal. Executing two versions side by side will be built and tested with ABI 30.

## How it is enforced

- **`tests/compatibility`** runs a corpus of scripts and their expected results two ways: through the current compiler, and through the modules frozen by every release, including a Rust guest. It covers every kind of value in both directions, types the corpus does not declare, suspending and resuming, cancellation, and the built-in operations. Frozen modules are never replaced. A failure means a change broke existing code, so the change has to be fixed rather than the test.
- **`cargo-semver-checks`** compares the API of `exs` and `exs-guest` with the latest published release in CI.
- **The core template:** CI rebuilds `exs-core-template.wasm` and compares it with the committed file, so a compiled module always comes from the reviewed source.

## Releasing a version

1. Update `CHANGELOG.md` and the workspace version.
2. Run `nu tests/compatibility/freeze.nu` and commit the new directory under `tests/compatibility/frozen`.
3. Run `cargo publish --dry-run --workspace`.
4. Tag the release and run `cargo publish --workspace`.
