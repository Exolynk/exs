# Type `std::Bool`

`Bool` has exactly the values `true` and `false`. Conditions require Bool explicitly; ExS does not apply implicit truthiness to numbers, strings, collections, or None.

```exs
type Bool
```

## Usage

```exs
fn main() {
    let ready = true;
    if ready {
        host.call("println", "ready");
    }
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

