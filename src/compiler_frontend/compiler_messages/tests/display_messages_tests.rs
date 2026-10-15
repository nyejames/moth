//! Guidance and portable path presentation contracts.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerErrorMetadataKey};
use crate::compiler_frontend::compiler_messages::render::infrastructure::format_error_guidance_lines;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, relative_display_path_from_root, special_file_name_from_path,
    support_root_import_suggestion,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

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

    assert_eq!(
        special_file_name_from_path(extensionless_path, context),
        "+pkg.moth"
    );

    let explicit_path = path_fork
        .try_intern_portable_path("input/+pkg.moth", &mut string_table)
        .expect("test path fits");
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);

    assert_eq!(
        special_file_name_from_path(explicit_path, context),
        "+pkg.moth"
    );
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
fn guidance_lines_include_replacement_and_location_variants() {
    let mut replacement_error =
        CompilerError::file_error(Path::new("output/page.html"), "could not write output");
    replacement_error.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedReplacement,
        String::from("let value = 1"),
    );

    let mut location_error = CompilerError::compiler_error("invalid retained state");
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
fn relative_display_path_strips_root_prefix() {
    let root = Path::new("/workspace/project");
    let scope = Path::new("/workspace/project/src/main.moth");

    let relative = relative_display_path_from_root(scope, root);

    assert_eq!(relative, "src/main.moth");
}
