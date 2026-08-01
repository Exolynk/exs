# Type `std::Object`

`Object` is a mutable insertion-ordered mapping from String keys to values. Dot properties and bracket access operate on the same ordered collection.

```exs
type Object
```

## Usage

```exs
fn main() -> Object {
    let user = { name: "Ada" };
    user.role = "admin";
    ret user;
}
```

## Implemented Methods

### `length() -> Int`

Returns the number of present keys. Replacing an existing key does not increase the count; creating or deleting a key does.

```exs
fn main() {
    let user = { name: "Ada" };
    let count = user.length(); // 1
}
```

### `is_empty() -> Bool`

Returns true when the Object has no keys. It does not mutate the Object.

```exs
fn main() {
    let options = {};
    if options.is_empty() {
        host.call("println", "using defaults");
    }
}
```

### `has(key: String) -> Bool | Error`

Returns whether a String key is present. A non-String key returns `TypeError` rather than coercing the key.

```exs
fn main() {
    let user = { name: "Ada" };
    let has_name = user.has("name"); // true
}
```

### `delete(key: String) -> Any | None | Error`

Removes a String key and returns its previous value. When the key is absent, it returns None; a non-String key returns `TypeError`.

```exs
fn main() {
    let user = { name: "Ada" };
    let name = user.delete("name"); // "Ada"
}
```

### `keys() -> List`

Returns a new List of String keys in insertion order. Changing the returned List does not change the Object.

```exs
fn main() {
    let user = { name: "Ada", role: "admin" };
    let keys = user.keys(); // ["name", "role"]
}
```

### `values() -> List`

Returns a new shallow List of values in the same insertion order as `keys()`. The values themselves retain their original identity.

```exs
fn main() {
    let user = { name: "Ada", role: "admin" };
    let values = user.values(); // ["Ada", "admin"]
}
```

