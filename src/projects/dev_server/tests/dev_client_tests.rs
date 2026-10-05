//! Execute the served dev client with a minimal DOM to protect observable reporting behaviour.

use super::dev_client_snippet;
use std::process::Command;

const PAGE_HARNESS: &str = r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
let nextInvocation = 0;
function page(script, transport = 'reject', secure = true) {
  const elements = [], requests = [], errors = [], sources = [];
  class Element {
    constructor(tag) { this.tag = tag; this.children = []; this.textContent = ''; this.className = ''; this.properties = {}; this.style = {setProperty: (name, value, priority) => { this.properties[name] = {value, priority}; }}; elements.push(this); }
    appendChild(child) { this.children.push(child); return child; }
    attachShadow() { this.shadowRoot = new Element('shadow'); return this.shadowRoot; }
    set innerHTML(_) { throw new Error('application text must never be HTML'); }
  }
  const root = new Element('html');
  let reloads = 0;
  const context = vm.createContext({
    document: {documentElement: root, createElement: tag => new Element(tag)},
    window: {location: {reload: () => reloads++}},
    crypto: secure ? {randomUUID: () => 'invocation_' + ++nextInvocation} : undefined,
    TextEncoder,
    console: {error: (...values) => errors.push(values)},
    EventSource: class {
      constructor(path) { this.path = path; this.listeners = {}; this.closed = false; sources.push(this); }
      addEventListener(name, callback) { this.listeners[name] = callback; }
      close() { this.closed = true; }
    },
    fetch: (path, options) => {
      requests.push({path, options});
      if (transport === 'throw') throw new Error('network unavailable');
      return transport === 'reject' ? Promise.reject(new Error('network unavailable')) : Promise.resolve({status: 204});
    }
  });
  vm.runInContext(script, context);
  return {context, root, elements, requests, errors, sources, reloads: () => reloads};
}
function textFor(tab, tag, className) {
  const element = tab.elements.find(element => element.tag === tag && (!className || element.className === className));
  assert.ok(element, 'missing ' + tag + ' ' + className);
  return element.textContent;
}
"#;

