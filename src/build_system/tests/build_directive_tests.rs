//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::build_project;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops;
use std::fs;
use std::path::PathBuf;

#[test]
fn frontend_builtin_directives_work_with_a_minimal_builder() {
    let mut discovery_stops = DiagnosticDiscoveryStops::NONE;
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    let entry_file = root.join("builtins.moth");
    fs::write(
        &entry_file,
        "[$before [:<li>], $after [:</li]:\n<ul>\n  [$md:\n# Docs\n]\n  [$raw:\n  keep\n]\n  [$fresh:\n    [: plain ]\n  ]\n</ul>\n]\n[$code \"moth\":\n    loop value\n]\n[$html:\n    <p>Hello</p>\n]",
    )
    .expect("should write source file");

    let builder = MinimalBuilder;
    let result = successful_compilation_value(
        build_project(
            &builder,
            entry_file
                .to_str()
                .expect("temp file path should be valid UTF-8 for this test"),
            &[],
            &BuildConfigInputSet::new(),
            &mut discovery_stops,
        )
        .unwrap_or_else(|never| match never {}),
    );

    assert_eq!(result.project.output_files.len(), 1);
    assert_eq!(
        result.project.output_files[0].relative_output_path(),
        PathBuf::from("index.html")
    );
    let FileKind::Html(rendered) = result.project.output_files[0].file_kind() else {
        panic!("the minimal builder should emit HTML");
    };
    assert!(
        rendered.contains("<span class='moth-code-keyword'>loop</span>"),
        "`$code` must format under a minimal builder, got: {rendered}"
    );
    assert!(
        rendered.contains("<p>Hello</p>"),
        "`$html` must format under a minimal builder, got: {rendered}"
    );
    assert!(
        !rendered.contains("\n    <p>Hello</p>"),
        "`$html` formatting must remove template-body indentation, got: {rendered}"
    );
}
