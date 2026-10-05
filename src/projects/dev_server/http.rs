//! Minimal HTTP routing for the std-only dev server.
//!
//! Routes reload, ping and bounded browser runtime reports, serves static output, and falls back
//! to compiler diagnostics on failed builds. Browser reports never mutate compilation state.

use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::projects::dev_server::error_page::render_runtime_error_page;
use crate::projects::dev_server::sse;
use crate::projects::dev_server::state::{
    BuildState, DevServerState, RuntimeReportCategory, RuntimeReportKey,
};
use crate::projects::dev_server::static_files::{self, ResolvedRequest, ResolvedRequestKind};
use crate::projects::routing::{origin_root_url, strip_origin_prefix};
use saying::say;
use serde::de::{MapAccess, Visitor, value::MapAccessDeserializer};
use serde::{Deserialize, Deserializer};
use std::fmt::{self, Write as _};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::marker::PhantomData;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_HEADER_LINE_BYTES: usize = 8192;
const MAX_HEADER_LINES: usize = 64;
const MAX_REPORT_BODY_BYTES: usize = 16 * 1024;
const MAX_RUNTIME_REPORTS: usize = 1024;

pub fn handle_connection(stream: TcpStream, state: Arc<DevServerState>) -> io::Result<()> {
    handle_connection_with_timeouts(stream, state, REQUEST_READ_TIMEOUT, RESPONSE_WRITE_TIMEOUT)
}

fn handle_connection_with_timeouts(
    mut stream: TcpStream,
    state: Arc<DevServerState>,
    read_timeout: Duration,
    write_timeout: Duration,
) -> io::Result<()> {
    stream.set_read_timeout(Some(read_timeout))?;
    stream.set_write_timeout(Some(write_timeout))?;

    // Keep the same reader for the body: header parsing may already have buffered body bytes.
    let mut reader = BufReader::new(stream.try_clone()?);
    let request = match parse_request(&mut reader) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return send_report_status(&mut stream, "400 BAD REQUEST");
        }
        Err(error) => return Err(error),
    };

    let (request_path, request_query) = split_path_and_query(&request.path);

    if request.method != "GET" {
        let site_local_path = {
            let build_state = state
                .build_state
                .lock()
                .map_err(|_| io::Error::other("build state lock was poisoned"))?;
            strip_origin_prefix(request_path, &build_state.html_site_config.origin)
        };
        if request.method == "POST" && site_local_path.as_deref() == Some("/__moth/runtime-report")
        {
            return handle_runtime_report(&mut stream, &mut reader, &request, &state);
        }

        return send_text_response(
            &mut stream,
            "405 METHOD NOT ALLOWED",
            "text/plain; charset=utf-8",
            "Method Not Allowed",
        );
    }
    drop(reader);

    #[cfg(test)]
    if let Some(hook) = state.capture_hook.lock().expect("read hook lock").as_ref() {
        hook(crate::projects::dev_server::state::OutputCapturePoint::BeforeLock);
    }

    let response = {
        let build_state = state
            .build_state
            .lock()
            .map_err(|_| io::Error::other("build state lock was poisoned"))?;
        let origin = &build_state.html_site_config.origin;

        if request_path == "/" && origin != "/" {
            GetResponse::Static(PreparedResponse::Redirect {
                status_line: "302 FOUND",
                location: origin_root_url(origin),
            })
        } else if let Some(site_local_path) = strip_origin_prefix(request_path, origin) {
            match site_local_path.as_str() {
                "/__moth/events" => GetResponse::Events,
                "/__moth/ping" => GetResponse::Static(PreparedResponse::text(
                    "200 OK",
                    "text/plain; charset=utf-8",
                    "ok",
                )),
                _ => {
                    #[cfg(test)]
                    if let Some(hook) = state.capture_hook.lock().expect("read hook lock").as_ref()
                    {
                        hook(crate::projects::dev_server::state::OutputCapturePoint::BeforeRead);
                    }

                    // WHAT: capture the response bytes while publication is excluded.
                    // WHY: companions fetched after a page's generation may be newer; its report
                    // still carries the page's version and is stale. The SSE generation
                    // handshake reloads that page even when it missed the publication broadcast.
                    GetResponse::Static(prepare_static_response(
                        &site_local_path,
                        request_query,
                        &build_state,
                    )?)
                }
            }
        } else {
            GetResponse::Static(PreparedResponse::text(
                "404 NOT FOUND",
                "text/plain; charset=utf-8",
                "Not Found (Outside Origin)",
            ))
        }
    };

    // Network backpressure must not hold up either publication or another reader's capture.
    match response {
        GetResponse::Events => sse::handle_sse_connection(stream, state),
        GetResponse::Static(response) => send_prepared_response(&mut stream, response),
    }
}

