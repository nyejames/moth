//! Header-stage dependency-path validation.
//!
//! WHAT: rejects authored dependency paths that cannot become a retained structural provider path.
//! WHY: dependencies affect file-local visibility and retained declaration-ordering hints, so this
//! validation belongs to the header stage rather than AST body parsing. Until a real
//! normalisation transform exists, the retained path is the validated authored path.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(super) fn validate_dependency_path(
    dependency_path: PathId,
    path_span: &SourceSpan,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    // Exact `@/` is represented by the empty root path. A dependency must name a provider
    // beneath the owning module root, so the site root is rejected under its own reason.
    if dependency_path == PathId::ROOT {
        return Err(CompilerDiagnostic::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::PublicRoot,
            Some(*path_span),
        ));
    }

    let mut components = Vec::new();
    path_fork.resolve_components(dependency_path, &mut components);
    if components
        .iter()
        .any(|component| string_table.resolve(*component).ends_with(".moth"))
    {
        return Err(CompilerDiagnostic::explicit_moth_extension(
            dependency_path,
            Some(*path_span),
        ));
    }

    if components
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
    {
        return Err(CompilerDiagnostic::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::ParentDirectorySegment,
            Some(*path_span),
        ));
    }

    let mut dependency_components = components.iter().copied();
    let first = dependency_components
        .next()
        .expect("empty dependency paths are rejected before component validation");

    let first_segment = string_table.resolve(first);
    if first_segment == "." {
        return Err(CompilerDiagnostic::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::CurrentDirectorySegment,
            Some(*path_span),
        ));
    }

    Ok(())
}

