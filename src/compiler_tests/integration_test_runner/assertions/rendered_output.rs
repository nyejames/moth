//! Node-backed rendered-output assertions for HTML integration artifacts.
//!
//! WHAT: extracts emitted scripts, executes them in the minimal Node harness and checks captured
//!       console and fragment output.
//! WHY: runtime semantics belong to one harness so rendered assertions do not inspect generated
//!      JavaScript structure or create a second execution path.

use super::super::{ArtifactKind, FailureKind};
use crate::build_system::build::BuildResult;
use crate::compiler_tests::integration_test_runner::types::RenderedOutputExpectation;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static RENDER_HARNESS_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn validate_rendered_output(
    build_result: &BuildResult,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    let Some(index_html_file) = super::artifacts::find_output_file(build_result, "index.html")
    else {
        return Some((
            "rendered_output assertion requires 'index.html', but it was not produced.".to_string(),
            FailureKind::HarnessFailed,
        ));
    };

    let Some(html) = super::artifacts::output_text_content(index_html_file, ArtifactKind::Html)
    else {
        return Some((
            "rendered_output assertion requires 'index.html' to be an HTML artifact.".to_string(),
            FailureKind::HarnessFailed,
        ));
    };

    let rendered = match execute_html_in_node(html) {
        Ok(output) => output,
        Err(reason) => return Some((reason, FailureKind::HarnessFailed)),
    };

    validate_rendered_output_fragments(&rendered.combined_output(), expectation)
}

/// Validates rendered fragments independently of harness execution.
///
/// WHAT: checks required and forbidden fragments against precomputed rendered output.
/// WHY: keeps harness failures separate from semantic mismatch failures and supports focused
///      self-tests without requiring a Node runtime.
pub(super) fn validate_rendered_output_fragments(
    rendered_output: &str,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    if let Some(expected) = &expectation.exact {
        let normalized_expected = normalize_line_endings(expected);
        let normalized_actual = normalize_line_endings(rendered_output);
        if normalized_expected != normalized_actual {
            return Some((
                format!(
                    "Rendered output did not exactly match.\nExpected output:\n{expected}\nActual output:\n{rendered_output}"
                ),
                FailureKind::RenderedOutputExactMismatch,
            ));
        }
    }

    if !expectation.contains_in_order.is_empty()
        && !super::contains_ordered_substrings(rendered_output, &expectation.contains_in_order)
    {
        return Some((
            format!(
                "Rendered output did not contain required ordered fragments {:?}.\nActual output:\n{rendered_output}",
                expectation.contains_in_order
            ),
            FailureKind::RenderedOutputOrderMismatch,
        ));
    }

    for fragment in &expectation.contains_exactly_once {
        let actual_count = rendered_output.match_indices(fragment).count();
        if actual_count != 1 {
            return Some((
                format!(
                    "Rendered output contained fragment '{fragment}' {actual_count} time(s), expected exactly once.\nActual output:\n{rendered_output}"
                ),
                FailureKind::RenderedOutputMultiplicityMismatch,
            ));
        }
    }

    for required in &expectation.contains {
        if !rendered_output.contains(required.as_str()) {
            return Some((
                format!(
                    "Rendered output did not contain required fragment '{required}'.\nActual output:\n{rendered_output}"
                ),
                FailureKind::RenderedOutputMismatch,
            ));
        }
    }

    for forbidden in &expectation.not_contains {
        if rendered_output.contains(forbidden.as_str()) {
            return Some((
                format!(
                    "Rendered output contained forbidden fragment '{forbidden}'.\nActual output:\n{rendered_output}"
                ),
                FailureKind::RenderedOutputMismatch,
            ));
        }
    }

    None
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[derive(Debug)]
pub(crate) struct RenderedOutput {
    events: Vec<RuntimeEvent>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeEvent {
    Console { text: String },
    FragmentInsert { id: String, html: String },
}

#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct SlotOutput {
    pub(crate) id: String,
    pub(crate) html: String,
}

impl RenderedOutput {
    #[cfg(test)]
    pub(crate) fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }

    #[cfg(test)]
    pub(crate) fn console_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for event in &self.events {
            if let RuntimeEvent::Console { text } = event {
                lines.push(text.to_owned());
            }
        }
        lines
    }

    #[cfg(test)]
    pub(crate) fn slot_outputs(&self) -> Vec<SlotOutput> {
        let mut outputs = Vec::new();
        for event in &self.events {
            if let RuntimeEvent::FragmentInsert { id, html } = event {
                outputs.push(SlotOutput {
                    id: id.to_owned(),
                    html: html.to_owned(),
                });
            }
        }
        outputs
    }

    pub(crate) fn combined_output(&self) -> String {
        let mut parts = Vec::with_capacity(self.events.len());
        for event in &self.events {
            match event {
                RuntimeEvent::Console { text } => parts.push(text.to_owned()),
                RuntimeEvent::FragmentInsert { html, .. } => parts.push(html.to_owned()),
            }
        }

        parts.join("\n")
    }
}