struct HttpRequest {
    method: String,
    path: String,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
    content_length: Option<String>,
    transfer_encoding: bool,
}

enum GetResponse {
    Events,
    Static(PreparedResponse),
}

enum PreparedResponse {
    Text {
        status_line: &'static str,
        content_type: &'static str,
        body: String,
    },
    Bytes {
        body: Vec<u8>,
        content_type: &'static str,
    },
    Redirect {
        status_line: &'static str,
        location: String,
    },
}

impl PreparedResponse {
    fn text(
        status_line: &'static str,
        content_type: &'static str,
        body: impl Into<String>,
    ) -> Self {
        Self::Text {
            status_line,
            content_type,
            body: body.into(),
        }
    }
}

fn parse_request(reader: &mut BufReader<TcpStream>) -> io::Result<Option<HttpRequest>> {
    let request_line = match read_header_line(reader) {
        Ok(Some(line)) => line,
        Ok(None) => return Ok(None),
        Err(error) if is_connection_timeout(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(path), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(malformed_request());
    };
    if parts.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(malformed_request());
    }

    let mut request = HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        host: None,
        origin: None,
        content_type: None,
        content_length: None,
        transfer_encoding: false,
    };

    // The request line counts towards the limit, as does the terminating empty header line.
    for _ in 1..MAX_HEADER_LINES {
        let line = match read_header_line(reader) {
            Ok(Some(line)) => line,
            Ok(None) => return Err(malformed_request()),
            Err(error) if is_connection_timeout(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        if line.is_empty() {
            return Ok(Some(request));
        }

        let Some((name, value)) = line.split_once(':') else {
            return Err(malformed_request());
        };
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            || value
                .chars()
                .any(|character| character.is_control() && character != '\t')
        {
            return Err(malformed_request());
        }
        let slot = if name.eq_ignore_ascii_case("host") {
            Some(&mut request.host)
        } else if name.eq_ignore_ascii_case("origin") {
            Some(&mut request.origin)
        } else if name.eq_ignore_ascii_case("content-type") {
            Some(&mut request.content_type)
        } else if name.eq_ignore_ascii_case("content-length") {
            Some(&mut request.content_length)
        } else {
            None
        };
        if let Some(slot) = slot {
            if slot.is_some() {
                return Err(malformed_request());
            }
            *slot = Some(value.trim().to_owned());
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            request.transfer_encoding = true;
        }
    }

    Err(malformed_request())
}

fn read_header_line(reader: &mut BufReader<TcpStream>) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let length = reader
        .take((MAX_HEADER_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if length == 0 {
        return Ok(None);
    }
    if length > MAX_HEADER_LINE_BYTES || !bytes.ends_with(b"\r\n") {
        return Err(malformed_request());
    }
    bytes.truncate(length - 2);
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| malformed_request())
}

fn malformed_request() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "malformed or oversized HTTP headers",
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeReport {
    build: u64,
    entry: String,
    invocation: String,
    category: RuntimeReportCategory,
    // A present null is not an absent code. Deserialize every present value as a strict U32.
    #[serde(default, deserialize_with = "deserialize_report_code")]
    code: Option<u32>,
    message: Option<String>,
    #[serde(default, deserialize_with = "deserialize_report_source")]
    source: Option<RuntimeReportSource>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeReportSource {
    file: String,
    line: u32,
    column: u32,
}

/// Serde's struct derive also accepts positional arrays. The wire protocol requires objects.
struct JsonObject<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for JsonObject<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = JsonObject<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(JsonObject)
            }
        }

        deserializer.deserialize_map(ObjectVisitor(PhantomData))
    }
}

