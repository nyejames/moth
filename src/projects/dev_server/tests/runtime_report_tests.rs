//! Socket-level runtime report validation and invocation-local state regressions.

use super::{
    MAX_HEADER_LINE_BYTES, MAX_HEADER_LINES, MAX_REPORT_BODY_BYTES, MAX_RUNTIME_REPORTS,
    RuntimeReport, append_terminal_text, format_runtime_report, handle_connection_with_timeouts,
};
use crate::projects::dev_server::state::{DevServerState, RuntimeReportKey};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

struct ReportServer {
    _output: tempfile::TempDir,
    state: Arc<DevServerState>,
}

impl ReportServer {
    fn new(origin: &str) -> Self {
        let output = tempfile::tempdir().expect("should create output directory");
        fs::create_dir(output.path().join("other")).expect("should create other entry directory");
        fs::write(
            output.path().join("index.html"),
            "<html><body>home page</body></html>",
        )
        .expect("should write home entry");
        fs::write(
            output.path().join("other/index.html"),
            "<html><body>other page</body></html>",
        )
        .expect("should write other entry");
        let state = Arc::new(DevServerState::new(output.path().to_path_buf()));
        {
            let mut build = state.build_state.lock().expect("should lock build state");
            build.last_build_ok = true;
            build.last_build_version = 7;
            build.entry_page_rel = Some("index.html".into());
            build.html_site_config.origin = origin.to_owned();
            build.last_build_messages_summary = "successful compilation".to_owned();
        }
        Self {
            _output: output,
            state,
        }
    }

    fn request(&self, request: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("should bind report test listener");
        let address = listener.local_addr().expect("listener should have address");
        let state = Arc::clone(&self.state);
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("should accept report request");
            handle_connection_with_timeouts(
                stream,
                state,
                Duration::from_millis(100),
                Duration::from_secs(2),
            )
            .expect("report request should be handled without an infrastructure error");
        });
        let mut client = TcpStream::connect(address).expect("should connect report client");
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("should bound client read");
        client
            .write_all(request.as_bytes())
            .expect("should send request");
        client
            .shutdown(Shutdown::Write)
            .expect("should finish request bytes");
        let mut response = Vec::new();
        let read_result = client.read_to_end(&mut response);
        worker.join().expect("report worker should finish");
        // Rejected bodies need not be drained. TCP may reset after delivering the HTTP response.
        if let Err(error) = read_result {
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
            assert!(!response.is_empty(), "reset must follow an HTTP response");
        }
        String::from_utf8(response).expect("response should be UTF-8")
    }

    fn post(&self, path: &str, body: &Value) -> String {
        self.post_text(
            path,
            &body.to_string(),
            "Origin: http://localhost\r\nContent-Type: application/json\r\n",
        )
    }

    fn post_text(&self, path: &str, body: &str, headers: &str) -> String {
        self.request(&format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len(),
        ))
    }

    fn count(&self) -> usize {
        self.state
            .runtime_reports
            .lock()
            .expect("should lock runtime ledger")
            .keys
            .len()
    }
}

fn entry_report() -> Value {
    json!({
        "build": 7,
        "entry": "index.html",
        "invocation": "page_1-A",
        "category": "entry_error",
        "code": 0,
        "message": "startup returned an Error",
        "source": {"file": "pages/@page.moth", "line": 12, "column": 3},
    })
}

fn assert_status(response: &str, status: u16) {
    assert!(
        response.starts_with(&format!("HTTP/1.1 {status} ")),
        "unexpected response: {response}"
    );
    if status == 204 {
        assert!(response.ends_with("\r\n\r\n"));
        assert!(response.contains("Content-Length: 0\r\n"));
    }
}

