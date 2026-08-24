# Exolynk Script (ExS) Language Reference

**Language version:** 0.1.0-draft

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

An ExS source file is a module. A module has an optional import and `use` prelude followed by declarations. Executable statements and `let` declarations are not permitted at module scope. A module may also declare source tests; normal program compilation omits them.

The root module must declare exactly one `fn main(...)` function. Imported modules must not declare `main`.

```exs
fn main(name: String) -> String {
    ret "Hello, " + name;
}
```

Statements end with `;`. Blocks and declarations do not require a trailing semicolon.

# 2. Lexical Structure

## Source text

Source files are UTF-8. `LF` and `CRLF` line endings are equivalent. Whitespace is otherwise insignificant except where it separates tokens.

Line comments begin with `//` and continue to the end of the line. Block comments begin with `/*` and end at the next `*/`; they do not nest.

Identifiers begin with `_` or an ASCII letter. Later characters may also be ASCII digits or `_`. Identifiers are case-sensitive.

The following words are reserved and cannot be used as identifiers:

```text
as break continue else enum Error false fn for host if impl import in
is let match None par ret trait true type use while
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

Float literals contain a decimal point, an exponent, or both:

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

Formatted strings use an `f` prefix and evaluate normal ExS expressions inside `{...}`. Each interpolated value is rendered through `ToString::to_string`. Built-in values have default renderers; nominal types and enums may override them with `impl ToString`. Use `{{` and `}}` for literal braces.

```exs
f"Hello {name}; next: {count + 1}"
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
| `None` | The only absence value. |
| `Bool` | `true` or `false`. |
| `Int` | A signed 64-bit integer. |
| `Float` | An IEEE 754 binary64 value. |
| `String` | An immutable Unicode-scalar sequence. |
| `List` | A mutable ordered sequence of values. |
| `Object` | A mutable insertion-ordered String-keyed mapping. |
| `Error` | A recoverable failure value. |
| `Fn` | A callable closure value. |
| nominal type | A value constructed by a `type` declaration. |
| enum | A value constructed by an `enum` declaration. |

`None` is the only source-level absence value. ExS has no `null` literal.

## Type contracts

Function parameters, function returns, type fields, enum payload fields, and trait signatures may use a union contract:

```exs
fn describe(value: String | Int | None) -> String | Error {
    // ...
}
```

The built-in contract names are `Any`, `None`, `Error`, `Bool`, `Int`, `Float`, `String`, `List`, `Object`, and `Fn`. User-defined nominal types, enums, and traits are also valid contract names. A type may optionally be written with the `std::` qualifier, such as `std::Int` or `std::None`.

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

Assignment targets are local identifiers, Object properties, and List or Object indexes:

```exs
count = 2;
user.name = "Ada";
values[0] = 42;
```

Closures capture referenced outer bindings. A captured binding is shared: assigning through the outer scope or any closure updates the same binding.

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

List indexes must be nonnegative `Int` values. Object indexes must be Strings. Invalid indexes produce `IndexError`; an invalid receiver or Object key produces `TypeError`. Missing Object properties and keys evaluate to `None`.

String indexing is not supported. Use `for` to iterate a String by Unicode scalar.

Direct calls use a declared function name, an imported function name, or a local binding containing an `Fn` value. Method calls evaluate the receiver before their arguments. Arguments evaluate left to right. Calling a non-callable value produces `TypeError`; a call with fewer than its required parameters, or any count other than the declaration allows, produces `ArityError`.

## Unary operators

| Operator | Accepted values | Result |
| --- | --- | --- |
| `!` | `Bool` | `Bool` |
| `-` | `Bool`, `Int`, `Float` | numeric negation |

`Bool` converts to `Int` for numeric operations: `false` is `0` and `true` is `1`.

## Arithmetic and comparison

`+`, `-`, and `*` accept numeric values. Integer-only operations produce `Int`; otherwise they produce `Float`. Overflowing an `Int` result produces `IntOverflowError`.

`/` accepts numeric values and always produces `Float`. Division by zero follows IEEE 754 binary64 behavior.

`%` is not part of ExS 0.1. Exact Int calculations can use `div_euclid(other)` for the Euclidean quotient and `rem_euclid(other)` for the non-negative remainder. Both require an Int receiver and Int divisor, return `DivisionByZeroError` for a zero divisor, and return `IntOverflowError` for the smallest Int divided by `-1`.

For nominal types and enums, `+`, `-`, `*`, and `/` first use a matching standard trait method. Built-in behavior applies when no nominal method is selected.

- `String + String | Bool | Int | Float` produces a concatenated String.
- `List + value` produces a new shallow List with `value` appended.
- `List + List` produces a new shallow List containing both lists' elements.

`==` and `!=` compare scalar values by value. Strings compare by contents. Lists, Objects, closures, and Errors compare by identity. Numeric equality applies Bool-to-Int conversion and Float promotion.

