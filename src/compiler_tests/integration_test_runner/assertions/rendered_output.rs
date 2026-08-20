//! Node-backed rendered-output assertions for HTML integration artifacts.
//!
//! WHAT: extracts emitted scripts, executes them in the minimal Node harness and checks captured
//!       console and fragment output.
//! WHY: runtime semantics belong to one harness so rendered assertions do not inspect generated
//!      JavaScript structure or create a second execution path.
//!
//! Workspace ownership, process bounds and output decoding belong to `node_harness`; supported
//! script shapes belong to `html_scripts`. This module owns the harness JavaScript, the event
//! protocol and the expectation checks.

use super::super::{ArtifactKind, FailureKind};
use super::artifacts::BuiltArtifactIndex;
use super::html_scripts::extract_executable_scripts;
use super::node_harness::{RenderHarnessError, run_node_script, with_harness_workspace};
use crate::build_system::build::OutputFile;
use crate::compiler_tests::integration_test_runner::types::RenderedOutputExpectation;
use std::path::Path;

pub(super) fn validate_rendered_output(
    index: &BuiltArtifactIndex<'_>,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    let rendered = match execute_html_in_node(index) {
        Ok(output) => output,
        Err(error) => return Some((error.message, FailureKind::HarnessFailed)),
    };

    validate_rendered_output_fragments(&rendered.combined_output(), expectation)
}

/// Executes the generated HTML-Wasm bootstrap and validates its hydrated slot output.
///
/// WHAT: runs the emitted `page.js` against its sibling `page.wasm` in Node with a small DOM and
///       fetch adapter, then applies the same rendered-output assertions as HTML mode.
/// WHY: HTML-Wasm backend tests must observe runtime semantics such as content-based String
///      equality; Wasm validity and lowering-shape assertions alone cannot prove that behavior.
pub(super) fn validate_wasm_rendered_output(
    index: &BuiltArtifactIndex<'_>,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    let rendered = match execute_wasm_page_in_node(index) {
        Ok(output) => output,
        Err(error) => return Some((error.message, FailureKind::HarnessFailed)),
    };

    validate_rendered_output_fragments(&rendered.combined_output(), expectation)
}

fn execute_wasm_page_in_node(
    index: &BuiltArtifactIndex<'_>,
) -> Result<RenderedOutput, RenderHarnessError> {
    let page_js = required_text_artifact(index, "page.js", ArtifactKind::Js)?;
    let page_wasm = required_wasm_artifact(index, "page.wasm")?;

    with_harness_workspace(|workspace| {
        workspace.write("page.js", page_js)?;
        workspace.write("page.wasm", page_wasm)?;
        run_wasm_harness_in(workspace.path())
    })
}

/// Writes and runs the HTML-Wasm harness inside an already-populated directory.
///
/// The harness resolves `page.js` and `page.wasm` through `__dirname`, so no path ever crosses a
/// text boundary and a non-UTF-8 workspace path cannot be lossily rewritten.
fn run_wasm_harness_in(directory: &Path) -> Result<RenderedOutput, RenderHarnessError> {
    let harness_path = directory.join("harness.js");
    std::fs::write(&harness_path, NODE_WASM_HARNESS).map_err(|error| {
        RenderHarnessError::workspace(format!(
            "rendered_output: failed to write the HTML-Wasm Node harness '{}': {error}",
            harness_path.display()
        ))
    })?;

    let run = run_node_script(&harness_path, directory)?;
    parse_harness_output(run.stdout.trim())
}

/// Runs the HTML-Wasm harness against a caller-supplied directory of page artifacts.
///
/// Self-tests use this to drive the harness with a hand-written `page.js` instead of a full build.
#[cfg(test)]
pub(crate) fn execute_wasm_harness_for_test(
    directory: &Path,
) -> Result<RenderedOutput, RenderHarnessError> {
    run_wasm_harness_in(directory)
}

/// Test-only view of the artifact-requirement boundary.
///
/// The harness reaches this boundary only through a full build, where the universal baselines
/// reject a missing or mis-kinded `index.html` first, so the boundary itself is exercised here
/// directly. Index construction is test setup: an ambiguous set has its own owner and cannot be
/// what this seam reports.
#[cfg(test)]
pub(crate) fn required_text_artifact_for_test(
    build_result: &crate::build_system::build::BuildResult,
    relative_path: &str,
    kind: ArtifactKind,
) -> Result<(), RenderHarnessError> {
    let index = BuiltArtifactIndex::build(build_result)
        .expect("the artifact-boundary seam needs an unambiguous artifact set");

    required_text_artifact(&index, relative_path, kind).map(|_| ())
}

fn required_artifact<'index>(
    index: &BuiltArtifactIndex<'index>,
    relative_path: &str,
) -> Result<&'index OutputFile, RenderHarnessError> {
    index.get(relative_path).ok_or_else(|| {
        RenderHarnessError::artifact(format!(
            "rendered_output assertion requires '{relative_path}', but it was not produced."
        ))
    })
}

