//! Self-tests for the rendered-output harness: script shapes, process bounds and event protocol.
//!
//! WHAT: proves which script shapes the harness executes, that a non-terminating or misbehaving
//!       Node process is reported rather than tolerated, and that harness output is decoded
//!       strictly.
//! WHY: every one of these boundaries can turn a case that never ran real runtime code into a
//!      passing case. Each failure class is asserted by its typed kind so a reworded message
//!      cannot quietly widen what the harness accepts.

use super::super::assertions::{
    RenderHarnessErrorKind, RuntimeEvent, SlotOutput, execute_wasm_harness_for_test,
    extract_executable_scripts, parse_harness_output, required_text_artifact_for_test,
    run_node_script_within, run_script_with_executable_for_test,
    validate_rendered_output_fragments, validate_success_result, with_harness_workspace,
};
use super::super::types::{ArtifactKind, GoldenExpectation, RenderedOutputExpectation};
use super::super::{BackendId, FailureKind, SuccessExpectation, WarningExpectation};
use super::synthetic_build_results::{
    VALID_HTML, build_result_with_index_html, build_result_with_output_files, success_test_case,
};
use crate::build_system::build::FileKind;
use crate::compiler_tests::test_fs::assert_path_missing;
use std::path::PathBuf;
use std::time::Duration;

// ─── Fragment expectation checks ────────────────────────────────────────────

#[test]
fn rendered_output_fragment_validation_reports_semantic_mismatch_kind() {
    let expectation = RenderedOutputExpectation {
        contains: vec!["missing-fragment".to_string()],
        ..Default::default()
    };
    let result = validate_rendered_output_fragments("rendered text", &expectation)
        .expect("missing required fragment should fail");
    assert_eq!(result.1, FailureKind::RenderedOutputMismatch);
}

#[test]
fn rendered_output_order_allows_repeated_fragments_at_distinct_occurrences() {
    let expectation = RenderedOutputExpectation {
        contains_in_order: vec!["first".to_owned(), "second".to_owned(), "first".to_owned()],
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("first\nsecond\nfirst", &expectation).is_none());
}

#[test]
fn rendered_output_order_reports_distinct_failure_kind() {
    let expectation = RenderedOutputExpectation {
        contains_in_order: vec!["second".to_owned(), "first".to_owned()],
        ..Default::default()
    };

    let result = validate_rendered_output_fragments("first\nsecond", &expectation)
        .expect("out-of-order fragments should fail");
    assert_eq!(result.1, FailureKind::RenderedOutputOrderMismatch);
}

#[test]
fn rendered_output_exactly_once_accepts_one_occurrence_and_rejects_missing_or_duplicate() {
    let expectation = RenderedOutputExpectation {
        contains_exactly_once: vec!["once".to_owned()],
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("before\nonce\nafter", &expectation).is_none());

    let missing = validate_rendered_output_fragments("before\nafter", &expectation)
        .expect("missing exact-once fragment should fail");
    assert_eq!(missing.1, FailureKind::RenderedOutputMultiplicityMismatch);

    let duplicate = validate_rendered_output_fragments("once\nonce", &expectation)
        .expect("duplicate exact-once fragment should fail");
    assert_eq!(duplicate.1, FailureKind::RenderedOutputMultiplicityMismatch);
}

#[test]
fn rendered_output_exact_normalizes_only_line_endings() {
    let expectation = RenderedOutputExpectation {
        exact: Some("first\nsecond\nthird".to_owned()),
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("first\r\nsecond\rthird", &expectation).is_none());

    let whitespace_difference =
        validate_rendered_output_fragments("first\r\n second\rthird", &expectation)
            .expect("ordinary whitespace differences should fail exact output");
    assert_eq!(
        whitespace_difference.1,
        FailureKind::RenderedOutputExactMismatch
    );
}

