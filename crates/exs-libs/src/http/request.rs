//! Conversion between the arguments of `http.request` and [`HttpRequest`], and from an
//! [`HttpResponse`] to the value scripts build their `Response` from.

use exs_abi::ExsValue;

use super::transport::{HttpRequest, HttpResponse};
use crate::error::error;

/// Reads the method, URL, and optional options Object of a `http.request` call.
pub(super) fn parse(arguments: Vec<ExsValue>) -> Result<HttpRequest, ExsValue> {
    let mut arguments = arguments.into_iter();
    let (Some(ExsValue::String(method)), Some(ExsValue::String(url)), options, None) = (
        arguments.next(),
        arguments.next(),
        arguments.next(),
        arguments.next(),
    ) else {
        return Err(type_error("expects a method, a URL, and options"));
    };
    let method = method.to_ascii_uppercase();
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(error(
            "ValueError",
            format!("http: `{method}` is no HTTP method"),
        ));
    }
    let mut request = HttpRequest {
        method,
        url,
        headers: Vec::new(),
        body: Vec::new(),
        timeout_ms: None,
    };
    match options {
        None => {}
        Some(ExsValue::List(options)) => match options.as_slice() {
            [] | [ExsValue::None] => {}
            [ExsValue::Object(entries)] => apply_options(&mut request, entries)?,
            _ => return Err(type_error("takes at most one options Object")),
        },
        Some(_) => return Err(type_error("expects options as a List")),
    }
    Ok(request)
}

/// Applies the `headers`, `body`, and `timeout` options to `request`.
fn apply_options(
    request: &mut HttpRequest,
    options: &[(String, ExsValue)],
) -> Result<(), ExsValue> {
    let mut content_type = None;
    for (key, value) in options {
        match key.as_str() {
            "headers" => request.headers = headers(value)?,
            "body" => {
                let (body, kind) = body(value)?;
                request.body = body;
                content_type = kind;
            }
            "timeout" => request.timeout_ms = Some(timeout_ms(value)?),
            other => {
                return Err(type_error(&format!(
                    "has no option `{other}`; use headers, body, or timeout"
                )));
            }
        }
    }
    let has_content_type = request
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if let (Some(content_type), false) = (content_type, has_content_type) {
        request
            .headers
            .push(("content-type".to_owned(), content_type.to_owned()));
    }
    Ok(())
}

/// Reads the `headers` option: an Object of String values.
fn headers(value: &ExsValue) -> Result<Vec<(String, String)>, ExsValue> {
    let ExsValue::Object(entries) = value else {
        return Err(type_error("expects headers as an Object"));
    };
    entries
        .iter()
        .map(|(name, value)| match value {
            ExsValue::String(value) => Ok((name.clone(), value.clone())),
            _ => Err(type_error(&format!(
                "expects the header `{name}` as a String"
            ))),
        })
        .collect()
}

/// Reads the `body` option and the content type it implies, if any.
fn body(value: &ExsValue) -> Result<(Vec<u8>, Option<&'static str>), ExsValue> {
    match value {
        ExsValue::None => Ok((Vec::new(), None)),
        ExsValue::String(text) => Ok((text.as_bytes().to_vec(), Some("text/plain; charset=utf-8"))),
        ExsValue::Bytes(bytes) => Ok((bytes.clone(), None)),
        ExsValue::Object(_) | ExsValue::List(_) => crate::json::to_text(value)
            .map(|text| (text.into_bytes(), Some("application/json")))
            .map_err(|message| error("JsonUnsupported", message)),
        _ => Err(type_error(
            "expects the body as a String, Bytes, an Object, or a List",
        )),
    }
}

/// Reads the `timeout` option, a Duration, in milliseconds.
fn timeout_ms(value: &ExsValue) -> Result<u64, ExsValue> {
    let ExsValue::Object(entries) = value else {
        return Err(type_error("expects the timeout as a Duration"));
    };
    let field = |name: &str| {
        entries.iter().find_map(|(key, value)| match value {
            ExsValue::Int(number) if key == name => u64::try_from(*number).ok(),
            _ => None,
        })
    };
    match (field("seconds"), field("nanoseconds")) {
        (Some(seconds), Some(nanoseconds)) => Ok(seconds
            .saturating_mul(1000)
            .saturating_add(nanoseconds / 1_000_000)),
        _ => Err(type_error("expects the timeout as a positive Duration")),
    }
}

