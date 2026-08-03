# Trait `std::Div`

`Div` is the protocol selected by `left / right` when the left operand is a nominal type or enum with an `impl Div` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.div(other)` and `value / other` have identical behavior. Built-in division always returns Float and follows IEEE 754 behavior for zero divisors.

## Required Method

### `div`

```exs
fn div(self, other: Any) -> Any;
```

Divides the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `/` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.div(other)`. Built-in division always returns Float and follows IEEE 754 behavior for zero divisors.

## Built-in Implementations

- [`std::Bool`](../types/bool.md)
- [`std::Int`](../types/int.md)
- [`std::Float`](../types/float.md)

## Usage

```exs
type Ratio { value: Float }

impl Div for Ratio {
    fn div(self, other: Any) -> Any {
        ret Ratio { value: self.value / other.value };
    }
}

fn main() -> Float {
    ret (Ratio { value: 84.0 } / Ratio { value: 2.0 }).value;
}
```