`<`, `<=`, `>`, and `>=` support numeric values and Strings. A comparison involving an unordered Float value, such as NaN, produces `TypeError`. Nominal types and enums may customize all comparison operators through `Compare`.

## Logical operators and type tests

`&&` and `||` require Bool operands and short-circuit: the right operand is evaluated only when necessary. A non-Bool operand produces `TypeError`.

```exs
if value is Error {
    ret value.message();
}
```

`value is Error` evaluates to a Bool.

## Clone and propagation

Every value has a synchronous `clone()` method. It deeply clones mutable graphs, preserves aliases and cycles, and does not mutate the source value. Immutable scalar values may be reused.

The postfix `?` operator propagates results:

- `Error` returns unchanged from the current function.
- `None` returns an `Error` with kind `MissingValue`.
- Any other value remains unchanged.

`?` is permitted only in a function whose return contract includes `Error`, or whose return contract is omitted.

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

`for` evaluates `iterable` once. A List is iterated over a shallow snapshot, so changes to the original List do not alter the iteration sequence. A String yields one-scalar Strings. Any other value produces `NotIterable`.

Each loop iteration creates a fresh binding for the loop variable. Closures created in separate iterations therefore capture distinct loop bindings.

`break;` exits the nearest loop. `continue;` proceeds to its next iteration. Both are compile errors outside a loop.

## Return

```exs
ret;
ret value;
```

`ret;` returns `None`. Falling off the end of a function also returns `None`.

# 7. Functions and Closures

## Functions

```exs
fn add(left: Int, right: Int) -> Int {
    ret left + right;
}
```

Parameters are positional. ExS has no default, named, or keyword parameters. Duplicate parameter names are compile errors. Named functions are visible throughout their enclosing module, enabling recursion.

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

The root `main` function may have zero or more parameters. Hosts may supply fewer input values, in which case missing fixed `main` parameters receive `None`. When `main` has a trailing variadic parameter, all remaining host input values are packed into its List. Supplying more values than a non-variadic `main` declares is a fatal `ArityError`.

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

Closure parameters cannot have type annotations. A closure can be passed to an `Fn` contract, returned, stored in a List or Object, and invoked through its binding.

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

Every declared field is present after construction. An omitted field becomes `None` when its contract permits `None` or is `Any`. Missing required fields, unknown fields, duplicate fields, and contract violations produce the ordinary `TypeError` behavior.

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

An arm may use an expression or a statement block. A `ret` in a block returns from the enclosing function; a block that finishes normally has value `None`.

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

`Self` is valid only in a trait method signature and a method inside `impl Trait for Type`; it means the implementation target.

The compiler-owned traits `Add`, `Sub`, `Mul`, `Div`, `Compare`, `ToString`, and `Debug` are reserved. A nominal type or enum may implement them with these fixed signatures:

```exs
fn add(self, other: Any) -> Any
fn sub(self, other: Any) -> Any
fn mul(self, other: Any) -> Any
fn div(self, other: Any) -> Any
fn compare(self, other: Any) -> Ordering
fn to_string(self) -> String
fn debug(self) -> String
```

`Ordering` is a built-in enum, also available as `std::Ordering`, with variants `Less`, `Equal`, `Greater`, and `Unordered`.

`ToString` is used by formatted string interpolation and may also be called explicitly. `Debug` is an explicit diagnostic renderer. Both have defaults for all runtime value categories: `None` renders as `"None"`, Error as `"Error"`, scalar values use their source spelling, String is unchanged, List and Object render as `"[]"` and `"{}"`, closures render as `"fn main()"`, and enum values render as their stable `source_id::Type::Variant` identity. An `impl ToString` or `impl Debug` overrides the corresponding default for its nominal type or enum.

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

Common Error kinds are `ArityError`, `CloneError`, `IndexError`, `IntOverflowError`, `MatchError`, `MethodNotFound`, `MissingValue`, `NotIterable`, `SerializationError`, `TypeError`, and `HostFunctionNotFound`.

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

Host calls may complete in a nondeterministic order. Apart from that external completion order, task scheduling is deterministic.

# 11. Modules

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

Imports and `use` declarations must precede type, enum, trait, implementation, and function declarations. A `use` alias is read-only and may name an imported function, type, enum, or trait.

# 12. Built-ins and Host Calls

## Built-in methods

Every value supports `clone()`.

Numeric values support `add(other)`, `sub(other)`, `mul(other)`, and `div(other)`. `Int` also supports `div_euclid(other)` and `rem_euclid(other)` for exact Euclidean calculations. `Int` and `Float` support `abs()`. `Float` also supports `floor()`, `ceil()`, and `round()`.

Strings, Lists, and Objects support `length()` and `is_empty()`.

Lists support:

```text
list.push(value)        // mutates and returns the new length
list.pop()              // removes the last value; None when empty
list.insert(index, v)   // mutates; None or IndexError
list.remove(index)      // mutates; removed value or IndexError
list.clear()            // mutates; None
```