#[test]
fn rendered_output_exact_accepts_empty_captured_text_only() {
    let expectation = RenderedOutputExpectation {
        exact: Some(String::new()),
        ..Default::default()
    };

    assert!(validate_rendered_output_fragments("", &expectation).is_none());
    let result = validate_rendered_output_fragments("\n", &expectation)
        .expect("a captured newline is not empty exact output");
    assert_eq!(result.1, FailureKind::RenderedOutputExactMismatch);
}

// ─── Supported script shapes ────────────────────────────────────────────────
//
// The harness decides what "the page's runtime code" means. Anything it silently skips is
// coverage a case can claim without ever running it, so unsupported shapes are rejected.

#[track_caller]
fn extracted_scripts(html: &str) -> Vec<String> {
    extract_executable_scripts(html).expect("supported script shapes should be extracted")
}

#[track_caller]
fn rejected_script_shape(html: &str, why: &str) -> String {
    let error = extract_executable_scripts(html).expect_err(why);
    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::ScriptShape,
        "an unsupported script shape is a script-shape failure: {error:?}"
    );
    error.message
}

#[test]
fn script_extraction_returns_supported_inline_sources_in_document_order() {
    let html = r#"
<script>first</script>
<script type="application/javascript">second</script>
<script type="text/javascript">third</script>
<script>   </script>
"#;

    assert_eq!(
        extracted_scripts(html),
        vec!["first".to_owned(), "second".to_owned(), "third".to_owned()]
    );
}

#[test]
fn script_extraction_skips_recognised_data_blocks() {
    // An import map is JSON, not a program. Executing it as JavaScript is what the previous
    // scanner did; skipping it is what a browser does.
    let html = r#"<script type="importmap">{"imports":{"a":"./a.js"}}</script>
<script>run()</script>"#;

    assert_eq!(extracted_scripts(html), vec!["run()".to_owned()]);
}

#[test]
fn script_extraction_rejects_external_script_sources() {
    let message = rejected_script_shape(
        r#"<script src="./page.js"></script>"#,
        "an external script the harness cannot execute must be rejected",
    );
    assert!(message.contains("./page.js"), "{message}");
}

#[test]
fn script_extraction_rejects_unknown_script_types() {
    let message = rejected_script_shape(
        r#"<script type="text/template">not javascript</script>"#,
        "an unknown script type must be rejected rather than executed or skipped",
    );
    assert!(message.contains("text/template"), "{message}");
}

#[test]
fn script_extraction_rejects_an_unterminated_script_block() {
    let message = rejected_script_shape(
        "<script>run()",
        "a script block with no closing tag must be rejected",
    );
    assert!(message.contains("no matching '</script>'"), "{message}");
}

#[test]
fn script_extraction_rejects_a_malformed_closing_tag() {
    // `</script` without the `>` used to end extraction quietly, dropping this block and every
    // later one from the harness input.
    let message = rejected_script_shape(
        "<script>run()</script\n<script>later()</script>",
        "a malformed closing tag must be rejected, not silently truncate the page",
    );
    assert!(
        message.contains("malformed '</script' closing tag"),
        "{message}"
    );
}

#[test]
fn script_extraction_rejects_an_unterminated_opening_tag() {
    for html in ["<script type=\"text/javascript\"", "<p>text</p><script"] {
        let message = rejected_script_shape(html, "an opening tag with no '>' must be rejected");
        assert!(message.contains("unterminated"), "{message}");
    }
}

#[test]
fn script_extraction_rejects_an_unterminated_attribute_value() {
    let message = rejected_script_shape(
        "<script type=\"module>run()</script>",
        "an unterminated attribute value must be rejected",
    );
    assert!(
        message.contains("unterminated attribute value"),
        "{message}"
    );
}

