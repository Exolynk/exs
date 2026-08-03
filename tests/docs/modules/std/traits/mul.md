# Trait `std::Mul`

`Mul` is the protocol selected by `left * right` when the left operand is a nominal type or enum with an `impl Mul` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.mul(other)` and `value * other` have identical behavior.

## Required Method

### `mul`

```exs
fn mul(self, other: Any) -> Any;
```

Multiplies the receiver by the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `*` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.mul(other)`.

## Built-in Implementations

- [`std::Bool`](../types/bool.md)
- [`std::Int`](../types/int.md)
- [`std::Float`](../types/float.md)

## Usage

```exs
type Scale { value: Int }

impl Mul for Scale {
    fn mul(self, other: Any) -> Any {
        ret Scale { value: self.value * other.value };
    }
}

fn main() -> Int {
    ret (Scale { value: 6 } * Scale { value: 7 }).value;
}
```
