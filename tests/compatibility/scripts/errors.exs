//! Errors are ordinary values, propagated by `?` and raised by invalid operations.

fn failing() -> Int | Error {
    ret Error("Custom", "it failed", 7);
}

fn propagate() {
    let value = failing()?;
    ret value + 1;
}

fn absent(object) -> Int | None {
    let value = object.value?;
    ret value + 1;
}

fn main() {
    let error = propagate();
    let zero = 0;
    let large = 9223372036854775807;
    let not_callable = 1;
    ret [
        error is Error,
        error.kind(),
        error.message(),
        error.data(),
        absent({}),
        absent({ value: 1 }),
        (1 % zero).kind(),
        ([1][5]).kind(),
        (large + 1).kind(),
        (not_callable()).kind(),
    ];
}

/// Ends with a recoverable Error.
fn recoverable() {
    ret Error("Custom", "it failed", 7);
}

/// Breaks its return contract, which has no Error, and so ends with a fatal Error.
fn fatal() -> Int {
    let value = "text";
    ret value;
}

/// Ends with an Error that crossed a call, which records its origin and trace.
fn traced() {
    ret propagate();
}

/// Reads an Error from the host, with its data and cause.
fn from_host() {
    let error = Host::call("fail");
    ret [error.kind(), error.message(), error.data(), error.cause().kind()];
}
