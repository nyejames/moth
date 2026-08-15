use super::{error_display_name, error_visual, format_error_guidance_lines};
use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType, SourceLocation,
};
use crate::compiler_frontend::compiler_messages::render::{
    relative_display_path_from_root, resolve_source_file_path, resolved_display_path,
    special_file_name_from_path, support_root_import_suggestion,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticKind, InfrastructureDiagnosticKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::basic::normalize_path;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "moth_display_messages_{name}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("should create temp test dir");
    dir
}

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
    let mut extensionless_path = InternedPath::new();
    extensionless_path.push_str("input", &mut string_table);
    extensionless_path.push_str("+pkg", &mut string_table);

    assert_eq!(
        special_file_name_from_path(&extensionless_path, &string_table),
        "+pkg.moth"
    );

    let mut explicit_path = InternedPath::new();
    explicit_path.push_str("input", &mut string_table);
    explicit_path.push_str("+pkg.moth", &mut string_table);

    assert_eq!(
        special_file_name_from_path(&explicit_path, &string_table),
        "+pkg.moth"
    );
}

#[test]
fn support_root_renderer_suggests_containing_package_directory() {
    let mut string_table = StringTable::new();
    let mut path = InternedPath::new();
    path.push_str("tools", &mut string_table);
    path.push_str("helpers", &mut string_table);
    path.push_str("+package", &mut string_table);

    assert_eq!(
        support_root_import_suggestion(&path, &string_table),
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
    let mut replacement_error =
        CompilerError::new("bad config", SourceLocation::default(), ErrorType::Config);
    replacement_error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedReplacement,
        String::from("let value = 1"),
    );

    let mut location_error =
        CompilerError::new("bad config", SourceLocation::default(), ErrorType::Config);
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
    let (_error_type, message, _location) = messages
        .first_infrastructure_error_for_tests()
        .expect("CompilerError should be wrapped as an infrastructure diagnostic payload");
    assert_eq!(message, "bad compiler state");
    assert_eq!(
        messages.first_error().map(|diagnostic| diagnostic.kind),
        Some(DiagnosticKind::Infrastructure(
            InfrastructureDiagnosticKind::InfrastructureFailure,
        )),
    );
    assert_eq!(
        messages
            .first_error()
            .map(|diagnostic| diagnostic.kind.code()),
        Some("MOTH-INFRA-0001"),
    );
}

#[test]
fn compiler_error_metadata_and_overrides_are_preserved() {
    let mut string_table = StringTable::new();
    let mut error = CompilerError::compiler_error("bad compiler state")
        .with_scope_path(Path::new("project/main.moth"), &mut string_table)
        .with_error_type(ErrorType::Config);
    error.new_metadata_entry(
        CompilerErrorMetadataKey::PrimarySuggestion,
        String::from("Rename the config key"),
    );

    assert_eq!(error.error_type, ErrorType::Config);
    assert_eq!(
        error.location.scope.to_path_buf(&string_table),
        PathBuf::from("project/main.moth")
    );
    assert_eq!(
        error
            .metadata
            .get(&CompilerErrorMetadataKey::PrimarySuggestion),
        Some(&String::from("Rename the config key"))
    );
}

#[test]
fn with_scope_path_preserves_existing_span_positions() {
    let mut string_table = StringTable::new();
    let mut error = CompilerError::new(
        "bad compiler state",
        SourceLocation::new(
            crate::compiler_frontend::symbols::interned_path::InternedPath::try_from_filesystem_path(
                Path::new("old.moth"),
                &mut string_table,
            ).expect("test path should be UTF-8"),
            crate::compiler_frontend::tokenizer::tokens::CharPosition {
                line_number: 4,
                char_column: 7,
            },
            crate::compiler_frontend::tokenizer::tokens::CharPosition {
                line_number: 4,
                char_column: 9,
            },
        ),
        ErrorType::Compiler,
    );

    error = error.with_scope_path(Path::new("project/main.moth"), &mut string_table);

    assert_eq!(error.location.start_pos.line_number, 4);
    assert_eq!(error.location.start_pos.char_column, 7);
    assert_eq!(
        error.location.scope.to_path_buf(&string_table),
        PathBuf::from("project/main.moth")
    );
}