#[test]
fn script_extraction_rejects_execution_changing_attributes() {
    // A browser skips `nomodule` and releases `async` from document order. Running either the way
    // the harness runs ordinary inline scripts would execute something the real page does not.
    for html in [
        "<script nomodule>run()</script>",
        "<script type=\"text/javascript\" async>run()</script>",
    ] {
        let message = rejected_script_shape(
            html,
            "an execution-changing script attribute must be rejected",
        );
        assert!(
            message.contains("changes whether or when a browser runs the script"),
            "{message}"
        );
    }
}

#[test]
fn script_extraction_rejects_a_valueless_external_source_attribute() {
    let message = rejected_script_shape(
        "<script src></script>",
        "a valueless src still makes the script external",
    );
    assert!(message.contains("external script"), "{message}");
}

#[test]
fn script_extraction_ignores_tag_case_and_quoted_angle_brackets() {
    // Tag and attribute names are case-insensitive in HTML, and a quoted attribute value may
    // contain `>`. A case-sensitive scanner that stops at the first `>` gets both wrong.
    let html = "<SCRIPT TYPE=\"TEXT/JAVASCRIPT\" data-note=\"a>b\">run()</SCRIPT>";

    assert_eq!(extracted_scripts(html), vec!["run()".to_owned()]);
}

#[test]
fn script_extraction_ignores_elements_whose_name_merely_starts_with_script() {
    let html = "<scriptish>not a script</scriptish><script>run()</script>";

    assert_eq!(extracted_scripts(html), vec!["run()".to_owned()]);
}

#[test]
fn script_extraction_rejects_module_scripts() {
    // The harness concatenates inline sources into one classic script in a workspace holding no
    // emitted glue, provider or runtime module and no import map. Accepting a module block would
    // execute the page's real module graph under a different runtime model, which is the exact
    // claim this owner exists to stop.
    for html in [
        r#"<script type="module">import { f } from "./_moth/js/glue/module-a.js"; f();</script>"#,
        r#"<SCRIPT TYPE="Module">run()</SCRIPT>"#,
    ] {
        let message = rejected_script_shape(html, "a module script must be rejected, not executed");
        assert!(message.contains("module"), "{message}");
        assert!(
            message.contains("import map") || message.contains("module semantics"),
            "the rejection must say why the harness cannot run it: {message}"
        );
    }
}

#[test]
fn script_extraction_rejects_nameless_attribute_tokens() {
    // A stray `=` or a `/` that does not close the tag is a shape the harness only half
    // understands. Skipping the token quietly accepted the surrounding tag anyway.
    for html in [
        "<script =\"value\">run()</script>",
        "<script / type=\"text/javascript\">run()</script>",
    ] {
        let message = rejected_script_shape(
            html,
            "a nameless attribute token must be rejected, not skipped",
        );
        assert!(message.contains("malformed attribute token"), "{message}");
    }
}

// ─── Bounded Node execution ─────────────────────────────────────────────────

/// Runs one Node program in an owned workspace and returns the harness outcome.
#[track_caller]
fn run_node_program(
    source: &str,
    timeout: Duration,
) -> Result<String, super::super::assertions::RenderHarnessError> {
    with_harness_workspace(|workspace| {
        let script = workspace.write("program.js", source)?;
        let run = run_node_script_within(&script, workspace.path(), timeout)?;
        Ok(run.stdout)
    })
}

#[test]
fn node_harness_kills_a_program_that_never_exits() {
    let error = run_node_program("while (true) {}", Duration::from_millis(300))
        .expect_err("a non-terminating program must be reported, not awaited forever");

    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::Timeout,
        "a page that never finishes is a timeout: {error:?}"
    );
}

#[test]
fn node_harness_reports_a_failing_exit_status_with_captured_stderr() {
    let error = run_node_program(
        "process.stderr.write('boom'); process.exit(3);",
        Duration::from_secs(30),
    )
    .expect_err("a failing exit status must be reported");

    assert_eq!(error.kind, RenderHarnessErrorKind::ExitStatus);
    assert!(error.message.contains("boom"), "{}", error.message);
}

