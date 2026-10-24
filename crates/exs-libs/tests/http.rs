//! Runs ExS scripts that send HTTP requests to a local test server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use exs_compiler::{
    CompileOptions, ModuleResolver, ResolvedSource, SourceInput, compile_with_resolver,
};
use exs_runner::{ExecutionCancellation, ExsValue, ServerRunner};

/// Provides the libraries of `exs-libs` and rejects file imports.
struct LibraryResolver;

impl ModuleResolver for LibraryResolver {
    /// Rejects every import; the tests only name libraries.
    fn resolve(&mut self, _importer: &str, path: &str) -> Result<ResolvedSource, String> {
        Err(format!("unexpected import `{path}`"))
    }

    /// Returns the enabled library at `path`.
    fn library(&mut self, path: &str) -> Option<ResolvedSource> {
        exs_libs::source(path).map(|text| ResolvedSource {
            source_id: path.to_owned(),
            text: text.to_owned(),
        })
    }
}

/// Starts a server on a free local port that answers every request and returns its base URL.
fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server has an address");
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || answer(stream));
        }
    });
    format!("http://{address}")
}

/// Reads one request and writes the response for its path.
fn answer(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().expect("stream clones"));
    let mut request_line = String::new();
    let _ = reader.read_line(&mut request_line);
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        headers.push(line.trim_end().to_owned());
    }
    let length = headers
        .iter()
        .find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let content_type = headers
        .iter()
        .find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();

    let response = match path.as_str() {
        "/json" => reply(
            "200 OK",
            "content-type: application/json\r\n",
            br#"{"id":7}"#,
        ),
        "/echo" => {
            let headers = format!("x-method: {method}\r\nx-content-type: {content_type}\r\n");
            reply("201 Created", &headers, &body)
        }
        "/missing" => reply("404 Not Found", "", b"no"),
        "/redirect" => reply("302 Found", "location: /json\r\n", b""),
        "/loop" => reply("302 Found", "location: /loop\r\n", b""),
        "/big" => format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            11 * 1024 * 1024
        )
        .into_bytes(),
        _ => reply("500 Internal Server Error", "", b""),
    };
    let _ = stream.write_all(&response);
}

/// Builds a complete response that closes the connection.
fn reply(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

/// Compiles `source` with the libraries and returns the result of its `main` for `base`.
fn run(source: &str, base: &str) -> ExsValue {
    let compiled = compile_with_resolver(
        SourceInput {
            source_id: "test.exs",
            text: source,
        },
        CompileOptions::default(),
        &mut LibraryResolver,
    )
    .unwrap_or_else(|error| panic!("{source}\n{error}"));
    let mut runner = ServerRunner::default();
    exs_libs::register(runner.registry_mut()).expect("library host functions register");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime starts");
    runtime
        .block_on(runner.execute(
            &compiled.wasm,
            "main",
            &[ExsValue::String(base.to_owned())],
            &ExecutionCancellation::new(),
        ))
        .unwrap_or_else(|error| panic!("{source}\n{error}"))
}

/// Shorthand for a String value.
fn string(text: &str) -> ExsValue {
    ExsValue::String(text.to_owned())
}

/// Reads a JSON response, also after a redirect.
#[test]
fn reads_json_responses() {
    let base = start_server();
    assert_eq!(
        run(
            r#"fn main(base: String) -> List | Error {
                let direct = http::get(f"{base}/json")?;
                let redirected = http::get(f"{base}/redirect")?;
                ret [direct.status, direct.ok(), direct.headers["content-type"], direct.json()?.id, redirected.json()?.id];
            }"#,
            &base,
        ),
        ExsValue::List(vec![
            ExsValue::Int(200),
            ExsValue::Bool(true),
            string("application/json"),
            ExsValue::Int(7),
            ExsValue::Int(7),
        ])
    );
}

/// Sends a JSON body with its content type and reads the echoed text.
#[test]
fn sends_bodies() {
    let base = start_server();
    assert_eq!(
        run(
            r#"fn main(base: String) -> List | Error {
                let response = http::post(f"{base}/echo", {
                    headers: { "x-test": "1" },
                    body: { name: "Ada" },
                    timeout: Duration::seconds(5),
                })?;
                ret [response.status, response.headers["x-method"], response.headers["x-content-type"], response.text()?];
            }"#,
            &base,
        ),
        ExsValue::List(vec![
            ExsValue::Int(201),
            string("POST"),
            string("application/json"),
            string(r#"{"name":"Ada"}"#),
        ])
    );
}

/// Returns error statuses as responses and failures as Errors.
#[test]
fn reports_failures() {
    let base = start_server();
    assert_eq!(
        run(
            r#"fn main(base: String) -> List {
                let missing = http::get(f"{base}/missing");
                let looping = http::get(f"{base}/loop");
                let big = http::get(f"{base}/big");
                let refused = http::get("http://127.0.0.1:1/");
                ret [missing.status, missing.ok(), looping.kind(), big.kind(), refused.kind()];
            }"#,
            &base,
        ),
        ExsValue::List(vec![
            ExsValue::Int(404),
            ExsValue::Bool(false),
            string("HttpError"),
            string("HttpError"),
            string("HttpError"),
        ])
    );
}
