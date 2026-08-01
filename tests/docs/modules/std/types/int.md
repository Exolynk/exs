# Type `std::Int`

`Int` is a signed 56-bit exact integer. It supports numeric operators and reports `IntOverflowError` when an operation cannot produce a value inside the ExS integer range.

```exs
type Int
```

## Usage

```exs
fn main() -> Int {
    let quantity = 42;
    ret quantity + 8;
}
```

## Implemented Methods

### `abs() -> Int`

Returns the non-negative magnitude of the receiver. The smallest representable Int has no representable positive counterpart, so calling `abs()` on it returns `IntOverflowError`.

```exs
fn main() {
    let change = -42;
    let magnitude = change.abs(); // 42
}
```

