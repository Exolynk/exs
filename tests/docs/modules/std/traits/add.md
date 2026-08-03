# Trait `std::Add`

`Add` is the protocol selected by `left + right` when the left operand is a nominal type or enum with an `impl Add` implementation. The implementation receives the unmodified right operand as `Any` and may return any ExS value, including a recoverable Error. Built-in Bool, Int, Float, String, and List values implement the same protocol, so `value.add(other)` and `value + other` have identical behavior. String receivers concatenate String, Bool, Int, and Float operands using their normal source spelling.

## Required Method

### `add`

```exs
fn add(self, other: Any) -> Any;
```

Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.

## Built-in Implementations

- [`std::Bool`](../types/bool.md)
- [`std::Int`](../types/int.md)
- [`std::Float`](../types/float.md)
- [`std::String`](../types/string.md)
- [`std::List`](../types/list.md)

## Usage

```exs
type Vector { value: Int }

impl Add for Vector {
    fn add(self, other: Any) -> Any {
        ret Vector { value: self.value + other.value };
    }
}

fn main() -> Int {
    ret (Vector { value: 20 } + Vector { value: 22 }).value;
}
```
