//! Tests for dev-server HTTP routing during successful and failed builds.

use super::{PreparedResponse, handle_connection_with_timeouts, prepare_static_response};
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::await_worker_completion;
use crate::projects::dev_server::state::{BuildState, DevServerState};
use crate::projects::routing::parse_html_site_config;
use crate::projects::settings::Config;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a partial request may hold its worker thread before the read timeout frees it.
const PARTIAL_REQUEST_DEADLINE: Duration = Duration::from_secs(1);

fn bind_loopback_listener() -> Option<TcpListener> {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(error) => panic!("should bind test listener: {error}"),
    }
}

/// Build a published state with captured artifacts when the output directory exists.
fn configure_build_state(
    output_dir: &Path,
    entry_page_rel: Option<PathBuf>,
    last_build_ok: bool,
) -> BuildState {
    let mut build_state = BuildState::new(output_dir.to_path_buf());
    build_state.last_build_ok = last_build_ok;
    build_state.entry_page_rel = entry_page_rel;
    build_state.served_artifacts = if output_dir.exists() {
        crate::projects::dev_server::build_loop::capture_served_artifacts(output_dir)
            .expect("capture served artifacts")
    } else {
        Arc::default()
    };
    build_state.served_html_entries =
        crate::projects::dev_server::static_files::validated_html_entries(
            output_dir,
            &build_state.served_artifacts,
        );
    build_state
}

fn configure_failed_build_state(
    output_dir: &Path,
    last_error_html: &str,
    entry_page_rel: Option<PathBuf>,
) -> BuildState {
    let mut build_state = configure_build_state(output_dir, entry_page_rel, false);
    build_state.last_error_html = Some(last_error_html.to_owned());
    build_state.last_build_version = 9;
    build_state
}

#[cfg(unix)]
#[test]
fn unrepresentable_output_name_serves_html_without_dev_injection() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    fs::write(output_dir.join("a*b.html"), "<html><body>raw</body></html>")
        .expect("should write a page whose name the output writer rejects");

    let build_state = configure_build_state(&output_dir, None, true);
    assert!(
        build_state.served_html_entries.is_empty(),
        "an invalid output component cannot be a published entry"
    );

    match prepare_static_response("/a*b.html", None, &build_state) {
        PreparedResponse::Text {
            status_line, body, ..
        } => {
            assert_eq!(status_line, "200 OK");
            assert_eq!(
                body, "<html><body>raw</body></html>",
                "a page without a validated entry must be served without the dev client"
            );
        }
        PreparedResponse::Bytes { .. } => panic!("html page should be served as text"),
        PreparedResponse::Redirect { .. } => panic!("html page should not redirect"),
    }
}

#[test]
fn nested_html_request_uses_stored_error_page_during_failed_build() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("docs/basics")).expect("should create docs output dir");
    fs::write(
        output_dir.join("docs/basics/index.html"),
        "<html><body>stale success</body></html>",
    )
    .expect("should write stale html");

    let error_html = "<html><body>compiler exploded</body></html>";
    let build_state =
        configure_failed_build_state(&output_dir, error_html, Some(PathBuf::from("index.html")));

    match prepare_static_response("/docs/basics/", None, &build_state) {
        PreparedResponse::Text {
            status_line,
            content_type,
            body,
        } => {
            assert_eq!(status_line, "200 OK");
            assert_eq!(content_type, "text/html; charset=utf-8");
            assert!(body.contains("compiler exploded"));
            assert!(!body.contains("stale success"));
        }
        PreparedResponse::Bytes { .. } => {
            panic!("nested html route should render stored error page")
        }
        PreparedResponse::Redirect { .. } => {
            panic!("nested html route should not redirect in this scenario")
        }
    }
}

