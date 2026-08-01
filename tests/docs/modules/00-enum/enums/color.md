# Enum `Color`

Test Color enum

```exs
enum Color {
    Rgb(r: Int, g: Int, b: Int),
    Name(value),
    Transparent,
}
```

## Variants

### `Rgb`

### `Name`

### `Transparent`

## Implemented Methods

### Inherent methods

#### `new_trans`

```exs
fn new_trans() -> Color { ... }
```

#### `as_number`

```exs
fn as_number(self) -> Int { ... }
```

