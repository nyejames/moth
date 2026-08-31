//! Unit tests for HTML tracked-asset planning and passthrough emission.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, InvalidCompileTimePathReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::tests::test_support::{
    RenderedPathUsageInput, create_test_module, expect_bytes_output, rendered_path_usage,
};
use std::fs;
use std::path::Path;

// ------------------------------------------------------------------
//  RelativeToFile traversal floor
// ------------------------------------------------------------------

#[test]
fn relative_one_segment_underflow_returns_escapes_project_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("img")).expect("should create img dir");
    fs::write(root.join("img/logo.png"), [7_u8, 8, 9]).expect("should write asset");

    let mut string_table = StringTable::new();
    let mut module = create_test_module(root.join("src/docs/guide/@page.moth"), &mut string_table);
    let usage = rendered_path_usage(
        &mut string_table,
        RenderedPathUsageInput {
            source_path_components: &["..", "..", "..", "img", "logo.png"],
            public_path_components: &["..", "..", "..", "img", "logo.png"],
            filesystem_path: root.join("img/logo.png"),
            base: CompileTimePathBase::RelativeToFile,
            source_file_scope_components: &["src", "docs", "guide", "@page.moth"],
            line_number: 5,
        },
    );
    let expected_source_path = usage.source_path.clone();
    let expected_render_location = usage.render_location.clone();
    module.metadata.rendered_path_usages.push(usage);

    let error = plan_module_tracked_assets(
        &module,
        Path::new("docs/guide/index.html"),
        &mut string_table,
    )
    .expect_err("one-segment underflow should produce error");

    let diagnostic = error
        .first_error()
        .expect("should have an error-severity diagnostic");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCompileTimePath)
    ));
    match &diagnostic.payload {
        DiagnosticPayload::InvalidCompileTimePath { path, reason } => {
            assert_eq!(path, &expected_source_path);
            assert_eq!(*reason, InvalidCompileTimePathReason::EscapesProjectRoot);
        }
        payload => panic!("expected invalid compile-time path payload, got {payload:?}"),
    }
    assert_eq!(diagnostic.primary_location, expected_render_location);
}

#[test]
fn relative_repeated_underflow_returns_escapes_project_root() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("img")).expect("should create img dir");
    fs::write(root.join("img/logo.png"), [10_u8, 11, 12]).expect("should write asset");

    let mut string_table = StringTable::new();
    let mut module = create_test_module(root.join("src/@page.moth"), &mut string_table);
    let usage = rendered_path_usage(
        &mut string_table,
        RenderedPathUsageInput {
            source_path_components: &["..", "..", "..", "img", "logo.png"],
            public_path_components: &["..", "..", "..", "img", "logo.png"],
            filesystem_path: root.join("img/logo.png"),
            base: CompileTimePathBase::RelativeToFile,
            source_file_scope_components: &["src", "@page.moth"],
            line_number: 7,
        },
    );
    let expected_source_path = usage.source_path.clone();
    let expected_render_location = usage.render_location.clone();
    module.metadata.rendered_path_usages.push(usage);

    let error = plan_module_tracked_assets(&module, Path::new("index.html"), &mut string_table)
        .expect_err("repeated underflow should produce error");

    let diagnostic = error
        .first_error()
        .expect("should have an error-severity diagnostic");
    assert!(matches!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCompileTimePath)
    ));
    match &diagnostic.payload {
        DiagnosticPayload::InvalidCompileTimePath { path, reason } => {
            assert_eq!(path, &expected_source_path);
            assert_eq!(*reason, InvalidCompileTimePathReason::EscapesProjectRoot);
        }
        payload => panic!("expected invalid compile-time path payload, got {payload:?}"),
    }
    assert_eq!(diagnostic.primary_location, expected_render_location);
}

