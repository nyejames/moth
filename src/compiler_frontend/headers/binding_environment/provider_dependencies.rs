//! Provider-backed dependency resolution helpers.
//!
//! WHAT: resolves selected and namespace dependencies against external files handled by registered dependency
//! providers (e.g., `.js` files parsed into typed external packages).
//! WHY: provider-backed dependencies bridge the Stage 0 external-file discovery path with the header
//! binding environment, turning provider results into ordinary external-package registrations.
//! MUST NOT: perform provider parsing or AST-level semantic validation.

use super::{
    BindingEnvironmentBuilder, BindingEnvironmentError, FileVisibility, VisibleNameRegistry,
};
use crate::builder_surface::external_import_providers::provider::ResolvedExternalImport;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::dependency_target::decode_dependency_target;
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::types::DependencySelection;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Result for provider-backed dependency resolution.
///
/// Provider selection preserves plain diagnosed values and typed infrastructure
/// failures in `BindingEnvironmentError` through the connected helper family.
type ProviderDependencyResult<T> = Result<T, BindingEnvironmentError>;

impl<'a> BindingEnvironmentBuilder<'a> {
    /// Try to resolve direct selections against a provider-backed external file.
    ///
    /// WHAT: the clause root identifies the provider-created external package and each direct
    /// selection is looked up within that one package surface.
    ///
    /// Returns `Ok(Some(()))` if resolved, `Ok(None)` if this dependency is not
    /// provider-backed, or `Err` for a diagnostic.
    pub(super) fn resolve_provider_backed_selection_dependencies(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        selections: &[DependencySelection],
        source_file: &PathId,
    ) -> ProviderDependencyResult<Option<()>> {
        let Some((resolved, remaining)) = self
            .find_provider_resolution_from_retained_target(source_file, &dependency.dependency)?
        else {
            return Ok(None);
        };

        // A retained provider root must match the provider prefix exactly. Any remaining path
        // component is a malformed provider identity, not a selected source name.
        if !remaining.is_empty() {
            return Err(CompilerDiagnostic::direct_symbol_path_import(
                dependency.dependency.path.clone(),
                Some(dependency.dependency.span),
            )
            .into());
        }

        let package = self
            .external_package_registry
            .get_package_by_id(resolved.package_id);
        let Some(package) = package else {
            return Err(super::diagnostics::missing_dependency_target(
                &dependency.dependency.path,
                Some(dependency.dependency.span),
            )
            .into());
        };

        for selection in selections {
            let symbol_id = self
                .lookup_external_symbol_id_by_name(&package.path, selection.source_name)
                .ok_or_else(|| {
                    super::diagnostics::missing_dependency_target(
                        &self
                            .path_fork
                            .try_intern_child(dependency.dependency.path, selection.source_name)
                            .expect("path interner fork exhausted while building provider diagnostic path"),
                        Some(selection.source_span),
                    )
                })?;
            let local_name = selection.local_name();
            self.register_external_import(
                file_visibility,
                registry,
                super::external_imports::ExternalImportInput {
                    symbol_name: selection.source_name,
                    local_name,
                    source_span: Some(selection.source_span),
                    local_alias: selection.local_alias(),
                    symbol_id,
                },
            )?;
        }

        Ok(Some(()))
    }

    /// Try to resolve a namespace dependency against a provider-backed external file.
    ///
    /// WHAT: `@helper.js` where `helper.js` has a registered provider.
    /// The dependency exposes the provider's package as a namespace record.
    ///
    /// Returns `Ok(Some(()))` if resolved, `Ok(None)` if not provider-backed,
    /// or `Err` for a diagnostic.
    pub(super) fn resolve_provider_backed_namespace_binding(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        source_file: &PathId,
    ) -> ProviderDependencyResult<Option<()>> {
        let Some((resolved, remaining)) = self
            .find_provider_resolution_from_retained_target(source_file, &dependency.dependency)?
        else {
            return Ok(None);
        };

        // If there are remaining components after the provider-backed prefix, this is a
        // direct symbol-path dependency, which is invalid for bare dependencies.
        if !remaining.is_empty() {
            return Err(CompilerDiagnostic::direct_symbol_path_import(
                dependency.dependency.path.clone(),
                Some(dependency.dependency.span),
            )
            .into());
        }

        let package = self
            .external_package_registry
            .get_package_by_id(resolved.package_id);
        let Some(package) = package else {
            return Err(super::diagnostics::missing_dependency_target(
                &dependency.dependency.path,
                Some(dependency.dependency.span),
            )
            .into());
        };

        let package_path_id = self.string_table.intern(&package.path);
        let namespace_record = self
            .build_external_namespace_record(package_path_id, Some(dependency.dependency.span))?;

        let local_name = self.derive_namespace_name(dependency)?;

        let local_name_span = match &dependency.binding {
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::Namespace {
                alias: Some(alias),
            } => Some(alias.span),
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::Namespace {
                alias: None,
            } => Some(dependency.dependency.span),
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::DirectSelections {
                ..
            } => None,
        };
        registry.register(
            local_name,
            super::VisibleNameBinding::NamespaceRecord {
                record_source: super::NamespaceRecordSource::ExternalPackage(package_path_id),
            },
            local_name_span,
        )?;

        file_visibility
            .visible_namespace_records
            .insert(local_name, namespace_record);

        Ok(Some(()))
    }

    /// Look up the Stage 0 provider resolution for the exact retained prefix.
    fn find_provider_resolution_from_retained_target(
        &self,
        source_file: &PathId,
        dependency: &crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath,
    ) -> ProviderDependencyResult<Option<(ResolvedExternalImport, Vec<StringId>)>> {
        let Some(decoded) =
            decode_dependency_target(
                dependency.path,
                &dependency.target,
                &*self.path_fork,
                self.string_table,
            )?
        else {
            return Ok(None);
        };

        let mut scratch = Vec::new();
        let source_str = self
            .path_fork
            .render_portable(*source_file, self.string_table, &mut scratch);
        let prefix_str = self
            .path_fork
            .render_portable(decoded.prefix_path_id(), self.string_table, &mut scratch);
        let Some(entry) = self
            .external_dependency_resolution_table
            .get(&source_str, &prefix_str)
        else {
            return Ok(None);
        };
        Ok(Some((
            entry.clone(),
            decoded.remaining_components().to_vec(),
        )))
    }

    /// Look up an external symbol ID by name within a provider-created package.
    fn lookup_external_symbol_id_by_name(
        &self,
        package_path: &str,
        name: StringId,
    ) -> Option<ExternalSymbolId> {
        let name_str = self.string_table.resolve(name);
        self.external_package_registry
            .resolve_package_symbol(package_path, name_str)
    }
}
