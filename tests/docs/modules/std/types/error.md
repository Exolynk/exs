# Type `std::Error`

`Error` is a structured language failure value. Operations return it instead of throwing, and functions that may return an Error should include `Error` in their return contract or use the implicit `Any` contract.

```exs
type Error
```

## Usage

```exs
fn main() -> Int | Error {
    ret Error("DivisionByZeroError", "cannot divide by zero", None);
}
```

## Implemented Methods

### `kind() -> String`

Returns the stable machine-readable category assigned when the Error was created. Use it to distinguish expected failures without parsing a human-facing message.

```exs
fn main() {
    let error = Error("MissingValue", "value is required", None);
    let category = error.kind();
}
```

### `message() -> String`

Returns the human-readable explanation stored in the Error. The message is intended for diagnostics and user-facing reporting, not control-flow classification.

```exs
fn main() {
    let error = Error("MissingValue", "value is required", None);
    let text = error.message();
}
```

### `data() -> Any`

Returns the language value attached to the Error. Runtime operations use this to retain the invalid input, index, or other relevant context that caused the failure.

```exs
fn main() {
    let error = Error("InvalidInput", "age must be positive", -1);
    let invalid_age = error.data();
}
```

### `cause() -> Error | None`

Returns a related prior Error or value when one is present. Errors created directly with `Error(...)` have no cause and therefore return None.

```exs
fn main() {
    let error = Error("Example", "no prior failure", None);
    let previous = error.cause();
}
```


## Constructor

```exs
Error(kind, message, data)
```

Constructs a recoverable Error with a stable category, a human-readable message, and any related language value. `kind` and `message` must be Strings. The constructor does not assign a cause; `cause()` consequently returns None for directly constructed Errors.

```exs
fn main() {
    let error = Error("InvalidInput", "age must be positive", -1);
    ret error.message();
}
```
