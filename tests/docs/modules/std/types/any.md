# Type `std::Any`

`Any` accepts every ExS value. It is the implicit contract when a parameter or return annotation is omitted, and is useful when a function deliberately forwards values without narrowing their type.

```exs
type Any
```

## Usage

```exs
fn main(value: Any) -> Any {
    ret value;
}
```

## Implemented Methods

### `clone() -> Any | Error`

Creates a synchronous deep copy of the reachable mutable value graph. Lists, Objects, nominal values, enum payloads, Errors, Cells, and Closures are copied while preserving aliases and cycles inside the copy; immutable values such as None, Bool, Int, Float, and String are reused. A reachable host-owned resource returns `CloneError`, and clone never mutates the source graph.

```exs
fn main() {
    let original = [[1]];
    let copy = original.clone();
    copy[0].push(2);
    ret [original[0].length(), copy[0].length()];
}
```

