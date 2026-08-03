# Exolynk Script (ExS) Specification

**Version:** 0.1.0-draft

## Table of Contents

- [00 – Overview](#00-overview)
- [01 – Design Goals](#01-design-goals)
- [02 – Lexical Structure](#02-lexical-structure)
- [03 – Values and Types](#03-values-and-types)
- [04 – Variables and Scope](#04-variables-and-scope)
- [05 – Expressions](#05-expressions)
- [06 – Statements](#06-statements)
- [07 – Functions and Closures](#07-functions-and-closures)
- [08 – Errors](#08-errors)
- [09 – Concurrency with `par`](#09-concurrency-with-par)
- [10 – Modules](#10-modules)
- [11 – Built-ins](#11-built-ins)
- [12 – Compiler](#12-compiler)
- [13 – Runtime](#13-runtime)
- [14 – Scheduler](#14-scheduler)
- [15 – Garbage Collection](#15-garbage-collection)
- [16 – Host ABI](#16-host-abi)
- [17 – Runner](#17-runner)
- [18 – Compiler ↔ Runtime ABI](#18-compiler-runtime-abi)
- [19 – Grammar Summary](#19-grammar-summary)
- [20 – Conformance and Security](#20-conformance-and-security)

# 00 – Overview

**Specification version:** 0.1.0-draft  
**Language:** Exolynk Script (ExS)  
**Status:** Normative draft

This specification is the canonical definition of the ExS source language, compiler, `exs-runtime`, scheduler, runner, Host ABI, and compiler-to-runtime ABI. The consolidated architecture rules in this section take precedence over older text in this document when the two differ.

## Normative language

The terms **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** express normative requirements.

A section marked _informative_ explains a rule but does not add requirements. A behavior marked _implementation-defined_ MUST be documented by the implementation. A behavior marked _host-defined_ is selected by the embedding host.

## Purpose

An experienced programmer or implementation system MUST be able to implement ExS from this specification without inventing additional language semantics.

## System model

```text
ExS source
  ↓
Lexer → Parser → AST → semantic analysis → HIR
  ↓
closure/capture and suspendability analysis
  ↓
state-machine lowering → low-level IR → Wasm code generation
  ↓
linking with `exs-runtime.wasm`
  ↓
executable WebAssembly module → runner → host
```

The compiler translates ExS source modules directly into WebAssembly. ExS has no virtual machine, bytecode, or interpreter. The compiler links generated program code with the committed `exs-runtime.wasm` template to produce one executable WebAssembly module.

`exs-runtime` runs inside that final module and owns the dynamic value model, heap, errors, cloning, task scheduling, garbage collection, CBOR conversion, and hostcall integration. The runner runs outside the module, owns WebAssembly instances, enforces limits, resolves host capabilities dynamically, and connects the module to the host. Browser and server runners implement the same logical Host ABI.

Normal source execution is strictly sequential. The compiler MUST NOT automatically parallelize calls or hostcalls. Only the explicit `par` construct creates language tasks. A hostcall is nevertheless potentially suspendable because the runner decides whether it is synchronous or asynchronous only at execution time.

## Core invariants

1.  ExS compiles directly to WebAssembly.
2.  ExS has no virtual machine, bytecode, interpreter, `async`, `await`, visible Promise/Future, `try`, `catch`, or `throw`.
3.  ExS is dynamically typed.
4.  Every language value is represented internally by a runtime `Value`.
5.  `Int` and `Float` are distinct runtime types.
6.  Recoverable failures are `Error` values.
7.  ExS has no exceptions and performs no exception stack unwinding.
8.  Only `par` creates language tasks.
9.  Tasks share one heap within a root execution.
10. Clone is a built-in, synchronous deep operation.
11. Clone preserves cycles and aliasing.
12. `HostResource` values are not ordinary CBOR values and are not cloneable.
13. One WebAssembly instance executes no more than one root execution at a time.
14. A root execution MAY have multiple hostcalls outstanding.
15. Runner-limit violations terminate the entire root execution.
16. Compiler-generated runtime calls use stable ABI names and MUST NOT depend on fixed WebAssembly function indices.
17. Internal heap handles and the `Value` bit layout never cross the Wasm-host boundary; that boundary uses CBOR.

# 01 – Design Goals

## Goals

ExS prioritizes:

- direct and portable WebAssembly compilation;
- a compact language and runtime;
- explicit concurrency;
- stable and versioned interfaces;
- deterministic behavior where the host is not inherently nondeterministic;
- recoverable errors as ordinary values;
- clear separation of language, runtime, runner, and host responsibilities; and
- suitability for independent human or machine implementation.

## Explicit non-goals

ExS does not provide:

- classes or inheritance;
- exception throwing, catching, or unwinding;
- `async`, `await`, or promises;
- user-defined macros;
- reflection over compiler or runtime internals;
- implicit language tasks created by hostcalls;
- a standardized JIT;
- a standardized bytecode format; or
- concurrent root executions inside one WebAssembly instance.

## Compatibility

The language version, compiler/runtime ABI version, and Host ABI version MUST be independently identifiable.

A component MUST reject an incompatible major version. A component MAY accept a newer minor version only when all used features are understood.

Reserved syntax, value tags, ABI fields, and CBOR tags MUST NOT be assigned implementation-specific meanings that conflict with future standardization.

# 02 – Lexical Structure

## Source encoding

Source files MUST be valid UTF-8. A byte-order mark MAY occur only at the beginning of a file and is ignored.

Line endings `LF` and `CRLF` are equivalent.

## Whitespace and comments

Spaces, tabs, and line endings separate tokens where required and are otherwise insignificant.

A line comment begins with `//` and continues to the end of the line.

A block comment begins with `/*` and ends at the next `*/`. Block comments do not nest. An unterminated block comment is a compile error.

## Identifiers

An identifier begins with `_` or a Unicode XID*Start code point. Remaining code points MUST be `*` or Unicode XID_Continue code points.

Identifiers are case-sensitive and are compared by Unicode scalar sequence without normalization.

Reserved keywords are:

```text
as break continue else export false fn for from if import in
is let None Error par ret true use while
```

## Literals

### Integer literals

Decimal integer literals contain digits and optional `_` separators.

```text
0
42
1_000_000
```

A separator MUST occur between digits.

### Float literals

A float literal contains a decimal point, exponent, or both.

```text
1.0
0.25
1e6
2.5e-3
```

### String literals

Strings use double quotes.

```text
"hello"
"line\nbreak"
```

Required escapes are `\"`, `\\`, `\n`, `\r`, `\t`, `\0`, and `\u{HEX}`. The Unicode escape MUST denote a valid Unicode scalar value.

### Collection literals

```text
[1, 2, 3]
{ name: "Ada", "display-name": "Ada" }
```

An unquoted object key is converted to its identifier spelling as a String key.

## Statement termination

Simple statements MUST end with `;`. A block statement does not require a trailing semicolon.

# 03 – Values and Types

ExS is dynamically typed. Types belong to runtime values, not variable bindings.

## Value categories

| Type           |                Mutable | Heap allocated |
| -------------- | ---------------------: | -------------: |
| `None`         |                     No |            Yes |
| `Bool`         |                     No |            Yes |
| `Int`          |                     No |            Yes |
| `Float`        |                     No |            Yes |
| `String`       |                     No |            Yes |
| `List`         |                    Yes |            Yes |
| `Object`       |                    Yes |            Yes |
| `Function`     |                     No |            Yes |
| `Closure`      | Through captured cells |            Yes |
| `Cell`         |                    Yes |            Yes |
| `Error`        |              Partially |            Yes |
| `HostResource` |           Host-defined |            Yes |

The shared Rust `value` crate defines the non-observable runtime carrier as:

```rust
#[repr(transparent)]
pub struct ValueRef(NonZeroU32);
```

`ValueRef` is a nonzero, one-based index into the runtime-owned value table. It has no tag or payload and MUST NOT cross the Wasm-host boundary. The compiler only uses stable runtime ABI operations to create, pass, and operate on values; it MUST NOT inspect the value table or runtime-object layouts.

`exs-abi` defines the host-safe `ExsValue` transport enum and its CBOR codec. The implemented subset supports None, Error, Bool, Int, Float, String, recursively nested List and Object values, and tagged nominal enum values. `ExsValue` is not a runtime heap value: the runtime converts between it and `RtValue` at the Wasm-host boundary.

The runtime stores the actual payload in `RtValue`. Primitive payloads are inline. Every complex variant MUST be boxed so adding it cannot increase the allocation size of primitive values:

```rust
#[repr(u8)]
pub enum RtValue {
    None,
    Error(Box<RuntimeError>),
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Box<RuntimeString>),
    List(Box<RuntimeList>),
    Object(Box<RuntimeObject>),
    // Other complex runtime values are boxed in the same manner.
}
```

## None

`None` is the only absence value. ExS has no `null` source literal. CBOR `null` is used only as the host-boundary representation of `None`.

## Option and Result

ExS has no `Ok` wrapper. An Option is either `None` or a direct value; a Result is either an `Error` or a direct value. The postfix `?` operator immediately returns an Error unchanged, converts `None` to `Error { kind: "MissingValue" }`, and otherwise leaves its direct value unchanged. In a function with an explicit return contract, `?` is valid only when that contract includes `Error`; an omitted `Any` contract also permits it.

## Bool

`true` and `false` are the only `Bool` values. ExS does not define implicit truthiness. Conditions MUST evaluate to `Bool`. In numeric arithmetic, equality, and ordering operations, `false` converts to `0` and `true` converts to `1`.

## Int

`Int` represents every integer in the inclusive range:

```text
-2^55 .. 2^55 - 1
```

This is a signed 56-bit range including the sign bit.

An integer literal outside this range is a compile error. Integer arithmetic that exceeds the range produces `IntOverflowError`.

`integer.abs()` returns its non-negative `Int` value. It returns `IntOverflowError` for the unique minimum-range value whose absolute value cannot be represented by `Int`.

## Float

`Float` uses IEEE 754 binary64 semantics. Implementations MUST preserve all finite binary64 values, infinities, signed zero, and NaN.

Mixed Bool/Int/Float arithmetic converts Bool to Int first. Any operation with a Float converts the other numeric operand to binary64 before applying the operation.

`float.abs()`, `float.floor()`, `float.ceil()`, and `float.round()` return Float values. `round()` rounds exact halfway values away from zero.

## String

A `String` is an immutable sequence of Unicode scalar values. Indexing is by scalar index, not UTF-8 byte offset.

String indexing returns a one-scalar String. An invalid index produces `IndexError`.

The current implementation supports double-quoted literals and the required escapes, boxed immutable runtime strings, CBOR String input and output, and content equality. The compiler emits each unique literal as a passive Wasm data segment. At evaluation, generated code copies the segment to a runtime-owned temporary buffer with `memory.init`; `__exs_rt_string_new` validates the complete UTF-8 sequence and copies it into `RuntimeString`. Literal data is therefore never placed in an address range that can overlap the runtime allocator.

String indexing and other String operations are deferred beyond the current implementation slice. `string.length()` returns its Unicode scalar count, and `string.is_empty()` returns whether that count is zero.

## List

A `List` is a mutable ordered sequence of `Value` references. Lists may contain cycles and repeated aliases.

List indexes are zero-based `Int` values. Negative indexes are invalid.

The current implementation supports `[]`, comma-separated list literals, dynamic `value[index]` reads, `value[index] = replacement;` writes, and member-call lowering through `__exs_rt_call_method(receiver, method, arguments)`. The compiler does not establish a receiver type: `__exs_rt_index_get`, `__exs_rt_index_set`, `__exs_rt_append`, and `__exs_rt_call_method` dispatch from the runtime `RtValue`. Lists implement `push(value)`, `pop()`, `insert(index, value)`, `remove(index)`, and `clear()`. `push` returns the new length; `pop` returns `None` when empty; `insert` and `clear` return `None`; and `remove` returns the removed value. `list + value` returns a new shallow List with `value` appended; `list + list` returns a new shallow chained List. Neither `+` form mutates its source Lists. Invalid receivers return `TypeError`, invalid indexes return `IndexError`, and incorrect method arity returns `ArityError`.

`list.length()` returns the current element count and `list.is_empty()` returns whether it is zero. Nested acyclic Lists cross the current CBOR boundary as arrays. The runtime can create cyclic Lists, but serializing one currently traps; graph-reference CBOR encoding is deferred to the Error/host-boundary work.

## Object

An `Object` is a mutable mapping from String keys to `Value` references. Key iteration order is insertion order. Replacing an existing key does not change its position.

The current implementation stores Objects as boxed insertion-ordered entries. It supports `{ key: value, "key": value }` literals, dynamic `object[key]` reads and writes, dot-property reads and writes, and identity equality. Missing Object reads return `None`; writes create or replace a property. `has(key)`, `delete(key)`, `keys()`, and `values()` are dispatched by `__exs_rt_call_method`; `keys()` and `values()` return new Lists in insertion order. Unsupported receivers and non-String keys return `TypeError`; an unknown method returns `MethodNotFound`.

`object.length()` returns the current entry count and `object.is_empty()` returns whether it is zero. Nested acyclic Objects cross the current CBOR boundary as text-keyed maps in their insertion order. The runtime currently traps when serializing a container cycle; graph-reference CBOR encoding is deferred to the Error/host-boundary work.

## Nominal Object types

An ExS module MAY declare a nominal Object type:

```text
type User {
    name: String,
    nickname: String | None,
    metadata,
}
```

Each field annotation is optional and an omitted annotation means `Any`. A nominal Object is constructed with `User { ... }`. Every declared field is present after construction: an omitted field whose contract permits `None`, including `Any`, is inserted as an explicit `None` property. Missing required fields, unknown fields, duplicate fields, and incompatible field values produce the same recoverable-or-fatal `TypeError` behavior as a function contract mismatch.

Nominal type identity is separate from Object shape. The runtime stores an opaque compiler-assigned tag only for Objects constructed with `Type { ... }`; decoded host Objects and ordinary Object literals are untyped. Nominal tags never cross the CBOR boundary.

An `impl` block declares direct methods for one nominal type:

```text
impl User {
    fn display(self) -> String {
        ret self.name;
    }

    fn named(name: String) -> User {
        ret User { name: name };
    }
}
```

A first bare `self` parameter declares an instance method, invoked as `user.display()`, and is implicitly constrained to the enclosing nominal type. A method without `self` is static and is invoked as `User::named("Ada")`. `impl Trait for Type` uses the same method forms and is specified in the Traits section. Method references are not supported. Runtime method names `abs`, `floor`, `ceil`, `round`, `clone`, `length`, `is_empty`, `kind`, `message`, `data`, `cause`, `push`, `pop`, `insert`, `remove`, `clear`, `has`, `delete`, `keys`, and `values` are reserved and MUST NOT be declared by an `impl` block.

## Enums

An ExS module MAY declare a nominal enum with zero-payload variants or variants with zero or more named, optionally typed payload fields:

```text
enum Color {
    Rgb(red: Int, green: Int, blue: Int),
    Named(name: String),
    Transparent,
}
```

Variants are constructed through their qualified enum name: `Color::Rgb(255, 0, 128)`, `Color::Named("brand")`, and `Color::Transparent`. Payload arguments are evaluated and checked in declaration order. An incorrect arity or a payload that violates its declared contract produces the normal function-contract `TypeError` behavior. A payload-bearing variant cannot be referenced without its call syntax.

Enums are nominal types. They are valid in function contracts, imports, and `use` declarations; `use namespace::{Color as Tone}` also makes `Tone::Rgb(...)` and other constructors available. Inherent `impl Color` and trait `impl Trait for Color` blocks use the same instance and static method rules as nominal Object types. Enum payload fields are private to their declaration until pattern matching is specified.

At the host boundary, an enum encodes as CBOR tag 60005 followed by the fixed array `[type_identity, variant, fields]`. `type_identity` is an opaque resolver-derived source identity and enum name, `variant` is the source-visible variant name, and `fields` is the ordered payload array. This identity is used to validate enum contracts for runner-provided values; compiler-local nominal dispatch tags never cross the boundary.

## Match

`match` is an expression that selects one arm from an enum value. Variant patterns are qualified and bind their ordered payload fields by name; `_` is an optional final fallback pattern:

```text
let brightness = match color {
    Color::Rgb(red, green, blue) => red + green + blue,
    Color::Named(name) => 1,
    Color::Transparent => 0,
};
```

Every non-fallback arm MUST name a declared variant of the same enum and supply exactly its payload binding count. A variant may occur at most once. Bindings are lexical to their arm and follow the ordinary capture rules when used by a closure. Arms are checked in source order; `_`, when present, MUST be last.

Without `_`, a match MUST list every declared variant of its enum. The compiler rejects non-exhaustive matches. At runtime, a value that is not one of the listed variants, including a host-provided value with an invalid variant name, produces a recoverable `MatchError`. As with every recoverable Error, an explicit enclosing result contract must permit `Error` for the Error to be returned unchanged.

An arm body may instead be a normal statement block, such as `Color::Transparent => { ret -1; }`. Its `ret` returns from the enclosing function. A block that completes normally produces `None` as its match value. The matched value is evaluated exactly once. Only the selected arm body is evaluated. Match lowering uses continuation graph branches, so an arm may call the host, invoke a suspendable function, or otherwise suspend like any other expression.

## Function and Closure

A `Function` is immutable executable code. A `Closure` combines a function with captured lexical cells.

## Cell

A `Cell` is the mutable storage used for a captured binding. Cells are not directly constructible or inspectable by source programs.

## Error

An `Error` is a recoverable failure value with the properties defined in the Errors chapter.

## HostResource

A `HostResource` is an opaque host-owned capability or handle. Its internal identity and lifetime are host-defined. A program may pass it back to compatible host operations but may not inspect, clone, persist, or serialize it as an ordinary value.

# 04 – Variables and Scope

## Bindings

A binding is declared with `let`.

```text
let count = 0;
let empty;
```

A declaration without an initializer stores None.

A binding MUST be declared before use. Duplicate declarations in the same lexical scope are compile errors. Shadowing in a nested scope is allowed.

## Assignment

```text
count = count + 1;
```

Assignment replaces the value stored in the binding and evaluates to the assigned value.

Assignment to an undeclared identifier is a compile error.

## Lexical scope

The following constructs introduce a lexical scope:

- a module;
- a function body;
- a block;
- each branch body;
- a loop body; and
- each task body created by `par`.

Bindings leave scope at the end of the declaring scope, but captured bindings remain alive while reachable closures exist.

## Captures

A local binding captured by a nested function MUST be represented by a shared `Cell`. Reads and writes by all closures and the declaring scope observe the same cell.

Capture is by reference, not by value.

## Module bindings

Top-level module bindings are initialized during module initialization. A `use` declaration creates a read-only compile-time alias to an imported declaration. Assignment to a used alias is a compile error.

# 05 – Expressions

## Evaluation order

Operands and arguments are evaluated from left to right. Each operand is evaluated exactly once unless the specification explicitly states otherwise.

## Primary expressions

Primary expressions include literals, identifiers, list literals, object literals, function expressions, parenthesized expressions, property access, index access, and calls.

## Property access

```text
value.name
value["name"]
```

Dot access is equivalent to indexing with the property name as a String except for runtime-owned built-in methods such as `clone()`.

Reading a missing Object property returns `None`. Writing a property creates or replaces it.

Property access on an unsupported value produces `TypeError`.

## Index access

```text
list[index]
string[index]
object[key]
```

List and String indexes MUST be `Int`. Object keys MUST be `String`.

An invalid List or String index produces `IndexError`.

## Calls

```text
function(arg1, arg2)
```

The callee is evaluated first, followed by arguments from left to right.

Calling a non-callable value produces `TypeError`. Arity is checked at runtime. A mismatched arity produces `ArityError`.

## Unary operators

| Operator | Accepted type | Result                      |
| -------- | ------------- | --------------------------- |
| `!`      | `Bool`        | `Bool`                      |
| `-`      | `Bool` or `Int` | `Int` or `IntOverflowError` |
| `-`      | `Float`       | `Float`                     |

## Arithmetic

`+`, `-`, and `*` accept Bool, Int, and Float operands. Bool converts to Int (`false` is `0`, `true` is `1`). Integer-only operations produce Int or `IntOverflowError`. If either operand is Float, the result is Float.

`/` accepts numeric operands and always produces `Float`. Division by zero follows IEEE 754 behavior after conversion.

`%` accepts two `Int` operands. Division by zero produces `DivisionByZeroError`. The result has the sign of the left operand.

`+`, `-`, `*`, and `/` first select the matching standard trait implementation when their left operand is a nominal value. `Add`, `Sub`, `Mul`, and `Div` respectively require `fn add(self, other: Any) -> Any`, `fn sub(self, other: Any) -> Any`, `fn mul(self, other: Any) -> Any`, and `fn div(self, other: Any) -> Any`. Each implementation receives the evaluated right operand unchanged, may return any value including Error, and may suspend. Built-in `Bool`, `Int`, and `Float` values expose all four methods, so each `value.add|sub|mul|div(other)` call has the same behavior as its operator. Built-in `String` and `List` values additionally implement `Add`: String concatenates String, Bool, Int, or Float right operands using their normal source spelling; `list + value` returns a new shallow List with `value` appended, while `list + list` chains both Lists' elements into a new shallow List. No other implicit conversions occur.

## Comparison

`Ordering` is a globally available compiler-owned enum, also available as `std::Ordering`:

```text
enum Ordering {
    Less,
    Equal,
    Greater,
    Unordered,
}
```

`Compare` is a compiler-owned standard trait with the fixed method signature `fn compare(self, other: Any) -> Ordering`. A nominal type or enum may implement it, and the implementation is selected before the built-in fallback for `==`, `!=`, `<`, `<=`, `>`, and `>=`. `Compare` methods may suspend. Their result MUST be an `Ordering`, enforced by the normal function return contract.

Built-in comparison returns `Less`, `Equal`, or `Greater` for two Bool/Int/Float numeric values or two Strings. Bool converts to Int; numeric mixed comparison converts the non-Float operand to Float. String comparison is lexicographic by Unicode scalar value. Float comparisons involving an unordered IEEE 754 value return `Unordered`. Other built-in values return `Equal` when they are equal under the equality rules below and `Unordered` otherwise.

`==` is true only for `Ordering::Equal`; `!=` is true for every other variant, including `Ordering::Unordered`. `<`, `<=`, `>`, and `>=` use `Less`, `Equal`, and `Greater` in the usual way. Applying an ordering operator to `Ordering::Unordered` produces `TypeError`.

## Equality

- None equals only None.
- Bool, Int, and Float compare numerically. Bool converts to Int; if either operand is Float, the other numeric operand converts to Float. Float equality uses IEEE 754 equality.
- Strings compare by scalar sequence.
- Lists, Objects, Functions, Closures, Cells, Errors, and HostResources compare by identity.

## Logical operators

`&&` and `||` require Bool operands and short-circuit.

## Type test

```text
value is Error
value is Int
```

The right side MUST name a built-in runtime type. A type test returns Bool and does not invoke user code.

## Clone

```text
let copy = value.clone();
```

`clone()` is a runtime-owned built-in method available on every value. It is synchronous, cannot be overridden by a type or trait implementation, and has the complete graph semantics defined in the Runtime chapter. User-defined nominal Objects and enums receive it automatically; their stored fields and variant payloads are cloned without requiring an `impl Clone` declaration.

## Error propagation

```text
let value = operation()?;
```

The operand is evaluated once. If it is an Error, the current function immediately returns that same Error value. Otherwise the expression evaluates to the operand value.

At module top level, `?` is a compile error.

# 06 – Statements

## Expression statement

An expression followed by `;` is evaluated and its result is discarded.

## Block

```text
{
    statement;
}
```

Statements execute sequentially. A block ends normally, returns, breaks, continues, suspends through a hostcall, or is terminated by the runner.

## Conditional

```text
if condition {
    ...
} else {
    ...
}
```

The condition MUST evaluate to Bool. A non-Bool condition completes the current function with `TypeError`.

Only the selected branch executes. `else if` is parsed as an `else` containing another `if`.

## While loop

```text
while condition {
    ...
}
```

The condition is evaluated before every iteration and MUST be Bool.

## For loop

```text
for item in iterable {
    ...
}
```

The iterable expression is evaluated once.

For a List, the runtime creates a shallow snapshot of its elements before the first iteration. Mutating the original List does not change the iteration sequence.

For a String, iteration yields one-scalar Strings.

Any other iterable value completes the current function with `TypeError`.

A loop variable is a fresh binding for each iteration. Closures created in different iterations capture different cells.

## Break and continue

`break;` exits the nearest loop. `continue;` advances the nearest loop. Using either outside a loop is a compile error.

### Phase 4 implementation

The current compiler lowers `while`, `for`, `break`, and `continue` directly to structured WebAssembly control flow. A compiled `for` evaluates its iterable once and calls generic runtime operations to create a shallow List snapshot or a List of Unicode-scalar Strings; the compiler never accesses List or String payload layouts. The iterator snapshot, index, and current binding remain in compiler root-frame slots for the loop lifetime.

Conditions are validated through the value-returning `__exs_rt_condition_value` runtime operation. It returns either the original Bool or a recoverable `TypeError`; the compiler returns that Error from the current function before converting a validated Bool into a Wasm branch condition. `for` iteration applies the same Error-return path to a non-List/non-String iterable.

## Return

```text
ret;
ret value;
```

`ret;` returns `None`. `ret value;` evaluates and returns the value.

Returning from a task body completes only that task, not the parent function containing the `par` construct.

## Parallel statement/expression

`par` is defined in its dedicated chapter. It is the only source-language construct that creates tasks.

# 07 – Functions and Closures

## Named function declaration

```text
fn add(a, b) {
    ret a + b;
}
```

A named function declaration creates an immutable binding in its lexical scope. The binding is visible throughout that scope, including earlier textual positions, enabling direct recursion.

## Function expression

```text
let add = fn(a, b) {
    ret a + b;
};
```

## Parameters

Parameters are positional and dynamically typed. Duplicate parameter names are compile errors.

ExS 0.1 does not define default, variadic, named, or keyword arguments.

### Optional function contracts

Non-entry functions MAY annotate each parameter and their return value with a union of current runtime type names:

```text
fn some(input: Int, offset: Float) -> String | Int | Bool | Error {
    ret input + offset;
}
```

The current built-in names are `Any`, `None`, `Error`, `Bool`, `Int`, `Float`, `String`, `List`, and `Object`. Every nominal Object type and enum declared by the same module is also valid in a union annotation. An omitted annotation is exactly `Any`. An annotation is checked dynamically at function entry for every parameter and at each explicit or implicit return. The compiler does not statically prove call argument types.

On a contract mismatch, the runtime returns a recoverable `Error { kind: "TypeError" }` when the function return annotation includes `Error` or is omitted (`Any`). If the return annotation excludes `Error`, compiler-generated contract lowering returns a fatal `TypeError` from the current function. This preserves source position and trace information while terminating the program through the normal Error-reporting path. A valid Error value satisfies an `Error` union member and is returned unchanged.

`main` uses the same optional parameter and return contracts as every other function. The runner supplies its ordered arguments through the entry ABI; missing arguments become `None` and excess arguments produce a fatal `ArityError`.

## Arity

A function has one exact arity. Calling it with a different argument count produces `ArityError`.

The entry point is the exception at the runner boundary: missing `main` inputs are supplied as `None`, while excess inputs produce a fatal `ArityError` before `main` runs.

## Return value

Falling off the end of a function returns `None`.

## Closures

A nested function captures every referenced binding from an enclosing local scope. Captured mutable state is shared through Cells.

The compiler MAY avoid capturing unused bindings and MAY represent immutable, non-mutated captures more efficiently, provided behavior is unchanged.

## Suspendability

A function is suspendable when it may directly or transitively invoke a suspending hostcall.

Suspendability is inferred by the compiler over the complete module graph. It is not part of source syntax.

A call from non-suspendable generated code to suspendable generated code is forbidden by the internal compiler IR and MUST be rejected or transformed before WebAssembly emission.

## Recursion

Recursion is allowed. Stack depth is constrained by runner and WebAssembly implementation limits. A stack-limit violation is a runner failure, not a recoverable Error.

# 08 – Errors

## Error model

Recoverable failures are Error values. ExS does not throw exceptions and does not unwind the stack. Fatal language failures are also Error values with fatal severity when the runtime can safely construct and serialize them; compiler-generated strict type-contract failures use this path. Heap corruption, invalid ABI state, malformed generated code, and other technical invariant failures remain Wasm traps and runner errors.

An operation may return either its normal value or an Error. Callers inspect it directly or propagate it using `?`.

## Error properties

Every Error contains:

| Property  | Mutability | Meaning                      |
| --------- | ---------- | ---------------------------- |
| `kind`    | Mutable    | Stable String error category |
| `message` | Mutable    | Human-readable String        |
| `data`    | Mutable    | Arbitrary Value              |
| `cause`   | Mutable    | Error or `None`              |
| `origin`  | Read-only  | Creation-site metadata       |
| `trace`   | Read-only  | Captured logical call trace  |

Reading every property is permitted.

Assigning `origin` or `trace` produces `ReadOnlyPropertyError`.

## Construction

The built-in function is:

```text
Error(kind, message, data)
```

`kind` and `message` MUST be Strings. `data` is any language value. The current implementation creates a recoverable Error with the active source position and direct-call trace; explicit source-level cause construction is deferred. Invalid arguments return `TypeError`. `error.kind()`, `error.message()`, `error.data()`, and `error.cause()` return these source-visible fields; `cause()` returns None when no related value is available.

## Standard kinds

Implementations MUST use these exact kinds where applicable:

```text
ArityError
CloneError
DivisionByZeroError
HostError
IndexError
IntOverflowError
InvalidStateError
ReadOnlyPropertyError
SerializationError
TypeError
UnknownExportError
```

Additional host-specific kinds MAY be used.

## Origin and trace

Every source-bearing AST and HIR node stores:

```rust
pub struct SourceSpan {
    pub source_id: SourceId,
    pub start_byte: u32,
    pub end_byte: u32,
}
```

Line and column are derived from byte offsets. The compiler assigns a compact non-zero `SourcePositionId` to every source span used by generated runtime operations and direct calls. `origin` identifies the active operation; direct runtime frames retain the function ID and call-site position.

The final Wasm module always contains `exs.source.map`. A single-source module uses the version-two payload `EXSMAP2\0`: source-ID byte length (`u32` little-endian), position-entry count (`u32` little-endian), function-entry count (`u32` little-endian), UTF-8 source ID, then one `(start_byte, end_byte)` pair of `u32` little-endian values for each position ID in ascending order starting at 1. It ends with one `(function_id, name_byte_length, UTF-8 function name)` record for each generated function.

A resolved module graph uses version three, `EXSMAP3\0`: source count (`u32` little-endian), position-entry count, function-entry count, then a source table of `(source-ID byte length, UTF-8 source ID)` records in ascending canonical-module identity order. Each position record is `(source index, start_byte, end_byte)`, with all fields encoded as `u32` little-endian. Function records have the same encoding as version two. Trace frames use this table to render source-level function names and source positions.

`CompileOptions::embed_sources` additionally emits `exs.sources`. A single-source module uses `EXSSRC1\0`, encoded as source-ID byte length, source byte length, UTF-8 source ID, and UTF-8 source text. A resolved module graph uses `EXSSRC2\0`: source count followed by `(source-ID byte length, source byte length, UTF-8 source ID, UTF-8 source text)` records in the same source-table order. Production builds can omit this second section while retaining the position and function maps.

`trace` is a List of immutable frame Objects ordered from creation frame toward the root call.

Error propagation through `?` MUST NOT duplicate frames. Implementations MAY lazily materialize traces.

## Cause cycles

`cause` may create cycles. Error rendering MUST detect cycles and MUST NOT recurse indefinitely.

# 09 – Concurrency with `par`

## Syntax

```text
let results = par {
    taskExpression1,
    taskExpression2,
    taskExpression3,
};
```

A `par` block contains comma-separated expressions. The expressions are task bodies; they are not evaluated before task creation. An empty `par` block is allowed and returns an empty List. The dynamic form is `par(callables)`, where `callables` evaluates to a List of zero-argument callables.

## Task creation

Each expression becomes one language task. Tasks are created in source order.

`par` is the only construct that creates language tasks. Calling a host operation does not itself create a language task.

## Shared heap

All tasks in one root execution share the same heap. Entering `par` does not clone arguments, captures, Lists, Objects, Errors, Cells, or other values.

Mutations made by one task are visible to other tasks according to scheduler order.

## Completion

The parent task suspends until every child task completes.

The `par` expression returns a List containing each task result in source order, independent of completion order.

A task that reaches the end of its expression returns that expression's value. An Error is an ordinary task result and does not automatically cancel sibling tasks.

Runner termination cancels the parent and all descendants.

## Nested parallelism

A task may execute another `par`. Parent-child relationships form a task tree. A parent task cannot complete while a `par` expression it entered still has incomplete direct children.

## Scheduling

Scheduling is cooperative and deterministic except where host completion delivery introduces nondeterminism.

The compiler MUST insert scheduler checkpoints at:

- function entry;
- loop backedges; and
- immediately after a task resumes from a hostcall.

The runtime uses round-robin order by ascending TaskId. Each runnable task receives 1024 checkpoints per quantum. The quantum is part of language-runtime ABI version 0.1 and is not runner fuel.

A task that performs no checkpoint before runner fuel is exhausted terminates the root execution through the runner.

## Hostcalls

Multiple tasks may have outstanding hostcalls simultaneously. Each hostcall has a unique monotonically increasing `HostCallId` within the root execution.

A suspended task becomes runnable when its completion is delivered. Completion events delivered in the same runner poll cycle are enqueued by ascending HostCallId.

# 10 – Modules

## Root node

Every source unit has exactly one AST root:

```rust
pub struct Module {
    pub source_id: SourceId,
    pub imports: Vec<ImportDeclaration>,
    pub uses: Vec<UseDeclaration>,
    pub items: Vec<Item>,
    pub span: SourceSpan,
}

pub enum Item {
    Function(FunctionDeclaration),
    Trait(TraitDeclaration),
    Impl(ImplDeclaration),
    Type(TypeDeclaration),
    Global(GlobalDeclaration),
}
```

Top-level executable statements are not permitted. A future top-level-code feature MUST lower that code into a generated initialization function before HIR lowering.

## Entry point

The root module required by a runner MUST declare `fn main(...)` with zero or more parameters. Imported modules MUST NOT declare `main`. The runner supplies an ordered CBOR array of `ExsValue` arguments; the runtime decodes each argument to an `RtValue` before calling `main`, supplies `None` for missing positions, and returns a fatal `ArityError` for excess positions. `main` returns one ExS value with `ret`; the runner exposes None, Error, Bool, Int, Float, String, and nested acyclic List and Object results as `ExsValue` without exposing `ValueRef`. The `exs run` CLI accepts positional values after `--` and prints the result in ExS source notation.

```text
fn main(left: Int, right: Int) -> Int {
    ret left + right;
}
```

The module root and one `main` entry remain mandatory; top-level statements are not allowed.

## Imports and namespaces

An import declaration loads one relative `.exs` source file into a compile-time namespace:

```text
import "./math.exs";
import "./models/user.exs" as account;
```

The default namespace is the imported file's stem. A file stem that is not a valid ExS identifier MUST be imported with `as`. `as` replaces the default namespace for that import.

More than one import MAY use the same namespace. Their directly declared functions, nominal types, and traits form one merged namespace. Every exported declaration name in a merged namespace MUST be unique, including names from different declaration categories. A collision is a compile error with related spans for both declarations. Implementation blocks are associated with their owning nominal type and are not independently named namespace members.

An implementation MUST resolve relative paths against the importing file, canonicalize every resolved identity before graph construction, load each canonical file at most once, and reject every import cycle. The diagnostic for a cycle MUST identify the complete import chain. Bare package names, URLs, implicit search paths, wildcard imports, and automatic namespace re-exports are not part of this language version.

Imports and `use` declarations form a module prelude and MUST precede every type, trait, implementation, and function declaration. A module's own imports are internal: importing that module does not expose its imported namespaces to another module.

Declarations remain qualified by their imported namespace unless shortened by `use`:

```text
import "./geometry.exs" as geo;

fn render(value: geo::Shape) -> geo::Point {
    ret geo::Point::new(0, 0);
}
```

`namespace::function(...)` invokes an imported direct function. `namespace::Type { ... }`, `namespace::Type::method(...)`, and `namespace::Trait` respectively name an imported nominal type construction, static method, and trait. Qualified type and trait names are valid in every type annotation position.

## `use` declarations

`use` does not load a source file. It introduces one or more local aliases for declarations already exposed by an imported namespace:

```text
import "./geometry.exs" as geo;
use geo::{Point, Shape};
use geo::display as render;
```

The single-element and grouped forms are equivalent. A used function becomes an unqualified direct call; a used nominal type or trait becomes valid as an unqualified construction, static-method receiver, or type annotation. `use` aliases are compile-time only and do not create runtime namespace values. A used name MUST NOT collide with a local declaration, another used alias, or a built-in top-level name.

## Formatting

The reference formatter is exposed by the compiler library and as `exs fmt <file.exs>`. It accepts a lexically and syntactically valid source unit, including an imported module without `main`, and rewrites the file in place through the CLI. Invalid input is not modified; formatter diagnostics use the standard source-excerpt rendering.

Canonical output uses four spaces per block level, one statement per line, a final newline, one blank line between top-level declarations, and one blank line between a module prelude and the first declaration. It normalizes spaces around binary operators, assignments, type annotations, function return arrows, commas, and object-property colons. It uses normal escaped string literals for all strings, so raw and dedented spelling is not retained when formatting.

## Generated Markdown API documentation

The compiler library exposes resolver-driven Markdown generation for a root source and its complete relative-import graph. It returns a project `index.md` with the general language description and links to a synthetic `std` module plus every reachable source module. Built-in types, standard traits, and standard enums remain globally available and may also use the optional `std::` qualifier; `std` is compiler-provided and cannot be imported. The module contains individual type pages for the out-of-the-box types; the `Error(kind, message, data)` constructor is documented on the `Error` type page. Its function page documents `host.call`.

Every source module has an index page that lists imports, `use` declarations, types, traits, and functions as links. Each source-defined type, trait, and direct function receives its own API page. Type pages include their fields and implemented methods; implementations are not repeated on their module index. Trait pages include method signatures; function pages include their signature. Source comments beginning with `///` immediately before a declaration are emitted as that declaration's Markdown description.

The `exs docs <file.exs> -o <directory>` CLI command uses the filesystem resolver and writes `index.md` plus the generated `modules/` tree to the selected directory. Function pages are stored in each module's `fn/` directory. The compiler library performs no filesystem writes.

# 11 – Built-ins

The following built-ins are always available and cannot be shadowed at module top level. A nested local binding MAY shadow a built-in.

## `Error(kind, message, data)`

Constructs a recoverable Error as defined in the Errors chapter. `Error` is a reserved keyword and cannot be declared as a source function.

## List operations

The runtime-recognized List methods are:

```text
list.push(value)        // mutates and returns new length
list.pop()              // removes last value; returns None when empty
list.insert(index, v)   // mutates; returns None or IndexError
list.remove(index)      // mutates; returns removed value or IndexError
list.clear()            // mutates; returns None
```

## Object operations

```text
object.has(key)         // Bool
object.delete(key)      // removed value or None
object.keys()           // new List of String keys in insertion order
object.values()         // new shallow List in insertion order
```

## Traits

Traits and nominal types share one source-visible namespace. A module MUST reject a trait whose name is already used by a nominal type, and vice versa. Built-in type names are reserved from trait declarations.

```text
trait Label {
    fn label(self) -> String;
    fn category() -> String { ret "person"; }
}

impl Label for User {
    fn label(self) -> String { ret self.name; }
}
```

A trait method ends either with `;`, making it required for every implementation, or with a block, making that block its inherited default implementation. An `impl Trait for Type` MAY omit a defaulted method and MUST implement every required method. Implementations MAY define both instance methods (first parameter `self`) and static methods. Instance methods are called with `value.method(...)`; static methods are called with `Type::method(...)`.

Every method supplied by a trait implementation MUST be declared by that trait and have the same receiver shape, arity, parameter type annotations, and return type annotation. Parameter names other than `self` do not form part of the method signature. A nominal type MUST NOT expose a method name more than once across inherent and trait implementations, including inherited defaults.

`Self` is a contextual type name available only in a trait method signature and in a method inside `impl Trait for Type`. It resolves to the implementation target, so `fn merge(self, other: Self) -> Self` inside `impl Merge for Document` has the same signature as `fn merge(self, other: Document) -> Document`. An implementation may use either spelling; other type annotation contexts reject `Self`.

A trait name is valid in every existing type annotation position. For the current nominal-trait implementation, a trait contract matches an Object whose nominal type has an `impl Trait for Type` declaration. A declared trait with no implementations is valid and matches no current value. `Add`, `Sub`, `Mul`, `Div`, `Compare`, and their `std::`-qualified spellings are compiler-owned standard traits and are reserved from source trait declarations. `Add` matches built-in Bool, Int, Float, String, and List values; `Sub`, `Mul`, and `Div` each match built-in Bool, Int, and Float values; `Compare` matches all built-in values. A nominal type or enum may implement the arithmetic traits only with their fixed `fn add|sub|mul|div(self, other: Any) -> Any` method signature, or `Compare` only with `fn compare(self, other: Any) -> Ordering`; parameter names other than `self` do not affect these signatures. Each matching operator dispatches exclusively through its nominal implementation before using the built-in fallback. Implementing traits for primitive values and defining further standard language traits are deferred.

Trait methods are potential suspension points unless the compiler proves their implementation non-suspendable. Trait dispatch is therefore represented explicitly in HIR and resolved through stable runtime ABI operations.

## Host invocation

```text
host.call(name, arguments...)
```

`host` is a reserved source keyword. `name` is evaluated at runtime and MUST produce a String naming a runner-registered host function. The remaining arguments are evaluated in source order and transported as one canonical CBOR List. The compiler has no host manifest and does not know whether the selected runner function is synchronous or asynchronous.

`host` is suspendable. It returns the host result or an Error. It does not create a language task.

# 12 – Compiler

## Required phases

A conforming compiler MUST perform, explicitly or equivalently:

1.  source loading, UTF-8 validation, and lexical analysis;
2.  parsing into a `Module` AST;
3.  scope construction, binding IDs, and name resolution;
4.  semantic validation and trait declaration collection;
5.  AST-to-HIR lowering;
6.  closure discovery, capture analysis, and Cell promotion;
7.  callgraph construction and suspendability analysis;
8.  error-propagation, clone, and `par` lowering;
9.  resumable state-machine lowering and root-frame construction where required;
10. low-level IR construction and WebAssembly code generation;
11. runtime ABI resolution by stable export name and linking with the embedded `crates/exs-runtime/exs-runtime.wasm` template;
12. custom-section generation; and
13. final WebAssembly validation.

The compiler library is independent of browsers, servers, concrete host functions, host schemas, and filesystem access. It depends on the `exs-runtime` crate, which embeds the committed `exs-runtime.wasm` template. Its public compilation API accepts root source input, compile options, and a module resolver that returns source text and canonical source identities for relative imports; it then produces final Wasm bytes plus module metadata. A CLI resolver MAY read local files. A server, browser, or IDE resolver supplies source text through its own storage boundary.

## Diagnostics

Compile errors MUST include:

- a stable diagnostic code;
- a stable category (`Lexical`, `Syntax`, `Semantic`, or `Internal`);
- module identity;
- source span;
- concise message; and
- related spans when relevant.

The compiler API MUST return the complete ordered `CompileDiagnostics` collection. Each diagnostic is structured data containing its code, category, primary span, message, and zero or more related spans; source text itself is not retained in that data. Consumers such as IDEs can therefore map byte spans directly to their own source buffers. A source-aware terminal renderer MAY add one-based line and column numbers plus source excerpts.

The lexer, parser, declaration validation, and independent function-body validation SHOULD recover at safe boundaries and report later independent errors in the same compilation. Recovery MUST NOT produce an executable module.

A compiler MUST NOT emit an executable module after an error unless explicitly operating in a non-conforming recovery mode.

## Suspendability analysis

Suspendability is a transitive fixed-point analysis.

A function is suspendable if it:

- contains a hostcall;
- calls a suspendable function;
- calls a dynamically selected function that cannot be proven non-suspendable; or
- contains `par`, a potentially suspendable trait call, or a potentially suspendable user-defined clone.

Dynamic calls are conservatively suspendable unless the compiler proves the complete target set non-suspendable.

## WebAssembly target

Version 0.1 targets WebAssembly 1.0 core instructions plus mutable globals and multi-value returns only when supported by the selected ABI profile.

The default portable profile MUST NOT require threads, shared linear memory, exceptions, tail calls, GC types, or stack switching.

Suspendable functions MUST therefore be lowered to resumable state machines.

## Validation metadata

The final module MUST contain an `exs.meta` custom section containing canonical CBOR with:

- language version;
- compiler/runtime ABI version;
- Host ABI version;
- required runtime features;
- exported source functions and arities;
- source module table; and
- optional debug mapping.

The runner MUST validate this section before invocation.

The compiler MAY also emit `exs.source.map` and `exs.sources` custom sections. `exs.source.map` maps compact source-position IDs to source spans. `exs.sources` contains source text only in builds that opt in to embedding it.

# 13 – Runtime

## Responsibilities

The runtime owns:

- Value representation;
- heap allocation;
- built-in operations;
- closures and Cells;
- Errors;
- deep clone;
- scheduler and task state;
- hostcall state;
- garbage collection; and
- ABI exports used by the runner.

`exs-runtime` is compiled as the committed `exs-runtime.wasm` template. It exports its supported stable runtime ABI names. The compiler resolves these names from the Wasm export section and links this template into every final module.

All allocated runtime values use this root enum. Complex payloads are boxed:

```rust
pub enum RtValue {
    None,
    Error(Box<RuntimeError>),
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Box<RuntimeString>),
    List(Box<RuntimeList>),
    Object(Box<RuntimeObject>),
    Closure(Box<RuntimeClosure>),
    Cell(Box<RuntimeCell>),
    Error(Box<RuntimeError>),
    Task(Box<RuntimeTask>),
    Stream(Box<RuntimeStream>),
    HostResource(Box<RuntimeHostResource>),
}
```

`Vec` is solely an implementation detail, for example for a runtime List's element storage. ExS source has Lists, not a `Vec` type.

## Deep clone

`value.clone()` is synchronous and runtime-owned. It does not perform trait dispatch and cannot be overridden by user code. This deliberate automatic behavior also applies to every user-defined nominal Object and enum, avoiding a clone protocol that could break alias or cycle preservation.

Clone returns a deep copy of the reachable language-value graph while preserving topology:

- repeated references in the input become repeated references to one cloned node;
- cycles remain cycles;
- immutable scalar values may be reused;
- Functions may be reused;
- Closures are cloned as closure objects and their captured Cell graph is cloned;
- Errors are deeply cloned, including `data` and `cause`;
- `origin` and `trace` may be reused because they are immutable.

A reachable HostResource makes clone return `CloneError` with no observable partial clone. The operation never mutates the source graph.

The runtime MUST use a source-identity-to-clone map.

## Mutation

List, Object, Cell, and mutable Error-property writes are immediately visible to all tasks sharing the heap.

Each individual built-in mutation is atomic with respect to scheduler checkpoints. No task switch occurs in the middle of one runtime mutation primitive.

## Runtime faults

Internal invariant failures MUST NOT be converted into arbitrary language Errors. They terminate the root execution as runtime failures reported by the runner.

## Serialization

Ordinary CBOR serialization supports None, Error, Bool, Int, Float, String, List, and Object.

Cycles and aliases are represented with the reference tags defined by the Host ABI.

Function, Closure, and Cell values are not serializable and produce `SerializationError`.

HostResources use capability-reference encoding only within a live hostcall and cannot be serialized for storage or generic export.

# 14 – Scheduler

## Execution context

Runtime mutable state is scoped to an execution context, not process-global state. It contains the current task, root stack, scheduler, active clone contexts, and error state. This permits reentrancy, isolated Wasm instances, and nested host callbacks without exposing internal heap identity.

## Task states

A task is in exactly one state:

```text
Created
Runnable
Running
WaitingHost
WaitingChildren
Completed
Cancelled
```

State transitions MUST follow the scheduler state machine. Invalid transitions are runtime invariant failures.

## Identifiers

TaskId and HostCallId are unsigned 64-bit counters scoped to one root execution. Zero is reserved. Allocation begins at one and increases monotonically. Exhaustion terminates the root execution.

## Runnable queue

The runnable queue is ordered by TaskId. Round-robin rotation occurs after quantum exhaustion, voluntary suspension, or completion.

Only one language task executes WebAssembly instructions at a time in the default portable profile.

## Root task

Every invocation creates exactly one root task. Completion of the root task completes the root execution only after no required child task remains.

## Cancellation

Runner cancellation marks all non-completed tasks Cancelled, invalidates outstanding hostcalls, and prevents further user-code execution.

A late host completion for an invalidated HostCallId MUST be ignored by the runner.

## Deadlock

If no task is runnable, at least one task is incomplete, and no valid hostcall is outstanding, the runtime reports an internal deadlock failure to the runner.

## Reentrancy

While a root execution is active, the runner MUST NOT call the invocation export again on the same instance.

Host completion delivery through the resume export is permitted and is not a second root execution.

# 15 – Garbage Collection

## Required behavior

The runtime MUST reclaim unreachable heap objects or otherwise enforce a runner memory limit that prevents unbounded allocation. A tracing garbage collector is the reference strategy.

## Roots

GC roots include:

- module bindings;
- all runnable and suspended task frames;
- closure environments;
- runtime scheduler structures;
- outstanding hostcall argument and continuation state;
- the current result value;
- temporary values explicitly registered by runtime primitives; and
- runner-pinned ABI buffers.

## Reachability

The collector MUST trace Lists, Objects, Closures, Cells, Errors, and runtime wrapper objects. It MUST preserve cycles and aliases.

HostResources reachable from language values remain live. When an unreachable HostResource wrapper is finalized, the runtime MUST notify the host exactly once if the Host ABI declares the resource finalizable.

## Finalization

ExS source code has no finalizers. HostResource release notification MUST NOT execute ExS code.

Finalization order is unspecified. Programs MUST NOT rely on prompt collection.

## Moving collectors

A runtime MAY move objects if every Value reference, runtime root, and host pin is updated safely. Raw heap addresses MUST NOT cross the Host ABI.

## Collection scheduling

GC MAY run at allocation safe points and scheduler checkpoints. It MUST NOT change observable language behavior except timing, runner fuel consumption, and HostResource release timing.

### Phase 4 implementation

The current runtime uses stop-the-world mark-and-sweep collection before each language-value allocation. `ValueRef` remains a 32-bit runtime-local slot reference; swept slots are reused only after compiler-generated root frames and runtime temporary roots prove the previous value unreachable. The collector marks List elements and Object property values, preserving aliases and cycles. Loop iteration snapshots, indexes, and bindings are compiler roots, and scalar String snapshot construction uses temporary runtime roots across its allocations. Future heap variants MUST add their owned `ValueRef` fields to this traversal.

# 16 – Host ABI

## Version

This chapter defines Host ABI `0.1`.

## Hostcalls

Source-level `host.call(name, arguments...)` creates a runtime hostcall record and invokes the runner import. Concrete host operations are unknown to the compiler; the runtime String `name` is resolved by the runner at execution time. The request payload is a canonical CBOR List whose items are the ordered host-function arguments.

The runtime allocates HostCallId before invoking the host.

## Required import

The WebAssembly module imports:

```text
module: "exs"
name:   "__host_call_start"

func __host_call_start(
    call_id: i64,
    name_ptr: i32,
    name_len: i32,
    request_ptr: i32,
    request_len: i32,
    source_position_id: i32
) -> i32
```

The operation name is UTF-8. The request is canonical CBOR.

Return codes:

```text
0  ready; the runtime continues using the returned response path
1  pending; completion will be delivered later
2  fatal runner failure
```

Unknown functions, invalid host arguments, and ordinary host failures are recoverable language Error values encoded as CBOR responses, not technical status codes.

The import MUST NOT reenter arbitrary ExS exports.

## Completion delivery

The runner delivers completion through the runtime export:

```text
__exs_resume_host(
    call_id: i64,
    completion_kind: i32,
    payload_ptr: i32,
    payload_len: i32
) -> i32
```

Completion kinds:

```text
0 success value
1 recoverable host Error
2 HostResource capability
```

The payload for kinds 0 and 1 is canonical CBOR. Kind 2 uses a CBOR map containing resource type and runner-owned unsigned handle. A synchronous host function uses the ready fast path and does not suspend; an asynchronous function returns pending, after which the runner resumes the task through `__exs_resume_host`.

## Canonical CBOR mapping

| ExS                        | CBOR               |
| --------------------------- | ------------------ |
| None                        | null               |
| Bool                        | boolean            |
| Int                         | integer            |
| Float                       | binary64           |
| String                      | text string        |
| List                        | array              |
| Object                      | map with text keys |
| Error                       | tag 60001 plus map |
| Enum                        | tag 60005 plus `[type_identity, variant, fields]` |
| graph definition            | tag 60002          |
| graph reference             | tag 60003          |
| HostResource live reference | tag 60004          |

Tags 60002 and 60003 preserve cycles and aliases. The encoder assigns monotonically increasing reference numbers in depth-first traversal order.

Tag 60004 is legal only in a live hostcall payload and contains the runner capability handle. It MUST be rejected by generic persistence serialization.

## Error encoding

An Error encodes the keys `kind`, `message`, `data`, `cause`, `origin`, and `trace`.

## Capability safety

Capability handles are scoped to one runner instance generation. A stale, foreign, or released handle produces `HostError`.

The host MUST validate operation authorization independently of source-level values.

# 17 – Runner

## Responsibilities

The runner:

- validates module metadata and WebAssembly imports/exports;
- creates and pools instances;
- provides host imports;
- starts exactly one root execution per checked-out instance;
- delivers host completions;
- enforces limits;
- collects the final result; and
- resets or discards instances safely.

## Host registry

The server runner owns a registry mapping dynamically resolved host names to independently registered synchronous or asynchronous implementations. A synchronous implementation has the logical signature `fn(Vec<ExsValue>) -> ExsValue`; an asynchronous implementation returns a `Future<Output = ExsValue>` for the same ordered arguments. The registry rejects empty or duplicate names. It validates that request CBOR decodes to an ExS List and that responses encode as ExS CBOR. A synchronous implementation returns a response through the hostcall fast path. An asynchronous implementation creates a HostCallId, returns pending, and resumes the waiting runtime task when its future completes.

The runner's public execution API is asynchronous because a loaded module may use asynchronous host functions. A purely synchronous execution path MAY complete without suspension. A normal recoverable host failure is returned as an ExS Error value; malformed Wasm, an ABI mismatch, engine failure, fuel exhaustion, and runner-internal failures are runner errors.

The browser runner follows the same logical ABI with native `WebAssembly.instantiate`, synchronous registration, and Promise-backed asynchronous registration.

## Instance pool

An InstancePool owns zero or more initialized WebAssembly instances.

An instance may be:

```text
Idle
Running
Poisoned
Disposed
```

Only Idle instances may be checked out. A Running instance cannot serve another root execution.

After normal completion, the runner MAY reset the instance to its post-initialization snapshot and return it to Idle. If complete reset cannot be proven, the instance MUST be disposed.

A runtime trap, ABI violation, failed cancellation, or internal runtime failure poisons the instance.

## Limits

The runner MUST support configurable limits for:

- WebAssembly memory;
- total allocations when measurable;
- fuel;
- wall-clock timeout;
- task count;
- outstanding hostcall count;
- call depth or stack;
- clone work/size;
- CBOR payload size;
- result size; and
- host capability count.

Exceeding a hard limit terminates the entire root execution and returns RunnerError outside the ExS value domain.

Fuel exhaustion never yields and is never resumable.

## Timeout and cancellation

On timeout or external cancellation, the runner cancels outstanding host operations where supported, invalidates their call IDs, invokes runtime cancellation, and stops executing user code.

## Result categories

A root execution ends with exactly one runner-level outcome:

```text
Success(Value)
LanguageError(Error)
RunnerError(code, message, details)
```

If the returned ExS value is Error, the runner reports `LanguageError`. Other values are `Success`.

Traps, limits, ABI violations, and runtime invariant failures are RunnerError.

## Host privacy boundary

The runner MUST NOT expose raw WebAssembly memory addresses or runtime heap identities to host application code.

# 18 – Compiler ↔ Runtime ABI

## Version

This chapter defines compiler/runtime ABI `10`.

## Phase-1 required exports

The Phase-1 linked module MUST export:

```text
__exs_abi_version() -> i32
__exs_input_alloc(length: i32) -> i32
__exs_start(input_ptr: i32, input_len: i32) -> i32
__exs_result_ptr() -> i32
__exs_result_len() -> i32
```

The runtime owns both input and result buffers in linear memory. The runner calls `__exs_input_alloc`, copies one CBOR array of ordered `ExsValue` arguments into the returned buffer, and passes that pointer-length pair to `__exs_start`. The completed result is one `ExsValue` CBOR item in the byte range returned by `__exs_result_ptr` and `__exs_result_len`. Later hostcall phases extend this export set with initialization, resume, and cancellation operations.

## ABI version value

`__exs_abi_version()` returns:

```text
(major << 16) | minor
```

For this specification the value is `0x0000000A`.

## Run status

Phase-1 `__exs_start(input_ptr, input_len)` returns:

```text
2 COMPLETE
```

`COMPLETE` means the result buffer exports are valid.

## Invocation

The Phase-1 entry point is `fn main(...)`. Before execution, the runner serializes its ordered `ExsValue` arguments as one CBOR array, calls `__exs_input_alloc`, copies the resulting CBOR bytes to the returned linear-memory range, and invokes `__exs_start(input_ptr, input_len)`. The runtime validates that pair against its owned input buffer, decodes the argument array, converts each item to `RtValue`, substitutes `None` for missing declared parameters, and passes the resulting values to `main`. More supplied values than declared parameters produce a fatal `ArityError` before `main` starts.

## Phase-1 result buffer

After COMPLETE, the runner reads the CBOR byte range given by `__exs_result_ptr` and `__exs_result_len`. The implemented subset supports exactly one None, Error, Bool, Int, Float, String, tagged enum, or recursively nested acyclic List or Object CBOR item. The internal `ValueRef` never crosses this boundary.

## Runtime intrinsics

Compiler-generated code MAY call linked runtime intrinsics whose names begin with `__exs_rt_`, such as `__exs_rt_list_new`, `__exs_rt_object_new`, `__exs_rt_error_new`, `__exs_rt_append`, `__exs_rt_index_get`, `__exs_rt_index_set`, `__exs_rt_call_method`, `__exs_rt_cell_new`, `__exs_rt_value_is_error`, `__exs_rt_task_create`, and `__exs_rt_cbor_encode`. Except for construction intrinsics such as `__exs_rt_list_new`, `__exs_rt_object_new`, and `__exs_rt_error_new`, operations dispatch from the runtime value rather than a compiler-proven receiver type. The compiler resolves intrinsic names from the `crates/exs-runtime/exs-runtime.wasm` export section at link time.

The compiler resolves runtime functions by these export names, never fixed Wasm indices. Source programs cannot import, export, or reference intrinsic names.

# 19 – Grammar Summary

This grammar is normative for syntax but omits lexical Unicode productions already defined.

```ebnf
module          = { moduleDecl } { item } ;
moduleDecl      = importDecl | useDecl ;
importDecl      = "import" string [ "as" identifier ] ";" ;
useDecl         = "use" qualifiedName [ "as" identifier ] ";"
                | "use" identifier "::" "{" useItem { "," useItem } [ "," ] "}" ";" ;
useItem         = identifier [ "as" identifier ] ;
qualifiedName   = identifier { "::" identifier } ;
item            = functionDecl | typeDecl | traitDecl | implDecl ;

functionDecl    = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] block ;
typeDecl        = "type" identifier "{" [ typeField { "," typeField } [ "," ] ] "}" ;
typeField       = identifier [ ":" typeUnion ] ;
traitDecl       = "trait" identifier "{" { traitMethod } "}" ;
traitMethod     = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] ( ";" | block ) ;
implDecl        = "impl" identifier [ "for" identifier ] "{" { functionDecl } "}" ;
functionExpr    = "fn" "(" parameters? ")" block ;
parameters      = parameter { "," parameter } [ "," ] ;
parameter       = identifier [ ":" typeUnion ] ;
typeUnion       = typeName { "|" typeName } ;
typeName        = qualifiedName | "None" | "Error" ;

letDecl         = "let" identifier [ "=" expression ] ";" ;

statement       = block
                | ifStmt
                | whileStmt
                | forStmt
                | breakStmt
                | continueStmt
                | returnStmt
                | expression ";" ;

block           = "{" { declaration | statement } "}" ;
ifStmt          = "if" expression block [ "else" ( block | ifStmt ) ] ;
whileStmt       = "while" expression block ;
forStmt         = "for" identifier "in" expression block ;
breakStmt       = "break" ";" ;
continueStmt    = "continue" ";" ;
returnStmt      = "ret" [ expression ] ";" ;

expression      = assignment ;
assignment      = logicOr [ "=" assignment ] ;
logicOr         = logicAnd { "||" logicAnd } ;
logicAnd        = equality { "&&" equality } ;
equality        = comparison { ( "==" | "!=" ) comparison } ;
comparison      = term { ( "<" | "<=" | ">" | ">=" | "is" ) term } ;
term            = factor { ( "+" | "-" ) factor } ;
factor          = unary { ( "*" | "/" | "%" ) unary } ;
unary           = ( "!" | "-" ) unary | postfix ;
postfix         = primary { call | index | property | "?" } ;
call            = "(" arguments? ")" ;
arguments       = expression { "," expression } [ "," ] ;
index           = "[" expression "]" ;
property        = "." identifier ;

primary         = literal
                | qualifiedName
                | functionExpr
                | listLiteral
                | objectLiteral
                | parExpr
                | "(" expression ")" ;

parExpr         = "par" "{" [ arguments ] "}"
                | "par" "(" expression ")" ;
listLiteral     = "[" [ arguments ] "]" ;
objectLiteral   = "{" [ objectItems ] "}" ;
objectItems     = objectItem { "," objectItem } [ "," ] ;
objectItem      = ( identifier | string ) ":" expression ;
```

Assignment targets MUST be identifiers, property accesses, or index accesses.

Top-level `statement` and `letDecl` occurrences are invalid. A module contains an optional import and `use` prelude followed by `functionDecl`, `typeDecl`, `traitDecl`, and `implDecl` items. The root module requires exactly one `fn main(...)` declaration; imported modules require none.

# 20 – Conformance and Security

## Conformance tests

A conforming distribution SHOULD provide tests for:

- lexical and parser acceptance/rejection;
- integer boundaries and overflow;
- Float edge cases;
- evaluation order;
- closure capture behavior;
- alias and cycle preservation;
- Error propagation;
- scheduler order;
- simultaneous hostcalls;
- CBOR graph encoding;
- runner limit termination;
- instance reset isolation; and
- ABI incompatibility rejection.

## Determinism

Given identical source, inputs, Host ABI completion sequence, runner limits, and ABI versions, observable ExS behavior MUST be identical.

Host operations such as time, random data, network access, and filesystem access are inherently host-defined.

## Security requirements

The runner MUST validate every pointer-length pair before reading WebAssembly memory.

CBOR decoders MUST enforce nesting, byte-size, item-count, and graph-reference limits.

The runtime and runner MUST reject invalid graph references, duplicate graph definitions, malformed UTF-8, unknown mandatory ABI fields, and stale HostResource handles.

No Error message, trace, or diagnostic may be treated as trusted HTML or executable code by default.

## Unspecified behavior

Behavior not defined by this specification is not a portability guarantee. Implementations SHOULD diagnose use of unsupported extensions and MUST NOT silently reinterpret standard syntax.
