//! Minimal HTTP routing for the std-only dev server.
//!
//! Routes SSE and ping endpoints, serves static files from the dev output directory, and falls
//! back to a generated error page when the latest build failed.

use crate::projects::dev_server::error_page::render_runtime_error_page;
use crate::projects::dev_server::sse;
use crate::projects::dev_server::state::{BuildState, DevServerState};
use crate::projects::dev_server::static_files::{self, ResolvedRequest, ResolvedRequestKind};
use crate::projects::routing::{origin_root_url, strip_origin_prefix};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

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

    let Some(request) = parse_request(&stream)? else {
        return Ok(());
    };

    if request.method != "GET" {
        return send_text_response(
            &mut stream,
            "405 METHOD NOT ALLOWED",
            "text/plain; charset=utf-8",
            "Method Not Allowed",
        );
    }

    let (request_path, request_query) = split_path_and_query(&request.path);

    let build_state = state
        .build_state
        .lock()
        .map_err(|_| io::Error::other("build state lock was poisoned"))?
        .clone();

    let origin = &build_state.html_site_config.origin;

    // Layer A - origin mount matching
    if request_path == "/" && origin != "/" {
        let location = origin_root_url(origin);
        return send_redirect_response(&mut stream, "302 FOUND", &location);
    }

    let Some(site_local_path) = strip_origin_prefix(request_path, origin) else {
        return send_text_response(
            &mut stream,
            "404 NOT FOUND",
            "text/plain; charset=utf-8",
            "Not Found (Outside Origin)",
        );
    };

    match site_local_path.as_str() {
        "/__moth/events" => sse::handle_sse_connection(stream, state),
        "/__moth/ping" => {
            send_text_response(&mut stream, "200 OK", "text/plain; charset=utf-8", "ok")
        }
        _ => serve_static_request(
            &mut stream,
            &site_local_path,
            request_query,
            &build_state,
            &state,
        ),
    }
}

struct HttpRequest {
    method: String,
    path: String,
}

enum PreparedResponse {
    Text {
        status_line: &'static str,
        content_type: &'static str,
        body: String,
    },
    File {
        path: PathBuf,
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

fn parse_request(stream: &TcpStream) -> io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    let request_line_bytes = match reader.read_line(&mut request_line) {
        Ok(bytes_read) => bytes_read,
        Err(error) if is_connection_timeout(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    if request_line_bytes == 0 {
        return Ok(None);
    }

    // Discard headers; v2 only needs method/path routing.
    loop {
        let mut header_line = String::new();
        let bytes_read = match reader.read_line(&mut header_line) {
            Ok(bytes_read) => bytes_read,
            Err(error) if is_connection_timeout(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        if bytes_read == 0 || header_line == "\r\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return Ok(None);
    };
    let Some(path) = parts.next() else {
        return Ok(None);
    };

    Ok(Some(HttpRequest {
        method: method.to_owned(),
        path: path.to_owned(),
    }))
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

fn serve_static_request(
    stream: &mut TcpStream,
    site_local_path: &str,
    request_query: Option<&str>,
    build_state: &BuildState,
    _state: &Arc<DevServerState>,
) -> io::Result<()> {
    match prepare_static_response(site_local_path, request_query, build_state) {
        PreparedResponse::Text {
            status_line,
            content_type,
            body,
        } => send_text_response(stream, status_line, content_type, &body),
        PreparedResponse::File { path, content_type } => {
            stream_file_response(stream, &path, content_type)
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
) -> PreparedResponse {
    let resolved_request = static_files::resolve_request(
        request_path,
        request_query,
        &build_state.output_dir,
        build_state.entry_page_rel.as_deref(),
        build_state.html_site_config.clone(),
    );

    if let ResolvedRequest::Redirect { location } = resolved_request {
        return PreparedResponse::Redirect {
            status_line: "302 FOUND",
            location,
        };
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

        return PreparedResponse::text("200 OK", "text/html; charset=utf-8", error_page);
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
            return PreparedResponse::text("200 OK", "text/html; charset=utf-8", error_page);
        }
        ResolvedRequest::NotFound | ResolvedRequest::InvalidPath => {
            return PreparedResponse::text(
                "404 NOT FOUND",
                "text/plain; charset=utf-8",
                "Not Found",
            );
        }
        ResolvedRequest::Redirect { .. } => {
            return PreparedResponse::text(
                "500 INTERNAL SERVER ERROR",
                "text/plain; charset=utf-8",
                "Internal Server Error",
            );
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

        return PreparedResponse::text("200 OK", "text/html; charset=utf-8", error_page);
    }

    let content_type = static_files::content_type_for_path(&resolved_path);
    if resolved_kind == ResolvedRequestKind::PageHtml
        || static_files::is_html_content_type(content_type)
    {
        let html = match std::fs::read_to_string(&resolved_path) {
            Ok(contents) => contents,
            Err(error) => {
                return PreparedResponse::text(
                    "500 INTERNAL SERVER ERROR",
                    "text/plain; charset=utf-8",
                    format!("Failed to read HTML file: {error}"),
                );
            }
        };

        let injected_html =
            static_files::inject_dev_client(&html, &build_state.html_site_config.origin);
        return PreparedResponse::text("200 OK", content_type, injected_html);
    }

    PreparedResponse::File {
        path: resolved_path,
        content_type,
    }
}

fn should_serve_failed_build_html(
    resolved_kind: ResolvedRequestKind,
    build_state: &BuildState,
) -> bool {
    !build_state.last_build_ok && resolved_kind == ResolvedRequestKind::PageHtml
}

fn stream_file_response(
    stream: &mut TcpStream,
    file_path: &Path,
    content_type: &str,
) -> io::Result<()> {
    let file = File::open(file_path)?;
    let content_length = file.metadata()?.len();
    send_response_headers(stream, "200 OK", content_type, content_length)?;

    let mut reader = BufReader::new(file);
    io::copy(&mut reader, stream)?;
    stream.flush()
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
