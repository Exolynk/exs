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
10. Clone is a built-in, deep operation. A built-in primitive clone is synchronous; a user-defined clone implementation MAY suspend.
11. Clone preserves cycles and aliasing.
12. `HostResource` values are not ordinary CBOR values and are not cloneable by default.
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
break continue else export false fn for from if import in
is let null par ret true while
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
| `Null`         |                     No |             No |
| `Bool`         |                     No |             No |
| `Int`          |                     No |             No |
| `Float`        |                     No |             No |
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
pub struct Value(u64);
```

The `u64` contains a runtime versioned tag and payload. Immediate values include `null`, Bool, Int, Float, and small internal IDs. Heap values contain generational handles. The exact tag assignment and heap layout are private to `exs-runtime` and MUST NOT be observable by ExS programs. The compiler only uses stable runtime ABI operations; it MUST NOT inspect heap-object layouts.

## Null

`null` is the only `Null` value.

## Bool

`true` and `false` are the only `Bool` values. ExS does not define implicit truthiness. Conditions MUST evaluate to `Bool`.

## Int

`Int` represents every integer in the inclusive range:

```text
-2^55 .. 2^55 - 1
```

This is a signed 56-bit range including the sign bit.

An integer literal outside this range is a compile error. Integer arithmetic that exceeds the range produces `IntOverflowError`.

## Float

`Float` uses IEEE 754 binary64 semantics. Implementations MUST preserve all finite binary64 values, infinities, signed zero, and NaN.

Mixed `Int`/`Float` arithmetic converts the `Int` operand to binary64 before applying the operation.

## String

A `String` is an immutable sequence of Unicode scalar values. Indexing is by scalar index, not UTF-8 byte offset.

String indexing returns a one-scalar String. An invalid index produces `IndexError`.

## List

A `List` is a mutable ordered sequence of `Value` references. Lists may contain cycles and repeated aliases.

List indexes are zero-based `Int` values. Negative indexes are invalid.

## Object

An `Object` is a mutable mapping from String keys to `Value` references. Key iteration order is insertion order. Replacing an existing key does not change its position.

## Function and Closure

A `Function` is immutable executable code. A `Closure` combines a function with captured lexical cells.

## Cell

A `Cell` is the mutable storage used for a captured binding. Cells are not directly constructible or inspectable by source programs.

## Error

An `Error` is a recoverable failure value with the properties defined in the Errors chapter.

## HostResource

A `HostResource` is an opaque host-owned capability or handle. Its internal identity and lifetime are host-defined. A program may pass it back to compatible host operations but may not inspect, persist, or serialize it as an ordinary value. It is cloneable only when an applicable user-defined Clone implementation explicitly supports its capability contract.

# 04 – Variables and Scope

## Bindings

A binding is declared with `let`.

```text
let count = 0;
let empty;
```

A declaration without an initializer stores `null`.

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

Top-level module bindings are initialized during module initialization. Imported bindings are read-only aliases to exported bindings. Assignment to an imported binding is a compile error.

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

Dot access is equivalent to indexing with the property name as a String except for compiler-recognized intrinsics such as `clone`.

Reading a missing Object property returns `null`. Writing a property creates or replaces it.

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
| `-`      | `Int`         | `Int` or `IntOverflowError` |
| `-`      | `Float`       | `Float`                     |

## Arithmetic

`+`, `-`, and `*` accept numeric operands. Two `Int` operands produce `Int` or `IntOverflowError`. If either operand is `Float`, the result is `Float`.

`/` accepts numeric operands and always produces `Float`. Division by zero follows IEEE 754 behavior after conversion.

`%` accepts two `Int` operands. Division by zero produces `DivisionByZeroError`. The result has the sign of the left operand.

`+` also concatenates two Strings and concatenates two Lists into a new shallow List. No other implicit conversions occur.

## Comparison

`<`, `<=`, `>`, and `>=` accept two numeric values or two Strings. Numeric mixed comparison converts `Int` to `Float`. String comparison is lexicographic by Unicode scalar value.

## Equality

`==` and `!=` never produce an Error.

- `null` equals only `null`.
- Bools compare by value.
- Ints compare by value.
- Floats compare using IEEE 754 equality.
- An Int and Float compare after Int-to-Float conversion.
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

`clone()` lowers to the `Clone` trait dispatch. Built-in values use the runtime intrinsic path; user-defined types MAY provide a `Clone` implementation. Such an implementation is potentially suspendable, so the compiler includes it in suspendability analysis. Its complete graph semantics are defined in the Runtime chapter.

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

## Return

```text
ret;
ret value;
```

`ret;` returns `null`. `ret value;` evaluates and returns the value.

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

## Arity

A function has one exact arity. Calling it with a different argument count produces `ArityError`.

## Return value

Falling off the end of a function returns `null`.

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

Recoverable failures are Error values. ExS does not throw exceptions and does not unwind the stack.

An operation may return either its normal value or an Error. Callers inspect it directly or propagate it using `?`.

## Error properties

Every Error contains:

| Property  | Mutability | Meaning                      |
| --------- | ---------- | ---------------------------- |
| `kind`    | Mutable    | Stable String error category |
| `message` | Mutable    | Human-readable String        |
| `data`    | Mutable    | Arbitrary Value              |
| `cause`   | Mutable    | Error or `null`              |
| `origin`  | Read-only  | Creation-site metadata       |
| `trace`   | Read-only  | Captured logical call trace  |

Reading every property is permitted.

Assigning `origin` or `trace` produces `ReadOnlyPropertyError`.

## Construction

The built-in function is:

```text
error(kind, message, data = null, cause = null)
```

`kind` and `message` MUST be Strings. `cause` MUST be Error or null. Invalid arguments return `TypeError`.

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

Line and column are derived from byte offsets. The compiler assigns a compact `SourcePositionId` to every potentially failing operation. `origin` identifies that position and the creating function; runtime frames retain the current position and each trace frame retains its function ID and call-site position.

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

## Phase-1 modules and entry point

Phase 1 accepts only `Function` items. The required entry point is a zero-argument `fn main()` function. It returns one ExS `Value` with `ret`; the phase-1 runner exposes that value to its caller. The `exs run` CLI prints supported Phase-1 results in ExS source notation, including integers and booleans. The phase-1 runner has no external input contract.

```text
fn main() {
    let value = 40 + 2;
    ret value;
}
```

The module root, function-only top level, and `main` entry remain mandatory even while the runner input/output ABI is intentionally deferred.

## Future module resolution

Static imports, exports, globals, traits, implementations, and user types are deferred beyond Phase 1. When introduced, imports MUST be resolved to canonical module identities before graph construction, imported bindings MUST be read-only aliases, and cycles MUST be compile errors unless a later language version explicitly defines their semantics.

# 11 – Built-ins

The following built-ins are always available and cannot be shadowed at module top level. A nested local binding MAY shadow a built-in.

## `type(value)`

Returns one of these Strings:

```text
"Null" "Bool" "Int" "Float" "String" "List" "Object"
"Function" "Closure" "Error" "HostResource"
```

Cells are not directly observable and are reported through the value stored in the cell.

## `len(value)`

Returns an Int length for String, List, or Object. Object length is its number of keys. Unsupported values return TypeError.

## `error(kind, message, data, cause)`

Constructs an Error as defined in the Errors chapter.

## List operations

The runtime-recognized List methods are:

```text
list.push(value)        // mutates and returns new length
list.pop()              // removes last value; returns null when empty
list.insert(index, v)   // mutates; returns null or IndexError
list.remove(index)      // mutates; returns removed value or IndexError
list.clear()            // mutates; returns null
```

## Object operations

```text
object.has(key)         // Bool
object.delete(key)      // removed value or null
object.keys()           // new List of String keys in insertion order
object.values()         // new shallow List in insertion order
```

## Traits

The built-in traits are `ToString`, `PropertyKey`, `Equality`, and `Clone`. User-defined types MAY implement traits once trait declarations and implementations are introduced. Equality for primitive values is by value and reference values use identity unless an applicable `Equality` implementation overrides that behavior. `PropertyKey` conversion is distinct from `ToString` conversion.

Trait methods are potential suspension points unless the compiler proves their implementation non-suspendable. Trait dispatch is therefore represented explicitly in HIR and resolved through stable runtime ABI operations.

## Host invocation

```text
host.operation(arguments)
```

`operation` is a static property identifier. `arguments` may be any Host-ABI-marshallable value. Dynamic `host.call(name, arguments)` is deferred.

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

The compiler library is independent of browsers, servers, concrete host functions, and host schemas. It depends on the `exs-runtime` crate, which embeds the committed `exs-runtime.wasm` template. Its public compilation API accepts source input and compile options, then produces final Wasm bytes plus module metadata.

## Diagnostics

Compile errors MUST include:

- a stable diagnostic code;
- module identity;
- source span;
- concise message; and
- related spans when relevant.

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

All GC-managed values have this runtime root enum:

```rust
pub enum HeapObject {
    String(RuntimeString),
    List(RuntimeList),
    Object(RuntimeObject),
    Closure(RuntimeClosure),
    Cell(RuntimeCell),
    Error(RuntimeError),
    Task(RuntimeTask),
    Stream(RuntimeStream),
    HostResource(RuntimeHostResource),
}
```

`Vec` is solely an implementation detail, for example for a runtime List's element storage. ExS source has Lists, not a `Vec` type.

## Deep clone

Built-in `value.clone()` operations are synchronous. A user-defined `Clone` trait implementation MAY suspend, in which case the call is lowered through the suspendable trait-call path while retaining the same observable clone semantics.

Clone returns a deep copy of the reachable language-value graph while preserving topology:

- repeated references in the input become repeated references to one cloned node;
- cycles remain cycles;
- immutable scalar values may be reused;
- Functions may be reused;
- Closures are cloned as closure objects and their captured Cell graph is cloned;
- Errors are deeply cloned, including `data` and `cause`;
- `origin` and `trace` may be reused because they are immutable.

By default, a reachable HostResource makes clone return `CloneError` with no observable partial clone. A user-defined Clone implementation MAY explicitly support a HostResource according to its capability contract.

The runtime MUST use a source-identity-to-clone map.

## Mutation

List, Object, Cell, and mutable Error-property writes are immediately visible to all tasks sharing the heap.

Each individual built-in mutation is atomic with respect to scheduler checkpoints. No task switch occurs in the middle of one runtime mutation primitive.

## Runtime faults

Internal invariant failures MUST NOT be converted into arbitrary language Errors. They terminate the root execution as runtime failures reported by the runner.

## Serialization

Ordinary CBOR serialization supports Null, Bool, Int, Float, String, List, Object, and Error.

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

# 16 – Host ABI

## Version

This chapter defines Host ABI `0.1`.

## Hostcalls

Source-level `host.operation(arguments)` creates a runtime hostcall record and invokes the runner import. Concrete host operations and their schemas are unknown to the compiler; their static property names are embedded in the final module and resolved by the runner at execution time.

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
| Null                        | null               |
| Bool                        | boolean            |
| Int                         | integer            |
| Float                       | binary64           |
| String                      | text string        |
| List                        | array              |
| Object                      | map with text keys |
| Error                       | tag 60001 plus map |
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

The server runner owns a registry mapping static host names to independently registered synchronous or asynchronous implementations. It validates CBOR input and output against the registered schemas. A synchronous implementation returns a response through the hostcall fast path. An asynchronous implementation creates a HostCallId, returns pending, and resumes the waiting runtime task when its future completes.

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

This chapter defines compiler/runtime ABI `0.1`.

## Required exports

The linked module MUST export:

```text
memory
__exs_abi_version() -> i32
__exs_initialize(config_ptr: i32, config_len: i32) -> i32
__exs_start(input_ptr: i32, input_len: i32) -> i32
__exs_run() -> i32
__exs_resume_host(call_id: i64, kind: i32, ptr: i32, len: i32) -> i32
__exs_cancel(reason_ptr: i32, reason_len: i32) -> i32
__exs_result_kind() -> i32
__exs_result_ptr() -> i32
__exs_result_len() -> i32
__exs_alloc_abi(size: i32) -> i32
__exs_free_abi(ptr: i32, size: i32) -> void
```

## ABI version value

`__exs_abi_version()` returns:

```text
(major << 16) | minor
```

For this specification the value is `0x00000001`.

## Run status

`__exs_start`, `__exs_run`, and `__exs_resume_host` return:

```text
0 READY
1 SUSPENDED
2 COMPLETE
3 CANCELLED
-1 INVALID_ARGUMENT
-2 INVALID_STATE
-3 INTERNAL_FAILURE
```

`READY` means runnable work remains and the runner SHOULD call `__exs_run` again.

`SUSPENDED` means no task is currently runnable and at least one hostcall is outstanding.

`COMPLETE` means result accessors are valid.

## Invocation

The input is canonical CBOR. The phase-1 entry point is `fn main()` and the runner invokes `__exs_start(0, 0)`; non-empty external input is deferred until a later phase introduces an entry signature that consumes it.

The runner MUST call `__exs_initialize` once before the first invocation.

`__exs_start` is legal only when the instance is initialized and idle.

## Result buffer

After COMPLETE, `__exs_result_kind` returns:

```text
0 success value
1 language Error
2 runtime failure details
```

The result buffer remains valid until the next ABI call that mutates runtime state or until explicitly freed according to runtime documentation.

## Runtime intrinsics

Compiler-generated code MAY call linked runtime intrinsics whose names begin with `__exs_rt_`, such as `__exs_rt_list_new`, `__exs_rt_list_get`, `__exs_rt_object_get`, `__exs_rt_cell_new`, `__exs_rt_value_is_error`, `__exs_rt_clone`, `__exs_rt_task_create`, and `__exs_rt_cbor_encode`. The intrinsic names are shared Rust ABI constants and the compiler resolves them from the `crates/exs-runtime/exs-runtime.wasm` export section at link time.

The compiler resolves runtime functions by these export names, never fixed Wasm indices. Source programs cannot import, export, or reference intrinsic names.

# 19 – Grammar Summary

This grammar is normative for syntax but omits lexical Unicode productions already defined.

```ebnf
module          = { functionDecl } ;
declaration     = functionDecl | letDecl ;

functionDecl    = "fn" identifier "(" parameters? ")" block ;
functionExpr    = "fn" "(" parameters? ")" block ;
parameters      = identifier { "," identifier } [ "," ] ;

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
                | identifier
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

Top-level `statement` and `letDecl` occurrences are invalid. Phase 1 accepts only `functionDecl` items and requires exactly one `fn main()` declaration with no parameters.

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