#[test]
fn node_harness_rejects_non_utf8_process_output() {
    // Lossy decoding would replace the invalid byte and hand an assertion text the process never
    // produced, so the encoding failure has to surface instead.
    let error = run_node_program(
        r"process.stdout.write(Buffer.from([0x66, 0x6f, 0xff, 0x6f]));",
        Duration::from_secs(30),
    )
    .expect_err("invalid UTF-8 on stdout must be reported");

    assert_eq!(error.kind, RenderHarnessErrorKind::OutputDecoding);
    assert!(
        error.message.contains("not valid UTF-8"),
        "{}",
        error.message
    );
}

#[test]
fn node_harness_rejects_output_that_exceeds_the_capture_bound() {
    // The bound keeps a runaway page from exhausting memory, but a truncated capture cannot be
    // reported as complete output, so it fails instead of being silently cut short.
    let error = run_node_program(
        "process.stdout.write('x'.repeat(5 * 1024 * 1024));",
        Duration::from_secs(30),
    )
    .expect_err("output past the capture bound must be reported");

    assert_eq!(error.kind, RenderHarnessErrorKind::OutputDecoding);
    assert!(error.message.contains("capture bound"), "{}", error.message);
}

#[test]
fn node_harness_removes_its_workspace_after_a_successful_run() {
    let mut workspace_path = PathBuf::new();
    let stdout = with_harness_workspace(|workspace| {
        workspace_path = workspace.path().to_path_buf();
        let script = workspace.write("program.js", "process.stdout.write('done');")?;
        let run = run_node_script_within(&script, workspace.path(), Duration::from_secs(30))?;
        Ok(run.stdout)
    })
    .expect("a well-behaved program should run");

    assert_eq!(stdout, "done");
    assert_path_missing(&workspace_path);
}

#[test]
#[cfg(unix)]
fn node_harness_reports_a_workspace_that_cannot_be_removed() {
    use std::os::unix::fs::PermissionsExt;

    // A workspace that outlives its run can change a later run's behaviour, so a removal failure
    // must fail the harness rather than be discarded.
    let mut locked_directory = PathBuf::new();
    let error = with_harness_workspace(|workspace| {
        workspace.write("kept.txt", "data")?;
        locked_directory = workspace.path().to_path_buf();
        std::fs::set_permissions(&locked_directory, std::fs::Permissions::from_mode(0o500))
            .expect("test should be able to make the workspace read-only");
        Ok(())
    })
    .expect_err("an unremovable workspace must be reported");

    // Restore write access before asserting so the directory never leaks out of this test.
    std::fs::set_permissions(&locked_directory, std::fs::Permissions::from_mode(0o700))
        .expect("test should be able to restore workspace permissions");
    std::fs::remove_dir_all(&locked_directory).expect("test should clean up its own workspace");

    assert_eq!(error.kind, RenderHarnessErrorKind::Cleanup);
}

#[test]
fn node_harness_reports_a_missing_interpreter_as_a_spawn_failure() {
    // The executable is a parameter of the process owner precisely so this boundary can be proved
    // without mutating the process-global PATH every other test in the run shares.
    let error = with_harness_workspace(|workspace| {
        let script = workspace.write("program.js", "process.stdout.write('never');")?;
        run_script_with_executable_for_test(
            "moth_render_harness_interpreter_that_does_not_exist",
            &script,
            workspace.path(),
            Duration::from_secs(30),
        )
        .map(|run| run.stdout)
    })
    .expect_err("an interpreter that cannot be started must be reported");

    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::Spawn,
        "a process that never started is a spawn failure: {error:?}"
    );
    assert!(
        error
            .message
            .contains("moth_render_harness_interpreter_that_does_not_exist"),
        "{}",
        error.message
    );
}

