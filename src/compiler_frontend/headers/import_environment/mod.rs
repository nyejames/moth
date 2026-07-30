//! Header-stage import environment construction.
//!
//! WHAT: resolves parsed imports, aliases, public export boundaries, and external symbols into
//! file-local visibility maps.
//! WHY: dependency sorting and AST need stable per-file visibility without rebuilding import
//! semantics in later stages.
//! MUST NOT: parse executable bodies, fold constants, or perform AST semantic validation.

mod bindings;
mod builder;
mod diagnostics;
mod external_imports;
mod namespace_imports;
mod provider_imports;
mod public_export_resolution;
mod receiver_imports;
mod source_imports;
mod target_resolution;
mod visible_names;

pub(crate) use bindings::{
    FileVisibility, HeaderImportEnvironment, ImportedFunctionContract, NamespaceMemberLookup,
    NamespaceRecord, NamespaceRecordSource, NamespaceTypeMember, NamespaceValueMember,
    ReceiverMethodVisibility, SourceDeclarationTarget, SourceFunctionTarget,
    lookup_namespace_member,
};
pub(crate) use public_export_resolution::{
    ModuleBoundaryCheckInput, PublicExportLookupResult, PublicExportResolutionInput,
    PublicExportSurfaceType, SourcePackageBoundaryCheckInput, check_module_boundary,
    check_source_package_boundary, resolve_public_export_boundary,
};

pub(crate) use target_resolution::{
    ExternalPackageSymbolLookup, ExternalPackageSymbolResolutionInput, ImportTargetResolutionInput,
    NamespaceTargetResolutionInput, ResolvedImportTarget, ResolvedNamespaceTarget,
    SourceImportAccess, has_explicit_moth_extension, resolve_external_package_symbol,
    resolve_import_target, resolve_namespace_target,
};
pub(crate) use visible_names::{VisibleNameBinding, VisibleNameRegistry, check_alias_case_warning};

pub(crate) use builder::ImportEnvironmentBuilder;

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::public_interface::SourceProviderImportSet;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Input bundle for preparing the module-wide import environment.
///
/// WHY: replaces the long parameter list of the old AST-side import resolver with one named struct.
pub(crate) struct ImportEnvironmentInput<'a> {
    pub(crate) module_symbols: &'a mut ModuleSymbols,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) external_import_resolution_table: &'a ExternalImportResolutionTable,
    pub(crate) source_provider_imports: &'a SourceProviderImportSet<'a>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Build the header-stage import environment for all parsed source files.
///
/// WHAT: builds per-file visibility maps by registering same-file declarations, prelude/builtin
/// names, and resolved imports.
/// WHY: this is the single entry point that AST will call to receive prepared visibility.
/// BOUNDARY: returns `CompilerMessages` because this is a true build boundary that carries the
/// shared `StringTable` needed for rendering and downstream transport. Inner helpers use
/// `Result<..., CompilerDiagnostic>` to avoid repeated `StringTable` cloning; conversion happens
/// only at this top-level boundary.
pub(crate) fn prepare_import_environment(
    input: ImportEnvironmentInput<'_>,
) -> Result<HeaderImportEnvironment, CompilerMessages> {
    input
        .source_provider_imports
        .validate_binding_targets(input.external_package_registry)
        .map_err(|error| CompilerMessages::from_error_ref(error, input.string_table))?;

    let importable_symbol_paths = input.module_symbols.importable_source_symbol_paths.clone();

    let mut builder = ImportEnvironmentBuilder {
        module_symbols: input.module_symbols,
        external_package_registry: input.external_package_registry,
        external_import_resolution_table: input.external_import_resolution_table,
        source_provider_imports: input.source_provider_imports,
        string_table: input.string_table,
        environment: HeaderImportEnvironment::default(),
        warnings: Vec::new(),
    };

    for source_file in input.module_symbols.module_file_paths.clone() {
        if let Err(boxed_diagnostic) =
            builder.build_file_visibility(&source_file, &importable_symbol_paths)
        {
            return Err(CompilerMessages::from_diagnostic(
                *boxed_diagnostic,
                builder.string_table.clone(),
            ));
        }
    }

    // CRITICAL: propagate collected warnings into the environment so downstream stages see them.
    builder.environment.warnings = builder.warnings;
    Ok(builder.environment)
}
