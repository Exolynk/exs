# Type `std::Bool`

`Bool` has exactly the values `true` and `false`. Conditions require Bool explicitly; ExS does not apply implicit truthiness to numbers, strings, collections, or None.

```exs
type Bool
```

## Usage

```exs
fn main() {
    let ready = true;
    if ready {
        host.call("println", "ready");
    }
}
```