fn deserialize_report_source<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<RuntimeReportSource>, D::Error> {
    Option::<JsonObject<RuntimeReportSource>>::deserialize(deserializer)
        .map(|source| source.map(|object| object.0))
}

fn deserialize_report_code<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    u32::deserialize(deserializer).map(Some)
}

fn handle_runtime_report(
    stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    request: &HttpRequest,
    state: &DevServerState,
) -> io::Result<()> {
    let same_origin = match (&request.host, &request.origin) {
        (Some(host), Some(origin)) if !host.is_empty() => {
            origin.strip_prefix("http://") == Some(host.as_str())
        }
        _ => false,
    };
    if !same_origin {
        return send_report_status(stream, "403 FORBIDDEN");
    }
    let is_json = request.content_type.as_deref().is_some_and(|content_type| {
        content_type
            .split(';')
            .next()
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
    });
    if !is_json {
        return send_report_status(stream, "415 UNSUPPORTED MEDIA TYPE");
    }
    let Some(length) = request.content_length.as_deref() else {
        return send_report_status(stream, "400 BAD REQUEST");
    };
    if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
        return send_report_status(stream, "400 BAD REQUEST");
    }
    let Ok(length) = length.parse::<u64>() else {
        return send_report_status(stream, "413 PAYLOAD TOO LARGE");
    };
    if length > MAX_REPORT_BODY_BYTES as u64 {
        return send_report_status(stream, "413 PAYLOAD TOO LARGE");
    }
    if request.transfer_encoding {
        return send_report_status(stream, "400 BAD REQUEST");
    }

    let mut body = vec![0; length as usize];
    if let Err(error) = reader.read_exact(&mut body) {
        if is_connection_timeout(&error) || error.kind() == io::ErrorKind::UnexpectedEof {
            return send_report_status(stream, "400 BAD REQUEST");
        }
        return Err(error);
    }
    let Ok(JsonObject(report)) = serde_json::from_slice::<JsonObject<RuntimeReport>>(&body) else {
        return send_report_status(stream, "400 BAD REQUEST");
    };
    if !valid_runtime_report(&report) {
        return send_report_status(stream, "400 BAD REQUEST");
    }

    // Hold build publication stable until the separate report ledger has accepted this identity.
    let build_state = state
        .build_state
        .lock()
        .map_err(|_| io::Error::other("build state lock was poisoned"))?;
    if !build_state.last_build_ok
        || report.build != build_state.last_build_version
        || !static_files::is_known_entry(&report.entry, &build_state.output_dir)
    {
        drop(build_state);
        return send_report_status(stream, "409 CONFLICT");
    }
    let mut runtime_reports = state
        .runtime_reports
        .lock()
        .map_err(|_| io::Error::other("runtime report lock was poisoned"))?;
    if runtime_reports
        .build
        .is_none_or(|build| build < report.build)
    {
        runtime_reports.keys.clear();
        runtime_reports.build = Some(report.build);
    }
    let key = RuntimeReportKey {
        build: report.build,
        entry: report.entry.clone(),
        invocation: report.invocation.clone(),
    };
    if runtime_reports.keys.contains(&key) {
        drop(runtime_reports);
        drop(build_state);
        return send_report_status(stream, "204 NO CONTENT");
    }
    if runtime_reports.keys.len() >= MAX_RUNTIME_REPORTS {
        drop(runtime_reports);
        drop(build_state);
        return send_report_status(stream, "429 TOO MANY REQUESTS");
    }
    runtime_reports.keys.insert(key);
    drop(runtime_reports);
    drop(build_state);

    let rendered = format_runtime_report(&report);
    say!(Bold Yellow rendered);
    send_report_status(stream, "204 NO CONTENT")
}

