# Function `host.call`

```exs
host.call(name, arguments...)
```

Invokes a runner-registered host function selected by a runtime String name. Arguments are collected into a List and transported through the runner CBOR boundary. A call may complete immediately or suspend; it returns the host result or a recoverable Error such as `HostFunctionNotFound`.

## Usage

```exs
fn main() {
    let greeting = host.call("greet", "Ada");
    ret greeting;
}
```