fn required_text_artifact<'index>(
    index: &BuiltArtifactIndex<'index>,
    relative_path: &str,
    kind: ArtifactKind,
) -> Result<&'index str, RenderHarnessError> {
    let output = required_artifact(index, relative_path)?;

    super::artifacts::output_text_content(output, kind).ok_or_else(|| {
        RenderHarnessError::artifact(format!(
            "rendered_output assertion requires '{relative_path}' to be a {} artifact.",
            super::artifacts::artifact_kind_name(kind)
        ))
    })
}

fn required_wasm_artifact<'index>(
    index: &BuiltArtifactIndex<'index>,
    relative_path: &str,
) -> Result<&'index [u8], RenderHarnessError> {
    let output = required_artifact(index, relative_path)?;

    super::artifacts::output_wasm_bytes(output).ok_or_else(|| {
        RenderHarnessError::artifact(format!(
            "rendered_output assertion requires '{relative_path}' to be a wasm artifact."
        ))
    })
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
            // Both sides are reported escaped and post-normalization, and the first differing
            // byte is named. Printing the raw text makes a whitespace-only mismatch — an extra
            // captured newline, a trailing space — look like two identical lines, which is a
            // failure report that cannot be acted on. Reporting the authored text instead of the
            // normalized text would also describe a difference the comparison never made.
            let difference_offset =
                first_difference_offset(&normalized_expected, &normalized_actual);
            return Some((
                format!(
                    "Rendered output did not exactly match; first difference at byte \
                     {difference_offset}.\nExpected output:\n{normalized_expected:?}\nActual \
                     output:\n{normalized_actual:?}"
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

/// Byte offset of the first difference between two already-normalized outputs.
///
/// When one side is a prefix of the other the offset is the shorter length, which is where the
/// extra bytes begin. Callers use this only after establishing the two differ.
fn first_difference_offset(expected: &str, actual: &str) -> usize {
    expected
        .as_bytes()
        .iter()
        .zip(actual.as_bytes())
        .position(|(expected_byte, actual_byte)| expected_byte != actual_byte)
        .unwrap_or_else(|| expected.len().min(actual.len()))
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
fn execute_html_in_node(
    index: &BuiltArtifactIndex<'_>,
) -> Result<RenderedOutput, RenderHarnessError> {
    let html = required_text_artifact(index, "index.html", ArtifactKind::Html)?;

    let scripts = extract_executable_scripts(html)?;
    if scripts.is_empty() {
        return Err(RenderHarnessError::script_shape(
            "rendered_output: no executable <script> blocks found in 'index.html'. \
             Ensure the fixture produces runtime output."
                .to_owned(),
        ));
    }

    let harness = build_node_harness(&scripts);

    with_harness_workspace(|workspace| {
        let harness_path = workspace.write("harness.js", &harness)?;
        let run = run_node_script(&harness_path, workspace.path())?;
        parse_harness_output(run.stdout.trim())
    })
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

/// HTML-Wasm harness source.
///
/// It resolves its artifacts through `__dirname` rather than an interpolated path, so the
/// workspace location never has to survive a UTF-8 text boundary.
const NODE_WASM_HARNESS: &str = r#"const fs = require("fs");
const path = require("path");
const __moth_wasm_dir = __dirname;
const __moth_events = [];
const __moth_slot_by_id = new Map();

console.log = (...args) => __moth_events.push({ type: 'console', text: args.map(String).join(' ') });
function __moth_get_slot(id) {
    if (!__moth_slot_by_id.has(id)) {
        const slot = {
            id,
            innerHTML: "",
            textContent: "",
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

globalThis.document = {
    getElementById: __moth_get_slot,
    createTextNode: (text) => ({ textContent: String(text) })
};
globalThis.fetch = async (url) => {
    const relative_path = String(url).replace(/^\.\//, "");
    const bytes = fs.readFileSync(path.join(__moth_wasm_dir, relative_path));
    return {
        arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
    };
};

(async () => {
    try {
        const page_js = fs.readFileSync(path.join(__moth_wasm_dir, "page.js"), "utf8");
        const page_completion = (0, eval)(page_js);
        await page_completion;
        await Promise.resolve();
        process.stdout.write(JSON.stringify({ events: __moth_events }) + '\n');
    } catch (error) {
        console.error(error);
        process.exitCode = 1;
    }
})();
"#;

pub(crate) fn parse_harness_output(json: &str) -> Result<RenderedOutput, RenderHarnessError> {
    let invalid_harness_output = |reason: String| {
        RenderHarnessError::output_protocol(format!(
            "rendered_output: invalid node harness output: {reason}\nRaw: {json}"
        ))
    };

    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        RenderHarnessError::output_protocol(format!(
            "rendered_output: failed to parse node harness JSON output: {error}\nRaw: {json}"
        ))
    })?;

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
