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
use crate::backends::js::ENTRY_FAILURE_NOTICE;
use crate::build_system::build::OutputFile;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_tests::integration_test_runner::types::RenderedOutputExpectation;
use std::io::Write;
use std::path::Path;

pub(super) fn validate_rendered_output(
    index: &BuiltArtifactIndex<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    let rendered = match execute_html_in_node(
        index,
        resource_inputs,
        expectation.math_random_samples.as_deref(),
    ) {
        Ok(output) => output,
        Err(error) => return Some((error.message, FailureKind::HarnessFailed)),
    };

    validate_rendered_output_result(&rendered, expectation)
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

    validate_rendered_output_result(&rendered, expectation)
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
    std::fs::File::create(&harness_path)
        .and_then(|mut harness| {
            harness.write_all(NODE_WASM_HARNESS_PREFIX.as_bytes())?;
            harness.write_all(entry_notice_observer().as_bytes())?;
            harness.write_all(NODE_TERMINAL_PROTOCOL.as_bytes())?;
            harness.write_all(NODE_WASM_HARNESS_SUFFIX.as_bytes())
        })
        .map_err(|error| {
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
    let index = BuiltArtifactIndex::build(
        &build_result.project.output_files,
        &build_result.project.deferred_resources,
    )
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

fn validate_rendered_output_result(
    rendered: &RenderedOutput,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    if rendered.events.last() == Some(&RuntimeEvent::EntryFailure) {
        return Some((
            format!("rendered_output: {}", ENTRY_FAILURE_NOTICE.trim_end()),
            FailureKind::EntryFailed,
        ));
    }

    let actual_error = rendered.runtime_error_message();
    let actual_trap = rendered.runtime_trap_message();
    let expects_error = !expectation.runtime_error_contains.is_empty();
    let expects_trap = !expectation.runtime_trap_contains.is_empty();

    if !expects_error && !expects_trap {
        if let Some(message) = actual_error {
            return Some((
                format!("rendered_output: unexpected uncaught Moth runtime Error: {message}"),
                FailureKind::HarnessFailed,
            ));
        }
        if let Some(message) = actual_trap {
            return Some((
                format!("rendered_output: unexpected uncaught WebAssembly trap: {message}"),
                FailureKind::HarnessFailed,
            ));
        }
    }

    if let Some(failure) =
        validate_rendered_output_fragments(&rendered.combined_output(), expectation)
    {
        return Some(failure);
    }

    if expects_error {
        let Some(actual) = actual_error else {
            if let Some(trap_message) = actual_trap {
                return Some((
                    format!(
                        "rendered_output: expected an uncaught Moth runtime Error containing {:?}, but a WebAssembly trap occurred with message {trap_message:?}.",
                        expectation.runtime_error_contains
                    ),
                    FailureKind::RenderedOutputMismatch,
                ));
            }

            return Some((
                format!(
                    "rendered_output: expected an uncaught Moth runtime Error containing {:?}, but no runtime error occurred.",
                    expectation.runtime_error_contains
                ),
                FailureKind::RenderedOutputMismatch,
            ));
        };

        for fragment in &expectation.runtime_error_contains {
            if !actual.contains(fragment) {
                return Some((
                    format!(
                        "rendered_output: runtime error message did not contain required fragment '{fragment}'.\nActual runtime error:\n{actual}"
                    ),
                    FailureKind::RenderedOutputMismatch,
                ));
            }
        }

        return None;
    }

    if expects_trap {
        let Some(actual) = actual_trap else {
            if let Some(error_message) = actual_error {
                return Some((
                    format!(
                        "rendered_output: expected an uncaught WebAssembly trap containing {:?}, but an uncaught Moth runtime Error occurred with message {error_message:?}.",
                        expectation.runtime_trap_contains
                    ),
                    FailureKind::RenderedOutputMismatch,
                ));
            }

            return Some((
                format!(
                    "rendered_output: expected an uncaught WebAssembly trap containing {:?}, but no WebAssembly trap occurred.",
                    expectation.runtime_trap_contains
                ),
                FailureKind::RenderedOutputMismatch,
            ));
        };

        for fragment in &expectation.runtime_trap_contains {
            if !actual.contains(fragment) {
                return Some((
                    format!(
                        "rendered_output: WebAssembly trap message did not contain required fragment '{fragment}'.\nActual WebAssembly trap message:\n{actual}"
                    ),
                    FailureKind::RenderedOutputMismatch,
                ));
            }
        }
    }

    None
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
    WasmTrap { message: String },
    RuntimeError { message: String },
    EntryFailure,
}

#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) struct SlotOutput {
    pub(crate) id: String,
    pub(crate) html: String,
}

impl RenderedOutput {
    pub(crate) fn runtime_error_message(&self) -> Option<&str> {
        match self.events.last()? {
            RuntimeEvent::RuntimeError { message } => Some(message),
            RuntimeEvent::Console { .. }
            | RuntimeEvent::FragmentInsert { .. }
            | RuntimeEvent::WasmTrap { .. }
            | RuntimeEvent::EntryFailure => None,
        }
    }

    fn runtime_trap_message(&self) -> Option<&str> {
        match self.events.last()? {
            RuntimeEvent::WasmTrap { message } => Some(message),
            RuntimeEvent::Console { .. }
            | RuntimeEvent::FragmentInsert { .. }
            | RuntimeEvent::RuntimeError { .. }
            | RuntimeEvent::EntryFailure => None,
        }
    }

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
                RuntimeEvent::RuntimeError { .. }
                | RuntimeEvent::WasmTrap { .. }
                | RuntimeEvent::EntryFailure => {}
            }
        }

        parts.join("\n")
    }
}