#[test]
fn runtime_reports_deduplicate_and_leave_successful_pages_and_build_unchanged() {
    let server = ReportServer::new("/");
    let before = server
        .state
        .build_state
        .lock()
        .expect("should lock build state")
        .clone();
    let report = entry_report();
    assert_status(&server.post("/__moth/runtime-report", &report), 204);
    assert_status(&server.post("/__moth/runtime-report", &report), 204);
    assert_eq!(server.count(), 1);

    let mut second = report.clone();
    second["invocation"] = json!("page_2");
    assert_status(&server.post("/__moth/runtime-report", &second), 204);
    assert_eq!(server.count(), 2);
    second["entry"] = json!("other/index.html");
    assert_status(&server.post("/__moth/runtime-report", &second), 204);
    assert_eq!(server.count(), 3);

    for (path, content) in [("/", "home page"), ("/other/", "other page")] {
        let response = server.request(&format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"));
        assert_status(&response, 200);
        assert!(response.contains(content));
        assert!(
            response.contains("/__moth/events"),
            "hot reload should remain injected"
        );
    }
    let after = server
        .state
        .build_state
        .lock()
        .expect("should lock build state");
    assert_eq!(after.last_build_ok, before.last_build_ok);
    assert_eq!(after.last_error_html, before.last_error_html);
    assert_eq!(after.last_build_version, before.last_build_version);
    assert_eq!(after.entry_page_rel, before.entry_page_rel);
    assert_eq!(after.output_dir, before.output_dir);
    assert_eq!(after.html_site_config, before.html_site_config);
    assert_eq!(
        after.last_build_messages_summary,
        before.last_build_messages_summary
    );
    assert!(
        server
            .state
            .clients
            .lock()
            .expect("should lock clients")
            .is_empty()
    );
}

#[test]
fn runtime_reports_reject_stale_future_unknown_and_failed_builds() {
    let server = ReportServer::new("/");
    for build in [6, 8] {
        let mut report = entry_report();
        report["build"] = json!(build);
        assert_status(&server.post("/__moth/runtime-report", &report), 409);
    }
    for entry in [
        "missing/index.html",
        "../index.html",
        "/index.html",
        "index.html?x=1",
    ] {
        let mut report = entry_report();
        report["entry"] = json!(entry);
        assert_status(&server.post("/__moth/runtime-report", &report), 409);
    }
    server
        .state
        .build_state
        .lock()
        .expect("should lock build state")
        .last_build_ok = false;
    assert_status(&server.post("/__moth/runtime-report", &entry_report()), 409);
    assert_eq!(server.count(), 0);
}

#[test]
fn runtime_report_categories_share_one_invocation_deduplication_key() {
    let server = ReportServer::new("/");
    assert_status(&server.post("/__moth/runtime-report", &entry_report()), 204);
    for category in ["assertion", "startup_fault"] {
        let mut report = entry_report();
        report["category"] = json!(category);
        report
            .as_object_mut()
            .expect("report should be an object")
            .remove("code");
        report
            .as_object_mut()
            .expect("report should be an object")
            .remove("message");
        report
            .as_object_mut()
            .expect("report should be an object")
            .remove("source");
        assert_status(&server.post("/__moth/runtime-report", &report), 204);
        assert_status(&server.post("/__moth/runtime-report", &report), 204);
        assert_eq!(server.count(), 1, "{category} must be a duplicate");
    }
    assert_eq!(server.count(), 1);
}

#[test]
fn runtime_reports_reject_invalid_shapes_and_numeric_codes() {
    let server = ReportServer::new("/");
    for text in ["{", "[]", "null", "{}"] {
        assert_status(
            &server.post_text(
                "/__moth/runtime-report",
                text,
                "Origin: http://localhost\r\nContent-Type: application/json\r\n",
            ),
            400,
        );
    }
    let mut invalid = Vec::new();
    for field in ["build", "entry", "invocation", "category", "code"] {
        let mut report = entry_report();
        report
            .as_object_mut()
            .expect("report should be an object")
            .remove(field);
        invalid.push(report);
    }
    for code in [
        json!(-1),
        json!(4294967296_u64),
        json!(1.5),
        json!("3"),
        Value::Null,
        json!(true),
    ] {
        let mut report = entry_report();
        report["code"] = code;
        invalid.push(report);
    }
    for category in ["assertion", "startup_fault"] {
        for code in [json!(0), Value::Null] {
            let mut report = entry_report();
            report["category"] = json!(category);
            report["code"] = code;
            invalid.push(report);
        }
    }
    for (field, value) in [
        ("build", json!(-1)),
        ("build", json!(1.5)),
        ("build", json!("7")),
        ("entry", json!(7)),
        ("invocation", json!(false)),
        ("category", json!("unknown")),
        ("message", json!(42)),
        ("source", json!("not a location")),
        ("unexpected", json!(true)),
    ] {
        let mut report = entry_report();
        report[field] = value;
        invalid.push(report);
    }
    for (field, value) in [
        ("extra", json!(1)),
        ("line", json!(-1)),
        ("column", json!(1.5)),
        ("file", json!(2)),
    ] {
        let mut report = entry_report();
        report["source"][field] = value;
        invalid.push(report);
    }
    for field in ["file", "line", "column"] {
        let mut report = entry_report();
        report["source"]
            .as_object_mut()
            .expect("source should be an object")
            .remove(field);
        invalid.push(report);
    }
    for report in invalid {
        assert_status(&server.post("/__moth/runtime-report", &report), 400);
    }
    assert_eq!(server.count(), 0);

    let mut maximum_code = entry_report();
    maximum_code["code"] = json!(u32::MAX);
    assert_status(&server.post("/__moth/runtime-report", &maximum_code), 204);
    let duplicate_field =
        entry_report()
            .to_string()
            .replacen("\"code\":0", "\"code\":0,\"code\":0", 1);
    assert_status(
        &server.post_text(
            "/__moth/runtime-report",
            &duplicate_field,
            "Origin: http://localhost\r\nContent-Type: application/json\r\n",
        ),
        400,
    );
}

#[test]
fn runtime_report_wire_shapes_require_objects_and_literal_category_strings() {
    let server = ReportServer::new("/");
    let positional_report = json!([
        7, "index.html", "page_1-A", "entry_error", 0, "returned Error",
        {"file": "pages/@page.moth", "line": 12, "column": 3},
    ]);
    assert_status(
        &server.post("/__moth/runtime-report", &positional_report),
        400,
    );
    let mut report = entry_report();
    report["source"] = json!(["pages/@page.moth", 12, 3]);
    assert_status(&server.post("/__moth/runtime-report", &report), 400);
    for category in ["entry_error", "assertion", "startup_fault"] {
        report = entry_report();
        report["category"] = json!({category: null});
        assert_status(&server.post("/__moth/runtime-report", &report), 400);
    }
    assert_eq!(server.count(), 0);
    report = entry_report();
    report["source"] = Value::Null;
    assert_status(&server.post("/__moth/runtime-report", &report), 204);
}

#[test]
fn concurrent_same_key_socket_reports_are_accepted_only_once() {
    let server = ReportServer::new("/");
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind concurrent listener");
    let address = listener.local_addr().expect("listener should have address");
    let body = entry_report().to_string();
    let request = format!(
        "POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len(),
    );
    let mut clients = Vec::new();
    for _ in 0..2 {
        let mut client = TcpStream::connect(address).expect("should connect concurrent client");
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("should bound client read");
        client
            .write_all(request.as_bytes())
            .expect("should send concurrent report");
        client
            .shutdown(Shutdown::Write)
            .expect("should finish concurrent request");
        clients.push(client);
    }
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (stream, _) = listener.accept().expect("should accept concurrent report");
        let state = Arc::clone(&server.state);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            handle_connection_with_timeouts(
                stream,
                state,
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
            .expect("concurrent report should be accepted");
        }));
    }
    barrier.wait();
    let mut responses = Vec::new();
    for mut client in clients {
        let mut response = String::new();
        client
            .read_to_string(&mut response)
            .expect("should read concurrent response");
        responses.push(response);
    }
    for worker in workers {
        worker
            .join()
            .expect("concurrent report worker should finish");
    }
    for response in responses {
        assert_status(&response, 204);
    }
    assert_eq!(server.count(), 1);
}

