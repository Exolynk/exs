//! Requests sent with `reqwest` on native targets.

use std::sync::OnceLock;
use std::time::Duration;

use super::transport::{HttpRequest, HttpResponse, MAX_REDIRECTS, MAX_RESPONSE_BYTES};

/// The client shared by every request; `Err` when it could not be built.
static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

/// Returns the shared client, which follows at most [`MAX_REDIRECTS`] redirects.
fn client() -> Result<&'static reqwest::Client, String> {
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
                .build()
                .map_err(|error| format!("could not create the HTTP client: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Sends `request` and reads its response, which may be at most [`MAX_RESPONSE_BYTES`] long.
///
/// Must run inside a tokio runtime. Applications use it to send requests on behalf of
/// other transports, such as browser requests forwarded to their server.
///
/// # Errors
///
/// Returns a message when the request is invalid, fails, times out, or its response is too
/// large.
pub async fn send(request: HttpRequest) -> Result<HttpResponse, String> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| format!("`{}` is no HTTP method", request.method))?;
    let mut builder = client()?.request(method, &request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    if !request.body.is_empty() {
        builder = builder.body(request.body);
    }
    if let Some(timeout) = request.timeout_ms {
        builder = builder.timeout(Duration::from_millis(timeout));
    }
    let mut response = builder.send().await.map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(too_large());
    }
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Returns the message for a response above the size limit.
fn too_large() -> String {
    format!("the response is larger than {MAX_RESPONSE_BYTES} bytes")
}
