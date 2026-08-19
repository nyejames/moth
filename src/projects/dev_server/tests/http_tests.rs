//! Tests for dev-server HTTP routing during successful and failed builds.

use super::{PreparedResponse, handle_connection_with_timeouts, prepare_static_response};
use crate::projects::dev_server::state::{BuildState, DevServerState};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn bind_loopback_listener() -> Option<TcpListener> {
    match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(error) => panic!("should bind test listener: {error}"),
    }
}

fn configure_failed_build_state(
    output_dir: &Path,
    last_error_html: &str,
    entry_page_rel: Option<PathBuf>,
) -> BuildState {
    let mut build_state = BuildState::new(output_dir.to_path_buf());
    build_state.last_build_ok = false;
    build_state.last_error_html = Some(last_error_html.to_owned());
    build_state.last_build_version = 9;
    build_state.entry_page_rel = entry_page_rel;
    build_state
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
        PreparedResponse::File { .. } => {
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
        PreparedResponse::File { path, content_type } => {
            assert_eq!(content_type, "text/css; charset=utf-8");
            assert_eq!(
                fs::read_to_string(path).expect("css file should be readable"),
                "body { color: red; }"
            );
        }
        PreparedResponse::Text { .. } => panic!("css request should keep serving the asset"),
        PreparedResponse::Redirect { .. } => panic!("css request should not redirect"),
    }

    match prepare_static_response("/scripts/app.js", None, &build_state) {
        PreparedResponse::File { path, content_type } => {
            assert_eq!(content_type, "application/javascript; charset=utf-8");
            assert_eq!(
                fs::read_to_string(path).expect("js file should be readable"),
                "console.log('hello');"
            );
        }
        PreparedResponse::Text { .. } => panic!("js request should keep serving the asset"),
        PreparedResponse::Redirect { .. } => panic!("js request should not redirect"),
    }

    match prepare_static_response("/images/icon.png", None, &build_state) {
        PreparedResponse::File { path, content_type } => {
            assert_eq!(content_type, "image/png");
            assert_eq!(
                fs::read(path).expect("png file should be readable"),
                vec![0x89, b'P', b'N', b'G']
            );
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
        PreparedResponse::File { .. } => panic!("traversal request should return not found"),
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
        PreparedResponse::File { .. } => panic!("root request should render the failed-build page"),
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
        PreparedResponse::Text { .. } | PreparedResponse::File { .. } => {
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
        let mut client = TcpStream::connect(address).expect("client should connect");
        client
            .write_all(b"GET /__moth/ping HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("client should send ping request");
        let mut response = String::new();
        client
            .read_to_string(&mut response)
            .expect("client should read ping response");
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
    thread::spawn(move || {
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
    done_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("timed-out partial request should not stall the worker thread");
}
