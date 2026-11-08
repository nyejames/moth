//! Header-stage binding, public export, alias, and collision diagnostic helpers.
//!
//! WHAT: named helpers that construct structured diagnostics for binding-environment failures.
//! WHY: centralizing diagnostic construction keeps error messages consistent and makes it
//! easy to update wording or metadata across all dependency-related failures.
//! MUST NOT: contain business logic for resolution or registration.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType;
use crate::compiler_frontend::compiler_messages::RootLocalDeclarationRole;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::FileRole;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
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
) -> DiagnosticDraft {
    let primary_span = span.or(previous_span);
    let previous_span = span.and(previous_span);
    imports::import_name_collision(local_name, previous_span, primary_span)
}

/// Diagnostic when a dependency through a public surface resolves to a symbol that the surface does
/// not expose.
pub(super) fn not_exported_by_public_surface(
    dependency_path: &PathId,
    public_surface_name: StringId,
    public_surface_type: ImportPublicSurfaceType,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::not_exported_by_public_surface(
        *dependency_path,
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
) -> DiagnosticDraft {
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
) -> DiagnosticDraft {
    imports::direct_special_file_import(*path, span)
}

/// Map an active root file role to the authored root role a diagnostic names.
fn root_local_declaration_role(role: FileRole) -> RootLocalDeclarationRole {
    match role {
        // LEGACY ROOT MIGRATION BRIDGE (removed at Phase 4 cutover): until every executable
        // fixture authors `@entry.moth`, a lone normal root keeps the combined export/start role.
        FileRole::LegacyCombinedRoot => RootLocalDeclarationRole::LegacyCombinedRoot,
        FileRole::ActiveEntryRoot => RootLocalDeclarationRole::EntryRoot,
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Normal) => {
            RootLocalDeclarationRole::NormalFacade
        }
        FileRole::ActiveApiOnlyModuleRoot(ModuleRootRole::Support) => {
            RootLocalDeclarationRole::SupportFacade
        }
        // Every remaining shape is unreachable: `active_root_file_role` admits only active
        // module roots, and Stage 0 prepares `ActiveApiOnlyModuleRoot` with a normal or support
        // role. A reachable file here would silently misname the root in a user diagnostic.
        FileRole::ActiveApiOnlyModuleRoot(
            ModuleRootRole::Entry
            | ModuleRootRole::Shared
            | ModuleRootRole::SyntheticProjectContext,
        )
        | FileRole::ReachedApiFacadeRoot(_)
        | FileRole::ConfigBootstrap
        | FileRole::Normal => crate::compiler_bug!(
            CompilerStage::HeaderParsing,
            "root-local declaration diagnostic reached a file role that is not an active module root"
        ),
    }
}

/// Diagnostic when a file resolves a declaration authored in another active root file.
///
/// WHAT: returns `None` when the declaration is visible from the consumer, including every
///       ordinary helper declaration and the authoring root file itself.
/// WHY: direct selections, namespaces and re-exports must share one root-local isolation answer
///      instead of rebuilding the policy in each resolution lane.
pub(crate) fn cross_file_root_local_declaration(
    module_symbols: &ModuleSymbols,
    consumer_file: &PathId,
    target_file: &PathId,
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> Option<DiagnosticDraft> {
    if module_symbols.authored_declarations_visible_from(consumer_file, target_file) {
        return None;
    }

    let role = module_symbols.active_root_file_role(target_file)?;
    let declaration_span = module_symbols
        .declaration_spans_by_symbol_path
        .get(symbol_path)
        .copied();
    Some(imports::root_local_declaration_not_visible(
        *symbol_path,
        root_local_declaration_role(role),
        *target_file,
        span,
        declaration_span,
    ))
}

/// Diagnostic when a dependency path matches a source file but not a symbol.
pub(super) fn bare_file_dependency(path: &PathId, span: Option<SourceSpan>) -> DiagnosticDraft {
    imports::bare_file_import(*path, span)
}

/// Diagnostic when a dependency path cannot be resolved to any known source or external symbol.
pub(super) fn missing_dependency_target(
    path: &PathId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::missing_import_target(*path, span)
}

/// Diagnostic when a direct source dependency targets a symbol that is not exported.
pub(super) fn not_exported_by_source_file(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::not_exported_by_source_file(*symbol_path, span)
}

/// Diagnostic when a dependency path matches multiple source symbols ambiguously.
pub(super) fn ambiguous_dependency_target(
    path: &PathId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::ambiguous_import_target(*path, span)
}

/// Diagnostic when a virtual package exists but the requested symbol is not found.
pub(super) fn missing_package_symbol(
    symbol: StringId,
    package_path: StringId,
    candidates: Vec<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::missing_package_symbol(symbol, package_path, candidates, span)
}

/// Diagnostic when a module has no public export and an external consumer tries to bind from it.
pub(super) fn missing_module_root_public_surface(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::missing_module_root_public_surface(*symbol_path, span)
}

/// Diagnostic when a dependency targets a symbol in another module root that is not exported by that module's public export.
pub(super) fn cross_module_dependency_not_exported(
    symbol_path: &PathId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    imports::cross_module_import_not_exported(*symbol_path, span)
}
