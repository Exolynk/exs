#![cfg_attr(all(target_arch = "wasm32", feature = "no-std"), no_std)]

//! Rust guest support for the stable ExS runner ABI.
//!
//! The default `std` feature supports conventional Rust `cdylib` guests for
//! `wasm32-unknown-unknown`. Export linear memory and invoke [`export!`] with an
//! `async fn(Vec<ExsValue>) -> ExsValue` entry point.
//!
//! For a `no_std` guest, disable default features and enable `no-std`. That mode supplies the
//! guest allocator and panic handler:
//!
//! ```toml
//! exs-guest = { version = "*", default-features = false, features = ["no-std"] }
//! ```

#[cfg(all(feature = "std", feature = "no-std"))]
compile_error!("the `std` and `no-std` features cannot be enabled together");

extern crate alloc;

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

pub use alloc::vec::Vec;
pub use exs_abi::{ErrorSeverity, ExsError, ExsValue, SourcePositionId};

mod execution;
mod imports;
mod state;
#[cfg(test)]
mod tests;

pub mod host;

pub(crate) use execution::{begin_host_call, guest_error, take_host_response};
pub use execution::{cancel, input_alloc, result_len, result_ptr, resume_host, start};

use exs_abi::ABI_VERSION;

/// One erased asynchronous guest entry point retained between runner callbacks.
pub type GuestFuture = Pin<Box<dyn Future<Output = ExsValue>>>;

/// Exports the ExS runner ABI for one `async fn(Vec<ExsValue>) -> ExsValue` guest entry point.
#[macro_export]
macro_rules! export {
    ($entry:path $(,)?) => {
        #[unsafe(no_mangle)]
        #[doc = "Returns the ExS ABI version implemented by this guest."]
        pub extern "C" fn __exs_abi_version() -> i32 {
            $crate::abi_version()
        }
        #[unsafe(no_mangle)]
        #[doc = "Allocates one runner-writable input buffer in guest linear memory."]
        pub extern "C" fn __exs_input_alloc(length: i32) -> i32 {
            $crate::input_alloc(length)
        }
        #[unsafe(no_mangle)]
        #[doc = "Starts one ExS ABI guest execution."]
        pub extern "C" fn __exs_start(pointer: i32, length: i32) -> i32 {
            $crate::start(pointer, length, |inputs| {
                $crate::boxed_future($entry(inputs))
            })
        }
        #[unsafe(no_mangle)]
        #[doc = "Resumes one asynchronous host call through the ExS ABI."]
        pub extern "C" fn __exs_resume_host(call_id: i64, pointer: i32, length: i32) -> i32 {
            $crate::resume_host(call_id, pointer, length)
        }
        #[unsafe(no_mangle)]
        #[doc = "Cancels the active asynchronous guest execution."]
        pub extern "C" fn __exs_cancel() {
            $crate::cancel();
        }
        #[unsafe(no_mangle)]
        #[doc = "Returns the result buffer pointer for a completed execution."]
        pub extern "C" fn __exs_result_ptr() -> i32 {
            $crate::result_ptr()
        }
        #[unsafe(no_mangle)]
        #[doc = "Returns the result buffer length for a completed execution."]
        pub extern "C" fn __exs_result_len() -> i32 {
            $crate::result_len()
        }
    };
}

/// Boxes one concrete async entry future for storage behind the ABI boundary.
#[must_use]
pub fn boxed_future<F>(future: F) -> GuestFuture
where
    F: Future<Output = ExsValue> + 'static,
{
    Box::pin(future)
}

/// Returns the ABI version exported by every Rust guest.
#[must_use]
pub const fn abi_version() -> i32 {
    ABI_VERSION as i32
}
