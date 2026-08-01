# Type `std::List`

`List` is a mutable ordered collection. Variables and closure captures preserve List identity, so mutations through one alias are visible through every alias of the same List.

```exs
type List
```

## Usage

```exs
fn main() -> Int {
    let items = ["Ada", "Lin"];
    ret items.push("Mia");
}
```

## Implemented Methods

### `length() -> Int`

Returns the current number of elements. The count changes immediately after List mutations such as `push`, `pop`, `insert`, `remove`, and `clear`.

```exs
fn main() {
    let items = ["Ada", "Lin"];
    let count = items.length(); // 2
}
```

### `is_empty() -> Bool`

Returns true when the List has no elements. It does not mutate the List.

```exs
fn main() {
    let queue = [];
    if queue.is_empty() {
        host.call("println", "queue is empty");
    }
}
```

### `push(value) -> Int`

Appends one value to the end of the List and returns the new element count. The operation mutates the existing List rather than allocating a replacement.

```exs
fn main() {
    let items = ["Ada"];
    let count = items.push("Lin"); // 2
}
```

### `pop() -> Any | None`

Removes and returns the final element. Calling `pop()` on an empty List leaves it unchanged and returns None.

```exs
fn main() {
    let items = ["Ada", "Lin"];
    let last = items.pop(); // "Lin"
}
```

### `insert(index: Int, value) -> None | Error`

Inserts one value before the zero-based index and returns None. The index may equal the current length to append; invalid indexes return `IndexError`.

```exs
fn main() {
    let items = ["Ada", "Mia"];
    items.insert(1, "Lin"); // ["Ada", "Lin", "Mia"]
}
```

### `remove(index: Int) -> Any | Error`

Removes and returns the element at a zero-based index. Invalid indexes return `IndexError` and leave the List unchanged.

```exs
fn main() {
    let items = ["Ada", "Lin"];
    let removed = items.remove(0); // "Ada"
}
```

### `clear() -> None`

Removes every element from the existing List and returns None. Aliases continue to refer to the now-empty same List.

```exs
fn main() {
    let items = [1, 2, 3];
    items.clear();
    let empty = items.is_empty(); // true
}
```

