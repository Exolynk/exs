# Type `std::Float`

`Float` uses IEEE-754 binary64 values, including infinities, signed zero, and NaN. Mixed arithmetic promotes the other numeric operand to Float.

```exs
type Float
```

## Usage

```exs
fn main() -> Float {
    let price = 19.95;
    ret price * 1.19;
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

### `abs() -> Float`

Returns the non-negative floating-point magnitude. It preserves Float semantics for signed zero, infinities, and NaN.

```exs
fn main() {
    let delta = -1.5;
    let magnitude = delta.abs(); // 1.5
}
```

### `floor() -> Float`

Rounds down to the greatest integral Float that is not greater than the receiver. The result remains Float so it composes with floating-point arithmetic.

```exs
fn main() {
    let page = 3.8;
    let first_index = page.floor(); // 3.0
}
```

### `ceil() -> Float`

Rounds up to the least integral Float that is not less than the receiver. The result remains Float.

```exs
fn main() {
    let pages = 3.2;
    let required = pages.ceil(); // 4.0
}
```

### `round() -> Float`

Rounds to the nearest integral Float. Exact halfway values are rounded away from zero, so `1.5` becomes `2.0` and `-1.5` becomes `-2.0`.

```exs
fn main() {
    let rating = 4.5;
    let displayed = rating.round(); // 5.0
}
```