fn valid_runtime_report(report: &RuntimeReport) -> bool {
    let valid_invocation = !report.invocation.is_empty()
        && report.invocation.len() <= 64
        && report
            .invocation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    let valid_code =
        (report.category == RuntimeReportCategory::EntryError) == report.code.is_some();

    valid_invocation
        && valid_code
        && !report.entry.is_empty()
        && report.entry.len() <= 1024
        && report
            .message
            .as_ref()
            .is_none_or(|message| message.len() <= 4096)
        && report
            .source
            .as_ref()
            .is_none_or(|source| source.file.len() <= 1024)
}

fn send_report_status(stream: &mut TcpStream, status: &str) -> io::Result<()> {
    send_response_bytes(stream, status, "text/plain; charset=utf-8", b"")
}

/// Escape terminal and bidi controls without obscuring ordinary Unicode text.
fn append_terminal_text(output: &mut String, text: &str) {
    for character in text.chars() {
        if matches!(
            character as u32,
            0..=0x1f | 0x7f..=0x9f | 0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
        ) {
            // Formatting into a String cannot fail.
            write!(output, "\\u{{{:04x}}}", character as u32)
                .expect("writing to String cannot fail");
        } else {
            output.push(character);
        }
    }
}

fn format_runtime_report(report: &RuntimeReport) -> String {
    let title = match report.category {
        RuntimeReportCategory::EntryError => "Entry Error",
        RuntimeReportCategory::Assertion => "Assertion Failure",
        RuntimeReportCategory::StartupFault => "Unexpected Startup Fault",
    };
    let mut rendered = format!(
        "Dev runtime: {title} (runtime; build #{} succeeded)\n  Entry: ",
        report.build
    );
    append_terminal_text(&mut rendered, &report.entry);
    rendered.push_str("\n  Invocation: ");
    append_terminal_text(&mut rendered, &report.invocation);
    if let Some(code) = report.code {
        // Formatting into a String cannot fail.
        write!(rendered, "\n  Error code: {code}").expect("writing to String cannot fail");
    }
    if let Some(message) = &report.message {
        rendered.push_str("\n  Message: ");
        append_terminal_text(&mut rendered, message);
    }
    if let Some(source) = &report.source {
        rendered.push_str("\n  Source: ");
        append_terminal_text(&mut rendered, &source.file);
        // Formatting into a String cannot fail.
        write!(rendered, ":{}:{}", source.line, source.column)
            .expect("writing to String cannot fail");
    }
    rendered
}

fn is_connection_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn split_path_and_query(path: &str) -> (&str, Option<&str>) {
    match path.split_once('?') {
        Some((request_path, request_query)) => (request_path, Some(request_query)),
        None => (path, None),
    }
}

fn send_prepared_response(stream: &mut TcpStream, response: PreparedResponse) -> io::Result<()> {
    match response {
        PreparedResponse::Text {
            status_line,
            content_type,
            body,
        } => send_text_response(stream, status_line, content_type, &body),
        PreparedResponse::Bytes { body, content_type } => {
            send_response_bytes(stream, "200 OK", content_type, &body)
        }
        PreparedResponse::Redirect {
            status_line,
            location,
        } => send_redirect_response(stream, status_line, &location),
    }
}

