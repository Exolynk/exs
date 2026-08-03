# Type `std::String`

`String` is an immutable UTF-8 sequence. Indexing and `length()` operate on Unicode scalar values rather than UTF-8 byte positions.

```exs
type String
```

## Usage

```exs
fn main() -> String {
    let greeting = "Hello";
    ret greeting[0];
}
```

## Implemented Methods

### Trait [`Add`](../traits/add.md)

#### `add`

Adds the receiver to the evaluated `other` operand. Implementations may return any ExS value, including a recoverable Error. The `+` operator selects this method for matching nominal receivers; built-in Add implementations expose the same behavior through `value.add(other)`.

```exs
fn add(self, other: Any) -> Any { ... }
```

### `length() -> Int`

Returns the number of Unicode scalar values in the String. This is not the UTF-8 byte length, so a single emoji scalar counts as one.

```exs
fn main() {
    let symbol = "🙂";
    let count = symbol.length(); // 1
}
```

### `is_empty() -> Bool`

Returns true when the String contains no Unicode scalar values and false otherwise. It does not trim or normalize the String.

```exs
fn main() {
    let input = "";
    if input.is_empty() {
        host.call("println", "missing input");
    }
}
```

