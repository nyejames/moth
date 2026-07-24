//! Error page rendering for dev-server build/runtime failures.
//!
//! Compiler diagnostics are converted into escaped HTML so failed builds still render safely and
//! remain connected to hot reload via the injected SSE client snippet.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::dev_server;
use crate::projects::dev_server::dev_client::dev_client_snippet;
use std::fmt::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn current_timestamp_unix_seconds() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    }
}

pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

pub fn format_compiler_messages(messages: &CompilerMessages) -> String {
    let mut output = String::new();

    if messages.has_errors() || messages.has_warnings() {
        write_structured_diagnostic_summary(messages, &mut output);
        return output;
    }

    output.push_str("No compiler diagnostics available.\n");
    output
}

fn write_structured_diagnostic_summary(messages: &CompilerMessages, output: &mut String) {
    for line in
        crate::compiler_frontend::compiler_messages::render::terse::format_terse_compiler_messages(
            messages,
        )
    {
        let _ = writeln!(output, "{line}");
    }
}

pub fn render_compiler_error_page(
    messages: &CompilerMessages,
    project_root: &Path,
    origin: &str,
    build_version: u64,
) -> String {
    let diagnostics_html = render_compiler_diagnostics(messages, project_root);
    render_error_page_shell("Build Failed", &diagnostics_html, origin, build_version)
}

pub fn render_runtime_error_page(
    title: &str,
    details: &str,
    origin: &str,
    build_version: u64,
) -> String {
    let escaped_details = escape_html(details);
    let details_html = format!("<pre class=\"msg\">{escaped_details}</pre>");
    render_error_page_shell(title, &details_html, origin, build_version)
}

fn render_error_page_shell(
    title: &str,
    body_html: &str,
    origin: &str,
    build_version: u64,
) -> String {
    let escaped_title = escape_html(title);
    let timestamp = current_timestamp_unix_seconds();
    let dev_client = dev_client_snippet(origin);

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Moth Dev Server Error</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #0a120b;
      --bg-glow: rgba(80, 180, 100, 0.20);
      --card: #061004e0;
      --panel: #0d160e;
      --fg: #f2f5f8;
      --accent: #fd494b;
      --muted: #9bb09b;
      --border: #243a25;
      --link: #7fd97f;
      --warning: #ff8f57;
      --shadow: rgba(0, 0, 0, 0.35);
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      background:
        radial-gradient(circle at top, var(--bg-glow), transparent 36%),
        var(--bg);
      color: var(--fg);
      font-family: Menlo, Monaco, Consolas, "Liberation Mono", monospace;
    }}
    main {{
      max-width: 1040px;
      margin: 2.5rem auto;
      padding: 0 1rem;
    }}
    .card {{
      background: var(--card);
      border: 1px solid var(--border);
      border-radius: 14px;
      box-shadow: 0 20px 60px var(--shadow);
      overflow: hidden;
    }}
    header {{
      padding: 1rem 1.2rem 0.95rem;
      border-bottom: 1px solid var(--border);
      background: rgba(255, 255, 255, 0.02);
    }}
    h1 {{
      margin: 0;
      font-size: 1.1rem;
      color: var(--accent);
    }}
    .meta {{
      padding: 0.8rem 1.2rem;
      color: var(--muted);
      font-size: 0.9rem;
      border-bottom: 1px solid var(--border);
      background: rgba(255, 255, 255, 0.02);
    }}
    .msg {{
      margin: 0;
      padding: 1.3rem;
      line-height: 1.45;
      font-size: 0.92rem;
      white-space: pre-wrap;
    }}
    .diagnostics {{
      padding: 1.1rem;
      display: grid;
      gap: 0.9rem;
    }}
    .diagnostic {{
      border: 1px solid var(--border);
      border-radius: 10px;
      background: var(--panel);
      padding: 1rem 1rem 0.95rem;
    }}
    .diagnostic-head {{
      display: flex;
      flex-wrap: wrap;
      gap: 0.55rem;
      align-items: center;
      margin-bottom: 0.7rem;
    }}
    .badge {{
      display: inline-flex;
      align-items: center;
      border-radius: 999px;
      padding: 0.22rem 0.55rem;
      font-size: 0.74rem;
      letter-spacing: 0.04em;
      background: rgba(255, 122, 144, 0.16);
      color: var(--accent);
    }}
    .badge.warning {{
      background: rgba(255, 212, 121, 0.16);
      color: var(--warning);
    }}
    .kind {{
      color: var(--muted);
      font-size: 0.86rem;
    }}
    .diagnostic-message {{
      margin: 0 0 0.8rem;
      line-height: 1.5;
    }}
    .detail-list {{
      margin: 0;
      padding: 0;
      list-style: none;
      display: grid;
      gap: 0.45rem;
    }}
    .detail-list li {{
      color: var(--muted);
      line-height: 1.45;
    }}
    .detail-label {{
      color: var(--fg);
      margin-right: 0.45rem;
    }}
    a {{
      color: var(--link);
      text-decoration: none;
    }}
    a:hover {{
      text-decoration: underline;
    }}
    .empty-state {{
      padding: 1.3rem;
      color: var(--muted);
    }}
    .source-frame {{
      margin: 0.8rem 0;
      padding: 0.85rem;
      border: 1px solid var(--border);
      border-radius: 8px;
      background: rgba(0, 0, 0, 0.18);
      white-space: pre;
      overflow-x: auto;
    }}
    .source-location {{
      color: var(--link);
    }}
    .source-line-number {{
      color: var(--muted);
    }}
    .source-caret {{
      color: var(--accent);
    }}
    .guidance {{
      margin-top: 0.7rem;
      color: var(--muted);
    }}
    .open-source {{
      display: inline-block;
      margin-top: 0.8rem;
    }}
  </style>
</head>
<body>
  <main>
    <section class="card">
      <header><h1>{escaped_title}</h1></header>
      <div class="meta">Build Version: {build_version} | Timestamp (unix): {timestamp}</div>
      {body_html}
    </section>
  </main>
  {dev_client}
</body>
</html>"#
    )
}

fn render_compiler_diagnostics(messages: &CompilerMessages, project_root: &Path) -> String {
    let mut diagnostics_html = String::from("<section class=\"diagnostics\">");

    if messages.diagnostic_slice().is_empty() {
        diagnostics_html.push_str(
            "<div class=\"empty-state\">No compiler diagnostics were available for this failed build.</div>",
        );
        diagnostics_html.push_str("</section>");
        return diagnostics_html;
    }

    diagnostics_html.push_str(&dev_server::render_compiler_messages_html(
        messages,
        project_root,
    ));

    diagnostics_html.push_str("</section>");
    diagnostics_html
}

#[cfg(test)]
#[path = "tests/error_page_tests.rs"]
mod tests;
