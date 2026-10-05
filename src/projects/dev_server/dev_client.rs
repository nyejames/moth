//! Browser-side development integration: hot reload and page-local startup reports.
//!
//! Generated applications only call optional outcome hooks. This injected client alone owns
//! invocation identity, the isolated runtime view and best-effort dev-server delivery.

use crate::projects::dev_server::error_page::ERROR_PAGE_STYLES;
use crate::projects::routing::prefix_origin;

pub const DEV_CLIENT_MARKER: &str = "<!-- moth-dev-client -->";

/// Inject hot reload for a page of one published `build`; `report_entry` adds startup reports.
pub fn dev_client_snippet(origin: &str, build: u64, report_entry: Option<&str>) -> String {
    let sse_path = script_string(&prefix_origin(origin, "/__moth/events"));
    // Keep the complete u64 as text rather than rounding it through JS Number.
    let build = script_string(&build.to_string());
    // The server announces its published generation on every (re)connection and after each
    // publication. Only a different generation starts a fresh page invocation, so reconnecting
    // to the same build never reloads and a failed invocation keeps its local report.
    let mut script = format!(
        "\n{DEV_CLIENT_MARKER}\n<script>\n(() => {{\n  const build = {build};\n  new EventSource({sse_path}).addEventListener('generation', event => {{\n    if (event.data !== build) window.location.reload();\n  }});\n"
    );

    if let Some(entry) = report_entry {
        let metadata = serde_json::json!({
            "entry": entry,
            "endpoint": prefix_origin(origin, "/__moth/runtime-report"),
            "styles": ERROR_PAGE_STYLES,
        })
        .to_string()
        .replace('<', "\\u003c");
        script.push_str(&format!("  const metadata = {metadata};\n"));
        script.push_str(RUNTIME_REPORT_CLIENT);
    }

    script.push_str("})();\n</script>\n");
    script
}

fn script_string(text: &str) -> String {
    serde_json::Value::String(text.to_owned())
        .to_string()
        .replace('<', "\\u003c")
}

const RUNTIME_REPORT_CLIENT: &str = r#"
  let invocation;
  try {
    invocation = globalThis.crypto.randomUUID();
  } catch (_) {
    invocation = Date.now().toString(36) + '_' + Math.random().toString(36).slice(2);
  }
  let reported = false;
  const titles = {
    entry_error: 'Entry Error',
    assertion: 'Assertion Failure',
    startup_fault: 'Unexpected Startup Fault'
  };
  const read = (value, name) => {
    try { return value == null ? undefined : value[name]; } catch (_) { return undefined; }
  };
  const text = (value, limit) => {
    let string;
    try { string = value == null ? '' : String(value); } catch (_) { string = 'Unavailable fault text'; }
    let result = '', bytes = 0;
    for (const character of string) {
      const code = character.codePointAt(0);
      const size = code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
      if (bytes + size > limit) break;
      // Host-thrown JS text can contain lone UTF-16 surrogates, which Rust JSON rejects.
      result += code >= 0xd800 && code <= 0xdfff ? '\ufffd' : character;
      bytes += size;
    }
    return result;
  };
  const u32 = value => Number.isInteger(value) && value >= 0 && value <= 4294967295;
  const node = (tag, className, content) => {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  };
  const show = report => {
    const host = node('div');
    // Inline !important properties prevent authored page CSS from hiding the dev view.
    for (const [name, value] of Object.entries({
      all: 'initial', display: 'block', position: 'fixed', inset: '0',
      'z-index': '2147483647', overflow: 'auto', isolation: 'isolate'
    })) host.style.setProperty(name, value, 'important');
    const shadow = host.attachShadow({mode: 'open'});
    shadow.appendChild(node('style', '', metadata.styles));
    const view = node('div', 'runtime-view');
    const main = node('main');
    const card = node('section', 'card');
    const header = node('header');
    header.appendChild(node('h1', '', titles[report.category]));
    card.appendChild(header);
    card.appendChild(node('div', 'meta', 'Build #' + build + ' succeeded | Runtime failure | Entry: ' + report.entry));
    let details = report.code === undefined ? report.message : 'code ' + report.code + ': ' + report.message;
    if (report.source) details += '\nat ' + report.source.file + ':' + report.source.line + ':' + report.source.column;
    card.appendChild(node('pre', 'msg', details));
    main.appendChild(card);
    view.appendChild(main);
    shadow.appendChild(view);
    document.documentElement.appendChild(host);
  };
  const report = (category, code, message, location) => {
    const outcome = {entry: text(metadata.entry, 1024), invocation, category, message: text(message, 4096)};
    if (category === 'entry_error') outcome.code = code;
    const file = read(location, 'file'), line = read(location, 'line'), column = read(location, 'column');
    if (typeof file === 'string' && u32(line) && u32(column)) {
      outcome.source = {file: text(file, 1024), line, column};
    }
    try { show(outcome); } catch (_) {}
    try { console.error('Dev runtime: ' + titles[category] + ' (successful build #' + build + ')', outcome); } catch (_) {}
    try {
      const encode = () => '{"build":' + build + ',' + JSON.stringify(outcome).slice(1);
      let body = encode();
      // JSON escapes can expand bounded UTF-8 text. Bound the actual transport body too.
      while (new TextEncoder().encode(body).length > 16384) {
        if (outcome.message.length) outcome.message = text(outcome.message, Math.floor(new TextEncoder().encode(outcome.message).length / 2));
        else if (outcome.source && outcome.source.file.length) outcome.source.file = text(outcome.source.file, Math.floor(new TextEncoder().encode(outcome.source.file).length / 2));
        else break;
        body = encode();
      }
      Promise.resolve(fetch(metadata.endpoint, {
        method: 'POST', headers: {'Content-Type': 'application/json'}, body, keepalive: true
      })).catch(() => {});
    } catch (_) {}
  };
  globalThis.__moth_record_entry_failure = (code, message, location) => {
    try {
      if (reported) return;
      reported = true;
      report('entry_error', code, message, location);
    } catch (_) {}
  };
  globalThis.__moth_record_startup_fault = (category, error) => {
    try {
      if (reported) return;
      reported = true;
      const message = read(error, 'message');
      report(category === 'assertion' ? 'assertion' : 'startup_fault', undefined,
        message === undefined ? error : message, read(error, '__moth_location'));
    } catch (_) {}
  };
"#;

#[cfg(test)]
#[path = "tests/dev_client_tests.rs"]
pub(super) mod tests;
