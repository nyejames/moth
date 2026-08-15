//! Header-stage binding environment construction.
//!
//! WHAT: resolves parsed dependency clauses, aliases, public export boundaries, and external symbols
//! into file-local visibility maps.
//! WHY: dependency sorting and AST need stable per-file visibility without rebuilding binding
//! semantics in later stages.
//! MUST NOT: parse executable bodies, fold constants, or perform AST semantic validation.

mod bindings;
mod builder;
mod diagnostics;
mod external_imports;
mod namespace_bindings;
mod provider_dependencies;
mod public_export_resolution;
mod receiver_bindings;
mod source_dependencies;
mod target_resolution;
mod visible_names;

pub(crate) use diagnostics::provider_public_surface_diagnostic;

pub(crate) use bindings::{
    FileVisibility, HeaderBindingEnvironment, ImportedFunctionContract, NamespaceMemberLookup,
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
    DependencyTargetResolutionInput, ExternalPackageSymbolLookup,
    ExternalPackageSymbolResolutionInput, NamespaceTargetResolutionInput, ResolvedDependencyTarget,
    ResolvedNamespaceTarget, SourceDependencyAccess, has_explicit_moth_extension,
    resolve_dependency_target, resolve_external_package_symbol, resolve_namespace_target,
};
pub(crate) use visible_names::{VisibleNameBinding, VisibleNameRegistry, check_alias_case_warning};

pub(crate) use builder::BindingEnvironmentBuilder;

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One binding-environment build failure.
///
/// WHAT: separates ordinary user-facing dependency diagnostics from internal successful-interface
///       invariants that must abort through the `CompilerError` lane.
/// WHY: provider publication and dense-lookup failures are trusted compiler state; degrading
///      them into source diagnostics would let malformed successful artefacts depend on dependency
///      order instead of failing closed.
#[derive(Debug)]
pub(super) enum BindingEnvironmentError {
    Diagnostic(Box<CompilerDiagnostic>),
    Internal(CompilerError),
}

impl From<Box<CompilerDiagnostic>> for BindingEnvironmentError {
    fn from(diagnostic: Box<CompilerDiagnostic>) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for BindingEnvironmentError {
    fn from(error: CompilerError) -> Self {
        Self::Internal(error)
    }
}

/// Input bundle for preparing the module-wide binding environment.
///
/// WHY: replaces the long parameter list of the old AST-side dependency resolver with one named struct.
pub(crate) struct BindingEnvironmentInput<'a> {
    pub(crate) module_symbols: &'a ModuleSymbols,
    pub(crate) external_package_registry: &'a ExternalPackageRegistry,
    pub(crate) external_dependency_resolution_table: &'a ExternalImportResolutionTable,
    pub(crate) source_provider_dependencies: &'a SourceProviderDependencySet<'a>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Build the header-stage binding environment for all parsed source files.
///
/// WHAT: builds per-file visibility maps by registering same-file declarations, prelude/builtin
/// names, and resolved dependency clauses.
/// WHY: this is the single entry point that AST will call to receive prepared visibility.
/// BOUNDARY: returns `CompilerMessages` because this is a true build boundary that carries the
/// shared `StringTable` needed for rendering and downstream transport. Inner helpers use
/// `Result<..., CompilerDiagnostic>` to avoid repeated `StringTable` cloning; conversion happens
/// only at this top-level boundary.
pub(crate) fn prepare_binding_environment(
    input: BindingEnvironmentInput<'_>,
) -> Result<HeaderBindingEnvironment, CompilerMessages> {
    input
        .source_provider_dependencies
        .validate_binding_targets(input.external_package_registry)
        .map_err(|error| CompilerMessages::from_error_ref(error, input.string_table))?;

    let mut builder = BindingEnvironmentBuilder {
        module_symbols: input.module_symbols,
        external_package_registry: input.external_package_registry,
        external_dependency_resolution_table: input.external_dependency_resolution_table,
        source_provider_dependencies: input.source_provider_dependencies,
        string_table: input.string_table,
        environment: HeaderBindingEnvironment::default(),
        warnings: Vec::new(),
        provider_semantics_registered: rustc_hash::FxHashSet::default(),
    };

    let source_files = builder
        .module_symbols
        .module_file_paths
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for source_file in source_files {
        let selection_table = builder
            .module_symbols
            .dependency_selections_by_source
            .get(&source_file)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        match builder.build_file_visibility(&source_file, selection_table) {
            Ok(()) => {}
            Err(BindingEnvironmentError::Diagnostic(diagnostic)) => {
                return Err(CompilerMessages::from_diagnostic(
                    *diagnostic,
                    builder.string_table.clone(),
                ));
            }
            Err(BindingEnvironmentError::Internal(error)) => {
                return Err(CompilerMessages::from_error_ref(
                    error,
                    builder.string_table,
                ));
            }
        }
    }

    // CRITICAL: propagate collected warnings into the environment so downstream stages see them.
    builder.environment.warnings = builder.warnings;
    Ok(builder.environment)
}
