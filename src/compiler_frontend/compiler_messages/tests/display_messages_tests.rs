use super::{error_display_name, error_visual, format_error_guidance_lines};
use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::render::{
    relative_display_path_from_root, special_file_name_from_path, support_root_import_suggestion,
    DiagnosticRenderContext,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

#[test]
fn guidance_lines_include_compiler_stage_and_suggestions_when_present() {
    let mut error = CompilerError::compiler_error("bad compiler state");
    error.new_metadata_entry(
        CompilerErrorMetadataKey::CompilationStage,
        String::from("Expression Parsing"),
    );
    error.new_metadata_entry(
        CompilerErrorMetadataKey::PrimarySuggestion,
        String::from("Do the thing"),
    );
    error.new_metadata_entry(
        CompilerErrorMetadataKey::AlternativeSuggestion,
        String::from("Try another thing"),
    );
    error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedInsertion,
        String::from("->"),
    );
    error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedLocation,
        String::from("after token X"),
    );

    let lines = format_error_guidance_lines(&error);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Stage: Expression Parsing"))
    );
    assert!(lines.iter().any(|line| line == "Do the thing"));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Alternative: Try another thing"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Suggested insertion: '->' after token X"))
    );
}

#[test]
fn guidance_lines_are_empty_when_metadata_is_missing() {
    let error = CompilerError::compiler_error("bad compiler state");
    let lines = format_error_guidance_lines(&error);
    assert!(lines.is_empty());
}

#[test]
fn special_file_renderer_names_support_roots() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let extensionless_path = path_fork
        .try_intern_portable_path("input/+pkg", &mut string_table)
        .expect("test path fits");
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);

    assert_eq!(special_file_name_from_path(extensionless_path, context), "+pkg.moth");

    let explicit_path = path_fork
        .try_intern_portable_path("input/+pkg.moth", &mut string_table)
        .expect("test path fits");
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);

    assert_eq!(special_file_name_from_path(explicit_path, context), "+pkg.moth");
}

#[test]
fn support_root_renderer_suggests_containing_package_directory() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("tools/helpers/+package", &mut string_table)
        .expect("test path fits");
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);

    assert_eq!(
        support_root_import_suggestion(path, context),
        " Bind the support package `@tools/helpers` instead of the root file."
    );
}

#[test]
fn default_error_headers_keep_friendly_type_specific_visuals() {
    let cases = [
        (ErrorType::Compiler, "🔥 ヽ༼☉ ‿ ⚆༽ﾉ 🔥", "Compiler Bug"),
        (ErrorType::File, "🔥📁🔥", "Missing File or Directory"),
        (ErrorType::Config, "🔥📄🔥", "Malformed Config"),
        (ErrorType::DevServer, "(ﾉ☉_⚆)ﾉ 🔥🖥️🔥", "Dev Server Issue"),
        (
            ErrorType::HirTransformation,
            "(☉_☉) 🔥",
            "HIR Transformation Error",
        ),
        (
            ErrorType::Backend(BackendErrorType::LirTransformation),
            "ヽ(°〇°)ﾉ 🔥",
            "LIR Transformation Bug",
        ),
        (
            ErrorType::Backend(BackendErrorType::WasmGeneration),
            "(° O °) 🔥",
            "WASM Generation Bug",
        ),
    ];

    for (error_type, expected_visual, expected_name) in cases {
        assert_eq!(error_visual(&error_type), expected_visual);
        assert_eq!(error_display_name(&error_type), expected_name);
    }
}

#[test]
fn guidance_lines_include_replacement_and_location_variants() {
    let mut replacement_error = CompilerError::new("bad config", None, ErrorType::Config);
    replacement_error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedReplacement,
        String::from("let value = 1"),
    );

    let mut location_error = CompilerError::new("bad config", None, ErrorType::Config);
    location_error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedLocation,
        String::from("before the closing ')'"),
    );

    let replacement_lines = format_error_guidance_lines(&replacement_error);
    let location_lines = format_error_guidance_lines(&location_error);

    assert!(
        replacement_lines
            .iter()
            .any(|line| line.contains("Suggested replacement: let value = 1"))
    );
    assert!(
        location_lines
            .iter()
            .any(|line| line.contains("Suggested location: before the closing ')'"))
    );
}

#[test]
fn compiler_messages_from_error_wraps_one_error_without_warnings() {
    let error = CompilerError::compiler_error("bad compiler state");
    let messages = CompilerMessages::from_error(error, StringTable::new());

    assert_eq!(messages.error_count(), 1);
    assert_eq!(messages.warnings().count(), 0);
    let error = messages
        .infrastructure_error()
        .expect("CompilerError should be preserved in the outer lane");
    assert_eq!(error.msg.as_str(), "bad compiler state");
    assert!(messages.first_error().is_none());
}

#[test]
fn compiler_error_metadata_and_host_path_are_preserved() {
    let mut error = CompilerError::new("bad compiler state", None, ErrorType::Config);
    error.host_path = Some(PathBuf::from("project/main.moth"));
    error.new_metadata_entry(
        CompilerErrorMetadataKey::PrimarySuggestion,
        String::from("Rename the config key"),
    );

    assert_eq!(error.error_type, ErrorType::Config);
    assert_eq!(error.host_path, Some(PathBuf::from("project/main.moth")));
    assert_eq!(
        error
            .metadata
            .get(&CompilerErrorMetadataKey::PrimarySuggestion),
        Some(&String::from("Rename the config key"))
    );
}

#[test]
fn compiler_error_preserves_exact_source_span_and_host_path() {
    let mut span_builder = ExtendedSpanBuilder::new();
    let source_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(7, 2, &mut span_builder).expect("test span should fit inline"),
    ));
    let mut error = CompilerError::new("bad compiler state", source_span, ErrorType::Compiler);
    error.host_path = Some(PathBuf::from("project/main.moth"));

    assert_eq!(error.source_span, source_span);
    assert_eq!(error.host_path, Some(PathBuf::from("project/main.moth")));
}

#[cfg(unix)]
#[test]
fn invalid_utf8_host_paths_remain_distinct() {
    let first_path = PathBuf::from(OsString::from_vec(b"source-\xFF.moth".to_vec()));
    let second_path = PathBuf::from(OsString::from_vec(b"source-\xFE.moth".to_vec()));

    let mut first_error = CompilerError::new("test", None, ErrorType::File);
    first_error.host_path = Some(first_path.clone());
    let mut second_error = CompilerError::new("test", None, ErrorType::File);
    second_error.host_path = Some(second_path.clone());

    assert_ne!(first_error.host_path, second_error.host_path);
    assert_eq!(first_error.host_path, Some(first_path));
    assert_eq!(second_error.host_path, Some(second_path));
}

#[test]
fn relative_display_path_strips_root_prefix() {
    let root = Path::new("/workspace/project");
    let scope = Path::new("/workspace/project/src/main.moth");

    let relative = relative_display_path_from_root(scope, root);

    assert_eq!(relative, "src/main.moth");
}