/// Executes the script blocks from compiled HTML through a minimal Node.js harness.
///
/// The harness stubs `document.getElementById` to capture `insertAdjacentHTML` calls, intercepts
/// `console.log` and emits a summary after the page script's queued microtasks drain so runtime
/// assertions can observe batched reactive flushes queued by the page bundle.
fn execute_html_in_node(
    index: &BuiltArtifactIndex<'_>,
    resource_inputs: &mut ResourceInputRegistry,
    math_random_samples: Option<&[f64]>,
) -> Result<RenderedOutput, RenderHarnessError> {
    let html = required_text_artifact(index, "index.html", ArtifactKind::Html)?;
    let scripts = extract_executable_scripts(html)?;
    let module_source = scripts.module.as_deref();
    if scripts.classic.is_empty() && module_source.is_none_or(|source| source.trim().is_empty()) {
        return Err(RenderHarnessError::script_shape(
            "rendered_output: no executable <script> blocks found in 'index.html'. \
             Ensure the fixture produces runtime output."
                .to_owned(),
        ));
    }

    with_harness_workspace(|workspace| {
        let harness_path = if let Some(module_source) = module_source {
            let module_path = Path::new("index.html").with_file_name("page.mjs");
            if index.contains("page.mjs") {
                return Err(RenderHarnessError::artifact(
                    "rendered_output: the emitted module cannot be staged as 'page.mjs' because \
                     that path is already occupied by a build artifact."
                        .to_owned(),
                ));
            }

            workspace.write("package.json", "{\"type\":\"module\"}\n")?;
            for (relative_path, source) in index.javascript_artifacts() {
                workspace.write_relative(Path::new(relative_path), source.as_bytes())?;
            }
            for (relative_path, resource) in index.deferred_javascript_artifacts() {
                let source = resource_inputs
                    .read_source(resource.source_id)
                    .map_err(|error| {
                        RenderHarnessError::artifact(format!(
                            "rendered_output: failed to read deferred JavaScript artifact \
                             '{relative_path}': {}",
                            error.msg
                        ))
                    })?;
                workspace.write_relative(Path::new(relative_path), source)?;
            }
            workspace.write_relative(&module_path, module_source)?;

            let module_specifier = format!(
                "./{}",
                module_path
                    .to_str()
                    .expect("the fixed inline module path is valid UTF-8")
            );
            let harness = build_node_module_harness(&module_specifier, math_random_samples);
            workspace.write("harness.cjs", &harness)?
        } else {
            let harness = build_node_harness(&scripts.classic, math_random_samples);
            workspace.write("harness.js", &harness)?
        };

        let run = run_node_script(&harness_path, workspace.path())?;
        parse_harness_output(run.stdout.trim())
    })
}