#[test]
fn duplicate_same_source_and_output_dedupes_within_module() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::write(root.join("assets/logo.png"), [1_u8, 2, 3]).expect("should write asset");

    let mut string_table = StringTable::new();
    let mut module = create_test_module(root.join("@page.moth"), &mut string_table);
    let usage = rendered_path_usage(
        &mut string_table,
        RenderedPathUsageInput {
            source_path_components: &["assets", "logo.png"],
            public_path_components: &["assets", "logo.png"],
            filesystem_path: root.join("assets/logo.png"),
            base: CompileTimePathBase::EntryRoot,
            source_file_scope_components: &["@page.moth"],
            line_number: 1,
        },
    );
    module.metadata.rendered_path_usages.push(usage.clone());
    module.metadata.rendered_path_usages.push(usage);

    let planned = plan_module_tracked_assets(&module, Path::new("index.html"), &mut string_table)
        .expect("planning succeeds");

    assert_eq!(planned.assets.len(), 1);
}

#[test]
fn large_asset_warning_dedupes_to_first_render_location() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::write(
        root.join("assets/video.mp4"),
        vec![0_u8; (DEFAULT_LARGE_TRACKED_ASSET_WARNING_BYTES as usize) + 1],
    )
    .expect("should write large asset");

    let mut string_table = StringTable::new();
    let mut module = create_test_module(root.join("@page.moth"), &mut string_table);
    let first_usage = rendered_path_usage(
        &mut string_table,
        RenderedPathUsageInput {
            source_path_components: &["assets", "video.mp4"],
            public_path_components: &["assets", "video.mp4"],
            filesystem_path: root.join("assets/video.mp4"),
            base: CompileTimePathBase::EntryRoot,
            source_file_scope_components: &["@page.moth"],
            line_number: 2,
        },
    );
    let second_usage = rendered_path_usage(
        &mut string_table,
        RenderedPathUsageInput {
            source_path_components: &["assets", "video.mp4"],
            public_path_components: &["assets", "video.mp4"],
            filesystem_path: root.join("assets/video.mp4"),
            base: CompileTimePathBase::EntryRoot,
            source_file_scope_components: &["@page.moth"],
            line_number: 8,
        },
    );
    module.metadata.rendered_path_usages.push(first_usage);
    module.metadata.rendered_path_usages.push(second_usage);

    let planned = plan_module_tracked_assets(&module, Path::new("index.html"), &mut string_table)
        .expect("planning succeeds");

    assert_eq!(planned.warnings.len(), 1);
    assert!(matches!(
        planned.warnings[0].kind,
        crate::compiler_frontend::compiler_messages::DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::LargeTrackedAsset
        )
    ));
    assert_eq!(
        planned.warnings[0].primary_location.start_pos.line_number,
        2
    );
}

#[test]
fn emit_tracked_assets_reads_source_bytes_into_binary_outputs() {
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    fs::create_dir_all(root.join("assets")).expect("should create assets dir");
    fs::write(root.join("assets/logo.png"), [9_u8, 8, 7, 6]).expect("should write asset");

    let mut string_table = StringTable::new();
    let mut module = create_test_module(root.join("@page.moth"), &mut string_table);
    module
        .metadata
        .rendered_path_usages
        .push(rendered_path_usage(
            &mut string_table,
            RenderedPathUsageInput {
                source_path_components: &["assets", "logo.png"],
                public_path_components: &["assets", "logo.png"],
                filesystem_path: root.join("assets/logo.png"),
                base: CompileTimePathBase::EntryRoot,
                source_file_scope_components: &["@page.moth"],
                line_number: 1,
            },
        ));
    let planned = plan_module_tracked_assets(&module, Path::new("index.html"), &mut string_table)
        .expect("planning succeeds");

    let output_files =
        emit_tracked_assets(&planned.assets, &string_table).expect("emission succeeds");
    assert_eq!(
        expect_bytes_output(&output_files, "assets/logo.png"),
        [9_u8, 8, 7, 6]
    );
}
