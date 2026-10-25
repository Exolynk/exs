# Exolynk Script (ExS) Language Reference

**Language version:** 0.2.0-draft

This document defines the ExS source language. It is written for authors of ExS programs: it specifies valid syntax and observable behavior. It deliberately does not describe compiler internals, WebAssembly, runtime layouts, host transport formats, scheduling implementation, or development phases.

## Table of Contents

- [1. Program Structure](#1-program-structure)
- [2. Lexical Structure](#2-lexical-structure)
- [3. Values and Type Contracts](#3-values-and-type-contracts)
- [4. Variables and Scope](#4-variables-and-scope)
- [5. Expressions](#5-expressions)
- [6. Statements and Control Flow](#6-statements-and-control-flow)
- [7. Functions and Closures](#7-functions-and-closures)
- [8. Nominal Types, Enums, and Traits](#8-nominal-types-enums-and-traits)
- [9. Errors](#9-errors)
- [10. Parallel Work with `par`](#10-parallel-work-with-par)
- [11. Modules](#11-modules)
- [12. Built-ins and Host Calls](#12-built-ins-and-host-calls)
- [13. Tests](#13-tests)
- [14. Grammar Summary](#14-grammar-summary)

# 1. Program Structure

An ExS source file is a module. A module has an optional import and `use` prelude followed by declarations, including constants. Executable statements and `let` declarations are not permitted at module scope. A module may also declare source tests; normal program compilation omits them.

The root module must declare exactly one `fn main(...)` function. Imported modules must not declare `main`. Every public root-module function is callable by an external runner; `private` root functions are not (see [Visibility](#visibility)).

```exs
fn main(name: String) -> String {
    ret "Hello, " + name;
}
```

Statements end with `;`. Blocks and declarations do not require a trailing semicolon.

Expressions, blocks, and type annotations may nest at most 32 levels deep by default (`E0114`); the embedding application may change this limit along with limits on source and project size.

# 2. Lexical Structure

## Source text

Source files are UTF-8. `LF` and `CRLF` line endings are equivalent. Whitespace is otherwise insignificant except where it separates tokens.

Line comments begin with `//` and continue to the end of the line. Block comments begin with `/*` and end at the next `*/`; they do not nest. Adjacent `///` lines document the declaration that follows them, and `//!` lines at the start of a file document the module; generated documentation uses both.

Identifiers begin with `_` or an ASCII letter. Later characters may also be ASCII digits or `_`. Identifiers are case-sensitive.

The following words are reserved and cannot be used as identifiers:

```text
as break const continue else enum Error false fn for if impl import in
is let match None par private ret trait true type Unit use while
```

## Literals

### Numbers

Integer literals are decimal digits with optional `_` separators:

```exs
0
42
1_000_000
```

An `Int` literal must be in the inclusive range `-2^63 .. 2^63 - 1`; an out-of-range literal is a compile error. A leading minus is the unary `-` operator, not part of the literal.

Float literals contain a decimal point followed by a digit, an exponent, or both, so `7.abs()` calls a method of the Int `7`:

```exs
1.0
0.25
1e6
2.5e-3
```

### Strings

Ordinary strings use double quotes and support `\"`, `\\`, `\n`, `\r`, `\t`, `\0`, and `\u{HEX}` escapes. The Unicode escape must name one valid Unicode scalar value.

```exs
"hello"
"line\nbreak"
"smile: \u{1f642}"
```

Formatted strings use an `f` prefix and evaluate normal ExS expressions inside `{...}`. Each interpolated value is rendered through `ToString::to_string` (see [Rendering](#rendering)). Use `{{` and `}}` for literal braces. In a formatted string with quotes, a String literal inside an interpolation may be written with plain or with escaped quotes; inside escaped quotes, its own escapes are escaped as well.

```exs
f"Hello {name}; next: {count + 1}"
f"{user["name"]} and {user[\"name\"]}"
```

### Bytes

Bytes use a `b` prefix and contain the UTF-8 source bytes after normal string escape decoding. Use `Bytes::from_list` for arbitrary octets, including values that do not form UTF-8.

```exs
b"header\n"
Bytes::from_list([0, 127, 255])
```

Raw strings use an `r` prefix and one or more `#` delimiters. Their contents are not escape-decoded.

```exs
r#"C:\\temp\\file"#
r##"a "# character is ordinary text"##
```

Formatted raw strings use `f` before the hash delimiters. They support interpolation but do not decode escapes, so they may span multiple lines. Formatted dedented raw strings use `fd` and apply the same indentation removal as `d` strings.

```exs
f#"path: {path}\n"#
fd#"
    Hello {name}
    Next line
"#
```

Dedented raw strings use `d` instead of `r`. For multiline content, ExS removes delimiter-only outer lines and the common leading spaces-and-tabs indentation of nonblank lines.

```exs
let message = d#"
    first line
      second line
"#;
```

### Collections

Lists and Objects are mutable:

```exs
[1, 2, 3]
{ name: "Ada", "display-name": "Ada" }
```

An unquoted Object key is converted to a String with the identifier spelling.

# 3. Values and Type Contracts

ExS is dynamically typed. Types describe values, not variable bindings. The source-visible value categories are:

| Type | Description |
| --- | --- |
| `None` | The absence of a value, such as a missing Object key. |
| `Unit` | The result of a function that completes without a value. |
| `Bool` | `true` or `false`. |
| `Int` | A signed 64-bit integer. |
| `Float` | An IEEE 754 binary64 value. |
| `String` | An immutable Unicode-scalar sequence. |
| `Bytes` | An immutable raw-octet sequence. |
| `List` | A mutable ordered sequence of values. |
| `Object` | A mutable insertion-ordered String-keyed mapping. |
| `Range` | A finite Int iterator created by range syntax. |
| `Error` | A recoverable failure value. |
| `Fn` | A callable closure value. |
| nominal type | A value constructed by a `type` declaration. |
| enum | A value constructed by an `enum` declaration. |

`None` is the only source-level absence value. ExS has no `null` literal. `Unit` is no absence: it means "done" and is what `ret;`, falling off the end of a function, and actions such as `list.clear()` return, so functions that only act declare `-> Unit | Error`. `Unit` is both the value and its type.

## Type contracts

Function parameters, function returns, type fields, enum payload fields, and trait signatures may use a union contract:

```exs
fn describe(value: String | Int | None) -> String | Error {
    // ...
}
```

The built-in contract names are `Any`, `None`, `Unit`, `Error`, `Bool`, `Int`, `Float`, `String`, `Bytes`, `List`, `Object`, `Range`, and `Fn`. User-defined nominal types, enums, and traits are also valid contract names. A type may optionally be written with the `std::` qualifier, such as `std::Int` or `std::None`.

An omitted annotation means `Any`. Contracts are checked at function entry and at each explicit or implicit return.

If a contract failure occurs in a function whose return contract includes `Error`, or whose return contract is omitted, evaluation produces a recoverable `TypeError`. A strict return contract that excludes `Error` terminates the current root execution with a fatal `TypeError`.

# 4. Variables and Scope

`let` creates a mutable local binding. An initializer is optional; an omitted initializer stores `None`.

```exs
let count = 0;
count = count + 1;
let later;
```

Bindings are lexical. A binding declared in a block is visible from its declaration to the end of that block. An inner binding may shadow an outer binding.

Assignment targets are local identifiers, Object properties, and List or Object indexes. When an index or property assignment fails, for example with an index outside a List or a field the nominal type does not declare, the current function returns the assignment's Error, like an invalid condition:

```exs
count = 2;
user.name = "Ada";
values[0] = 42;
```

Closures capture referenced outer bindings. A captured binding is shared: assigning through the outer scope or any closure updates the same binding.

## Constants

A module may declare constants. A constant's value is a literal of None, Unit, Bool, Int, Float, String, or Bytes, a negated number literal, or the name of another constant:

```exs
/// The default page size.
const PAGE_SIZE = 50;
const DEFAULT_TITLE = "Untitled";
private const LIMIT = PAGE_SIZE;
```

Constants are exported, imported, used, and kept private like functions, also from libraries (`log::LEVEL`, `use exo::LIMIT;`). Compilation substitutes a constant's value into every reference that no local binding shadows. Assigning to a constant (`E0120`), a value of another form (`E0119`), constants that refer to each other in a cycle (`E0121`), and a constant named like another declaration are compile errors.

# 5. Expressions

Expressions evaluate left to right unless a rule below specifies short-circuit behavior.

## Access and calls

```exs
value[index]
object.name
function(argument)
value.method(argument)
Type::method(argument)
```

List and Bytes indexes must be nonnegative `Int` values. Bytes indexing returns an `Int` from 0 through 255. Object indexes must be Strings. Invalid indexes produce `IndexError`; an invalid receiver or Object key produces `TypeError`. Missing Object properties and keys evaluate to `None`. Nominal values have only their declared fields: reading or writing another field is a `TypeError`, and so are their `delete` and `clear` methods. Enums have no properties; `match` binds their payload.

String indexing is not supported. Use `for` to iterate a String by Unicode scalar.

Direct calls use a declared function name, an imported function name, or a local binding containing an `Fn` value. Method calls evaluate the receiver before their arguments. Arguments evaluate left to right. Calling a non-callable value produces `TypeError`; a call with fewer than its required parameters, or any count other than the declaration allows, produces `ArityError`.

## Unary operators

| Operator | Accepted values | Result |
| --- | --- | --- |
| `!` | `Bool` | `Bool` |
| `-` | `Int`, `Float` | numeric negation |

Bools are not numbers: arithmetic, negation, and ordering with a Bool produce `TypeError`, and `true == 1` is `false`.

## Arithmetic and comparison

`+`, `-`, and `*` accept `Int` and `Float` values. Integer-only operations produce `Int`; otherwise they produce `Float`. Overflowing an `Int` result produces `IntOverflowError`.

`/` accepts numeric values and always produces `Float`. Division by zero follows IEEE 754 binary64 behavior.

`%` produces the Euclidean remainder, which is never negative: `7 % 3 == 1`, `-1 % 24 == 23`, `7 % -3 == 1`. With two Ints it behaves like `rem_euclid`: a zero divisor produces `DivisionByZeroError` and the smallest Int divided by `-1` produces `IntOverflowError`. With a Float operand the result is a Float and a zero divisor produces NaN. `div_euclid(other)` produces the matching Euclidean quotient.

For nominal types and enums, `+`, `-`, `*`, `/`, and `%` first use a matching standard trait method. Built-in behavior applies when no nominal method is selected.

- `String + String | Bool | Int | Float` produces a concatenated String.
- `List + value` produces a new shallow List with `value` appended.
- `List + List` produces a new shallow List containing both lists' elements.

`==` and `!=` compare values by content:

- Numbers compare by value with Float promotion; Strings and Bytes compare by contents; None, Unit and Bools compare by value.
- Lists compare element by element, Objects by their entries in any key order, nominal values of one type by their fields, enums by their variant and payload, and Errors by kind, message, data, and cause.
- Closures and host resources compare by identity.
- A nominal type or enum with an `Eq` implementation decides its own equality; without one, a `Compare` implementation decides through its Equal result. Both also apply where such values are parts of compared Lists, Objects, fields, or payloads.
- Values that contain themselves compare without recursing forever: a pair already being compared counts as equal.

`list.contains`, `index_of`, `last_index_of`, and `assert_eq` use the same equality.

`<`, `<=`, `>`, and `>=` order `Int`, `Float`, and `String` values; numbers order numerically and Strings by their UTF-8 bytes. Other built-in values have no order and produce `TypeError`, as does an unordered Float such as NaN. Nominal types and enums define ordering through `Compare`.

## Logical operators and type tests

`&&` and `||` require Bool operands and short-circuit: the right operand is evaluated only when necessary. A non-Bool operand produces `TypeError`.

```exs
if value is Error {
    ret value.message();
}
```

`value is Type` tests any type contract, such as `value is Int`, `value is Point`, or `value is String | None`, and evaluates to a Bool.

## Clone and propagation

Every value has a synchronous `clone()` method. It deeply clones mutable graphs, preserves aliases and cycles, and does not mutate the source value. Immutable scalar values may be reused.

The postfix `?` operator propagates results by the return contract of the current function:

- `None` returns `None` when the contract includes `None` (or is omitted or `Any`), and otherwise an `Error` with kind `MissingValue`.
- `Error` returns unchanged when the contract includes `Error` (or is omitted or `Any`). When the contract includes `None` but not `Error`, an Error is a fatal `TypeError` with the Error as its data.
- Any other value, including `Unit`, remains unchanged.

`?` is permitted only in a function whose return contract includes `Error` or `None`, or is omitted; elsewhere, such as in `-> String` or `-> Unit`, it is a compile error.

```exs
fn name(record: Object) -> String | None {
    let value = record.get("name")?;   // a missing name returns None
    ret value.trim();
}

fn save(record: Object) -> Unit | Error {
    Host::call("db.save", record)?;    // Unit passes through, an Error returns
    ret;
}
```

# 6. Statements and Control Flow

## Blocks and expressions

An expression statement evaluates and discards its result:

```exs
values.push(value);
```

A block creates a lexical scope and executes its statements in order.

## Conditionals

```exs
if condition {
    ret 1;
} else if other_condition {
    ret 2;
} else {
    ret 3;
}
```

The condition must be Bool. Otherwise the current function returns `TypeError`.

## Loops

```exs
while condition {
    // ...
}

for item in iterable {
    // ...
}
```

`while` evaluates its Bool condition before every iteration.

`for` evaluates `iterable` once and advances it through `Iterator::next() -> IteratorStep | Error`. `IteratorStep::Item(value)` enters the body with `value`; `IteratorStep::Done` exits the loop. A List is iterated over a shallow snapshot, so changes to the original List do not alter the iteration sequence. A String yields one-scalar Strings. Bytes yields one Int octet from 0 through 255. An Object yields its `[key, value]` entries from a snapshot in insertion order. `start..end` creates a half-open Int Range and `start..=end` creates an inclusive Int Range; both infer direction from their endpoints. `Range::step(start, end, step)` is an explicit half-open range, while `Range::step_through(start, end, step)` is inclusive. A step must be non-zero and move toward its endpoint. A user-defined nominal value must implement `Iterator`; any other value produces `NotIterable`. Advancing an Iterator may suspend.

Each loop iteration creates a fresh binding for the loop variable. Closures created in separate iterations therefore capture distinct loop bindings.

`break;` exits the nearest loop. `continue;` proceeds to its next iteration. Both are compile errors outside a loop.

## Return

```exs
ret;
ret value;
```

`ret;` returns `Unit`. Falling off the end of a function also returns `Unit`.

# 7. Functions and Closures

## Functions

```exs
fn add(left: Int, right: Int) -> Int {
    ret left + right;
}
```

Parameters are positional. ExS has no default, named, or keyword parameters. Duplicate parameter names are compile errors. Named functions are visible throughout their enclosing module, enabling recursion.

Every public root-module function is callable by an external runner. The runner selects the function by name and supplies its positional arguments. Private root-module functions, methods, trait methods, closures, prelude functions, imported-module functions, and library functions are not externally callable.

A function, method, trait method, or closure may declare one trailing variadic parameter with `...` after its name or type annotation:

```exs
fn total(prefix: Int, values: Int...) -> Int {
    let result = prefix;
    for value in values {
        result = result + value;
    }
    ret result;
}
```

The variadic binding is always a `List` and may be empty. It must be the final parameter. A type annotation applies independently to every supplied trailing value, not to the `List` itself. Calls must provide every non-variadic parameter and may then provide any number of trailing values. A non-variadic declaration still requires its exact argument count. Trait implementations must declare the same variadic position as their trait method.

An externally callable function may have zero or more parameters. Hosts may supply fewer input values, in which case missing fixed parameters receive `None`. When a root function has a trailing variadic parameter, all remaining host input values are packed into its List. Supplying more values than a non-variadic root function declares is a fatal `ArityError`.

## Closures

Closures use arrow syntax:

```exs
let add_offset = (value) => {
    ret value + offset;
};

let later = () => {
    ret "done";
};
```

A closure whose body is an expression instead of a block returns that expression. A body starting with `{` is always a block, so a closure returns an Object literal in parentheses:

```exs
let doubled = values.map((value) => value * 2);
let make = () => ({ name: "Ada" });
```

Closure parameters may have type annotations, which every call checks like function parameter contracts:

```exs
let double = (value: Int | Float) => {
    ret value * 2;
};
```

A closure can be passed to an `Fn` contract, returned, stored in a List or Object, and invoked through its binding.

# 8. Nominal Types, Enums, and Traits

## Nominal Object types

```exs
type User {
    name: String,
    nickname: String | None,
    metadata,
}

let user = User { name: "Ada" };
```

`Name {` starts a typed Object only when `}` or a field name and `:` follow the brace, so `match Color::Red { ... }` and `if point == Point { x: 1 } { ... }` parse as expected.

Every declared field is present after construction. An omitted field becomes `None` when its contract permits `None` or is `Any`. Missing required fields, unknown fields, duplicate fields, and contract violations produce the ordinary `TypeError` behavior. A nominal value has only its declared fields; reading or assigning another field is a `TypeError`.

An inherent `impl` block declares methods for a type:

```exs
impl User {
    fn display(self) -> String {
        ret self.name;
    }

    fn named(name: String) -> User {
        ret User { name: name };
    }
}

let user = User::named("Ada");
let display = user.display();
```

The first bare `self` parameter declares an instance method. A method without `self` is static. Method references are not supported.

## Enums and matching

```exs
enum Color {
    Rgb(red: Int, green: Int, blue: Int),
    Named(name: String),
    Transparent,
}

let color = Color::Rgb(255, 0, 128);
let brightness = match color {
    Color::Rgb(red, green, blue) => red + green + blue,
    Color::Named(name) => 1,
    Color::Transparent => 0,
};
```

Enum constructors are qualified by the enum name. Payload arguments are checked in declaration order. Enums may have inherent and trait `impl` blocks like nominal Object types.

A `match` expression evaluates its input once and evaluates only the selected arm. Variant patterns must name variants of the same enum and bind exactly the declared payload count. `_` is an optional fallback and must be last. Without `_`, every declared variant must be covered exactly once.

Literal patterns match values equal to a literal of None, Unit, Bool, Int, Float, String, or Bytes, or a negated number:

```exs
let label = match status {
    200 => "ok",
    404 => "not found",
    -1 => "unknown",
    _ => "error",
};
```

One `match` uses either variant patterns or literal patterns. A literal may appear only once. Without `_`, a value that no literal matches produces `MatchError`.

An arm may use an expression or a statement block. A `ret` in a block returns from the enclosing function; a block that finishes normally has value `Unit`.

## Traits and operators

```exs
trait Label {
    fn label(self) -> String;
    fn category() -> String { ret "person"; }
}

impl Label for User {
    fn label(self) -> String {
        ret self.name;
    }
}
```

A trait method ending in `;` is required. A trait method with a block is a default implementation. An implementation must provide every required method and may omit defaulted methods. Trait and nominal type names share one namespace.

`Self` is valid in trait method signatures and in the method signatures of any `impl` block; it means the implementation target.

The compiler-owned traits `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Eq`, `Compare`, `ToString`, and `Debug` are reserved. A nominal type or enum may implement them with these fixed signatures:

```exs
fn add(self, other: Any) -> Any
fn sub(self, other: Any) -> Any
fn mul(self, other: Any) -> Any
fn div(self, other: Any) -> Any
fn rem(self, other: Any) -> Any
fn eq(self, other: Any) -> Bool
fn compare(self, other: Any) -> Ordering
fn to_string(self) -> String
fn debug(self) -> String
```

`Eq` decides `==` and `!=`; `Compare` decides `<`, `<=`, `>`, and `>=`, and equality when the type implements no `Eq`.

`Ordering` is a built-in enum, also available as `std::Ordering`, with variants `Less`, `Equal`, `Greater`, and `Unordered`.

`ToString` is used by formatted string interpolation and may also be called explicitly. `Debug` is an explicit diagnostic renderer. An `impl ToString` or `impl Debug` overrides the corresponding default for its nominal type or enum.

## Rendering

The default `to_string()` and `debug()` render values in ExS literal syntax:

| Value | Rendering |
| --- | --- |
| None, Unit, Bool, Int, Float | `None`, `Unit`, `true`, `42`, `1.5` |
| String | the text itself; `debug()` quotes and escapes it: `"a\"b"` |
| Bytes | `b"hi"`, or `Bytes::from_hex("00ff")` for bytes that are not UTF-8 |
| List | `[1, "a", [2]]` |
| Object | `{ name: "Ada", "display-name": "x" }`, `{}` |
| nominal value | `Point { x: 1, y: 2 }`, `Empty {}` |
| enum | `Color::Red`, `Color::Rgb(255, 0, 0)` |
| Error | `Error("Kind", "message", data)` |
| Range | `0..3`, `1..=5`, `Range::step(0, 10, 2)` |
| closure | `Fn` |

Parts of Lists, Objects, fields, payloads, and Error data render through their type's `Debug` implementation, else its `ToString` implementation, else the default with quoted Strings. A value that contains itself renders as `...` where it repeats.

# 9. Errors

Recoverable failures are ordinary `Error` values. ExS has no `try`, `catch`, `throw`, or exception unwinding.

```exs
let error = Error("ValidationError", "invalid input", { field: "name" });
if error is Error {
    ret error.message();
}
```

`Error(kind, message, data)` creates a recoverable Error. `kind` and `message` must be Strings; `data` may be any value. `Error` is reserved and cannot be redeclared.

Errors expose these methods:

```text
error.kind()      // String
error.message()   // String
error.data()      // any value
error.cause()     // related value or None
```

Common Error kinds are `ArityError`, `CloneError`, `DivisionByZeroError`, `EncodingError`, `IndexError`, `IntOverflowError`, `MatchError`, `MethodNotFound`, `MissingValue`, `NotIterable`, `SerializationError`, `TypeError`, `ValueError`, and `HostFunctionNotFound`.

# 10. Parallel Work with `par`

`par` is the only ExS construct that creates language tasks.

```exs
let results = par {
    fetch("first");
    fetch("second");
};
```

Each semicolon-terminated expression in a static `par` block becomes a task. The expressions are not evaluated before task creation. An empty block returns an empty List.

The dynamic form accepts a List of closures callable with zero arguments, including variadic closures with no required parameters:

```exs
let results = par([first, second]);
```

Tasks share the same values and mutations. `par` returns a List of task results in source order, independent of completion order. An Error is a normal task result and does not cancel its siblings. A parent task waits until all of its direct `par` children finish.

Host calls may complete in a nondeterministic order. Apart from that external completion order, task scheduling is deterministic. A task runs until it suspends, for example at a host call or `Host::sleep`, or reaches a scheduler checkpoint in a loop or call of code that can suspend; synchronous code runs without interruption. Programs must not rely on a particular interleaving of mutations that parallel tasks make to shared values.

# 11. Modules

## Files and imports

Imports load relative `.exs` files into namespaces:

```exs
import "./math.exs" as math;
use math::{add as plus, Point};

fn main() -> Int {
    let point = Point { value: 20 };
    ret plus(point.value, 22);
}
```

Without `as`, an import namespace is derived from its file name. More than one import may use the same namespace only when their exported declaration names do not conflict. Import cycles are compile errors.

## Libraries

The application that embeds ExS may provide library modules, such as `log`, `exo`, or `exo::db`. Like the standard library, library modules need no import: every module names their items by path. Library paths start with a lowercase letter, and a library module may have child modules, such as `exo` and `exo::db`.

```exs
fn main(id: Int) -> exo::Record {
    log::info("loading record");
    ret exo::db::load(id);
}
```

`use` brings library items and modules into scope:

```exs
use log::info;
use exo::db::{load, save as store};
use exo::db;

fn main(id: Int) -> Unit {
    info("loading record");
    let record = load(id);
    store(record);
    db::load(id);
}
```

Importing a library like a file (`import "log";`) is a compile error; name it by its path or with `use`.

## Use declarations

A `use` declaration names an imported or library declaration, or a library module, through a path; `as` gives it another name, and a braced list names several items of one module. `use` names are read-only and may stand for functions, types, enums, traits, and library modules. Imports and `use` declarations must precede type, enum, trait, implementation, and function declarations.

## Visibility

Declarations are public by default. The `private` keyword before a module-level function, type, enum, trait, or constant, or before a method of an inherent `impl` block, restricts it to its package:

- a library package contains every module of one library, such as `exo` and `exo::db`;
- the project package contains the root file and every file it imports.

```exs
private fn normalize(name: String) -> String {
    ret name.trim();
}

type User { name: String }

impl User {
    fn create(name: String) -> User {
        ret User { name: normalize(name) };
    }

    private fn audit(self) -> Unit {
        ret;
    }
}
```

Naming a private function, type, enum, or trait from another package is a compile error that names the item as private to its library; calling a private static method from another package is a compile error as well. Instance methods are dispatched at run time, so a private instance method behaves like a missing method for receivers called from another package. Methods of trait implementations are always public, and `private` before any other declaration is a compile error. Private declarations are left out of generated documentation and editor completions for other packages.

# 12. Built-ins and Host Calls

## Built-in methods

Every value supports `clone()`.

Numeric values support `add(other)`, `sub(other)`, `mul(other)`, `div(other)`, and `rem(other)`, which behave like the operators. `Int` also supports `div_euclid(other)` and `rem_euclid(other)` for exact Euclidean calculations. `Int`, `Float`, and `String` support `compare(other)`, which returns an `Ordering`. `Int` and `Float` support `abs()`. `Float` also supports `floor()`, `ceil()`, and `round()`.

`Int::parse(value)` and `Float::parse(value)` perform checked decimal parsing and return `ParseError` for invalid input. Int values additionally provide `signum()`, `is_even()`, `is_odd()`, `min(other)`, `max(other)`, `clamp(minimum, maximum)`, `pow(exponent)`, `gcd(other)`, `lcm(other)`, and `to_float()`. Float values additionally provide `trunc()`, `fract()`, `min(other)`, `max(other)`, `clamp(minimum, maximum)`, `pow(exponent)`, `sqrt()`, `to_int()`, `is_nan()`, `is_finite()`, and `is_infinite()`. Checked operations return `Error` rather than coercing or saturating.

`Math` provides strict-Float static constants `pi()`, `tau()`, and `e()`, plus `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `ln`, `log2`, `log10`, and `hypot`. Trigonometric values use radians. Every function argument must be a Float; Int values are not coerced and return `TypeError`.

Strings, Bytes, Lists, and Objects support `length()` and `is_empty()`.

Bytes are immutable and support:

```text
Bytes::from_list(values)    // Bytes or TypeError / ValueError
Bytes::from_utf8(value)     // Bytes or TypeError
bytes.to_list()             // new List of Int octets
bytes.slice(start, end)     // new Bytes or IndexError
bytes.concat(other)         // new Bytes or TypeError
bytes.decode_utf8()         // String or EncodingError
bytes.starts_with(other)    // Bool or TypeError
bytes.ends_with(other)      // Bool or TypeError
bytes.contains(other)       // Bool or TypeError
bytes.repeat(count)         // new Bytes or Error
bytes.to_hex()              // lowercase String
Bytes::from_hex(value)      // Bytes or ParseError
bytes.to_base64()           // padded RFC 4648 String
Bytes::from_base64(value)   // Bytes or ParseError
```

Strings additionally provide Unicode-scalar `slice(start, end)`, `contains`, `starts_with`, `ends_with`, `split`, `trim`, `trim_start`, `trim_end`, `replace`, `to_lowercase`, `to_uppercase`, `repeat`, and `encode_utf8()`. String ranges are half-open scalar ranges, matching `String.length()` and String iteration; they are never UTF-8 byte offsets.

Lists support:

```text
list.push(value)        // mutates and returns the new length
list.pop()              // removes the last value; None when empty
list.insert(index, v)   // mutates; Unit or IndexError
list.remove(index)      // mutates; removed value or IndexError
list.clear()            // mutates; Unit
list.get(index)         // value or None when out of bounds
list.first()            // first value or None
list.last()             // final value or None
list.slice(start, end)  // new shallow List or IndexError
list.extend(other)      // mutates and returns the new length
list.reverse()          // mutates; Unit
list.reversed()         // new shallow reversed List
list.sort()             // sorts in place under `<`, stable; Unit or Error
list.sort(compare)      // sorts in place by the Ordering compare(a, b) returns
list.sorted()           // new List sorted under `<`
list.sorted(compare)    // new List sorted by compare
list.sum()              // sum under `+`, 0 when empty
list.min()              // smallest item under `<`, None when empty
list.max()              // largest item under `>`, None when empty
list.flatten()          // new List with nested Lists inlined one level deep
list.unique()           // new List without repetitions under `==`
list.contains(value)    // Bool under ExS equality
list.index_of(value)    // first Int index or None
list.last_index_of(v)   // final Int index or None
list.join(separator)    // String or TypeError for non-String items
list.map(callback)      // new List or the first callback Error
list.filter(callback)   // new List or the first callback Error
list.find(callback)     // matching value, None, or callback Error
list.any(callback)      // Bool or callback Error
list.all(callback)      // Bool or callback Error
list.each(callback)     // Unit or callback Error
list.reduce(initial, callback) // accumulated value or callback Error
list.collect()            // new shallow List snapshot
list.count()              // number of yielded items
list.to_list()            // alias for collect()
```

Every iterable, including Objects through their `[key, value]` entries, supports `map`, `filter`, `find`, `any`, `all`, `each`, `reduce`, `collect`, `count`, and `to_list`. These operations are eager. Callback-based operations return the first Error produced by their callback; Error values already present in the iterable are ordinary items.

Objects support:

```text
object.has(key)         // Bool
object.delete(key)      // removed value or None
object.keys()           // new List of String keys in insertion order
object.values()         // new shallow List in insertion order
object.get_or(key, d)   // value or d when absent
object.clear()          // mutates; Unit
object.entries()        // new [String, value] Lists in insertion order
object.merge(other)     // new Object with right-hand values winning
Object::from_entries(entries) // Object from [String, value] Lists
```

Calling an unsupported method produces `MethodNotFound`.

## Durations and Host sleep

```exs
let retry_after = Duration::seconds(2);
Host::sleep(retry_after);
```

`Duration` is a normal prelude type with `seconds: Int` and `nanoseconds: Int` fields. Durations order by length and support `+` and `-`; a negative difference produces `ValueError`. A `DateTime` plus or minus a Duration is a DateTime, a DateTime minus an earlier DateTime is the Duration between them, and DateTimes order and compare as instants independently of their display offsets. Valid Duration values have non-negative seconds and a nanosecond field from `0` to `999999999`. `Duration::nanoseconds(value)`, `Duration::microseconds(value)`, `Duration::milliseconds(value)`, and `Duration::seconds(value)` construct normalized values from non-negative Int input. Conversions return `ValueError` for negative input and `IntOverflowError` if an intermediate multiplication exceeds the Int range. `as_seconds()` returns the whole-second component; `as_milliseconds()`, `as_microseconds()`, and `as_nanoseconds()` return truncated total-unit Int values or `IntOverflowError` when that total exceeds the Int range.

`Host::sleep(duration)` accepts exactly one normalized Duration and returns `Unit` after the duration elapses. It may suspend, but it is provided by every ExS runner and does not require an application host-function registration.

## Host calls

```exs
let profile = Host::call("profile.load", user_id);
```

`Host::call(name, arguments...)` invokes a host-provided operation. `name` must evaluate to a String. Arguments evaluate left to right. The host call returns its result or an Error and may suspend; it does not create an ExS task.

Nominal values keep their type across the host boundary. A value of a `type` declaration reaches the host with its fields and the type's identity: the module's source id or library path followed by the type name, such as `exo::dom::Div`. A host value with the identity of a type the program uses becomes a value of that type, in host responses and in the inputs of `main`, wherever it is nested; its declared fields are taken by name, missing ones are `None`, and other fields are dropped. A host value of a type the program does not use behaves as an Object, and keeps its identity, so it reaches the host as that type again.

```exs
type Point { x: Int, y: Int }

fn main() -> Bool | Error {
    let point = Host::call("echo", Point { x: 1, y: 2 })?; // the host sees "app.exs::Point"
    ret point is Point;
}
```

## Host streams

```exs
let events = Host::stream("events.subscribe", user_id)?;
for event in events {
    Host::call("events.record", event);
}
```

`Host::stream(name, arguments...)` opens a runner-registered pull stream and returns `HostStream | Error`. Its arguments follow the same variadic convention as `Host::call`; the name itself is not passed to the registered factory. A HostStream implements `Iterator`; each `next()` may suspend and returns `IteratorStep::Item(value)`, `IteratorStep::Done`, or an Error.

Each stream is single-consumer. Starting a second `next()` before the first completes returns a recoverable Error. A stream is closed and dropped after `Done`, when its root execution ends, or when that execution is cancelled. A completed handle cannot be advanced again.

Host-call arguments and final program results cross an acyclic, by-value runner boundary. Closures, cells, and cyclic value graphs cannot cross it; shared references are duplicated. An unserializable host call returns a recoverable `SerializationError` without invoking the host. An unserializable final program result becomes a fatal `SerializationError`.

The available names, their argument contracts, capabilities, and side effects are defined by the embedding host, not by ExS.

# 13. Tests

Tests are named module declarations executed by `exs test`. The command discovers test-bearing `.exs` files recursively, excluding `.git` and `target`, and runs every declared test in a fresh execution instance. `exs test path/to/file.exs` and `exs test path/to/directory` restrict discovery.

```exs
test "adds two values" {
    assert_eq(20 + 22, 42, "addition must preserve both operands");
}
```

Test declarations are not included by `exs run` or `exs compile`. A failed assertion creates a fatal `AssertionFailed` Error, so execution stops immediately whether the assertion appears in a test or ordinary program function.

Every item in `std::` is automatically available in every module under its final name, so the `std::` qualifier is optional. `assert(condition[, description])` requires a Bool condition and accepts an optional String description. It returns Unit when the condition is true. Without a description, assertion failures use `"assert failed"`. `assert_eq(actual, expected[, description])` compares values with the normal `==` equality semantics and returns Unit when they compare equal. It accepts an optional String description and otherwise uses `"assert_eq failed"`. Assertion failures carry the description as their message; `assert_eq` carries actual and expected values as error data. `std::test::assert` and `std::test::assert_eq` are the equivalent qualified spellings.

# 14. Grammar Summary

This grammar summarizes the source syntax. Lexical rules, including string forms and ASCII identifiers, are defined earlier in this document.

```ebnf
module          = { moduleDecl } { item } ;
moduleDecl      = importDecl | useDecl ;
importDecl      = "import" string [ "as" identifier ] ";" ;
useDecl         = "use" qualifiedName [ "as" identifier ] ";"
                | "use" qualifiedName "::" "{" useItem { "," useItem } [ "," ] "}" ";" ;
useItem         = identifier [ "as" identifier ] ;
qualifiedName   = identifier { "::" identifier } ;
item            = [ "private" ] ( functionDecl | typeDecl | enumDecl | traitDecl | constDecl )
                | testDecl | implDecl ;
constDecl       = "const" identifier "=" expression ";" ;

functionDecl    = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] block ;
testDecl        = "test" string block ;
typeDecl        = "type" identifier "{" [ typeField { "," typeField } [ "," ] ] "}" ;
enumDecl        = "enum" identifier "{" [ enumVariant { "," enumVariant } [ "," ] ] "}" ;
enumVariant     = identifier [ "(" [ typeField { "," typeField } [ "," ] ] ")" ] ;
typeField       = identifier [ ":" typeUnion ] ;
traitDecl       = "trait" identifier "{" { traitMethod } "}" ;
traitMethod     = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] ( ";" | block ) ;
implDecl        = "impl" identifier [ "for" identifier ] "{" { [ "private" ] functionDecl } "}" ;
closure         = "(" closureParameters? ")" "=>" ( block | expression ) ;
parameters      = parameter { "," parameter } [ "," ] ;
closureParameters = closureParameter { "," closureParameter } [ "," ] ;
closureParameter = identifier [ ":" typeUnion ] [ "..." ] ;
parameter       = identifier [ ":" typeUnion ] [ "..." ] ;
typeUnion       = typeName { "|" typeName } ;
typeName        = qualifiedName | "None" | "Unit" | "Error" ;

statement       = block | ifStmt | whileStmt | forStmt | breakStmt | continueStmt
                | returnStmt | letDecl | expression ";" ;
block           = "{" { statement } "}" ;
letDecl         = "let" identifier [ "=" expression ] ";" ;
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
comparison      = range { ( "<" | "<=" | ">" | ">=" | "is" ) range } ;
range           = term [ ( ".." | "..=" ) term ] ;
term            = factor { ( "+" | "-" ) factor } ;
factor          = unary { ( "*" | "/" | "%" ) unary } ;
unary           = ( "!" | "-" ) unary | postfix ;
postfix         = primary { call | index | property | "?" } ;
call            = "(" arguments? ")" ;
arguments       = expression { "," expression } [ "," ] ;
index           = "[" expression "]" ;
property        = "." identifier ;

primary         = literal | qualifiedName | closure | listLiteral | objectLiteral
                | typedObject | matchExpr | parExpr | hostCall | "(" expression ")" ;
typedObject     = qualifiedName "{" [ objectItems ] "}" ;
matchExpr       = "match" expression "{" matchArm { "," matchArm } [ "," ] "}" ;
matchArm        = ( qualifiedName [ "(" identifiers? ")" ] | literalPattern | "_" )
                  "=>" ( expression | block ) ;
literalPattern  = [ "-" ] ( integer | float ) | string | bytes | "true" | "false" | "None" | "Unit" ;
parExpr         = "par" "{" { expression ";" } "}" | "par" "(" expression ")" ;
hostCall        = "Host" "::" "call" "(" arguments? ")" ;
listLiteral     = "[" [ arguments ] "]" ;
objectLiteral   = "{" [ objectItems ] "}" ;
objectItems     = objectItem { "," objectItem } [ "," ] ;
objectItem      = ( identifier | string ) ":" expression ;
```

Assignment targets must be identifiers, property accesses, or index accesses.