/// Executes real HTML artifacts without applying a success expectation.
///
/// Failure tests inspect the captured prefix independently of the terminal classification.
#[cfg(test)]
pub(crate) fn execute_html_harness_for_test(
    build_result: &mut crate::build_system::build::BuildResult,
) -> Result<RenderedOutput, RenderHarnessError> {
    let index = BuiltArtifactIndex::build(
        &build_result.project.output_files,
        &build_result.project.deferred_resources,
    )
    .expect("the HTML execution seam needs an unambiguous artifact set");

    execute_html_in_node(&index, &mut build_result.project.resource_inputs, None)
}
fn entry_notice_observer() -> String {
    let notice = serde_json::to_string(ENTRY_FAILURE_NOTICE).expect("the entry notice is a string");
    format!(
        "const __moth_entry_notice_text = {notice};\n\
         let __moth_entry_notice_seen = false;\n\
         const __moth_stderr_write = process.stderr.write.bind(process.stderr);\n\
         process.stderr.write = function (chunk, encoding, callback) {{\n\
             if (chunk === __moth_entry_notice_text) __moth_entry_notice_seen = true;\n\
             return __moth_stderr_write(chunk, encoding, callback);\n\
         }};\n"
    )
}


const NODE_TERMINAL_PROTOCOL: &str = r#"// A Moth runtime Error is a value thrown as `new Error(message)`; subclasses and
// non-Wasm engine errors such as TypeError remain harness faults. Only the HTML-Wasm adapter
// recognizes WebAssembly.RuntimeError as a Wasm trap. A returned entry error is the fixed
// terminal notice plus status 1. Any other nonzero status is a host fault.
// Summary serialization is synchronous so queued host work cannot extend a terminal event prefix.
let __moth_summary_error_hook = null;
let __moth_runtime_trap_message_hook = null;
let __moth_finished = false;
function __moth_write_summary() {
    if (__moth_finished) return;
    if (__moth_summary_error_hook !== null) {
        const error = __moth_summary_error_hook();
        if (error !== null) {
            __moth_report_harness_failure(error);
            return;
        }
    }
    const terminal_event = __moth_events.at(-1);
    const returned_entry_error = __moth_entry_notice_seen && process.exitCode === 1
        && terminal_event?.type !== 'runtime_error' && terminal_event?.type !== 'wasm_trap';
    if (returned_entry_error) {
        __moth_events.push({ type: 'entry_failure' });
    } else if (typeof process.exitCode === "number" && process.exitCode !== 0
        && terminal_event?.type !== 'runtime_error' && terminal_event?.type !== 'wasm_trap') {
        __moth_report_harness_failure(new Error("host process status " + process.exitCode));
        return;
    }
    __moth_finished = true;
    process.stdout.write(JSON.stringify({ events: __moth_events }) + '\n', () => process.exit(0));
}
function __moth_report_harness_failure(error) {
    if (__moth_finished) return;
    let message;
    try {
        message = error instanceof Error ? (error.stack || String(error)) : String(error);
    } catch {
        message = '<unprintable thrown value>';
    }
    __moth_finished = true;
    process.stderr.write(message + '\n', () => process.exit(1));
}
function __moth_handle_runtime_error(error) {
    if (__moth_finished) return;
    if (error !== null && typeof error === "object" && Object.getPrototypeOf(error) === Error.prototype) {
        __moth_events.push({ type: 'runtime_error', message: String(error.message) });
        __moth_write_summary();
        return;
    }
    if (__moth_runtime_trap_message_hook !== null) {
        let message;
        try {
            message = __moth_runtime_trap_message_hook(error);
        } catch (classifier_error) {
            __moth_report_harness_failure(classifier_error);
            return;
        }
        if (message !== null) {
            __moth_events.push({ type: 'wasm_trap', message });
            __moth_write_summary();
            return;
        }
    }
    __moth_report_harness_failure(error);
}
process.on('uncaughtException', __moth_handle_runtime_error);
process.on('unhandledRejection', __moth_handle_runtime_error);
"#;

fn build_node_harness(scripts: &[String], math_random_samples: Option<&[f64]>) -> String {
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
    let random_setup = build_math_random_setup(math_random_samples);

    let suffix = r#"
setImmediate(__moth_write_summary);
"#;

    format!(
        "{prefix}{}{NODE_TERMINAL_PROTOCOL}{random_setup}{}\n{suffix}",
        entry_notice_observer(),
        scripts.join("\n")
    )
}