#[test]
fn node_harness_reports_a_workspace_file_it_cannot_write() {
    // A workspace write that silently failed would run the harness against a stale or absent
    // file, so the collision has to surface as the workspace boundary.
    let error = with_harness_workspace(|workspace| {
        std::fs::create_dir(workspace.path().join("harness.js"))
            .expect("test should be able to occupy the harness file name");
        workspace.write("harness.js", "process.stdout.write('never');")?;
        Ok(())
    })
    .expect_err("a workspace file that cannot be written must be reported");

    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::Workspace,
        "a workspace write failure is a workspace failure: {error:?}"
    );
    assert!(error.message.contains("harness.js"), "{}", error.message);
}

// ─── Required artifacts ─────────────────────────────────────────────────────

#[test]
fn rendered_output_reports_a_missing_required_artifact() {
    let build_result = build_result_with_index_html(VALID_HTML);

    let error = required_text_artifact_for_test(&build_result, "page.js", ArtifactKind::Js)
        .expect_err("an artifact the build never produced must be reported");

    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::Artifact,
        "an absent artifact is an artifact failure: {error:?}"
    );
    assert!(error.message.contains("page.js"), "{}", error.message);
}

#[test]
fn rendered_output_reports_a_required_artifact_of_the_wrong_kind() {
    // Bytes at 'page.js' are not JavaScript the harness can run, and reading them as text would
    // execute something the build did not emit as a script.
    let build_result = build_result_with_output_files(vec![(
        PathBuf::from("page.js"),
        FileKind::Bytes(vec![0x00, 0x01]),
    )]);

    let error = required_text_artifact_for_test(&build_result, "page.js", ArtifactKind::Js)
        .expect_err("an artifact of the wrong kind must be reported");

    assert_eq!(
        error.kind,
        RenderHarnessErrorKind::Artifact,
        "a wrong-kind artifact is an artifact failure: {error:?}"
    );
    assert!(error.message.contains("js artifact"), "{}", error.message);
}

// ─── Harness output protocol ────────────────────────────────────────────────

#[test]
fn html_wasm_rendered_output_waits_for_bootstrap_completion() {
    let temp_dir = tempfile::tempdir().expect("temporary Wasm harness directory should exist");
    std::fs::write(
        temp_dir.path().join("page.js"),
        r#"(async () => {
    await new Promise((resolve) => setTimeout(resolve, 80));
    document.getElementById("delayed-slot").insertAdjacentHTML("beforeend", "delayed");
})();
"#,
    )
    .expect("delayed bootstrap fixture should be written");

    let output = execute_wasm_harness_for_test(temp_dir.path())
        .expect("HTML-Wasm harness should await delayed bootstrap completion");
    assert_eq!(output.combined_output(), "delayed");
}

#[test]
fn rendered_output_decodes_typed_runtime_events() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"hello"},{"type":"fragment_insert","id":"root","html":"<p>hi</p>"}]}"#,
    )
    .expect("valid runtime events should decode");

    assert_eq!(
        output.events(),
        &[
            RuntimeEvent::Console {
                text: "hello".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<p>hi</p>".to_owned(),
            },
        ]
    );
}

#[test]
fn rendered_output_preserves_interleaved_event_chronology() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"before"},{"type":"fragment_insert","id":"root","html":"<b>one</b>"},{"type":"console","text":"after"},{"type":"fragment_insert","id":"root","html":"<b>two</b>"}]}"#,
    )
    .expect("interleaved runtime events should decode");

    assert_eq!(
        output.events(),
        &[
            RuntimeEvent::Console {
                text: "before".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<b>one</b>".to_owned(),
            },
            RuntimeEvent::Console {
                text: "after".to_owned(),
            },
            RuntimeEvent::FragmentInsert {
                id: "root".to_owned(),
                html: "<b>two</b>".to_owned(),
            },
        ]
    );
}

