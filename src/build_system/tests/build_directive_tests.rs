//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use std::fs;
use std::path::PathBuf;

#[test]
fn html_project_directives_fail_when_builder_does_not_register_them() {
    let root = temp_dir("directive_boundary_missing");
    fs::create_dir_all(&root).expect("should create temp root");

    for (directive_name, source) in [
        ("html", "[$html:\n<div>Hello</div>\n]"),
        ("css", "[$css:\n.button { color: red; }\n]"),
        ("escape_html", "[$escape_html:\n<b>Hello</b>\n]"),
    ] {
        let entry_file = root.join(format!("{directive_name}.moth"));
        fs::write(&entry_file, source).expect("should write source file");

        let builder = ProjectBuilder::new(Box::new(NoDirectiveBuilder));
        let result = build_project(
            &builder,
            entry_file
                .to_str()
                .expect("temp file path should be valid UTF-8 for this test"),
            &[],
        );

        let Err(messages) = result else {
            panic!("project-owned directives should fail when not registered");
        };
        assert!(
            messages.error_diagnostics().any(|diagnostic| matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidStyleDirective { directive_name: name, .. }
                    if messages.string_table.resolve(*name) == directive_name
            )),
            "expected unsupported directive error for '${directive_name}', got: {:?}",
            messages
                .error_diagnostics()
                .map(|diagnostic| &diagnostic.payload)
                .collect::<Vec<_>>()
        );
    }

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn frontend_builtin_directives_work_without_builder_registered_project_directives() {
    let root = temp_dir("frontend_builtin_boundary");
    fs::create_dir_all(&root).expect("should create temp root");

    let entry_file = root.join("builtins.moth");
    fs::write(
        &entry_file,
        "[$children([:<li>[$slot]</li>]):\n<ul>\n  [$md:\n# Docs\n]\n  [$raw:\n  keep\n]\n  [$fresh:\n    [: plain ]\n  ]\n</ul>\n]",
    )
    .expect("should write source file");

    let builder = ProjectBuilder::new(Box::new(NoDirectiveBuilder));
    let result = build_project(
        &builder,
        entry_file
            .to_str()
            .expect("temp file path should be valid UTF-8 for this test"),
        &[],
    )
    .expect("frontend built-ins should compile without project-owned style registrations");

    assert_eq!(result.project.output_files.len(), 1);
    assert_eq!(
        result.project.output_files[0].relative_output_path(),
        PathBuf::from("index.html")
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

use crate::compiler_tests::test_support::temp_dir;