fn run_client_test(snippet: &str, assertions: &str) -> Vec<u8> {
    let script = snippet
        .split_once("<script>\n")
        .expect("client script starts")
        .1
        .strip_suffix("</script>\n")
        .expect("client script ends");
    let quoted_script = serde_json::Value::String(script.to_owned());
    let node_script = format!(
        "{PAGE_HARNESS}\nconst clientScript = {quoted_script};\n(async () => {{\n{assertions}\nawait new Promise(resolve => setImmediate(resolve));\n}})().catch(error => {{ process.stderr.write(String(error.stack)); process.exitCode = 1; }});"
    );
    let output = Command::new("node")
        .args(["--eval", &node_script])
        .output()
        .expect("Node must be available to exercise the dev client");
    assert!(
        output.status.success(),
        "dev client failed in Node:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn entry_error_is_literal_prominent_and_reported_once_with_hot_reload() {
    run_client_test(
        &dev_client_snippet("/docs", Some((7, "guide/index.html"))),
        r#"
const tab = page(clientScript);
const hostile = '<img src=x onerror="alert(1)"></script> & application text';
assert.doesNotThrow(() => tab.context.__moth_record_entry_failure(301, hostile, {file: '<source>', line: 4, column: 2}));
tab.context.__moth_record_entry_failure(302, 'duplicate', null);
tab.context.__moth_record_startup_fault('startup_fault', new Error('second outcome'));
assert.equal(textFor(tab, 'h1'), 'Entry Error');
assert.equal(textFor(tab, 'pre', 'msg'), 'code 301: ' + hostile + '\nat <source>:4:2');
assert.equal(textFor(tab, 'div', 'meta'), 'Build #7 succeeded | Runtime failure | Entry: guide/index.html');
assert.equal(tab.root.children.length, 1);
const host = tab.root.children[0];
assert.ok(host.shadowRoot);
assert.equal(host.properties.position.value, 'fixed');
assert.equal(host.properties.inset.value, '0');
assert.equal(host.properties['z-index'].value, '2147483647');
assert.equal(host.properties['z-index'].priority, 'important');
assert.equal(tab.errors.length, 1);
assert.equal(tab.requests.length, 1);
assert.equal(tab.requests[0].path, '/docs/__moth/runtime-report');
assert.equal(tab.requests[0].options.method, 'POST');
assert.equal(tab.requests[0].options.keepalive, true);
assert.equal(tab.requests[0].options.headers['Content-Type'], 'application/json');
assert.deepEqual(JSON.parse(tab.requests[0].options.body), {build: 7, entry: 'guide/index.html', invocation: 'invocation_1', category: 'entry_error', code: 301, message: hostile, source: {file: '<source>', line: 4, column: 2}});
assert.equal(tab.sources.length, 1);
assert.equal(tab.sources[0].path, '/docs/__moth/events');
assert.equal(tab.sources[0].closed, false);
tab.sources[0].listeners.reload();
assert.equal(tab.reloads(), 1);
assert.equal(tab.context.onerror, undefined);
assert.equal(tab.context.onunhandledrejection, undefined);
"#,
    );
}

#[test]
fn startup_categories_and_independent_tabs_keep_invocations_local() {
    run_client_test(
        &dev_client_snippet("/", Some((9, "index.html"))),
        r#"
const assertion = page(clientScript), fault = page(clientScript);
assert.equal(fault.requests.length, 0);
assertion.context.__moth_record_startup_fault('assertion', new Error('<b>assert message</b>'));
assert.equal(textFor(assertion, 'h1'), 'Assertion Failure');
assert.equal(textFor(assertion, 'pre', 'msg'), '<b>assert message</b>');
assert.equal(fault.root.children.length, 0);
fault.context.__moth_record_startup_fault('startup_fault', 'unknown thrown text');
assert.equal(textFor(fault, 'h1'), 'Unexpected Startup Fault');
assert.equal(textFor(fault, 'pre', 'msg'), 'unknown thrown text');
for (const tab of [assertion, fault]) {
  tab.context.__moth_record_startup_fault('assertion', 'duplicate');
  assert.equal(tab.requests.length, 1);
  assert.equal(tab.errors.length, 1);
  assert.equal(tab.sources[0].closed, false);
  assert.equal(JSON.parse(tab.requests[0].options.body).code, undefined);
}
assert.notEqual(JSON.parse(assertion.requests[0].options.body).invocation, JSON.parse(fault.requests[0].options.body).invocation);
assert.equal(JSON.parse(assertion.requests[0].options.body).category, 'assertion');
assert.equal(JSON.parse(fault.requests[0].options.body).category, 'startup_fault');
"#,
    );
}

#[test]
fn transport_failures_and_hostile_thrown_values_never_recurse_or_throw() {
    run_client_test(
        &dev_client_snippet("/", Some((2, "index.html"))),
        r#"
for (const transport of ['reject', 'throw']) {
  const tab = page(clientScript, transport);
  const error = {get message() {throw new Error('hostile getter');}, get __moth_location() {throw new Error('hostile location');}, toString() {throw new Error('hostile conversion');}};
  assert.doesNotThrow(() => tab.context.__moth_record_startup_fault('startup_fault', error));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(textFor(tab, 'h1'), 'Unexpected Startup Fault');
  assert.equal(textFor(tab, 'pre', 'msg'), 'Unavailable fault text');
  tab.context.__moth_record_startup_fault('startup_fault', new Error('second fault'));
  assert.equal(tab.root.children.length, 1);
  assert.equal(tab.requests.length, 1);
  assert.equal(tab.errors.length, 1);
  assert.equal(tab.sources[0].closed, false);
}
"#,
    );
}

#[test]
fn client_bounds_unicode_and_json_expansion_and_preserves_build_identity() {
    run_client_test(
        &dev_client_snippet("/", Some((u64::MAX, "index.html"))),
        r#"
for (const message of ['😀'.repeat(5000), '\u0000'.repeat(5000)]) {
  const tab = page(clientScript, 'accept');
  tab.context.__moth_record_entry_failure(0, message, {file: '😀'.repeat(1000), line: 0, column: 4294967295});
  assert.equal(tab.requests.length, 1);
  const body = tab.requests[0].options.body;
  assert.ok(Buffer.byteLength(body) <= 16384);
  assert.ok(body.startsWith('{"build":18446744073709551615,'));
  const report = JSON.parse(body);
  assert.ok(Buffer.byteLength(report.message) <= 4096);
  assert.ok(Buffer.byteLength(report.source.file) <= 1024);
  assert.equal(report.code, 0);
  assert.match(report.invocation, /^[A-Za-z0-9_-]{1,64}$/);
}
const insecure = page(clientScript, 'accept', false);
insecure.context.__moth_record_entry_failure(0, 'fallback invocation', null);
assert.match(JSON.parse(insecure.requests[0].options.body).invocation, /^[A-Za-z0-9_-]{1,64}$/);
"#,
    );
}

#[test]
fn lone_surrogates_are_normalized_into_server_decodable_report_text() {
    let body = run_client_test(
        &dev_client_snippet("/", Some((1, "index.html"))),
        r#"
const tab = page(clientScript, 'accept');
const message = '\ud800+\udfff+😀';
const error = new Error(message);
error.__moth_location = {file: message, line: 1, column: 2};
tab.context.__moth_record_startup_fault('startup_fault', error);
assert.equal(textFor(tab, 'pre', 'msg'), '�+�+😀\nat �+�+😀:1:2');
process.stdout.write(tab.requests[0].options.body);
"#,
    );
    let report: serde_json::Value =
        serde_json::from_slice(&body).expect("host text must produce a report Rust can decode");
    assert_eq!(report["message"], "�+�+😀");
    assert_eq!(report["source"]["file"], "�+�+😀");
}

#[test]
fn script_metadata_cannot_end_the_injected_script() {
    let snippet = dev_client_snippet(
        "/preview<!--<ScRiPt></script>",
        Some((3, "hostile<!--<script></script>.html")),
    );
    assert_eq!(snippet.matches("</script>").count(), 1);
    assert_eq!(snippet.matches("<script").count(), 1);
    assert!(!snippet.contains("<!--<"));
    run_client_test(
        &snippet,
        r#"
const tab = page(clientScript, 'accept');
tab.context.__moth_record_entry_failure(4, 'message', null);
assert.equal(tab.sources[0].path, '/preview<!--<ScRiPt></script>/__moth/events');
assert.equal(tab.requests[0].path, '/preview<!--<ScRiPt></script>/__moth/runtime-report');
assert.equal(JSON.parse(tab.requests[0].options.body).entry, 'hostile<!--<script></script>.html');
"#,
    );
}

#[test]
fn server_error_pages_install_only_hot_reload() {
    run_client_test(
        &dev_client_snippet("/", None),
        r#"
const tab = page(clientScript);
assert.equal(tab.context.__moth_record_entry_failure, undefined);
assert.equal(tab.context.__moth_record_startup_fault, undefined);
assert.equal(tab.sources.length, 1);
assert.equal(tab.requests.length, 0);
"#,
    );
}
