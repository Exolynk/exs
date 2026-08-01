# ExS API Documentation

This reference is generated from the root module and every reachable relative import. Adjacent `///` comments describe source declarations.

## Language

ExS is a dynamically typed scripting language compiled to WebAssembly. Root modules declare `fn main(...)`; imported modules provide functions, nominal types, traits, and implementations. `host.call(name, args...)` invokes a runner-provided host function and may suspend. `par { ... }` runs fixed tasks concurrently, while `par(functions)` runs a List of zero-argument closures.

## Modules

- [`std`](modules/std/index.md) - globally available built-in types and operations.
- [`./enum.exs`](modules/00-enum/index.md)
