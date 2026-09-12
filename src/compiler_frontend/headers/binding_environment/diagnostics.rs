//! Header-stage binding, public export, alias, and collision diagnostic helpers.
//!
//! WHAT: named helpers that construct structured diagnostics for binding-environment failures.
//! WHY: centralizing diagnostic construction keeps error messages consistent and makes it
//! easy to update wording or metadata across all dependency-related failures.
//! MUST NOT: contain business logic for resolution or registration.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Diagnostic when two visible bindings in the same file target different symbols.
///
/// If only one binding has authored provenance, that span becomes the primary owner rather than
/// emitting a duplicate secondary label for the same location.
pub(super) fn dependency_name_collision(
    local_name: StringId,
    span: Option<SourceSpan>,
    previous_span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    let primary_span = span.or(previous_span);
    let previous_span = span.and(previous_span);
    CompilerDiagnostic::import_name_collision(local_name, previous_span, primary_span)
}

/// Diagnostic when a dependency through a public surface resolves to a symbol that the surface does
/// not expose.
pub(super) fn not_exported_by_public_surface(
    dependency_path: &PathId,
    public_surface_name: StringId,
    public_surface_type: ImportPublicSurfaceType,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::not_exported_by_public_surface(
        dependency_path.clone(),
        public_surface_name,
        public_surface_type,
        span,
    )
}

/// Build the public-surface diagnostic for a missing name on a resolved provider interface.
///
/// WHAT: derives the provider's public-surface identity and uses the same typed dependency diagnostic
///       for ordinary provider binding and public-export preparation.
/// WHY: a provider interface is valid compiler state, but an authored selection may still name a
///      member that the provider does not expose. That source error retains the selected-name span.
pub(crate) fn provider_public_surface_diagnostic(
    requested_path: &PathId,
    interface: &PublicSemanticInterface,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let module_path = interface.module_origin.logical_module_path();
    let (surface_name, surface_type) = module_path
        .rsplit('/')
        .find(|component| !component.is_empty())
        .map_or_else(
            || {
                (
                    interface.module_origin.package().name(),
                    ImportPublicSurfaceType::SourcePackage,
                )
            },
            |name| (name, ImportPublicSurfaceType::ModuleRoot),
        );
    let surface_name = string_table.intern(surface_name);

    not_exported_by_public_surface(requested_path, surface_name, surface_type, span)
}

/// Diagnostic when a dependency path directly references a module-root file or canonical `config.moth`.
pub(super) fn direct_special_file_dependency(
    path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::direct_special_file_import(path.clone(), span)
}

/// Diagnostic when a dependency path matches a source file but not a symbol.
pub(super) fn bare_file_dependency(
    path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::bare_file_import(path.clone(), span)
}

/// Diagnostic when a dependency path cannot be resolved to any known source or external symbol.
pub(super) fn missing_dependency_target(
    path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::missing_import_target(path.clone(), span)
}

/// Diagnostic when a direct source dependency targets a symbol that is not exported.
pub(super) fn not_exported_by_source_file(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::not_exported_by_source_file(symbol_path.clone(), span)
}

/// Diagnostic when a dependency path matches multiple source symbols ambiguously.
pub(super) fn ambiguous_dependency_target(
    path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::ambiguous_import_target(path.clone(), span)
}

/// Diagnostic when a virtual package exists but the requested symbol is not found.
pub(super) fn missing_package_symbol(
    symbol: StringId,
    package_path: StringId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::missing_package_symbol(symbol, package_path, span)
}

/// Diagnostic when a module has no public export and an external consumer tries to bind from it.
pub(super) fn missing_module_root_public_surface(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::missing_module_root_public_surface(symbol_path.clone(), span)
}

/// Diagnostic when a dependency targets a symbol in another module root that is not exported by that module's public export.
pub(super) fn cross_module_dependency_not_exported(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::cross_module_import_not_exported(symbol_path.clone(), span)
}
