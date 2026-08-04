//! External import registration.
//!
//! WHAT: registers imports that resolve to external package symbols.
//! WHY: external package imports use stable IDs rather than source paths, while receiver method
//! syntax remains source-owned and compiler-owned rather than external-package metadata.
//! MUST NOT: register source declarations or build source namespace records.

use super::ImportEnvironmentError;
use super::{FileVisibility, ImportEnvironmentBuilder, VisibleNameBinding, VisibleNameRegistry};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::parse_file_headers::FileImport;

/// Boxed diagnostic result for external import registration.
///
/// WHAT: gives external import registration one small error boundary.
/// WHY: local-name derivation is already boxed, so registration can propagate it directly
///      and adapt the plain visible-name registry once.
type ExternalImportResult<T> = Result<T, ImportEnvironmentError>;

impl<'a> ImportEnvironmentBuilder<'a> {
    pub(super) fn register_external_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        symbol_id: ExternalSymbolId,
    ) -> ExternalImportResult<()> {
        let local_name = self.derive_import_local_name(import)?;

        if let Some(symbol_name) = import.provider.path.name() {
            self.emit_alias_case_warning_if_needed(import, symbol_name);
        }

        registry.register(
            local_name,
            VisibleNameBinding::ExternalImport { symbol_id },
            Some(import.location.clone()),
        )?;

        file_visibility
            .visible_external_symbols
            .insert(local_name, symbol_id);

        file_visibility
            .visible_external_symbol_locations
            .insert(local_name, import.location.clone());

        Ok(())
    }
}
