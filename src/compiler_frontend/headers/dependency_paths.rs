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

    // The exact `@config.moth` spelling stays a source-classified path: Stage 0 resolves it
    // against the indexed directory surfaces and reports the reserved-file diagnostic only when
    // no module surface owns it. Checking it here would reject a config-named child directory
    // before its ordinary module surface could be selected.
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

/// True for an explicit `.moth` extension, excluding the exact reserved config spelling.
///
/// WHAT: rejects ordinary explicit Moth source extensions while deferring `@config.moth`.
/// WHY: the reserved spelling may name an ordinary directory module surface, so Stage 0 decides
///      whether it is that surface or the reserved file. Any other `.moth` component remains an
///      invalid explicit extension here.
pub(super) fn has_explicit_moth_extension(components: &[StringId], strings: &StringTable) -> bool {
    components.iter().any(|&component| {
        let name = strings.resolve(component);
        name.ends_with(SourceFileKind::Moth.extension_suffix()) && name != CONFIG_FILE_NAME
    })
}
