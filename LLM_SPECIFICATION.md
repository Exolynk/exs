# Exolynk Script (ExS) LLM Specification

**Language version:** 0.1.0-draft

This compact authoring reference is derived from [`SPECIFICATION.md`](SPECIFICATION.md), which
remains authoritative. The generated Compact API Reference supplies all standard-library and
application signatures, types, traits, and enum variants.

# Program and Lexical Rules

- A UTF-8 `.exs` file is one module. `LF` and `CRLF` are equivalent. Module scope permits imports,
  `use`, declarations, and tests, but not executable statements or `let`.
- The root module declares exactly one `fn main(...)`; imported modules never declare `main`.
  Every root-module function is externally callable by name. Statements end with `;`; blocks and
  declarations do not.
- Comments are `//` to end of line or non-nesting `/* ... */`. Adjacent `///` comments document a
  declaration. Identifiers are case-sensitive ASCII names beginning with `_` or a letter and then
  containing letters, digits, or `_`.
- Reserved words: `as break continue else enum Error false fn for host if impl import in is let
  match None par ret trait true type use while`.
- `Int` is signed 64-bit. Decimal Int literals allow `_` and must fit `-2^63..2^63-1`; leading `-`
  is an operator. `Float` is IEEE-754 binary64 and its literal has a decimal point, exponent, or
  both.
- Strings use `"` with `\"`, `\\`, `\n`, `\r`, `\t`, `\0`, and `\u{HEX}` escapes. Formatted strings use
  `f` and interpolate `{expression}` through `ToString::to_string`; `{{` and `}}` are literal
  braces. `b` Strings become decoded UTF-8 bytes. `r` hash-delimited Strings do not decode
  escapes; `f` raw Strings interpolate; `d` raw Strings dedent multiline content; `fd` combines
  dedenting and interpolation.
- Lists are mutable ordered values: `[item, ...]`. Objects are mutable insertion-ordered,
  String-keyed mappings: `{ key: value, "string-key": value }`. Unquoted Object keys become
  Strings.

# Values, Contracts, and Bindings

- Values are `None`, `Bool`, `Int`, `Float`, `String`, `Bytes`, `List`, `Object`, `Range`, `Error`,
  `Fn`, nominal-type values, and enum values. `None` is the only absence value; there is no `null`.
- Contracts use union syntax: `String | Int | None`. Built-ins are `Any`, `None`, `Error`, `Bool`,
  `Int`, `Float`, `String`, `Bytes`, `List`, `Object`, `Range`, and `Fn`; user nominal types, enums,
  and traits are valid too. `std::` qualification is optional. Missing annotations mean `Any`.
- Contracts apply at function entry and explicit or implicit return. A failure is recoverable
  `TypeError` if the return contract includes `Error` or is omitted; otherwise it is fatal.
- `let name = value;` makes a mutable lexical binding; `let name;` stores `None`. Inner scopes may
  shadow. Valid assignment targets are local identifiers, Object properties, and List/Object
  indexes. Closures share captured bindings.
- Calls evaluate arguments left to right; method receivers evaluate first. A non-callable value is
  `TypeError`; an invalid argument count is `ArityError`. List/Bytes indexing needs nonnegative
  Int; Bytes return Int octets. Object keys must be Strings. Invalid access is `IndexError` or
  `TypeError`; a missing Object key/property is `None`. Strings are not indexable.

# Expressions and Control Flow

- Evaluation is left to right except `&&` and `||`, which short-circuit and require Bool. `!`
  accepts Bool. Unary `-` accepts Bool, Int, or Float. Bool promotes to Int (`false = 0`, `true = 1`)
  for numeric operations.
- `+`, `-`, and `*` accept numeric values; an all-Int result is Int, otherwise Float. Int overflow
  is `IntOverflowError`. `/` returns Float and follows IEEE-754 zero-division behavior. `%` does
  not exist. Exact Int division uses `div_euclid` and `rem_euclid`; zero is
  `DivisionByZeroError`, and smallest-Int divided by `-1` is `IntOverflowError`.
- Nominal types/enums dispatch arithmetic through `Add`, `Sub`, `Mul`, and `Div`. Built-in String
  concatenates String, Bool, Int, or Float; List addition shallowly appends or concatenates.
- `==` and `!=` compare scalars by value, Strings by contents, and Lists, Objects, closures, and
  Errors by identity. Numeric equality promotes Bool/Int and Float. Ordering supports numbers and
  Strings; unordered Float (including NaN) is `TypeError`; nominal values may implement `Compare`.
- `value is Type` returns Bool. `clone()` deeply clones mutable graphs, preserves aliases/cycles,
  and does not mutate its source. Postfix `?` propagates Error, turns None into `MissingValue`, and
  otherwise leaves the value unchanged; it requires an Error-capable or omitted return contract.