/// Returns the value scripts build their `Response` from.
pub(super) fn response_value(response: HttpResponse) -> ExsValue {
    let mut headers: Vec<(String, ExsValue)> = Vec::new();
    for (name, value) in response.headers {
        match headers.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, ExsValue::String(existing))) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => headers.push((name, ExsValue::String(value))),
        }
    }
    ExsValue::Object(vec![
        (
            "status".to_owned(),
            ExsValue::Int(i64::from(response.status)),
        ),
        ("headers".to_owned(), ExsValue::Object(headers)),
        ("body".to_owned(), ExsValue::Bytes(response.body)),
    ])
}

/// Returns a type error about the arguments of an HTTP request.
fn type_error(message: &str) -> ExsValue {
    error("TypeError", format!("http: the request {message}"))
}

#[cfg(test)]
mod tests {
    use exs_abi::ExsValue;

    use super::{HttpResponse, parse, response_value};

    /// Shorthand for a String value.
    fn string(text: &str) -> ExsValue {
        ExsValue::String(text.to_owned())
    }

    /// Reads headers, a JSON body with its content type, and a timeout.
    #[test]
    fn reads_options() {
        let request = parse(vec![
            string("post"),
            string("https://example.com"),
            ExsValue::List(vec![ExsValue::Object(vec![
                (
                    "headers".to_owned(),
                    ExsValue::Object(vec![("accept".to_owned(), string("application/json"))]),
                ),
                (
                    "body".to_owned(),
                    ExsValue::Object(vec![("name".to_owned(), string("Ada"))]),
                ),
                (
                    "timeout".to_owned(),
                    ExsValue::Object(vec![
                        ("seconds".to_owned(), ExsValue::Int(2)),
                        ("nanoseconds".to_owned(), ExsValue::Int(500_000_000)),
                    ]),
                ),
            ])]),
        ])
        .expect("valid request");
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.headers,
            [
                ("accept".to_owned(), "application/json".to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
            ]
        );
        assert_eq!(request.body, br#"{"name":"Ada"}"#);
        assert_eq!(request.timeout_ms, Some(2500));
    }

    /// Accepts a request without options and keeps an explicit content type.
    #[test]
    fn reads_requests_without_options() {
        let request = parse(vec![
            string("GET"),
            string("https://example.com"),
            ExsValue::List(Vec::new()),
        ])
        .expect("valid request");
        assert!(request.headers.is_empty() && request.body.is_empty());

        let request = parse(vec![
            string("PUT"),
            string("https://example.com"),
            ExsValue::List(vec![ExsValue::Object(vec![
                (
                    "headers".to_owned(),
                    ExsValue::Object(vec![("Content-Type".to_owned(), string("text/csv"))]),
                ),
                ("body".to_owned(), string("a,b")),
            ])]),
        ])
        .expect("valid request");
        assert_eq!(
            request.headers,
            [("Content-Type".to_owned(), "text/csv".to_owned())]
        );
    }

    /// Rejects unknown options and invalid methods.
    #[test]
    fn rejects_invalid_requests() {
        for arguments in [
            vec![
                string("GET"),
                string("https://example.com"),
                ExsValue::List(vec![ExsValue::Object(vec![(
                    "header".to_owned(),
                    ExsValue::Object(Vec::new()),
                )])]),
            ],
            vec![
                string("G E T"),
                string("https://example.com"),
                ExsValue::List(Vec::new()),
            ],
        ] {
            assert!(parse(arguments).is_err());
        }
    }

    /// Joins repeated response headers.
    #[test]
    fn joins_repeated_headers() {
        let value = response_value(HttpResponse {
            status: 200,
            headers: vec![
                ("set-cookie".to_owned(), "a=1".to_owned()),
                ("set-cookie".to_owned(), "b=2".to_owned()),
            ],
            body: b"ok".to_vec(),
        });
        assert_eq!(
            value,
            ExsValue::Object(vec![
                ("status".to_owned(), ExsValue::Int(200)),
                (
                    "headers".to_owned(),
                    ExsValue::Object(vec![("set-cookie".to_owned(), string("a=1, b=2"))])
                ),
                ("body".to_owned(), ExsValue::Bytes(b"ok".to_vec())),
            ])
        );
    }
}