fn prepare_static_response(
    request_path: &str,
    request_query: Option<&str>,
    build_state: &BuildState,
) -> io::Result<PreparedResponse> {
    let resolved_request = static_files::resolve_request(
        request_path,
        request_query,
        &build_state.output_dir,
        build_state.entry_page_rel.as_deref(),
        build_state.html_site_config.clone(),
    );

    if let ResolvedRequest::Redirect { location } = resolved_request {
        return Ok(PreparedResponse::Redirect {
            status_line: "302 FOUND",
            location,
        });
    }

    if matches!(resolved_request, ResolvedRequest::MissingEntryPage) && !build_state.last_build_ok {
        let error_page = build_state.last_error_html.clone().unwrap_or_else(|| {
            render_runtime_error_page(
                "Build Failed",
                "The latest build failed, but no diagnostics were stored.",
                &build_state.html_site_config.origin,
                build_state.last_build_version,
            )
        });

        return Ok(PreparedResponse::text(
            "200 OK",
            "text/html; charset=utf-8",
            error_page,
        ));
    }

    let (resolved_path, resolved_kind) = match resolved_request {
        ResolvedRequest::File { path, kind } => (path, kind),
        ResolvedRequest::MissingEntryPage => {
            let error_page = render_runtime_error_page(
                "Missing Entry Page",
                "Build did not produce a HTML entry page for '/'.",
                &build_state.html_site_config.origin,
                build_state.last_build_version,
            );
            return Ok(PreparedResponse::text(
                "200 OK",
                "text/html; charset=utf-8",
                error_page,
            ));
        }
        ResolvedRequest::NotFound | ResolvedRequest::InvalidPath => {
            return Ok(PreparedResponse::text(
                "404 NOT FOUND",
                "text/plain; charset=utf-8",
                "Not Found",
            ));
        }
        ResolvedRequest::Redirect { .. } => {
            return Ok(PreparedResponse::text(
                "500 INTERNAL SERVER ERROR",
                "text/plain; charset=utf-8",
                "Internal Server Error",
            ));
        }
    };

    // Failed builds replace resolved page HTML requests with diagnostics while still allowing
    // supporting assets to load so the browser can keep the previous shell alive.
    if should_serve_failed_build_html(resolved_kind, build_state) {
        let error_page = build_state.last_error_html.clone().unwrap_or_else(|| {
            render_runtime_error_page(
                "Build Failed",
                "The latest build failed, but no diagnostics were stored.",
                &build_state.html_site_config.origin,
                build_state.last_build_version,
            )
        });

        return Ok(PreparedResponse::text(
            "200 OK",
            "text/html; charset=utf-8",
            error_page,
        ));
    }

    let content_type = static_files::content_type_for_path(&resolved_path);
    if resolved_kind == ResolvedRequestKind::PageHtml
        || static_files::is_html_content_type(content_type)
    {
        let html = match std::fs::read_to_string(&resolved_path) {
            Ok(contents) => contents,
            Err(error) => {
                return Ok(PreparedResponse::text(
                    "500 INTERNAL SERVER ERROR",
                    "text/plain; charset=utf-8",
                    format!("Failed to read HTML file: {error}"),
                ));
            }
        };

        let entry = match resolved_path.strip_prefix(&build_state.output_dir) {
            Ok(relative_path) => portable_path_text(relative_path),
            Err(_) => {
                return Ok(PreparedResponse::text(
                    "500 INTERNAL SERVER ERROR",
                    "text/plain; charset=utf-8",
                    "Resolved HTML path is outside the output directory",
                ));
            }
        };
        let injected_html = static_files::inject_dev_client(
            &html,
            &build_state.html_site_config.origin,
            build_state.last_build_version,
            &entry,
        );
        return Ok(PreparedResponse::text(
            "200 OK",
            content_type,
            injected_html,
        ));
    }

    Ok(PreparedResponse::Bytes {
        body: std::fs::read(resolved_path)?,
        content_type,
    })
}

fn should_serve_failed_build_html(
    resolved_kind: ResolvedRequestKind,
    build_state: &BuildState,
) -> bool {
    !build_state.last_build_ok && resolved_kind == ResolvedRequestKind::PageHtml
}

fn send_text_response(
    stream: &mut TcpStream,
    status_line: &str,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    send_response_bytes(stream, status_line, content_type, body.as_bytes())
}

fn send_response_bytes(
    stream: &mut TcpStream,
    status_line: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    send_response_headers(stream, status_line, content_type, body.len() as u64)?;
    stream.write_all(body)?;
    stream.flush()
}

fn send_response_headers(
    stream: &mut TcpStream,
    status_line: &str,
    content_type: &str,
    content_length: u64,
) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes())
}

fn send_redirect_response(
    stream: &mut TcpStream,
    status_line: &str,
    location: &str,
) -> io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {status_line}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
#[path = "tests/http_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/runtime_report_tests.rs"]
mod runtime_report_tests;