#[test]
fn runtime_reports_enforce_utf8_byte_and_invocation_limits() {
    let server = ReportServer::new("/");
    for (field, value) in [
        ("entry", "é".repeat(513)),
        ("message", "é".repeat(2049)),
        ("invocation", "a".repeat(65)),
        ("invocation", String::new()),
        ("invocation", "bad id".to_owned()),
        ("invocation", "é".to_owned()),
    ] {
        let mut report = entry_report();
        report[field] = json!(value);
        assert_status(&server.post("/__moth/runtime-report", &report), 400);
    }
    let mut report = entry_report();
    report["source"]["file"] = json!("é".repeat(513));
    assert_status(&server.post("/__moth/runtime-report", &report), 400);
    report["source"]["file"] = json!("é".repeat(512));
    report["message"] = json!("é".repeat(2048));
    report["invocation"] = json!("A_-9".repeat(16));
    assert_status(&server.post("/__moth/runtime-report", &report), 204);
}

#[test]
fn runtime_reports_require_same_origin_json_and_bounded_exact_body() {
    let server = ReportServer::new("/");
    let body = entry_report().to_string();
    for headers in [
        "Content-Type: application/json\r\n",
        "Origin: http://foreign\r\nContent-Type: application/json\r\n",
        "Origin: https://localhost\r\nContent-Type: application/json\r\n",
        "Origin: null\r\nContent-Type: application/json\r\n",
    ] {
        assert_status(
            &server.post_text("/__moth/runtime-report", &body, headers),
            403,
        );
    }
    for headers in [
        "Origin: http://localhost\r\n",
        "Origin: http://localhost\r\nContent-Type: text/plain\r\n",
    ] {
        assert_status(
            &server.post_text("/__moth/runtime-report", &body, headers),
            415,
        );
    }
    for length in ["", "-1", "no", "+10"] {
        assert_status(&server.request(&format!("POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n")), 400);
    }
    assert_status(&server.request("POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\n\r\n"), 400);
    assert_status(&server.request(&format!("POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", MAX_REPORT_BODY_BYTES + 1)), 413);
    assert_status(&server.request("POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{}"), 400);
    assert_status(&server.post_text("/__moth/runtime-report", &body, "Origin: http://localhost\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n"), 400);
    let padded = format!("{body}{}", " ".repeat(MAX_REPORT_BODY_BYTES - body.len()));
    assert_status(
        &server.post_text(
            "/__moth/runtime-report",
            &padded,
            "Origin: http://localhost\r\nContent-Type: application/json; charset=utf-8\r\n",
        ),
        204,
    );
    assert_eq!(server.count(), 1);
}

