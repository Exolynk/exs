# Enum `Abc`

```exs
enum Abc {
    A,
    B,
    C,
}
```

## Variants

### `A`

### `B`

### `C`

## Implemented Methods

### Inherent methods

#### `ts`

```exs
fn ts(self) { ... }
```

### Trait [`Add`](../../std/traits/add.md)

#### `add`

Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.

```exs
fn add(self, other: Any) -> Any { ... }
```