- Blocks create lexical scopes. `if`, `else if`, `else`, and `while` require Bool conditions.
  `ret;` and falling off a function return None. `break;` and `continue;` only apply inside loops.
- `for item in iterable` evaluates once and advances `Iterator::next() -> IteratorStep | Error`.
  Lists iterate a shallow snapshot; Strings yield scalar Strings; Bytes yield Int octets. Ranges are
  `start..end` (half-open), `start..=end` (inclusive), `Range::step`, or `Range::step_through`; a
  step is nonzero and moves toward its endpoint. Other iterables implement `Iterator`; otherwise
  `NotIterable`. Each iteration gets a fresh loop binding.

# Functions, Types, and Traits

- Parameters are positional; there are no defaults, named arguments, or keyword arguments.
  Duplicate names are compile errors. Named functions are module-visible and recursive.
- One final parameter may be variadic (`name...` or `name: Type...`), receives a List, may be
  empty, and applies its contract to each supplied trailing argument. Normal calls provide all fixed
  arguments. Hosts may omit fixed root-function inputs (they become None); excess inputs to a
  non-variadic root are fatal `ArityError`.
- Closures use `(parameters) => { ... }`; their parameters cannot be annotated. They capture,
  return, store, and call as `Fn` values.
- `type Name { field: Contract }` creates a nominal Object. Every field exists after construction;
  omitted fields become None only under `Any` or a None-capable contract. Invalid, missing required,
  unknown, or duplicate fields are `TypeError`. `impl Name` supplies inherent methods; a first bare
  `self` is an instance method, otherwise static.
- `enum Name { Variant(field: Contract), Empty }` has qualified constructors. Payloads are checked
  in order. `match` evaluates its input once and only one arm. Variant patterns use one enum and
  its exact payload count; `_` is last. Without `_`, every variant appears once.
- Trait methods ending `;` are required; those with bodies are defaults. Implementations provide
  required methods and may omit defaults. Trait and nominal type names share a namespace. `Self` is
  valid only in a trait signature and an `impl Trait for Type` method.
- `Add`, `Sub`, `Mul`, `Div`, `Compare`, `ToString`, and `Debug` are reserved with the exact
  signatures listed in the API reference. `Ordering` variants are `Less`, `Equal`, `Greater`, and
  `Unordered`. Interpolation uses `ToString`; `Debug` is an explicit diagnostic renderer.

# Errors, Parallelism, Modules, and Hosts

- Failures are ordinary `Error` values; there are no exceptions, `try`, `catch`, or `throw`.
  `Error(kind, message, data)` requires String `kind` and `message`; `data` is arbitrary. Common
  kinds: `ArityError`, `CloneError`, `EncodingError`, `IndexError`, `IntOverflowError`, `MatchError`,
  `MethodNotFound`, `MissingValue`, `NotIterable`, `SerializationError`, `TypeError`, `ValueError`,
  and `HostFunctionNotFound`.
- `par { expression; ... }` creates one task per expression and returns source-ordered results;
  empty is `[]`. `par(list_of_zero_argument_closures)` is dynamic. Tasks share values/mutations;
  Error is a normal result; parents wait for direct children. Host completion can be nondeterministic;
  other scheduling is deterministic.
- `import "./path.exs" as namespace;` loads a relative module; the filename supplies an omitted
  namespace. `use namespace::{name as alias};` creates read-only imported names. Imports/uses come
  before declarations and cycles are errors.
- `Host::call(name, arguments...)` calls a host-provided operation, may suspend, and creates no ExS
  task. The embedding host defines available names, contracts, capabilities, and side effects.
- `Host::sleep(duration)` may suspend. Duration is nonnegative normalized `seconds: Int` and
  `nanoseconds: Int` (0 through 999999999); factories/conversions are in the API reference.
- `Host::stream(name, arguments...)` opens a single-consumer pull stream. `next()` may suspend; a
  concurrent second `next()` is Error. Done, cancellation, or root completion closes the stream.
- Host-call arguments and final results use an acyclic by-value boundary. Closures, cells, and
  cyclic graphs cannot cross it; shared references duplicate. Invalid host arguments are recoverable
  `SerializationError`; invalid final results are fatal `SerializationError`.

# Tests

- `test "description" { ... }` is run by `exs test` and excluded from `exs run`/`exs compile`.
- `assert(condition[, description])` requires Bool. `assert_eq(actual, expected[, description])`
  uses ExS equality. Failures are fatal `AssertionFailed`; `assert_eq` records actual/expected.

# Grammar

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
comparison      = range { ( "<" | "<=" | ">" | ">=" | "is" ) range } ;
range           = term [ ( ".." | "..=" ) term ] ;
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

Assignment targets are identifiers, property accesses, or index accesses.
