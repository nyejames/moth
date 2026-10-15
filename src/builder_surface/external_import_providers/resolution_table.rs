//! Build-owned external import resolution table.
//!
//! Provider preparation looks up an import prefix in its importing source's namespace. Runtime
//! inventory borrows the same canonical package records through that source index, without copying
//! exported symbols or diagnostics. Package deduplication and compatibility belong to the frontend
//! runtime-import projection, not this availability table.

use super::provider::ResolvedExternalImport;
use crate::compiler_frontend::external_packages::ExternalPackageId;
use std::collections::HashMap;

/// Provider import prefixes are scoped to their importing source and owning module root.
#[derive(Clone, Debug, Default)]
pub struct ExternalImportResolutionTable {
    entries_by_source_file: HashMap<String, HashMap<String, ExternalPackageId>>,
    resolved_by_package_id: HashMap<ExternalPackageId, ResolvedExternalImport>,
}

impl ExternalImportResolutionTable {
    /// Creates an empty resolution table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a provider resolution result for one source file and import prefix.
    pub fn insert(
        &mut self,
        source_file_logical_path: impl Into<String>,
        import_prefix: impl Into<String>,
        result: ResolvedExternalImport,
    ) {
        let package_id = result.package_id;
        self.entries_by_source_file
            .entry(source_file_logical_path.into())
            .or_default()
            .insert(import_prefix.into(), package_id);
        self.resolved_by_package_id.insert(package_id, result);
    }

    /// Looks up a previously resolved provider result.
    pub fn get(
        &self,
        source_file_logical_path: &str,
        import_prefix: &str,
    ) -> Option<&ResolvedExternalImport> {
        let package_id = self
            .entries_by_source_file
            .get(source_file_logical_path)?
            .get(import_prefix)?;
        self.resolved_by_package_id.get(package_id)
    }

    /// Borrows the canonical runtime metadata for a provider-created package.
    pub(crate) fn get_by_package_id(
        &self,
        package_id: ExternalPackageId,
    ) -> Option<&ResolvedExternalImport> {
        self.resolved_by_package_id.get(&package_id)
    }

    /// Borrow source-owned availability, leaving package deduplication to the candidate owner.
    ///
    /// Several prefixes may refer to the same canonical record. The iterator allocates nothing
    /// and its incidental prefix order is not runtime publication order.
    pub(crate) fn resolved_imports_for_source_files<'a>(
        &'a self,
        source_file_logical_paths: &'a [String],
    ) -> impl Iterator<Item = &'a ResolvedExternalImport> + 'a {
        source_file_logical_paths
            .iter()
            .filter_map(|source_file| self.entries_by_source_file.get(source_file))
            .flat_map(|prefixes| prefixes.values())
            .filter_map(|package_id| self.resolved_by_package_id.get(package_id))
    }
}
