//! The secure random source of the platform.

/// Fills `buffer` with random bytes from the operating system.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn fill(buffer: &mut [u8]) -> Result<(), String> {
    getrandom::fill(buffer).map_err(|error| error.to_string())
}

/// Fills `buffer` with random bytes from the browser's `crypto.getRandomValues`.
#[cfg(target_arch = "wasm32")]
pub(super) fn fill(buffer: &mut [u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    /// Most bytes `getRandomValues` fills in one call.
    const MAX_CHUNK: usize = 65_536;

    let crypto = js_sys::Reflect::get(&js_sys::global(), &"crypto".into())
        .map_err(|_| String::from("the global object has no crypto"))?;
    let get_random_values = js_sys::Reflect::get(&crypto, &"getRandomValues".into())
        .ok()
        .and_then(|function| function.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| String::from("crypto.getRandomValues is not available"))?;
    for chunk in buffer.chunks_mut(MAX_CHUNK) {
        let length = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
        let array = js_sys::Uint8Array::new_with_length(length);
        get_random_values
            .call1(&crypto, &array)
            .map_err(|_| String::from("crypto.getRandomValues failed"))?;
        array.copy_to(chunk);
    }
    Ok(())
}

/// Returns a random `u64`.
pub(super) fn next_u64() -> Result<u64, String> {
    let mut bytes = [0_u8; 8];
    fill(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
