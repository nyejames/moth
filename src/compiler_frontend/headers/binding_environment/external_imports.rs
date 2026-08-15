//! External import registration.
//!
//! WHAT: registers imports that resolve to external package symbols.
//! WHY: external package imports use stable IDs rather than source paths, while receiver method
//! syntax remains source-owned and compiler-owned rather than external-package metadata.
//! MUST NOT: register source declarations or build source namespace records.

use super::BindingEnvironmentError;
use super::{BindingEnvironmentBuilder, FileVisibility, VisibleNameBinding, VisibleNameRegistry};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyAlias;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Boxed diagnostic result for external import registration.
///
/// WHAT: gives external import registration one small error boundary.
/// WHY: local-name derivation is already boxed, so registration can propagate it directly
///      and adapt the plain visible-name registry once.
type ExternalImportResult<T> = Result<T, BindingEnvironmentError>;

/// Registration facts for one external symbol binding.
///
/// WHAT: bundles the source clause and selected-name facts needed to publish one stable external
///       symbol ID into file visibility.
/// WHY: external registration owns this data shape, keeping callers from growing repetitive
///      parameter lists as diagnostics and alias metadata travel together.
pub(super) struct ExternalImportInput<'a> {
    pub(super) symbol_name: StringId,
    pub(super) local_name: StringId,
    pub(super) source_location: &'a SourceLocation,
    pub(super) local_alias: Option<&'a DependencyAlias>,
    pub(super) symbol_id: ExternalSymbolId,
}

impl<'a> BindingEnvironmentBuilder<'a> {
    pub(super) fn register_external_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: ExternalImportInput<'_>,
    ) -> ExternalImportResult<()> {
        let ExternalImportInput {
            symbol_name,
            local_name,
            source_location,
            local_alias,
            symbol_id,
        } = input;

        self.emit_alias_case_warning_if_needed(local_alias, symbol_name);

        let local_name_location = local_alias.map_or(source_location, |alias| &alias.location);

        registry.register(
            local_name,
            VisibleNameBinding::ExternalImport { symbol_id },
            Some(local_name_location.clone()),
        )?;

        file_visibility
            .visible_external_symbols
            .insert(local_name, symbol_id);

        file_visibility
            .visible_external_symbol_locations
            .insert(local_name, local_name_location.clone());

        Ok(())
    }
}