#[test]
fn runtime_report_route_respects_origin_prefix_and_other_methods_remain_disallowed() {
    let server = ReportServer::new("/docs");
    assert_status(
        &server.post("/docs/__moth/runtime-report", &entry_report()),
        204,
    );
    for path in [
        "/__moth/runtime-report",
        "/docsevil/__moth/runtime-report",
        "/docs/other",
        "/docs/__moth/ping",
    ] {
        assert_status(&server.post(path, &entry_report()), 405);
    }
    for method in ["PUT", "DELETE", "OPTIONS", "HEAD"] {
        assert_status(
            &server.request(&format!(
                "{method} /docs/__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )),
            405,
        );
    }
    assert_status(
        &server.request("GET /docs/__moth/ping HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        200,
    );
    let home = server.request("GET /docs/ HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_status(&home, 200);
    assert!(home.contains("home page"));
    assert!(home.contains("/docs/__moth/events"));
}

#[test]
fn runtime_report_cap_admits_duplicates_and_resets_only_for_current_new_build() {
    let server = ReportServer::new("/");
    {
        let mut ledger = server
            .state
            .runtime_reports
            .lock()
            .expect("should lock runtime ledger");
        ledger.build = Some(7);
        for invocation in 0..MAX_RUNTIME_REPORTS - 1 {
            ledger.keys.insert(RuntimeReportKey {
                build: 7,
                entry: "index.html".to_owned(),
                invocation: format!("seed-{invocation}"),
            });
        }
    }
    assert_status(&server.post("/__moth/runtime-report", &entry_report()), 204);
    assert_eq!(server.count(), MAX_RUNTIME_REPORTS);
    assert_status(&server.post("/__moth/runtime-report", &entry_report()), 204);
    let mut extra = entry_report();
    extra["invocation"] = json!("over-cap");
    assert_status(&server.post("/__moth/runtime-report", &extra), 429);
    extra["build"] = json!(8);
    assert_status(&server.post("/__moth/runtime-report", &extra), 409);
    assert_eq!(server.count(), MAX_RUNTIME_REPORTS);
    server
        .state
        .build_state
        .lock()
        .expect("should lock build state")
        .last_build_version = 8;
    assert_status(&server.post("/__moth/runtime-report", &entry_report()), 409);
    assert_eq!(server.count(), MAX_RUNTIME_REPORTS);
    assert_status(&server.post("/__moth/runtime-report", &extra), 204);
    assert_eq!(server.count(), 1);
    assert_eq!(
        server
            .state
            .runtime_reports
            .lock()
            .expect("should lock ledger")
            .build,
        Some(8)
    );
}

#[test]
fn header_line_and_count_bounds_include_the_request_line() {
    let server = ReportServer::new("/");
    let long_request = format!(
        "GET /{} HTTP/1.1\r\n\r\n",
        "a".repeat(MAX_HEADER_LINE_BYTES)
    );
    assert_status(&server.request(&long_request), 400);
    let long_header = format!(
        "GET /__moth/ping HTTP/1.1\r\nX: {}\r\n\r\n",
        "a".repeat(MAX_HEADER_LINE_BYTES)
    );
    assert_status(&server.request(&long_header), 400);
    let boundary = format!(
        "GET /__moth/ping HTTP/1.1\r\n{}\r\n",
        "X: a\r\n".repeat(MAX_HEADER_LINES - 2)
    );
    assert_status(&server.request(&boundary), 200);
    let excessive = format!(
        "GET /__moth/ping HTTP/1.1\r\n{}\r\n",
        "X: a\r\n".repeat(MAX_HEADER_LINES - 1)
    );
    assert_status(&server.request(&excessive), 400);
    let boundary_line = format!(
        "GET /__moth/ping HTTP/1.1\r\nX: {}\r\n\r\n",
        "a".repeat(MAX_HEADER_LINE_BYTES - 5)
    );
    assert_status(&server.request(&boundary_line), 200);
}

#[test]
fn partial_runtime_body_times_out_without_holding_the_worker() {
    let server = ReportServer::new("/");
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind partial-body listener");
    let address = listener.local_addr().expect("listener should have address");
    let state = Arc::clone(&server.state);
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("should accept partial body");
        handle_connection_with_timeouts(
            stream,
            state,
            Duration::from_millis(50),
            Duration::from_secs(2),
        )
        .expect("body timeout should produce an HTTP response");
    });
    let mut client = TcpStream::connect(address).expect("should connect partial-body client");
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("should bound client read");
    client.write_all(
        b"POST /__moth/runtime-report HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{}",
    ).expect("should send partial body without closing the connection");
    let mut response = String::new();
    client
        .read_to_string(&mut response)
        .expect("body timeout should release connection");
    worker.join().expect("partial-body worker should finish");
    assert_status(&response, 400);
    assert_eq!(server.count(), 0);
}

#[test]
fn terminal_reports_escape_every_control_and_identify_successful_build_and_category() {
    let control_values: Vec<u32> = (0..=0x1f)
        .chain(0x7f..=0x9f)
        .chain([0x061c, 0x200e, 0x200f])
        .chain(0x202a..=0x202e)
        .chain(0x2066..=0x2069)
        .collect();
    let controls: String = control_values
        .iter()
        .map(|&value| char::from_u32(value).expect("control should be a Unicode scalar"))
        .collect();
    let mut escaped = String::new();
    append_terminal_text(&mut escaped, &controls);
    for value in &control_values {
        assert!(escaped.contains(&format!("\\u{{{value:04x}}}")));
    }
    assert!(!escaped.chars().any(char::is_control));
    for (category, title) in [
        ("entry_error", "Entry Error"),
        ("assertion", "Assertion Failure"),
        ("startup_fault", "Unexpected Startup Fault"),
    ] {
        let mut value = entry_report();
        value["category"] = json!(category);
        if category != "entry_error" {
            value
                .as_object_mut()
                .expect("report should be object")
                .remove("code");
        }
        value["entry"] = json!("index\u{001b}.html");
        value["message"] = json!(format!("<script>unsafe</script>café 漢字{controls}"));
        value["source"]["file"] = json!(format!("source\n\u{009b}café 漢字{controls}.moth"));
        let report: RuntimeReport =
            serde_json::from_value(value).expect("report shape should decode");
        let rendered = format_runtime_report(&report);
        assert!(rendered.starts_with(&format!("Dev runtime: {title}")));
        assert!(rendered.contains("build #7 succeeded"));
        assert!(rendered.contains("index\\u{001b}.html"));
        assert!(rendered.contains(&format!(
            "\n  Source: source\\u{{000a}}\\u{{009b}}café 漢字{escaped}.moth:12:3"
        )));
        assert!(rendered.contains(&format!(
            "\n  Message: <script>unsafe</script>café 漢字{escaped}"
        )));
        for &value in control_values.iter().filter(|&&value| value >= 0x061c) {
            let character = char::from_u32(value).expect("bidi control is a Unicode scalar");
            assert!(!rendered.contains(character), "bidi control {value:04x}");
        }
        assert!(
            !rendered
                .chars()
                .any(|character| character.is_control() && character != '\n')
        );
        assert_eq!(
            rendered.contains("Error code: 0"),
            category == "entry_error"
        );
    }
}