#[test]
fn failed_build_keeps_css_js_and_image_assets_reachable() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("styles")).expect("should create styles dir");
    fs::create_dir_all(output_dir.join("scripts")).expect("should create scripts dir");
    fs::create_dir_all(output_dir.join("images")).expect("should create images dir");
    fs::write(output_dir.join("styles/site.css"), "body { color: red; }")
        .expect("should write css");
    fs::write(output_dir.join("scripts/app.js"), "console.log('hello');").expect("should write js");
    fs::write(output_dir.join("images/icon.png"), [0x89, b'P', b'N', b'G'])
        .expect("should write png");

    let build_state = configure_failed_build_state(
        &output_dir,
        "<html><body>error</body></html>",
        Some(PathBuf::from("index.html")),
    );

    match prepare_static_response("/styles/site.css", None, &build_state) {
        PreparedResponse::Bytes { body, content_type } => {
            assert_eq!(content_type, "text/css; charset=utf-8");
            assert_eq!(body.as_slice(), b"body { color: red; }");
        }
        PreparedResponse::Text { .. } => panic!("css request should keep serving the asset"),
        PreparedResponse::Redirect { .. } => panic!("css request should not redirect"),
    }

    match prepare_static_response("/scripts/app.js", None, &build_state) {
        PreparedResponse::Bytes { body, content_type } => {
            assert_eq!(content_type, "application/javascript; charset=utf-8");
            assert_eq!(body.as_slice(), b"console.log('hello');");
        }
        PreparedResponse::Text { .. } => panic!("js request should keep serving the asset"),
        PreparedResponse::Redirect { .. } => panic!("js request should not redirect"),
    }

    match prepare_static_response("/images/icon.png", None, &build_state) {
        PreparedResponse::Bytes { body, content_type } => {
            assert_eq!(content_type, "image/png");
            assert_eq!(body.as_slice(), &[0x89, b'P', b'N', b'G']);
        }
        PreparedResponse::Text { .. } => panic!("image request should keep serving the asset"),
        PreparedResponse::Redirect { .. } => panic!("image request should not redirect"),
    }
}

#[test]
fn failed_build_traversal_request_still_returns_not_found() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    let build_state = configure_failed_build_state(
        &output_dir,
        "<html><body>error</body></html>",
        Some(PathBuf::from("index.html")),
    );

    match prepare_static_response("/../secret.txt", None, &build_state) {
        PreparedResponse::Text {
            status_line,
            content_type,
            body,
        } => {
            assert_eq!(status_line, "404 NOT FOUND");
            assert_eq!(content_type, "text/plain; charset=utf-8");
            assert_eq!(body, "Not Found");
        }
        PreparedResponse::Bytes { .. } => panic!("traversal request should return not found"),
        PreparedResponse::Redirect { .. } => panic!("traversal request should not redirect"),
    }
}

#[test]
fn root_request_uses_failed_build_error_page_without_entry_page() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    let build_state =
        configure_failed_build_state(&output_dir, "<html><body>root error</body></html>", None);

    match prepare_static_response("/", None, &build_state) {
        PreparedResponse::Text {
            status_line,
            content_type,
            body,
        } => {
            assert_eq!(status_line, "200 OK");
            assert_eq!(content_type, "text/html; charset=utf-8");
            assert!(body.contains("root error"));
        }
        PreparedResponse::Bytes { .. } => {
            panic!("root request should render the failed-build page")
        }
        PreparedResponse::Redirect { .. } => panic!("root request should not redirect"),
    }
}

#[test]
fn redirects_are_returned_even_during_failed_build() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("about")).expect("should create about output dir");
    fs::write(
        output_dir.join("about/index.html"),
        "<html><body>about</body></html>",
    )
    .expect("should write page");
    let build_state = configure_failed_build_state(
        &output_dir,
        "<html><body>build failed</body></html>",
        Some(PathBuf::from("index.html")),
    );

    match prepare_static_response("/about", Some("x=1"), &build_state) {
        PreparedResponse::Redirect {
            status_line,
            location,
        } => {
            assert_eq!(status_line, "302 FOUND");
            assert_eq!(location, "/about/?x=1");
        }
        PreparedResponse::Text { .. } | PreparedResponse::Bytes { .. } => {
            panic!("canonical page redirects should run before failed-build html substitution")
        }
    }
}

#[test]
fn rapid_loopback_ping_requests_do_not_stall_request_handling() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = listener
        .local_addr()
        .expect("listener should report bound address");
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));

    let server_state = Arc::clone(&state);
    let server_thread = thread::spawn(move || {
        for _ in 0..5 {
            let (stream, _) = listener.accept().expect("should accept client");
            handle_connection_with_timeouts(
                stream,
                Arc::clone(&server_state),
                Duration::from_millis(100),
                Duration::from_millis(100),
            )
            .expect("ping request should succeed");
        }
    });

    for _ in 0..5 {
        let response = loopback_get(address, "/__moth/ping");
        assert!(response.contains("200 OK"));
        assert!(response.ends_with("ok"));
    }

    server_thread.join().expect("server thread should finish");
}

