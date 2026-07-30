//! Build-owned external import resolution table.
//!
//! WHAT: maps `(importing source file logical path, provider-backed import path prefix)` to the
//!       provider resolution result so header import preparation can turn provider-backed imports
//!       into typed external symbols without re-parsing external files.
//! WHY: Stage 0 resolves provider-backed imports during reachable-file discovery; the header stage
//!      needs the same results to build namespace records and grouped import bindings.
//!
//! The table keeps three internal structures:
//! - `entries`: primary lookup from `(source_file, import_prefix)` to `package_id`;
//! - `source_file_index`: secondary index from source file to the import prefixes it uses,
//!   so module-level collection does not scan the entire entry set;
//! - `resolved_by_package_id`: stores one canonical `ResolvedExternalImport` per unique package,
//!   avoiding duplicate storage of the large struct when multiple prefixes or files target the
//!   same package.

use super::provider::ResolvedExternalImport;
use crate::compiler_frontend::external_packages::ExternalPackageId;
use std::collections::{HashMap, HashSet};

/// Build-owned table that records which external packages were created for each provider-backed
/// import prefix in each source file.
///
/// WHAT: keyed by `(source_file_logical_path, import_prefix_portable_string)` so relative imports
///       from different directories resolve to distinct canonical files and distinct packages.
/// WHY: two files in different directories may both import `@./helper.js`, but they resolve to
///      different canonical filesystem paths and therefore different provider-created packages.
#[derive(Clone, Debug, Default)]
pub struct ExternalImportResolutionTable {
    entries: HashMap<(String, String), ExternalPackageId>,
    source_file_index: HashMap<String, HashSet<String>>,
    resolved_by_package_id: HashMap<ExternalPackageId, ResolvedExternalImport>,
}

impl ExternalImportResolutionTable {
    /// Creates an empty resolution table.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            source_file_index: HashMap::new(),
            resolved_by_package_id: HashMap::new(),
        }
    }

    /// Records a provider resolution result for one source file and import prefix.
    pub fn insert(
        &mut self,
        source_file_logical_path: impl Into<String>,
        import_prefix: impl Into<String>,
        result: ResolvedExternalImport,
    ) {
        let source_file = source_file_logical_path.into();
        let prefix = import_prefix.into();
        let package_id = result.package_id;

        self.entries
            .insert((source_file.clone(), prefix.clone()), package_id);
        self.source_file_index
            .entry(source_file)
            .or_default()
            .insert(prefix);
        self.resolved_by_package_id.insert(package_id, result);
    }

    /// Looks up a previously resolved provider result.
    pub fn get(
        &self,
        source_file_logical_path: &str,
        import_prefix: &str,
    ) -> Option<&ResolvedExternalImport> {
        let package_id = self.entries.get(&(
            source_file_logical_path.to_owned(),
            import_prefix.to_owned(),
        ))?;
        self.resolved_by_package_id.get(package_id)
    }

    /// Looks up the runtime metadata retained for one provider-created package.
    ///
    /// Generated sidecars no longer have an authored source-file set from which to repeat the
    /// source-index query. Their HIR already records exact external function identities, so link
    /// planning resolves those functions to package IDs and joins the corresponding provider
    /// metadata through this build-owned index.
    pub(crate) fn get_by_package_id(
        &self,
        package_id: ExternalPackageId,
    ) -> Option<&ResolvedExternalImport> {
        self.resolved_by_package_id.get(&package_id)
    }

    /// Collects unique resolved external imports for a set of source file logical paths.
    ///
    /// WHAT: looks up all resolution entries whose source file matches one of the provided logical
    ///       paths, then deduplicates by package ID so repeated imports of the same provider result
    ///       appear only once.
    /// WHY: module compilation needs a flat list of external imports used by that module without
    ///      carrying the full per-source-file keyed table.
    ///
    /// This method uses the source-file secondary index instead of scanning the full entry set,
    /// and clones `ResolvedExternalImport` values only once into the final returned vector.
    pub fn collect_unique_resolved_imports_for_source_files(
        &self,
        source_file_logical_paths: &[String],
    ) -> Vec<ResolvedExternalImport> {
        let mut seen = HashSet::new();
        let mut resolved_refs = Vec::new();

        for source_file in source_file_logical_paths {
            if let Some(import_prefixes) = self.source_file_index.get(source_file) {
                for import_prefix in import_prefixes {
                    let entry_key = (source_file.clone(), import_prefix.clone());
                    let Some(package_id) = self.entries.get(&entry_key) else {
                        continue;
                    };

                    if seen.insert(*package_id)
                        && let Some(resolved) = self.resolved_by_package_id.get(package_id)
                    {
                        resolved_refs.push(resolved);
                    }
                }
            }
        }

        // Deterministic ordering by package ID for stable backend behavior.
        resolved_refs.sort_by_key(|resolved| resolved.package_id.0);
        resolved_refs.into_iter().cloned().collect()
    }
}
