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

### Trait [`Add`](../traits/add.md)

#### `add`

Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.

```exs
fn add(self, other: Any) -> Any { ... }
```

### Trait [`Sub`](../traits/sub.md)

#### `sub`

Subtracts the evaluated `other` operand from the receiver. Implementations may return any ExS value, including a recoverable Error. The `-` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.sub(other)`.

```exs
fn sub(self, other: Any) -> Any { ... }
```

### Trait [`Mul`](../traits/mul.md)

#### `mul`

Multiplies the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `*` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.mul(other)`.

```exs
fn mul(self, other: Any) -> Any { ... }
```

### Trait [`Div`](../traits/div.md)

#### `div`

Divides the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `/` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.div(other)`. Built-in division always returns Float and follows IEEE 754 behavior for zero divisors.

```exs
fn div(self, other: Any) -> Any { ... }
```

### `abs() -> Int`

Returns the non-negative magnitude of the receiver. The smallest representable Int has no representable positive counterpart, so calling `abs()` on it returns `IntOverflowError`.

```exs
fn main() {
    let change = -42;
    let magnitude = change.abs(); // 42
}
```