/// Executes the script blocks from compiled HTML through a minimal Node.js harness.
///
/// The harness stubs `document.getElementById` to capture `insertAdjacentHTML` calls, intercepts
/// `console.log` and emits a JSON summary after one microtask tick so runtime assertions can
/// observe batched reactive flushes queued by the page bundle.
fn execute_html_in_node(html: &str) -> Result<RenderedOutput, String> {
    let scripts = extract_script_blocks(html);
    if scripts.is_empty() {
        return Err(
            "rendered_output: no <script> blocks found in 'index.html'. \
             Ensure the fixture produces runtime output."
                .to_string(),
        );
    }

    let harness = build_node_harness(&scripts);

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = RENDER_HARNESS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = std::env::temp_dir().join(format!(
        "moth_render_harness_{}_{}_{}.js",
        std::process::id(),
        unique,
        sequence
    ));

    std::fs::write(&temp_path, &harness)
        .map_err(|error| format!("rendered_output: failed to write node harness: {error}"))?;

    let output = std::process::Command::new("node")
        .arg(&temp_path)
        .output()
        .map_err(|error| {
            let _ = remove_temp_harness_file_with_retry(&temp_path);
            format!(
                "rendered_output: failed to invoke node: {error}. \
                 Ensure 'node' is on PATH to use rendered-output assertions."
            )
        })?;

    let _ = remove_temp_harness_file_with_retry(&temp_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "rendered_output: node harness execution failed:\n{stderr}"
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_harness_output(stdout.trim())
}

/// Best-effort cleanup for temporary Node harness files.
///
/// WHAT: retries removal briefly to tolerate Windows file-sharing race windows after process exit.
/// WHY: cleanup races must not surface as semantic rendered-output mismatches.
fn remove_temp_harness_file_with_retry(path: &Path) -> Result<(), std::io::Error> {
    const MAX_ATTEMPTS: usize = 6;
    const BASE_RETRY_DELAY_MS: u64 = 8;

    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < MAX_ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(
                        BASE_RETRY_DELAY_MS * (attempt as u64 + 1),
                    ));
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| std::io::Error::other("failed to remove file")))
}

fn build_node_harness(scripts: &[String]) -> String {
    let prefix = r#"const __moth_events = [];
const __moth_slot_by_id = new Map();
console.log = (...args) => __moth_events.push({ type: 'console', text: args.map(String).join(' ') });
function __moth_get_slot(id) {
    if (!__moth_slot_by_id.has(id)) {
        const slot = {
            id,
            innerHTML: "",
            insertAdjacentHTML: (_, html) => {
                const text = String(html);
                slot.innerHTML += text;
                __moth_events.push({ type: 'fragment_insert', id: String(id), html: text });
            }
        };
        __moth_slot_by_id.set(id, slot);
    }
    return __moth_slot_by_id.get(id);
}
const document = {
    getElementById: __moth_get_slot
};
"#;

    let suffix = r#"
Promise.resolve().then(() => {
    process.stdout.write(JSON.stringify({ events: __moth_events }) + '\n');
});
"#;

    format!("{prefix}{}\n{suffix}", scripts.join("\n"))
}

