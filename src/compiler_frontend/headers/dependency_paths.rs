//! Header-stage dependency-path validation.
//!
//! WHAT: rejects authored dependency paths that cannot become a retained structural provider path.
//! WHY: dependencies affect file-local visibility and retained declaration-ordering hints, so this
//! validation belongs to the header stage rather than AST body parsing. Until a real
//! normalisation transform exists, the retained path is the validated authored path.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::InvalidImportPathReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::source_packages::root_file::dependency_path_references_config_file;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::CONFIG_FILE_NAME;

pub(super) fn validate_dependency_path(
    dependency_path: PathId,
    path_span: &SourceSpan,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<(), DiagnosticDraft> {
    // Exact `@/` is represented by the empty root path. A dependency must name a provider
    // beneath the owning module root, so the site root is rejected under its own reason.
    if dependency_path == PathId::ROOT {
        return Err(imports::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::PublicRoot,
            Some(*path_span),
        ));
    }

    // The reserved build-only config file wins over the generic `.moth` extension rejection so
    // the author sees why that exact source is not importable.
    if dependency_path_references_config_file(dependency_path, path_fork, string_table) {
        return Err(imports::direct_special_file_import(
            dependency_path,
            Some(*path_span),
        ));
    }

    let mut components = Vec::new();
    let components = path_fork.resolve_components(dependency_path, &mut components);
    if has_explicit_moth_extension(components, string_table) {
        return Err(imports::explicit_moth_extension(
            dependency_path,
            Some(*path_span),
        ));
    }

    if components
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
    {
        return Err(imports::invalid_import_path(
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
        return Err(imports::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::CurrentDirectorySegment,
            Some(*path_span),
        ));
    }

    Ok(())
}

/// True for an explicit `.moth` extension, excluding config-named directories.
pub(super) fn has_explicit_moth_extension(components: &[StringId], strings: &StringTable) -> bool {
    components.iter().enumerate().any(|(index, &component)| {
        let name = strings.resolve(component);
        name.ends_with(SourceFileKind::Moth.extension_suffix())
            && (name != CONFIG_FILE_NAME || index + 1 == components.len())
    })
}