Objects support:

```text
object.has(key)         // Bool
object.delete(key)      // removed value or None
object.keys()           // new List of String keys in insertion order
object.values()         // new shallow List in insertion order
```

Calling an unsupported method produces `MethodNotFound`.

## Durations and Host sleep

```exs
let retry_after = Duration::seconds(2);
Host::sleep(retry_after);
```

`Duration` is a normal prelude type with `seconds: Int` and `nanoseconds: Int` fields. Valid Duration values have non-negative seconds and a nanosecond field from `0` to `999999999`. `Duration::nanoseconds(value)`, `Duration::microseconds(value)`, `Duration::milliseconds(value)`, and `Duration::seconds(value)` construct normalized values from non-negative Int input. Conversions return `ValueError` for negative input and `IntOverflowError` if an intermediate multiplication exceeds the Int range. `as_seconds()` returns the whole-second component; `as_milliseconds()`, `as_microseconds()`, and `as_nanoseconds()` return truncated total-unit Int values or `IntOverflowError` when that total exceeds the Int range.

`Host::sleep(duration)` accepts exactly one normalized Duration and returns `None` after the duration elapses. It may suspend, but it is provided by every ExS runner and does not require an application host-function registration.

## Host calls

```exs
let profile = Host::call("profile.load", user_id);
```

`Host::call(name, arguments...)` invokes a host-provided operation. `name` must evaluate to a String. Arguments evaluate left to right. The host call returns its result or an Error and may suspend; it does not create an ExS task.

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

Every item in `std::` is automatically available in every module under its final name, so the `std::` qualifier is optional. `assert(condition[, description])` requires a Bool condition and accepts an optional String description. It returns None when the condition is true. Without a description, assertion failures use `"assert failed"`. `assert_eq(actual, expected[, description])` compares values with the normal `==` equality semantics and returns None when they compare equal. It accepts an optional String description and otherwise uses `"assert_eq failed"`. Assertion failures carry the description as their message; `assert_eq` carries actual and expected values as error data. `std::test::assert` and `std::test::assert_eq` are the equivalent qualified spellings.

# 14. Grammar Summary

This grammar summarizes the source syntax. Lexical rules, including string forms and ASCII identifiers, are defined earlier in this document.

```ebnf
module          = { moduleDecl } { item } ;
moduleDecl      = importDecl | useDecl ;
importDecl      = "import" string [ "as" identifier ] ";" ;
useDecl         = "use" qualifiedName [ "as" identifier ] ";"
                | "use" identifier "::" "{" useItem { "," useItem } [ "," ] "}" ";" ;
useItem         = identifier [ "as" identifier ] ;
qualifiedName   = identifier { "::" identifier } ;
item            = functionDecl | testDecl | typeDecl | enumDecl | traitDecl | implDecl ;

functionDecl    = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] block ;
testDecl        = "test" string block ;
typeDecl        = "type" identifier "{" [ typeField { "," typeField } [ "," ] ] "}" ;
enumDecl        = "enum" identifier "{" [ enumVariant { "," enumVariant } [ "," ] ] "}" ;
enumVariant     = identifier [ "(" [ typeField { "," typeField } [ "," ] ] ")" ] ;
typeField       = identifier [ ":" typeUnion ] ;
traitDecl       = "trait" identifier "{" { traitMethod } "}" ;
traitMethod     = "fn" identifier "(" parameters? ")" [ "->" typeUnion ] ( ";" | block ) ;
implDecl        = "impl" identifier [ "for" identifier ] "{" { functionDecl } "}" ;
closure         = "(" closureParameters? ")" "=>" block ;
parameters      = parameter { "," parameter } [ "," ] ;
closureParameters = closureParameter { "," closureParameter } [ "," ] ;
closureParameter = identifier [ "..." ] ;
parameter       = identifier [ ":" typeUnion ] [ "..." ] ;
typeUnion       = typeName { "|" typeName } ;
typeName        = qualifiedName | "None" | "Error" ;

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
comparison      = term { ( "<" | "<=" | ">" | ">=" | "is" ) term } ;
term            = factor { ( "+" | "-" ) factor } ;
factor          = unary { ( "*" | "/" ) unary } ;
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
matchArm        = ( qualifiedName [ "(" identifiers? ")" ] | "_" ) "=>" ( expression | block ) ;
parExpr         = "par" "{" { expression ";" } "}" | "par" "(" expression ")" ;
hostCall        = "Host" "::" "call" "(" arguments? ")" ;
listLiteral     = "[" [ arguments ] "]" ;
objectLiteral   = "{" [ objectItems ] "}" ;
objectItems     = objectItem { "," objectItem } [ "," ] ;
objectItem      = ( identifier | string ) ":" expression ;
```

Assignment targets must be identifiers, property accesses, or index accesses.