#[cfg(unix)]
#[test]
fn invalid_utf8_diagnostic_scopes_remain_distinct() {
    let first_path = PathBuf::from(OsString::from_vec(b"source-\xFF.moth".to_vec()));
    let second_path = PathBuf::from(OsString::from_vec(b"source-\xFE.moth".to_vec()));
    let mut string_table = StringTable::new();

    let first_scope = SourceLocation::from_path(&first_path, &mut string_table)
        .scope
        .to_path_buf(&string_table);
    let second_scope = CompilerError::compiler_error("test")
        .with_scope_path(&second_path, &mut string_table)
        .location
        .scope
        .to_path_buf(&string_table);

    assert_ne!(first_scope, second_scope);
    assert!(first_scope.to_string_lossy().contains("\\xFF"));
    assert!(second_scope.to_string_lossy().contains("\\xFE"));
}

#[test]
fn relative_display_path_strips_root_prefix() {
    let root = Path::new("/workspace/project");
    let scope = Path::new("/workspace/project/src/main.moth");

    let relative = relative_display_path_from_root(scope, root);

    assert_eq!(relative, "src/main.moth");
}

#[cfg(windows)]
#[test]
fn normalize_display_path_strips_windows_extended_prefix() {
    let normalized = normalize_path(Path::new(r"\\?\C:\workspace\main.moth"));
    assert_eq!(normalized, PathBuf::from(r"C:\workspace\main.moth"));
}

#[test]
fn resolve_source_file_path_strips_header_suffix_before_lookup() {
    let root: PathBuf = temp_dir("header_scope");
    let source_file = root.join("main.moth");
    fs::write(&source_file, "page #= []").expect("should write source file");

    let mut string_table = StringTable::new();
    let header_scope = source_file.join("title.header");
    let header_scope =
        crate::compiler_frontend::symbols::interned_path::InternedPath::try_from_filesystem_path(
            &header_scope,
            &mut string_table,
        )
        .expect("test path should be UTF-8");
    let resolved = resolve_source_file_path(&header_scope, &string_table);

    let expected = fs::canonicalize(&source_file).expect("should canonicalize source file");

    let normalized_resolved = normalize_path(&resolved);
    let normalized_expected = normalize_path(&expected);
    assert_eq!(normalized_resolved, normalized_expected);

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn formatted_warning_uses_resolved_source_file_path_for_header_scopes() {
    let root: PathBuf = temp_dir("header_warning_scope");
    let source_file = root.join("main.moth");
    fs::write(&source_file, "page #= []").expect("should write source file");

    let mut string_table = StringTable::new();
    let header_scope = source_file.join("title.header");
    let header_scope =
        crate::compiler_frontend::symbols::interned_path::InternedPath::try_from_filesystem_path(
            &header_scope,
            &mut string_table,
        )
        .expect("test path should be UTF-8");

    let displayed = resolved_display_path(&header_scope, &string_table);
    assert!(displayed.ends_with("main.moth"));
    assert!(!displayed.contains(".header"));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn resolve_source_file_path_normalizes_canonical_windows_paths() {
    let root: PathBuf = temp_dir("resolve_normalize");
    let source_file = root.join("main.moth");
    fs::write(&source_file, "page #= []").expect("should write source file");

    let mut string_table = StringTable::new();
    let interned = InternedPath::try_from_filesystem_path(&source_file, &mut string_table)
        .expect("test path should be UTF-8");

    let resolved = resolve_source_file_path(&interned, &string_table);
    let expected = normalize_path(&fs::canonicalize(&source_file).expect("should canonicalize"));

    assert_eq!(
        resolved, expected,
        "resolve_source_file_path should return a normalized canonical path \
         (no Windows verbatim prefix)"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
