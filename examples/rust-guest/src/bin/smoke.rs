//! Native smoke test for the Rust guest's Host stream integration.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::fs;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use exs_runner::{
        ExecutionCancellation, ExsValue, HostStream, HostStreamFuture, HostStreamItem, ServerRunner,
    };

    /// A finite integer stream registered exclusively by this example smoke test.
    struct CounterStream {
        /// Next value to yield.
        next: i64,
        /// Final inclusive value.
        end: i64,
    }

    impl HostStream for CounterStream {
        /// Yields the next integer or ends after the configured upper bound.
        fn next(&mut self) -> HostStreamFuture {
            if self.next > self.end {
                return Box::pin(core::future::ready(HostStreamItem::End));
            }
            let value = self.next;
            self.next += 1;
            Box::pin(core::future::ready(HostStreamItem::Item(ExsValue::Int(
                value,
            ))))
        }
    }

    /// Executes the guest's stream entry point against a runner-registered counter stream.
    pub fn run() -> Result<(), String> {
        let wasm = guest_wasm_path();
        let wasm = fs::read(&wasm)
            .map_err(|error| format!("could not read {}: {error}", wasm.display()))?;
        let mut runner = ServerRunner::default();
        runner
            .registry_mut()
            .stream_raw("counter", |arguments: Vec<ExsValue>| {
                let end = match arguments.as_slice() {
                    [ExsValue::Int(end)] if *end >= 0 => *end,
                    _ => 0,
                };
                Ok(CounterStream { next: 1, end })
            })
            .map_err(|error| format!("could not register counter stream: {error}"))?;
        let result =
            block_on(runner.execute(&wasm, "sum_stream", &[], &ExecutionCancellation::new()))
                .map_err(|error| format!("guest execution failed: {error}"))?;
        if result != ExsValue::Int(6) {
            return Err(format!("expected stream sum 6, received {result:?}"));
        }
        println!("guest Host stream smoke test passed");
        Ok(())
    }

    /// Returns the debug guest Wasm path built by the example test script.
    fn guest_wasm_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wasm32-unknown-unknown/debug/exs_rust_guest.wasm")
    }

    /// Polls the runner future without adding an executor dependency to the example.
    fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    native::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
