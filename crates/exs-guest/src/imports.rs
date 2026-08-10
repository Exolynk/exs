//! Imported runner controls and native test stubs.

#[cfg(not(target_arch = "wasm32"))]
use exs_abi::HOST_CALL_FATAL;

#[cfg(all(test, not(target_arch = "wasm32")))]
use core::sync::atomic::{AtomicI32, Ordering};

// Imports the runner host and task controls on the `wasm32` guest target.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "exs")]
unsafe extern "C" {
    /// Starts one Host ABI request.
    #[link_name = "__exs_host_call_start"]
    fn host_call_start_import(
        call_id: i64,
        name_pointer: *const u8,
        name_length: i32,
        request_pointer: *const u8,
        request_length: i32,
        source_position: i32,
    ) -> i32;
    /// Returns one immediate host-response byte length.
    #[link_name = "__exs_host_call_response_len"]
    fn host_call_response_len_import(call_id: i64) -> i32;
    /// Copies one immediate host response into guest linear memory.
    #[link_name = "__exs_host_call_response_copy"]
    fn host_call_response_copy_import(call_id: i64, pointer: *mut u8, length: i32) -> i32;
}

// Imports the mandatory runner task meter on the `wasm32` guest target.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "runner")]
unsafe extern "C" {
    /// Acquires one runner task permit.
    #[link_name = "__runner_task_acquire"]
    fn task_acquire_import() -> i32;
    /// Releases one runner task permit.
    #[link_name = "__runner_task_release"]
    fn task_release_import() -> i32;
}

/// Calls the imported Host ABI start function or a native test stub.
pub(crate) fn host_call_start(
    call_id: i64,
    name_pointer: *const u8,
    name_length: i32,
    request_pointer: *const u8,
    request_length: i32,
    source_position: i32,
) -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return host_call_start_import(
            call_id,
            name_pointer,
            name_length,
            request_pointer,
            request_length,
            source_position,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ignored = (
            call_id,
            name_pointer,
            name_length,
            request_pointer,
            request_length,
            source_position,
        );
        #[cfg(test)]
        {
            TEST_HOST_CALL_STATUS.load(Ordering::SeqCst)
        }
        #[cfg(not(test))]
        {
            HOST_CALL_FATAL
        }
    }
}

/// Calls the imported immediate-response length function or a native test stub.
pub(crate) fn host_call_response_len(call_id: i64) -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return host_call_response_len_import(call_id);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ignored = call_id;
        -1
    }
}

/// Calls the imported immediate-response copy function or a native test stub.
pub(crate) fn host_call_response_copy(call_id: i64, pointer: *mut u8, length: i32) -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return host_call_response_copy_import(call_id, pointer, length);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ignored = (call_id, pointer, length);
        1
    }
}

/// Calls the imported task-acquire function or a native test stub.
pub(crate) fn task_acquire() -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return task_acquire_import();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// Calls the imported task-release function or a native test stub.
pub(crate) fn task_release() -> i32 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        return task_release_import();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// Configures the native Host ABI stub used by unit tests.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn set_test_host_call_status(status: i32) {
    TEST_HOST_CALL_STATUS.store(status, Ordering::SeqCst);
}

/// Provides the native Host ABI status consumed by guest unit tests.
#[cfg(all(test, not(target_arch = "wasm32")))]
static TEST_HOST_CALL_STATUS: AtomicI32 = AtomicI32::new(HOST_CALL_FATAL);