#[test]
fn partial_loopback_requests_time_out_without_stalling_worker_threads() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = listener
        .local_addr()
        .expect("listener should report bound address");
    let state = Arc::new(DevServerState::new(PathBuf::from("dev")));
    let (done_sender, done_receiver) = mpsc::channel();

    let server_state = Arc::clone(&state);
    let server_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("should accept client");
        handle_connection_with_timeouts(
            stream,
            server_state,
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .expect("partial request should time out cleanly");
        done_sender
            .send(())
            .expect("server thread should signal completion");
    });

    let _client = TcpStream::connect(address).expect("client should connect");
    // The 50ms read timeout is the contract: a partial request must free its worker inside this
    // bound instead of holding it until the client goes away.
    await_worker_completion(
        "partial-request server",
        &done_receiver,
        server_thread,
        PARTIAL_REQUEST_DEADLINE,
    );
}

fn loopback_get(address: std::net::SocketAddr, request_target: &str) -> String {
    let mut client = TcpStream::connect(address).expect("client should connect");
    client
        .write_all(format!("GET {request_target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
        .expect("client should send request");
    let mut response = String::new();
    client
        .read_to_string(&mut response)
        .expect("client should read response");
    response
}

#[test]
fn encoded_loopback_targets_resolve_and_redirect_with_encoded_locations() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = listener
        .local_addr()
        .expect("listener should report bound address");
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let output_dir = root.join("dev");
    fs::create_dir_all(output_dir.join("docs")).expect("should create docs dir");
    fs::write(output_dir.join("docs/hello world.txt"), "spaced asset")
        .expect("should write spaced asset");
    fs::create_dir_all(output_dir.join("about us")).expect("should create page dir");
    fs::write(output_dir.join("about us/index.html"), "<h1>about</h1>").expect("should write page");

    let mut build_state =
        configure_build_state(&output_dir, Some(PathBuf::from("index.html")), true);
    let mut config = Config::new(root);
    config
        .builder_settings
        .set_string("html", "origin", String::from("/Café docs/%20"), None);
    let mut strings = StringTable::new();
    build_state.html_site_config =
        parse_html_site_config(&config, &mut DiagnosticStringBuilder::new(&mut strings))
            .expect("decoded origin should parse");
    let state = DevServerState::new(output_dir);
    *state
        .build_state
        .lock()
        .expect("build-state lock should be available") = build_state;
    let state = Arc::new(state);

    let server_state = Arc::clone(&state);
    let server_thread = thread::spawn(move || {
        for _ in 0..5 {
            let (stream, _) = listener.accept().expect("should accept client");
            handle_connection_with_timeouts(
                stream,
                Arc::clone(&server_state),
                Duration::from_millis(200),
                Duration::from_millis(200),
            )
            .expect("request should succeed");
        }
    });

    let asset_response = loopback_get(address, "/Caf%c3%a9%20docs/%2520/docs/hello%20world.txt");
    assert!(
        asset_response.starts_with("HTTP/1.1 200 OK"),
        "{asset_response}"
    );
    assert!(asset_response.ends_with("spaced asset"), "{asset_response}");

    let redirect_response = loopback_get(address, "/Caf%C3%A9%20docs/%2520/about%20us");
    assert!(
        redirect_response.starts_with("HTTP/1.1 302 FOUND"),
        "{redirect_response}"
    );
    assert!(
        redirect_response.contains("Location: /Caf%C3%A9%20docs/%2520/about%20us/\r\n"),
        "{redirect_response}"
    );

    let malformed_response = loopback_get(address, "/Caf%C3%A9%20docs/%2520/bad%FF.txt");
    assert!(
        malformed_response.starts_with("HTTP/1.1 400 BAD REQUEST"),
        "{malformed_response}"
    );

    let root_response = loopback_get(address, "/");
    assert!(
        root_response.contains("Location: /Caf%C3%A9%20docs/%2520/\r\n"),
        "{root_response}"
    );

    let outside_origin = loopback_get(address, "/Caf%C3%A9%20docs/%2520%2Fdocs/hello%20world.txt");
    assert!(
        outside_origin.starts_with("HTTP/1.1 404 NOT FOUND"),
        "{outside_origin}"
    );

    server_thread.join().expect("server thread should finish");
}

#[test]
fn request_target_split_keeps_encoded_hash_and_drops_fragment() {
    assert_eq!(
        super::split_request_target("/docs/a%23b/index.html?x=1#anchor"),
        ("/docs/a%23b/index.html", Some("x=1"))
    );
    assert_eq!(
        super::split_request_target("/docs#fragment"),
        ("/docs", None)
    );
    assert_eq!(super::split_request_target("/"), ("/", None));
}
