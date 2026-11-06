//! Parallel tasks, host calls, sleeping and host streams.

fn slow(value) {
    Host::sleep(Duration::milliseconds(1)?);
    ret value * 2;
}

fn main() {
    let results = par {
        slow(1);
        slow(2);
        Host::call("later", 3);
    };
    let dynamic = par([() => 10, () => slow(5)]);
    let stream = Host::stream("counter", 3)?;
    let total = 0;
    for value in stream {
        total = total + value;
    }
    ret [
        results,
        dynamic,
        total,
        Host::call("echo", { name: "Ada", values: [1, 2.5, b"x", None] }),
        Host::call("missing").kind(),
    ];
}

/// Suspends on a host call, during which a runner can cancel the execution.
fn waits(value) {
    ret Host::call("echo", value);
}
