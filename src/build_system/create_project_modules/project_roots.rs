//! Project root and path-resolver setup for Stage 0.
//!
//! WHAT: interprets config paths, canonicalizes the project/entry roots, wires source-backed package
//! discovery, constructs the shared `ProjectPathResolver`, and builds the canonical
//! `ProjectModuleGraph` that owns entry classification and compile-wave scheduling for the rest
//! of Stage 0.
//! WHY: config path interpretation is build-system input preparation, while the frontend path
//! resolver should focus on resolving already-established project roots. The graph is built once
//! from the single source-tree traversal so entry classification and dependency ordering have one
//! structural owner instead of a parallel entry-candidate path.

use crate::build_system::output::ValidatedDirectoryOutputSettings;
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::{SourceFileKindRegistry, SourcePackageRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;

use std::fs;
use std::path::PathBuf;

use super::module_namespace::ModuleNamespaceSet;
use super::project_module_graph::ProjectModuleGraph;
use super::project_structure_diagnostics::{config_diagnostic_messages, path_id};
use super::source_package_discovery::{
    build_source_package_boundary_indexes, discover_project_local_source_packages,
    merge_source_packages,
};
use super::source_tree_index::{SourceTreeIndex, SourceTreeProjectContext};

/// Canonical roots used to construct project-aware path resolution.
pub(super) struct ProjectRootResolution {
    pub(super) project_root: PathBuf,
    pub(super) entry_root: PathBuf,
}

/// Canonical directory-project roots plus the canonical project module graph built from the
/// single Stage 0 source-tree traversal.
///
/// The `SourceTreeIndex` is retained beside the graph so later Stage 0 code can resolve `SourceId`s
/// through the central source record table. The graph is the structural owner of entry
/// classification and compile-wave scheduling; the index remains the sole source inventory/ownership
/// owner, so later Stage 0 steps do not keep a parallel entry-candidate or owned-source path.
pub(super) struct ProjectPathResolverSetup {
    pub(super) resolver: ProjectPathResolver,
    pub(super) source_tree_index: SourceTreeIndex,
    pub(super) project_module_graph: ProjectModuleGraph,
    pub(super) module_namespace_set: ModuleNamespaceSet,
}

/// Build only the resolver for callers that don't need the directory module inventory.
#[cfg(test)]
pub(super) fn build_project_path_resolver(
    config: &Config,
    builder_source_packages: &SourcePackageRegistry,
    source_file_kinds: &SourceFileKindRegistry,
    string_table: &mut StringTable,
) -> Result<ProjectPathResolver, CompilerMessages> {
    // Test-only resolver construction defaults to no external import providers; tests that
    // exercise provider-owned indexing call `build_project_path_resolver_with_index` directly.
    let external_import_providers = ExternalImportProviderRegistry::default();
    let binding_packages = ExternalPackageRegistry::new();
    build_project_path_resolver_with_index(
        config,
        None,
        builder_source_packages,
        source_file_kinds,
        &external_import_providers,
        &binding_packages,
        string_table,
    )
    .map(|setup| setup.resolver)
}

/// Build the canonical path resolver for a directory project.
///
/// WHY: both `project_root` and `entry_root` must be canonicalized before path resolution; doing
/// this in one owner keeps config interpretation out of later module inventory and frontend paths.
pub(super) fn build_project_path_resolver_with_index(
    config: &Config,
    validated_output_settings: Option<&ValidatedDirectoryOutputSettings>,
    builder_source_packages: &SourcePackageRegistry,
    source_file_kinds: &SourceFileKindRegistry,
    external_import_providers: &ExternalImportProviderRegistry,
    binding_packages: &ExternalPackageRegistry,
    string_table: &mut StringTable,
) -> Result<ProjectPathResolverSetup, CompilerMessages> {
    let roots = resolve_project_roots(config, string_table)?;

    let project_local_packages =
        discover_project_local_source_packages(config, &roots.project_root, string_table)?;

    let merged_packages = merge_source_packages(
        config,
        builder_source_packages,
        &project_local_packages,
        string_table,
    )?;

    let source_package_boundary_indexes = build_source_package_boundary_indexes(
        &merged_packages,
        source_file_kinds,
        external_import_providers,
        string_table,
    )?;
    let prepared_source_package_roots =
        source_package_boundary_indexes.prepared_source_package_roots();

    let entry_root = roots.entry_root.clone();
    let source_tree_index = SourceTreeIndex::discover(
        entry_root.clone(),
        SourceTreeProjectContext {
            project_root: &roots.project_root,
            validated_output_settings,
        },
        config,
        &merged_packages,
        source_file_kinds,
        external_import_providers,
        string_table,
    )?;

    let resolver = ProjectPathResolver::new_with_module_roots(
        roots.project_root,
        entry_root.clone(),
        prepared_source_package_roots,
        source_file_kinds,
        source_tree_index.module_roots().clone(),
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // Build the canonical project module graph directly from the single source-tree traversal.
    // The graph consumes the index's identity table rather than recomputing it and reads owned
    // source data through the retained index rather than duplicating it, so the index is kept
    // beside the graph as the sole source inventory/ownership owner.
    let project_module_graph = ProjectModuleGraph::from_source_tree_index(&source_tree_index);

    let module_namespace_set = ModuleNamespaceSet::build(
        &source_tree_index,
        &project_module_graph,
        source_package_boundary_indexes,
        binding_packages,
    );

    Ok(ProjectPathResolverSetup {
        resolver,
        source_tree_index,
        project_module_graph,
        module_namespace_set,
    })
}

/// Resolve the directory configured as the project entry root.
pub(crate) fn resolve_project_entry_root(config: &Config) -> PathBuf {
    if config.entry_root.as_os_str().is_empty() {
        return config.entry_dir.clone();
    }

    if config.entry_root.is_absolute() {
        config.entry_root.clone()
    } else {
        config.entry_dir.join(&config.entry_root)
    }
}

fn resolve_project_roots(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<ProjectRootResolution, CompilerMessages> {
    let project_root = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to canonicalize project root: {error}"),
                string_table,
            );

            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };

    let entry_root_path = resolve_project_entry_root(config);
    if !entry_root_path.exists() {
        return Err(config_diagnostic_messages(
            config,
            "entry_root",
            InvalidConfigReason::ConfiguredEntryRootMissing {
                entry_root: path_id(&entry_root_path, string_table),
            },
            string_table,
        ));
    }

    let entry_root = match fs::canonicalize(&entry_root_path) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &entry_root_path,
                format!("Failed to canonicalize configured entry root: {error}"),
                string_table,
            );

            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };

    Ok(ProjectRootResolution {
        project_root,
        entry_root,
    })
}
