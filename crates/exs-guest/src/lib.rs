#![cfg_attr(all(target_arch = "wasm32", not(feature = "std")), no_std)]

//! Write ExS extensions in Rust: a guest is a WebAssembly module with `async` entry points that
//! any ExS runner executes, and that reaches its application through [`host`].
//!
//! ```no_run
//! use exs_guest::{ExsError, ExsValue, host};
//!
//! async fn greet(inputs: Vec<ExsValue>) -> Result<ExsValue, ExsError> {
//!     let Some(ExsValue::String(name)) = inputs.into_iter().next() else {
//!         return Err(ExsError::recoverable("TypeError", "greet takes a name"));
//!     };
//!     let now = host::now().await?;
//!     host::call("log", [ExsValue::String(format!("{name} at {}", now.unix_seconds))]).await;
//!     Ok(ExsValue::String(format!("Hello, {name}!")))
//! }
//!
//! exs_guest::export!(greet);
//! ```
//!
//! Build it as a `cdylib` for `wasm32-unknown-unknown`. An entry point returns [`ExsValue`], or a
//! `Result<ExsValue, ExsError>` so that `?` works. A runner executes a guest as long as it supports
//! the ABI version the guest was built with.
//!
//! # Without `std`
//!
//! A guest without the default `std` feature is `no_std` and uses `alloc` for its collections;
//! this crate then supplies the allocator and a panic handler that traps:
//!
//! ```toml
//! exs-guest = { version = "0.2", default-features = false }
//! ```
//!
//! # Typed values
//!
//! The `serde` feature adds `decode` and `encode`, which convert between [`ExsValue`] and Rust
//! types that implement the Serde traits, and `Bytes` for a field that crosses as a byte buffer.

extern crate alloc;

#[cfg(feature = "serde")]
pub use exs_abi::Bytes;
pub use exs_abi::value::ExsStackFrame;
pub use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};
#[cfg(feature = "serde")]
pub use typed::{decode, encode};

mod execution;
mod imports;
mod state;
#[cfg(test)]
mod tests;
#[cfg(feature = "serde")]
mod typed;

pub mod host;

pub(crate) use execution::{begin_host_call, guest_error, take_host_response};

/// What [`export!`] expands to; not part of the public API.
#[doc(hidden)]
pub mod __private {
    use alloc::boxed::Box;
    use core::future::Future;
    use core::pin::Pin;

    use exs_abi::ABI_VERSION;

    use crate::ExsValue;

    pub use crate::execution::{cancel, input_alloc, result_image, resume_host, start};

    /// One erased asynchronous guest entry point retained between runner callbacks.
    pub type GuestFuture = Pin<Box<dyn Future<Output = ExsValue>>>;

    /// Boxes one concrete async entry future for storage behind the ABI boundary.
    pub fn boxed_future<F>(future: F) -> GuestFuture
    where
        F: Future + 'static,
        F::Output: Into<ExsValue>,
    {
        Box::pin(async move { future.await.into() })
    }

    /// Returns the ABI version exported by every Rust guest.
    #[must_use]
    pub const fn abi_version() -> i32 {
        ABI_VERSION as i32
    }
}

/// Exports the ExS runner ABI for one or more named asynchronous guest entry points.
///
/// Each supplied top-level function identifier becomes callable through the runner under the
/// same name. For example, `export!(main, add)` lets a runner execute `main` and `add`. Each must
/// be an `async fn(Vec<ExsValue>)` returning a value that converts into [`ExsValue`], and a crate
/// invokes the macro once.
#[macro_export]
macro_rules! export {
    ($($entry:ident),+ $(,)?) => {
        #[unsafe(no_mangle)]
        #[doc = "Returns the ExS ABI version implemented by this guest."]
        pub extern "C" fn __exs_abi_version() -> i32 {
            $crate::__private::abi_version()
        }
        #[unsafe(no_mangle)]
        #[doc = "Allocates one runner-writable input buffer in guest linear memory."]
        pub extern "C" fn __exs_input_alloc(length: i32) -> i32 {
            $crate::__private::input_alloc(length)
        }
        $(
            const _: () = {
                #[unsafe(export_name = concat!("__exs_start_", stringify!($entry)))]
                #[doc = concat!("Starts the ExS ABI guest entry point `", stringify!($entry), "`.")]
                pub extern "C" fn entry(pointer: i32, length: i32) -> i32 {
                    $crate::__private::start(pointer, length, |inputs| {
                        $crate::__private::boxed_future($entry(inputs))
                    })
                }
            };
        )+
        #[unsafe(no_mangle)]
        #[doc = "Resumes one asynchronous host call through the ExS ABI."]
        pub extern "C" fn __exs_resume_host(call_id: i64, pointer: i32, length: i32) -> i32 {
            $crate::__private::resume_host(call_id, pointer, length)
        }
        #[unsafe(no_mangle)]
        #[doc = "Cancels the active asynchronous guest execution."]
        pub extern "C" fn __exs_cancel() {
            $crate::__private::cancel();
        }
        #[unsafe(no_mangle)]
        #[doc = "Returns the address and length of the result image of a completed execution."]
        pub extern "C" fn __exs_result_image() -> i64 {
            $crate::__private::result_image()
        }
    };
}
