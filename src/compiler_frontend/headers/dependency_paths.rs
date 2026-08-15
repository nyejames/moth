//! Header-stage dependency-path validation.
//!
//! WHAT: rejects authored dependency paths that cannot become a retained structural provider path.
//! WHY: dependencies affect file-local visibility and retained declaration-ordering hints, so this
//! validation belongs to the header stage rather than AST body parsing. Until a real
//! normalisation transform exists, the retained path is the validated authored path.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

pub(super) fn validate_dependency_path(
    dependency_path: &InternedPath,
    path_location: &SourceLocation,
    string_table: &StringTable,
) -> Result<(), Box<CompilerDiagnostic>> {
    // Exact `@/` is represented by an empty canonical path row. It remains valid for compile-time
    // path expressions, but a dependency must name a provider beneath the owning module root.
    if dependency_path.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_import_path(
            dependency_path.to_owned(),
            InvalidImportPathReason::PublicRoot,
            path_location.clone(),
        )));
    }

    if dependency_path
        .as_components()
        .iter()
        .any(|component| string_table.resolve(*component).ends_with(".moth"))
    {
        return Err(Box::new(CompilerDiagnostic::explicit_moth_extension(
            dependency_path.to_owned(),
            path_location.clone(),
        )));
    }

    if dependency_path
        .as_components()
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
    {
        return Err(Box::new(CompilerDiagnostic::invalid_import_path(
            dependency_path.to_owned(),
            InvalidImportPathReason::ParentDirectorySegment,
            path_location.clone(),
        )));
    }

    let mut dependency_components = dependency_path.as_components().iter().copied();
    let first = dependency_components
        .next()
        .expect("empty dependency paths are rejected before component validation");

    let first_segment = string_table.resolve(first);
    if first_segment == "." {
        return Err(Box::new(CompilerDiagnostic::invalid_import_path(
            dependency_path.to_owned(),
            InvalidImportPathReason::CurrentDirectorySegment,
            path_location.clone(),
        )));
    }

    Ok(())
}
