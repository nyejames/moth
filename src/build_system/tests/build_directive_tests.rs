//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::build_project;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::{DiagnosticCategory, DiagnosticSeverity};
use std::fs;
use std::path::PathBuf;

#[test]
fn html_project_directives_fail_when_builder_does_not_register_them() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    for (directive_name, source) in [
        ("html", "[$html:\n<div>Hello</div>\n]"),
        ("css", "[$css:\n.button { color: red; }\n]"),
        ("escape_html", "[$escape_html:\n<b>Hello</b>\n]"),
    ] {
        let entry_file = root.join(format!("{directive_name}.moth"));
        fs::write(&entry_file, source).expect("should write source file");

        let builder = NoDirectiveBuilder;
        let result = build_project(
            &builder,
            entry_file
                .to_str()
                .expect("temp file path should be valid UTF-8 for this test"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {});
        let messages = diagnosed_reports(&result);
        let (report, view) = messages
            .iter_severity(DiagnosticSeverity::Error)
            .next()
            .expect("unsupported directive should have an error report");
        assert_eq!(view.category(), DiagnosticCategory::Syntax);
        assert_eq!(view.descriptor().code, "MOTH-SYNTAX-0010");
        let rendered =
            terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
        let message = rendered
            .split('|')
            .nth(4)
            .expect("terse diagnostic should include its message field");
        let (directive_message, _) = message
            .split_once(" Registered directives are ")
            .expect("unsupported style directive message should list registered directives");
        assert_eq!(
            directive_message,
            format!("Style directive '${directive_name}' is unsupported here."),
            "expected unsupported directive error for '${directive_name}', got: {message}"
        );
    }
}

#[test]
fn frontend_builtin_directives_work_without_builder_registered_project_directives() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("builtins.moth");
    fs::write(
        &entry_file,
        "[$children([:<li>[$slot]</li>]):\n<ul>\n  [$md:\n# Docs\n]\n  [$raw:\n  keep\n]\n  [$fresh:\n    [: plain ]\n  ]\n</ul>\n]",
    )
    .expect("should write source file");

    let builder = NoDirectiveBuilder;
    let result = successful_compilation_value(
        build_project(
            &builder,
            entry_file
                .to_str()
                .expect("temp file path should be valid UTF-8 for this test"),
            &[],
            &BuildConfigInputSet::new(),
        )
        .unwrap_or_else(|never| match never {}),
    );

    assert_eq!(result.project.output_files.len(), 1);
    assert_eq!(
        result.project.output_files[0].relative_output_path(),
        PathBuf::from("index.html")
    );
}
