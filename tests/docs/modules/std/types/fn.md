# Type `std::Fn`

`Fn` is the callable closure contract used in annotations. A closure captures lexical bindings and can be called with its declared parameter count.

```exs
type Fn
```

## Usage

```exs
fn apply(function: Fn, value: Int) -> Int {
    ret function(value);
}

fn main() -> Int {
    let increment = (value) => { ret value + 1; };
    ret apply(increment, 1);
}
```