#[test]
fn rendered_output_derives_channel_views_in_event_order() {
    let output = parse_harness_output(
        r#"{"events":[{"type":"console","text":"before"},{"type":"fragment_insert","id":"root","html":"<b>one</b>"},{"type":"console","text":"after"},{"type":"fragment_insert","id":"root","html":"<b>two</b>"}]}"#,
    )
    .expect("interleaved runtime events should decode");

    assert_eq!(
        output.console_lines(),
        vec!["before".to_owned(), "after".to_owned()]
    );
    assert_eq!(
        output.slot_outputs(),
        vec![
            SlotOutput {
                id: "root".to_owned(),
                html: "<b>one</b>".to_owned(),
            },
            SlotOutput {
                id: "root".to_owned(),
                html: "<b>two</b>".to_owned(),
            },
        ]
    );
    assert_eq!(
        output.combined_output(),
        "before\n<b>one</b>\nafter\n<b>two</b>"
    );
}

#[test]
fn rendered_output_rejects_unknown_or_malformed_runtime_events() {
    for (json, expected_reason) in [
        (
            r#"{"events":[{"type":"unknown","text":"value"}]}"#,
            "unknown type",
        ),
        (
            r#"{"events":[{"type":"fragment_insert","id":"root"}]}"#,
            "missing string field 'html'",
        ),
        (
            r#"{"events":[{"type":"console","text":"value","extra":true}]}"#,
            "unknown field 'extra'",
        ),
    ] {
        let error =
            parse_harness_output(json).expect_err("malformed runtime events must fail decoding");
        assert_eq!(error.kind, RenderHarnessErrorKind::OutputProtocol);
        assert!(error.message.contains(expected_reason), "{}", error.message);
    }
}

#[test]
fn rendered_output_rejects_stdout_noise_around_the_event_payload() {
    // A stray log line from user code would otherwise be parsed as part of the protocol.
    let error = parse_harness_output("debug noise\n{\"events\":[]}")
        .expect_err("extra stdout text must fail the protocol");

    assert_eq!(error.kind, RenderHarnessErrorKind::OutputProtocol);
}

// ─── Harness wiring through success validation ──────────────────────────────

#[test]
fn rendered_output_validation_reports_harness_failure_without_script_blocks() {
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: RenderedOutputExpectation {
            contains: vec!["anything".to_string()],
            ..Default::default()
        },
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let build_result = build_result_with_index_html(VALID_HTML);

    let result = validate_success_result(&case, build_result, &expectation);

    assert_eq!(result.failure_kind, Some(FailureKind::HarnessFailed));
}

#[test]
fn rendered_output_node_is_not_invoked_without_a_rendered_assertion() {
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: RenderedOutputExpectation::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let result = validate_success_result(
        &case,
        build_result_with_index_html(VALID_HTML),
        &expectation,
    );

    assert!(
        result.passed,
        "Node should not be needed without assertions"
    );
}

#[test]
fn rendered_output_rejects_a_module_script_before_executing_the_page() {
    // Extraction-level coverage is not enough: this proves the executing entry point refuses the
    // page rather than concatenating a module body into the classic harness script.
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: Vec::new(),
        golden: GoldenExpectation::default(),
        rendered_output: RenderedOutputExpectation {
            contains: vec!["hydrated".to_string()],
            ..Default::default()
        },
        artifacts_must_not_exist: Vec::new(),
    };
    let case = success_test_case(BackendId::Html, expectation.clone());
    let module_page = VALID_HTML.replace(
        "  </body>",
        "<script type=\"module\">\nimport { render } from \"./_moth/js/glue/module-a.js\";\nrender();\n</script>\n  </body>",
    );

    let result = validate_success_result(
        &case,
        build_result_with_index_html(&module_page),
        &expectation,
    );

    assert!(
        !result.passed,
        "a module page cannot claim runtime evidence"
    );
    assert_eq!(
        result.failure_kind,
        Some(FailureKind::HarnessFailed),
        "an unsupported script shape is a harness fact, not a rendered-output mismatch"
    );
    assert!(
        result
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("module")),
        "failure must name the unsupported module shape: {:?}",
        result.failure_reason
    );
}
