//! Sends HTTP requests and reads their responses.
//!
//! Every request function takes an optional options Object:
//!
//! - `headers`: an Object of header names and String values,
//! - `body`: a String, Bytes, or an Object or List sent as JSON with the content type
//!   `application/json`,
//! - `timeout`: a Duration after which the request fails.
//!
//! Responses larger than 10 MiB and more than 5 redirects return an `HttpError`, as do
//! failed connections. A response with an error status, such as 404, is no Error; check
//! `ok()` or `status`.

/// The response to an HTTP request.
type Response {
    /// Status code, such as 200.
    status: Int,
    /// Header names in lowercase with their values; repeated headers are joined by `, `.
    headers: Object,
    /// Response body.
    body: Bytes,
}

impl Response {
    /// Reports whether the status is a success status from 200 to 299.
    fn ok(self) -> Bool { ret self.status >= 200 && self.status < 300; }

    /// Returns the body as UTF-8 text.
    fn text(self) -> String | Error { ret self.body.decode_utf8(); }

    /// Parses the body as JSON.
    fn json(self) -> Any | Error { ret json::parse(self.text()?); }
}

/// Sends a request with any method, such as `"HEAD"` or `"OPTIONS"`.
///
/// ```exs
/// fn main() -> Int | Error {
///     let response = http::request("HEAD", "https://example.com")?;
///     ret response.status;
/// }
/// ```
fn request(method: String, url: String, options...) -> Response | Error {
    ret send(method, url, options);
}

/// Sends a GET request.
///
/// ```exs
/// fn main() -> Any | Error {
///     let response = http::get("https://example.com/api/items", {
///         headers: { accept: "application/json" },
///     })?;
///     ret response.json();
/// }
/// ```
fn get(url: String, options...) -> Response | Error { ret send("GET", url, options); }

/// Sends a POST request.
///
/// ```exs
/// fn main() -> Bool | Error {
///     let response = http::post("https://example.com/api/items", { body: { name: "Ada" } })?;
///     ret response.ok();
/// }
/// ```
fn post(url: String, options...) -> Response | Error { ret send("POST", url, options); }

/// Sends a PUT request.
fn put(url: String, options...) -> Response | Error { ret send("PUT", url, options); }

/// Sends a PATCH request.
fn patch(url: String, options...) -> Response | Error { ret send("PATCH", url, options); }

/// Sends a DELETE request.
fn delete(url: String, options...) -> Response | Error { ret send("DELETE", url, options); }

/// Sends a request through the host and builds its Response.
private fn send(method: String, url: String, options: List) -> Response | Error {
    let raw: Object = Host::call("http.request", method, url, options)?;
    ret Response { status: raw.status, headers: raw.headers, body: raw.body };
}
