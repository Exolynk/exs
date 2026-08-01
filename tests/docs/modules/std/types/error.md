# Type `std::Error`

A structured recoverable language error value.

```exs
type Error
```

## Constructor

```exs
Error(kind, message, data)
```

Constructs a recoverable Error. `kind` and `message` must be Strings; `data` may be any value.
