//! Requests sent with `fetch` in browsers.
//!
//! The browser follows redirects itself, so [`super::MAX_REDIRECTS`] does not apply, and CORS
//! decides which responses scripts may read.

use js_sys::{Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortSignal, Headers, Request, RequestInit, Response};

use super::transport::{HttpRequest, HttpResponse, MAX_RESPONSE_BYTES};

/// Sends `request` with the global `fetch` of the window or worker.
pub(super) async fn send(request: HttpRequest) -> Result<HttpResponse, String> {
    let response = fetch(&request).await?;
    let status = response.status();
    let headers = read_headers(&response.headers())?;
    let buffer = JsFuture::from(response.array_buffer().map_err(message)?)
        .await
        .map_err(message)?;
    let body = Uint8Array::new(&buffer);
    if body.length() as usize > MAX_RESPONSE_BYTES {
        return Err(format!(
            "the response is larger than {MAX_RESPONSE_BYTES} bytes"
        ));
    }
    Ok(HttpResponse {
        status,
        headers,
        body: body.to_vec(),
    })
}

/// Starts the request and waits for its response headers.
async fn fetch(request: &HttpRequest) -> Result<Response, String> {
    let init = RequestInit::new();
    init.set_method(&request.method);
    let headers = Headers::new().map_err(message)?;
    for (name, value) in &request.headers {
        headers.append(name, value).map_err(message)?;
    }
    init.set_headers(&headers);
    if !request.body.is_empty() {
        init.set_body(&Uint8Array::from(request.body.as_slice()));
    }
    if let Some(timeout) = request.timeout_ms {
        let timeout = u32::try_from(timeout).unwrap_or(u32::MAX);
        init.set_signal(Some(&AbortSignal::timeout_with_u32(timeout)));
    }
    let request = Request::new_with_str_and_init(&request.url, &init).map_err(message)?;
    let fetch = Reflect::get(&js_sys::global(), &"fetch".into())
        .ok()
        .and_then(|fetch| fetch.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| String::from("fetch is not available"))?;
    let promise = fetch
        .call1(&js_sys::global(), &request)
        .map_err(message)?
        .dyn_into::<js_sys::Promise>()
        .map_err(message)?;
    JsFuture::from(promise)
        .await
        .map_err(message)?
        .dyn_into::<Response>()
        .map_err(message)
}

/// Reads the response headers the browser exposes.
fn read_headers(headers: &Headers) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    let iterator = js_sys::try_iter(headers)
        .map_err(message)?
        .ok_or_else(|| String::from("the response headers cannot be read"))?;
    for entry in iterator {
        let pair = js_sys::Array::from(&entry.map_err(message)?);
        entries.push((
            pair.get(0).as_string().unwrap_or_default(),
            pair.get(1).as_string().unwrap_or_default(),
        ));
    }
    Ok(entries)
}

/// Returns the message of a JavaScript error value.
fn message(value: JsValue) -> String {
    value
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| value.as_string())
        .unwrap_or_else(|| String::from("the request failed"))
}