/// Builds the CJS entry point for one inline native-ESM page module.
///
/// The page and all emitted JS artifacts are staged in the same relative tree. Setting the
/// document stub on `globalThis` lets the imported ESM observe the same DOM and console event
/// protocol as the classic harness.
fn build_node_module_harness(
    module_specifier: &str,
    math_random_samples: Option<&[f64]>,
) -> String {
    let prefix = r#"const __moth_events = [];
const __moth_slot_by_id = new Map();
globalThis.console.log = (...args) => __moth_events.push({ type: 'console', text: args.map(String).join(' ') });
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
globalThis.document = {
    getElementById: __moth_get_slot
};

"#;
    let random_setup = build_math_random_setup(math_random_samples);
    let module_specifier = serde_json::to_string(module_specifier)
        .expect("the inline module specifier is a valid string");

    format!(
        "{prefix}{}{NODE_TERMINAL_PROTOCOL}{random_setup}import({module_specifier})\n\
         .then(() => setImmediate(__moth_write_summary), __moth_handle_runtime_error);\n",
        entry_notice_observer(),
    )
}

fn build_math_random_setup(math_random_samples: Option<&[f64]>) -> String {
    let samples_json = math_random_samples
        .map(|samples| {
            serde_json::to_string(samples)
                .expect("math_random_samples are validated as finite before harness execution")
        })
        .unwrap_or_else(|| "null".to_owned());
    let mut random_setup = String::from(
        r#"__moth_events.__moth_random_samples_exhausted = false;
__moth_events.__moth_random_samples_exhausted_message = null;
__moth_summary_error_hook = () => {
    if (!__moth_events.__moth_random_samples_exhausted) return null;
    return new TypeError(__moth_events.__moth_random_samples_exhausted_message);
};
{
    const samples = "#,
    );
    random_setup.push_str(&samples_json);
    random_setup.push_str(
        r#";
    if (samples !== null) {
        let index = 0;
        Math.random = () => {
            if (index >= samples.length) {
                __moth_events.__moth_random_samples_exhausted = true;
                __moth_events.__moth_random_samples_exhausted_message =
                    "rendered_output: Math.random sample sequence exhausted after "
                    + samples.length + " values";
                throw new TypeError(__moth_events.__moth_random_samples_exhausted_message);
            }
            return samples[index++];
        };
    }
}
"#,
    );
    random_setup
}

/// HTML-Wasm harness adapter source, composed with the shared terminal protocol.
///
/// It resolves artifacts through `__dirname` rather than an interpolated path, so the workspace
/// location never has to survive a UTF-8 text boundary.
const NODE_WASM_HARNESS_PREFIX: &str = r#"const fs = require("fs");
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
"#;

const NODE_WASM_HARNESS_SUFFIX: &str = r#"

__moth_runtime_trap_message_hook = (error) => {
    if (error !== null
        && typeof error === "object"
        && Object.getPrototypeOf(error) === WebAssembly.RuntimeError.prototype) {
        return String(error.message);
    }
    return null;
};

(async () => {
    try {
        const page_js = fs.readFileSync(path.join(__moth_wasm_dir, "page.js"), "utf8");
        const page_completion = (0, eval)(page_js);
        await page_completion;
        await new Promise((resolve) => setImmediate(resolve));
        __moth_write_summary();
    } catch (error) {
        __moth_handle_runtime_error(error);
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
    let mut terminal_event_seen = false;
    for (index, event_value) in events_array.iter().enumerate() {
        if terminal_event_seen {
            return Err(invalid_harness_output(
                "a runtime_error, wasm_trap or entry_failure event must be the final event".to_owned(),
            ));
        }
        let event = decode_runtime_event(index, event_value).map_err(invalid_harness_output)?;
        terminal_event_seen = matches!(
            &event,
            RuntimeEvent::RuntimeError { .. }
                | RuntimeEvent::WasmTrap { .. }
                | RuntimeEvent::EntryFailure
        );
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

        "runtime_error" => {
            reject_unknown_fields(object, &["type", "message"], &format!("event {index}"))?;
            let message = required_string_field(object, "message", &format!("event {index}"))?;
            Ok(RuntimeEvent::RuntimeError { message })
        }

        "wasm_trap" => {
            reject_unknown_fields(object, &["type", "message"], &format!("event {index}"))?;
            let message = required_string_field(object, "message", &format!("event {index}"))?;
            Ok(RuntimeEvent::WasmTrap { message })
        }

        "entry_failure" => {
            reject_unknown_fields(object, &["type"], &format!("event {index}"))?;
            Ok(RuntimeEvent::EntryFailure)
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
