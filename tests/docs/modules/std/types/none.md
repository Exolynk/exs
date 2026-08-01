# Type `std::None`

`None` is the single absence value. It is used for missing optional results, empty mutations, and Object reads whose key does not exist; ExS has no `null` source literal.

```exs
type None
```

## Usage

```exs
fn main() -> None {
    let absent = None;
    ret absent;
}
```
