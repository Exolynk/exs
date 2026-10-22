//! Imported runner controls and native test stubs.

#[cfg(not(target_arch = "wasm32"))]
use exs_abi::HOST_CALL_FATAL;

#[cfg(all(test, not(target_arch = "wasm32")))]
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

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

/// Calls the imported Host ABI start function.
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_call_start(
    call_id: i64,
    name_pointer: *const u8,
    name_length: i32,
    request_pointer: *const u8,
    request_length: i32,
    source_position: i32,
) -> i32 {
    // SAFETY: the runner reads the name and request from guest memory the caller keeps alive.
    unsafe {
        host_call_start_import(
            call_id,
            name_pointer,
            name_length,
            request_pointer,
            request_length,
            source_position,
        )
    }
}

/// Stands in for the Host ABI start function natively: fatal, or the status tests configure.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_call_start(
    call_id: i64,
    name_pointer: *const u8,
    name_length: i32,
    request_pointer: *const u8,
    request_length: i32,
    source_position: i32,
) -> i32 {
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
        TEST_HOST_CALL_START_COUNT.fetch_add(1, Ordering::SeqCst);
        TEST_HOST_CALL_STATUS.load(Ordering::SeqCst)
    }
    #[cfg(not(test))]
    {
        HOST_CALL_FATAL
    }
}

/// Calls the imported immediate-response length function.
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_call_response_len(call_id: i64) -> i32 {
    // SAFETY: the import only reads the runner's response table.
    unsafe { host_call_response_len_import(call_id) }
}

/// Stands in for the immediate-response length function natively: no response.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_call_response_len(call_id: i64) -> i32 {
    let _ignored = call_id;
    -1
}

/// Calls the imported immediate-response copy function.
#[cfg(target_arch = "wasm32")]
pub(crate) fn host_call_response_copy(call_id: i64, pointer: *mut u8, length: i32) -> i32 {
    // SAFETY: the caller passes a guest buffer of `length` writable bytes.
    unsafe { host_call_response_copy_import(call_id, pointer, length) }
}

/// Stands in for the immediate-response copy function natively: the copy fails.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn host_call_response_copy(call_id: i64, pointer: *mut u8, length: i32) -> i32 {
    let _ignored = (call_id, pointer, length);
    1
}

/// Calls the imported task-acquire function.
#[cfg(target_arch = "wasm32")]
pub(crate) fn task_acquire() -> i32 {
    // SAFETY: the import only updates the runner's task count.
    unsafe { task_acquire_import() }
}

/// Stands in for the task-acquire function natively: always granted.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn task_acquire() -> i32 {
    0
}

/// Calls the imported task-release function.
#[cfg(target_arch = "wasm32")]
pub(crate) fn task_release() -> i32 {
    // SAFETY: the import only updates the runner's task count.
    unsafe { task_release_import() }
}

/// Stands in for the task-release function natively: always released.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn task_release() -> i32 {
    0
}

/// Configures the native Host ABI stub used by unit tests.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn set_test_host_call_status(status: i32) {
    TEST_HOST_CALL_STATUS.store(status, Ordering::SeqCst);
    TEST_HOST_CALL_START_COUNT.store(0, Ordering::SeqCst);
}

/// Returns the number of Host ABI starts observed by the native test stub.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn test_host_call_start_count() -> usize {
    TEST_HOST_CALL_START_COUNT.load(Ordering::SeqCst)
}

/// Provides the native Host ABI status consumed by guest unit tests.
#[cfg(all(test, not(target_arch = "wasm32")))]
static TEST_HOST_CALL_STATUS: AtomicI32 = AtomicI32::new(HOST_CALL_FATAL);

/// Counts Host ABI starts observed by the native test stub.
#[cfg(all(test, not(target_arch = "wasm32")))]
static TEST_HOST_CALL_START_COUNT: AtomicUsize = AtomicUsize::new(0);
