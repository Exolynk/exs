# Trait `std::Sub`

`Sub` is the protocol selected by `left - right` when the left operand is a nominal type or enum with an `impl Sub` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, and Float values implement the same protocol, so `value.sub(other)` and `value - other` have identical behavior.

## Required Method

### `sub`

```exs
fn sub(self, other: Any) -> Any;
```

Subtracts the evaluated `other` operand from the receiver. Implementations may return any ExS value, including a recoverable Error. The `-` operator selects this method for matching nominal receivers; built-in numeric implementations expose the same behavior through `value.sub(other)`.

## Built-in Implementations

- [`std::Bool`](../types/bool.md)
- [`std::Int`](../types/int.md)
- [`std::Float`](../types/float.md)

## Usage

```exs
type Temperature { value: Float }

impl Sub for Temperature {
    fn sub(self, other: Any) -> Any {
        ret Temperature { value: self.value - other.value };
    }
}

fn main() -> Float {
    ret (Temperature { value: 22.5 } - Temperature { value: 2.5 }).value;
}
```