/// Extracts the text content between `<script>` and `</script>` tag pairs.
pub(crate) fn extract_script_blocks(html: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(open_end) = find_script_open_end(html, search_from) {
        let close_tag = "</script>";
        let Some(close_start) = html[open_end..].find(close_tag) else {
            break;
        };
        let block = &html[open_end..open_end + close_start];
        if !block.trim().is_empty() {
            blocks.push(block.to_owned());
        }
        search_from = open_end + close_start + close_tag.len();
    }

    blocks
}

/// Finds the end position of a `<script>` opening tag starting from `from`.
fn find_script_open_end(html: &str, from: usize) -> Option<usize> {
    let slice = &html[from..];
    let tag_start = slice.find("<script")?;
    let tag_slice = &slice[tag_start..];
    let close_bracket = tag_slice.find('>')?;
    Some(from + tag_start + close_bracket + 1)
}

pub(crate) fn parse_harness_output(json: &str) -> Result<RenderedOutput, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        format!("rendered_output: failed to parse node harness JSON output: {error}\nRaw: {json}")
    })?;

    let invalid_harness_output = |reason: String| {
        format!("rendered_output: invalid node harness output: {reason}\nRaw: {json}")
    };

    let Some(object) = value.as_object() else {
        return Err(invalid_harness_output(
            "top-level value must be an object".to_owned(),
        ));
    };

    if let Err(reason) = reject_unknown_fields(object, &["events"], "harness output") {
        return Err(invalid_harness_output(reason));
    }

    let Some(events_value) = object.get("events") else {
        return Err(invalid_harness_output("missing field 'events'".to_owned()));
    };
    let Some(events_array) = events_value.as_array() else {
        return Err(invalid_harness_output(
            "field 'events' must be an array".to_owned(),
        ));
    };

    let mut events = Vec::with_capacity(events_array.len());
    for (index, event_value) in events_array.iter().enumerate() {
        let event = decode_runtime_event(index, event_value).map_err(invalid_harness_output)?;
        events.push(event);
    }

    Ok(RenderedOutput { events })
}

fn decode_runtime_event(index: usize, value: &serde_json::Value) -> Result<RuntimeEvent, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("event {index} must be an object"));
    };

    let event_type = required_string_field(object, "type", &format!("event {index}"))?;
    match event_type.as_str() {
        "console" => {
            reject_unknown_fields(object, &["type", "text"], &format!("event {index}"))?;
            let text = required_string_field(object, "text", &format!("event {index}"))?;
            Ok(RuntimeEvent::Console { text })
        }

        "fragment_insert" => {
            reject_unknown_fields(object, &["type", "id", "html"], &format!("event {index}"))?;
            let id = required_string_field(object, "id", &format!("event {index}"))?;
            let html = required_string_field(object, "html", &format!("event {index}"))?;
            Ok(RuntimeEvent::FragmentInsert { id, html })
        }

        other => Err(format!("event {index} has unknown type '{other}'")),
    }
}

fn required_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    context: &str,
) -> Result<String, String> {
    let Some(value) = object.get(field) else {
        return Err(format!("{context} is missing string field '{field}'"));
    };

    let Some(value) = value.as_str() else {
        return Err(format!("{context} field '{field}' must be a string"));
    };

    Ok(value.to_owned())
}

fn reject_unknown_fields(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed_fields: &[&str],
    context: &str,
) -> Result<(), String> {
    for field in object.keys() {
        if !allowed_fields
            .iter()
            .any(|allowed_field| *allowed_field == field)
        {
            return Err(format!("{context} has unknown field '{field}'"));
        }
    }

    Ok(())
}
