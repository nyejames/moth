//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: separating the two flows keeps each path readable as orchestration over named steps.

use crate::build_system::build::{
    CompiledModuleArtifact, ModuleSemanticDraft, ProjectFrontendCompilation,
};

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::{SourceProviderImport, SourceProviderImportSet};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::source_packages::root_file::file_name_is_hash_root_file;
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::frontend_orchestration::{
    FrontendModuleBuildContext, ModuleCompilationOutcome, ModulePreparationContext,
    SourceProviderMaterialisationSet, module_timing_label, record_module_input_counters,
};
use super::generated_worklist::BoundaryGeneratedFunctionStore;
use super::module_identity::ModuleId;
use super::module_inventory;
use super::module_namespace::DirectoryImportResolution;
use super::prepared_module::PreparedModule;
use super::project_module_graph::ProjectModuleGraph;
use super::project_roots;
use super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::provider_store::{ModuleProviderStore, ProviderSlot};
use super::source_discovery;
use super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageImport};
use super::source_package_discovery::build_source_package_boundary_indexes;
use super::source_tree_index::SourceTreeIndex;

/// Record a Stage 0 build-system timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  single-file and directory Stage 0 flows use dotted `stage0.*` metric names
///      through the concise `timers` substrate. The start token is zero-sized when
///      `timers` is off, so regular builds do not read clocks for instrumentation-only
///      measurements.
fn log_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

// -------------------------
//  Single-File Compilation
// -------------------------

/// Compile a single `.moth` file as its own module.
pub(crate) fn compile_single_file_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    // 1. Verify standard Moth file extension.
    //
    // A non-UTF-8 extension is an unrepresentable filesystem input. Reject it before
    // any lossy conversion can collapse it into the empty extension.
    let extension_text = match extension.to_str() {
        Some(text) => text,
        None => {
            let error = CompilerError::file_error(
                &config.entry_dir,
                "Entry file extension is not valid UTF-8".to_owned(),
                string_table,
            );
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    if extension_text != LANGUAGE_SOURCE_EXTENSION {
        if SourceFileKind::from_extension(extension_text).is_some() {
            let interned_path =
                match InternedPath::try_from_filesystem_path(&config.entry_dir, string_table) {
                    Ok(path) => path,
                    Err(non_utf8) => {
                        return Err(non_utf8_filesystem_name_error(
                            &non_utf8.path,
                            "single-file entry path",
                            string_table,
                        ));
                    }
                };
            let extension = string_table.intern(extension_text);
            let location = SourceLocation {
                scope: interned_path.clone(),
                ..Default::default()
            };
            let diagnostic =
                CompilerDiagnostic::invalid_source_file_entry(interned_path, extension, location);

            return Err(CompilerMessages::from_diagnostic(
                diagnostic,
                string_table.clone(),
            ));
        }

        let err = CompilerError::file_error(
            &config.entry_dir,
            format!(
                "Unsupported file extension for compilation. Moth files use .{LANGUAGE_SOURCE_EXTENSION}"
            ),
            string_table,
        );

        return Err(CompilerMessages::from_error_ref(err, string_table));
    }

    let total_start = crate::timing::start_pipeline_timing();

    // 2. Resolve canonical entry path.
    let entry_canonicalize_start = crate::timing::start_pipeline_timing();
    let entry_path = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to resolve entry file path: {error}"),
                string_table,
            );

            log_stage_timing(
                "stage0.single_file.entry_canonicalize",
                entry_canonicalize_start,
            );
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };
    log_stage_timing(
        "stage0.single_file.entry_canonicalize",
        entry_canonicalize_start,
    );

    let source_root = entry_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // 3. Initialize path resolver for imports.
    let path_resolver_start = crate::timing::start_pipeline_timing();
    // Build one independent source-package boundary index per registered package. The traversal
    // owns direct root discovery and sibling collision checks, so the resolver view is derived
    // from indexed facts and no separate package-root or package-tree scan remains.
    let prepared_source_package_roots = match build_source_package_boundary_indexes(
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
        string_table,
    ) {
        Ok(indexes) => indexes.prepared_source_package_roots(),
        Err(messages) => {
            log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    let entry_file_name = match entry_path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            let messages =
                non_utf8_filesystem_name_error(&entry_path, "single-file entry name", string_table);
            log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    let module_roots = if file_name_is_hash_root_file(entry_file_name) {
        match SourceTreeIndex::bounded_module_roots_for_single_file(
            &entry_path,
            config,
            &builder_surface.source_packages,
            &builder_surface.source_file_kinds,
            &builder_surface.external_import_providers,
            string_table,
        ) {
            Ok(module_roots) => module_roots,
            Err(messages) => {
                log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
                log_stage_timing("stage0.single_file.total", total_start);
                return Err(messages);
            }
        }
    } else {
        crate::compiler_frontend::paths::module_roots::ModuleRootTable::empty()
    };
    let project_path_resolver = match ProjectPathResolver::new_with_module_roots(
        source_root.clone(),
        source_root.clone(),
        prepared_source_package_roots,
        &builder_surface.source_file_kinds,
        module_roots,
    ) {
        Ok(resolver) => resolver,
        Err(error) => {
            log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);

    // 4. Discover all transitively reachable files.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    let reachable_files_start = crate::timing::start_pipeline_timing();
    let input_files = match source_discovery::collect_reachable_input_files(
        &entry_path,
        &project_path_resolver,
        style_directives,
        &mut external_imports,
        string_table,
    ) {
        Ok(collected) => collected.input_files,
        Err(messages) => {
            log_stage_timing("stage0.single_file.reachable_files", reachable_files_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };
    log_stage_timing("stage0.single_file.reachable_files", reachable_files_start);

    // Share the effective external package registry immutably for the rest of the frontend
    // pipeline so each stage does not need its own deep clone.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 5. Run the module compilation pipeline with a local string-table delta.
    add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, 1);

    let string_table_fork_start = crate::timing::start_pipeline_timing();
    let string_table_fork = string_table.fork_for_module();
    let (local_table, base_len) = string_table_fork.into_parts();
    log_stage_timing(
        "stage0.single_file.string_table_fork",
        string_table_fork_start,
    );

    let compile_module_start = crate::timing::start_pipeline_timing();

    // Record module-input counters and the per-module timing label before preparation so the
    // frontend module total can be attributed even when preparation fails.
    let source_byte_count = record_module_input_counters(&input_files);
    let module_label_text = module_timing_label(&entry_path, input_files.len(), source_byte_count);
    let module_label: Option<&str> = Some(&module_label_text);

    // Record the total frontend time for this module (success or error).
    let module_total_start = crate::timing::start_pipeline_timing();

    // Single-file compilation is a separate synthetic-module mode: it builds one deterministic
    // normal-module origin from the configured project identity, the empty logical module path
    // and `ModuleRootRole::Normal`. The empty path is the entry-root spelling and is always valid,
    // so construction failure is a proven internal invariant surfaced through the existing
    // `CompilerError`/`CompilerMessages` lane rather than a panic. The origin travels through
    // preparation into semantic compilation so the single-file module receives the same canonical
    // identity contract as a directory-discovered module.
    let stable_origin = match StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local(&config.project_name),
        Path::new(""),
        ModuleRootRole::Normal,
    ) {
        Ok(origin) => origin,
        Err(error) => {
            crate::timing::record_started_pipeline_timing_with_label(
                "frontend.module.total",
                module_total_start,
                module_label,
            );
            log_stage_timing("stage0.single_file.compile_module", compile_module_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    // Preparation is provider-independent: it owns no external package registry, import
    // resolution table or builder runtime packages. Construct it before the semantic context so
    // Phase 5 can schedule provider binding between `prepare_module` and `compile_module_semantic`.
    let preparation_context = ModulePreparationContext {
        style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };

    let prepared = match preparation_context.prepare_module(
        stable_origin,
        &input_files,
        &entry_path,
        local_table,
        source_byte_count,
        module_label,
    ) {
        Ok(prepared) => prepared,
        Err(messages) => {
            crate::timing::record_started_pipeline_timing_with_label(
                "frontend.module.total",
                module_total_start,
                module_label,
            );
            log_stage_timing("stage0.single_file.compile_module", compile_module_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(messages);
        }
    };

    // Semantic compilation is provider-dependent: it binds retained `PreparedHeaderSyntax`
    // against provider interfaces, then resolves dependencies, builds AST, lowers HIR and runs
    // borrow validation.
    let source_provider_imports = SourceProviderImportSet::default();
    let source_provider_materialisations = SourceProviderMaterialisationSet::default();
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    let compile_context = FrontendModuleBuildContext {
        config,
        build_profile,
        project_path_resolver: Some(project_path_resolver),
        style_directives,
        external_packages: Arc::clone(&external_packages),
        external_import_resolution_table: &builder_surface.external_import_resolution_table,
        source_provider_imports: &source_provider_imports,
        source_provider_materialisations: &source_provider_materialisations,
        builder_runtime_packages: &builder_surface.builder_runtime_packages,
    };

    let result = match compile_context.compile_module_semantic(
        prepared,
        &entry_path,
        module_label,
        generated_store.session(),
    ) {
        Ok(ModuleCompilationOutcome::Success(compiled)) => *compiled,
        Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
            crate::timing::record_started_pipeline_timing_with_label(
                "frontend.module.total",
                module_total_start,
                module_label,
            );
            log_stage_timing("stage0.single_file.compile_module", compile_module_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(diagnostics.into_messages());
        }
        Err(error) => {
            crate::timing::record_started_pipeline_timing_with_label(
                "frontend.module.total",
                module_total_start,
                module_label,
            );
            log_stage_timing("stage0.single_file.compile_module", compile_module_start);
            log_stage_timing("stage0.single_file.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    crate::timing::record_started_pipeline_timing_with_label(
        "frontend.module.total",
        module_total_start,
        module_label,
    );
    log_stage_timing("stage0.single_file.compile_module", compile_module_start);

    // 6. Merge local results back into the global build context.
    let merge_delta_start = crate::timing::start_pipeline_timing();
    let remap = string_table.merge_delta_from(&result.string_table, base_len);
    // The internal `ModuleSemanticDraft` carries the completed local public interface for the
    // future graph consumer. The pre-provider project-compilation boundary drops it here because
    // the current three-lane `Module` does not store it.
    let ModuleSemanticDraft {
        mut module,
        mut generated_worklist_delta,
        string_table: _,
        public_interface,
    } = result;
    // The direct public interface is a semantic draft that the future graph consumer will resolve
    // into a completed provider interface. Drop it until that consumer lands.
    drop(public_interface);
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_worklist_delta.remap_string_ids(&remap);
    }
    generated_store
        .publish(generated_worklist_delta)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    log_stage_timing("stage0.single_file.merge_delta", merge_delta_start);

    log_stage_timing("stage0.single_file.total", total_start);

    Ok(ProjectFrontendCompilation::new(
        vec![module],
        generated_store.into_sidecars(),
    ))
}

// -------------------------
//  Directory Compilation
// -------------------------

struct DirectoryModuleTaskResult {
    module_id: ModuleId,
    string_table_base_len: usize,
    outcome: DirectoryModuleTaskOutcome,
}

enum DirectoryModuleTaskOutcome {
    Success(Box<ModuleSemanticDraft>),
    Diagnosed(CompilerMessages),
    Infrastructure(CompilerError),
}

struct FailedModuleCompilation {
    string_table_base_len: usize,
    messages: CompilerMessages,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BlockedProvider {
    Module(ModuleId),
    SourcePackage(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BlockedModule {
    module_id: ModuleId,
    required_provider: BlockedProvider,
}

struct DirectoryModuleCompileContext<'a> {
    config: &'a Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    external_packages: &'a Arc<ExternalPackageRegistry>,
    builder_surface: &'a BuilderSurface,
    provider_store: &'a ModuleProviderStore,
    provider_bindings: &'a [ResolvedDependencyEdge],
    source_package_imports: &'a [ResolvedSourcePackageImport],
    completed_source_packages: &'a [CompletedSourcePackage],
}

struct SourcePackageModuleInventory {
    import_prefix: String,
    root_module_id: ModuleId,
    path_resolver: ProjectPathResolver,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: Vec<ResolvedDependencyEdge>,
    source_package_imports: Vec<ResolvedSourcePackageImport>,
}

struct CompletedSourcePackage {
    import_prefix: String,
    root_module_id: ModuleId,
    outcome: GraphCompilationOutcome,
}

/// Complete result of one project or source-package graph compilation.
///
/// Successful independent artefacts remain available after another module is diagnosed. Blocked
/// modules record the direct provider that prevented semantic compilation but produce no cascade
/// diagnostic of their own.
struct GraphCompilationOutcome {
    provider_store: ModuleProviderStore,
    generated_store: BoundaryGeneratedFunctionStore,
    diagnosed: Vec<FailedModuleCompilation>,
    blocked: Vec<BlockedModule>,
}

impl CompletedSourcePackage {
    fn interface(
        &self,
    ) -> Result<&crate::compiler_frontend::public_interface::PublicSemanticInterface, CompilerError>
    {
        self.outcome
            .provider_store
            .interface(self.root_module_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Source package @{} completed without a successful facade interface",
                    self.import_prefix
                ))
            })
    }

    fn root_slot(&self) -> Result<ProviderSlot, CompilerError> {
        self.outcome.provider_store.slot(self.root_module_id)
    }
}

fn build_source_provider_imports<'a>(
    consumer_module_id: ModuleId,
    prepared: &PreparedModule,
    provider_bindings: &[ResolvedDependencyEdge],
    provider_store: &'a ModuleProviderStore,
    source_package_imports: &[ResolvedSourcePackageImport],
    completed_source_packages: &'a [CompletedSourcePackage],
) -> Result<SourceProviderImportSet<'a>, CompilerError> {
    let mut imports = Vec::new();

    for (importer_source, file_imports) in &prepared
        .prepared_header_syntax
        .module_symbols
        .file_imports_by_source
    {
        for import in file_imports {
            if let Some(binding) = provider_bindings.iter().find(|binding| {
                binding.consumer_module_id == consumer_module_id
                    && provider_binding_matches_import(
                        &binding.provider,
                        import,
                        &prepared.string_table,
                    )
            }) {
                let interface = provider_store
                    .interface(binding.provider_module_id)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "ModuleId {} started semantic binding before provider ModuleId {} published a complete interface",
                            consumer_module_id.index(),
                            binding.provider_module_id.index()
                        ))
                    })?;

                imports.push(SourceProviderImport {
                    importer_source: owned_path_components(importer_source, &prepared.string_table),
                    imported_path: owned_path_components(
                        &import.provider.path,
                        &prepared.string_table,
                    ),
                    from_grouped: import.from_grouped,
                    interface,
                });
                continue;
            }

            let Some(package_import) = source_package_imports.iter().find(|package_import| {
                package_import.consumer_module_id == consumer_module_id
                    && provider_binding_matches_import(
                        &package_import.provider,
                        import,
                        &prepared.string_table,
                    )
            }) else {
                continue;
            };
            let completed_package = completed_source_packages
                .iter()
                .find(|package| package.import_prefix == package_import.import_prefix)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "ModuleId {} started semantic binding before source package @{} completed",
                        consumer_module_id.index(),
                        package_import.import_prefix
                    ))
                })?;

            imports.push(SourceProviderImport {
                importer_source: owned_path_components(importer_source, &prepared.string_table),
                imported_path: owned_path_components(&import.provider.path, &prepared.string_table),
                from_grouped: import.from_grouped,
                interface: completed_package.interface()?,
            });
        }
    }

    Ok(SourceProviderImportSet::new(imports))
}

fn owned_path_components(path: &InternedPath, string_table: &StringTable) -> Vec<String> {
    path.as_components()
        .iter()
        .map(|component| string_table.resolve(*component).to_owned())
        .collect()
}

/// Match one Stage 0 authored provider edge to its header-normalized import shell.
///
/// Header normalization prefixes module-root-relative paths with the active module's canonical
/// namespace. Stage 0 retains the authored path, so a normalized import may have additional
/// leading components but must end with the complete authored path. Matching the full suffix
/// preserves the imported symbol and provider path and avoids the former parent-only fallback.
fn provider_binding_matches_import(
    binding: &crate::compiler_frontend::paths::const_paths::StructuralProviderReference,
    import: &crate::compiler_frontend::headers::parse_file_headers::FileImport,
    string_table: &StringTable,
) -> bool {
    if binding.from_grouped != import.from_grouped {
        return false;
    }

    let binding_components = binding.path.as_components();
    let import_components = import.provider.path.as_components();
    if binding_components.len() > import_components.len() {
        return false;
    }

    import_components[import_components.len() - binding_components.len()..]
        .iter()
        .zip(binding_components)
        .all(|(import_component, binding_component)| {
            string_table.resolve(*import_component) == string_table.resolve(*binding_component)
        })
}

impl DirectoryModuleCompileContext<'_> {
    fn compile(
        &self,
        job: module_inventory::ModuleCompilationJob,
        generated_worklist: super::generated_worklist::GeneratedFunctionWorklist,
    ) -> DirectoryModuleTaskResult {
        let module_inventory::ModuleCompilationJob {
            module_id,
            entry_point,
            string_table_base_len: base_len,
            prepared,
            ..
        } = job;

        // Record module-input counters and the per-module timing label before preparation so
        // the frontend module total can be attributed even when preparation fails.
        let module_label_text = module_timing_label(
            &entry_point,
            prepared.source_file_count,
            prepared.source_byte_count,
        );
        let module_label: Option<&str> = Some(&module_label_text);

        // Record the total frontend time for this module (success or error).
        let module_total_start = crate::timing::start_pipeline_timing();

        // Semantic compilation is provider-dependent: it binds retained `PreparedHeaderSyntax`
        // against provider interfaces, then resolves dependencies, builds AST, lowers HIR and
        // runs borrow validation.
        let source_provider_imports = match build_source_provider_imports(
            module_id,
            &prepared,
            self.provider_bindings,
            self.provider_store,
            self.source_package_imports,
            self.completed_source_packages,
        ) {
            Ok(imports) => imports,
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        };
        let compile_context = FrontendModuleBuildContext {
            config: self.config,
            build_profile: self.build_profile,
            project_path_resolver: Some(self.project_path_resolver.clone()),
            style_directives: self.style_directives,
            external_packages: Arc::clone(self.external_packages),
            external_import_resolution_table: &self
                .builder_surface
                .external_import_resolution_table,
            source_provider_imports: &source_provider_imports,
            source_provider_materialisations: &SourceProviderMaterialisationSet::new(
                self.provider_store
                    .materialisation_contexts()
                    .chain(self.completed_source_packages.iter().flat_map(|package| {
                        package.outcome.provider_store.materialisation_contexts()
                    }))
                    .collect(),
            ),
            builder_runtime_packages: &self.builder_surface.builder_runtime_packages,
        };

        // Package the typed semantic result into the build/render-boundary `CompilerMessages` the
        // directory aggregation already consumes. The semantic boundary's classification
        // (`ModuleDiagnostics::from_messages`) is not re-run here or by the aggregation. A
        // `Diagnosed` module becomes its `CompilerMessages` inverse through `into_messages`, which
        // carries the module-local `StringTable` directly. An infrastructure `CompilerError`
        // carries its own attached render-identity context (the module-local `StringTable` that
        // issued its location), so `from_error` merges that context into a fresh module-local fork
        // used as the merge target and remaps the location exactly once. The fresh fork only
        // supplies the shared base prefix the aggregation's `merge_delta_from` expects; the
        // error's attached context supplies the post-base path strings, so the location table is
        // preserved instead of reconstructed lossily.
        let outcome = match compile_context.compile_module_semantic(
            prepared,
            &entry_point,
            module_label,
            generated_worklist,
        ) {
            Ok(ModuleCompilationOutcome::Success(compiled)) => {
                DirectoryModuleTaskOutcome::Success(compiled)
            }
            Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics.into_messages())
            }
            Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
        };
        crate::timing::record_started_pipeline_timing_with_label(
            "frontend.module.total",
            module_total_start,
            module_label,
        );

        DirectoryModuleTaskResult {
            module_id,
            string_table_base_len: base_len,
            outcome,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_module_waves(
    config: &Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_packages: &Arc<ExternalPackageRegistry>,
    builder_surface: &BuilderSurface,
    graph: &ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_imports: &[ResolvedSourcePackageImport],
    completed_source_packages: &[CompletedSourcePackage],
    string_table: &mut StringTable,
) -> Result<GraphCompilationOutcome, CompilerMessages> {
    let mut provider_store = ModuleProviderStore::new(graph.nodes().len());
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    for package in completed_source_packages {
        generated_store
            .import_completed_summaries(&package.outcome.generated_store)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    let mut diagnosed = Vec::new();
    let mut blocked = Vec::new();

    for wave in module_waves {
        add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, wave.len());
        let mut ready = Vec::new();
        for job in wave {
            let mut blocked_provider = None;
            for provider_id in graph
                .dependency_providers(job.module_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
            {
                match provider_store
                    .slot(*provider_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                {
                    ProviderSlot::Successful(_) => {}
                    ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                        blocked_provider = Some(BlockedProvider::Module(*provider_id));
                        break;
                    }
                    ProviderSlot::Unavailable => {
                        let error = CompilerError::compiler_error(format!(
                            "ModuleId {} became ready before provider ModuleId {} completed",
                            job.module_id.index(),
                            provider_id.index()
                        ));
                        return Err(CompilerMessages::from_error_ref(error, string_table));
                    }
                }
            }

            if blocked_provider.is_none() {
                for package_import in source_package_imports
                    .iter()
                    .filter(|source_import| source_import.consumer_module_id == job.module_id)
                {
                    let package = completed_source_packages
                        .iter()
                        .find(|package| package.import_prefix == package_import.import_prefix)
                        .ok_or_else(|| {
                            CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(format!(
                                    "ModuleId {} became ready before source package @{} completed",
                                    job.module_id.index(),
                                    package_import.import_prefix
                                )),
                                string_table,
                            )
                        })?;

                    match package
                        .root_slot()
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                    {
                        ProviderSlot::Successful(_) => {}
                        ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                            blocked_provider = Some(BlockedProvider::SourcePackage(
                                package.import_prefix.clone(),
                            ));
                            break;
                        }
                        ProviderSlot::Unavailable => {
                            let error = CompilerError::compiler_error(format!(
                                "ModuleId {} became ready before source package @{} completed its facade",
                                job.module_id.index(),
                                package.import_prefix
                            ));
                            return Err(CompilerMessages::from_error_ref(error, string_table));
                        }
                    }
                }
            }

            if let Some(required_provider) = blocked_provider {
                provider_store
                    .mark_blocked(job.module_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                blocked.push(BlockedModule {
                    module_id: job.module_id,
                    required_provider,
                });
            } else {
                ready.push(job);
            }
        }

        ready.sort_by_key(|job| job.module_id.index());
        for job in ready {
            // The boundary worklist publishes each successful module transaction before the
            // next ModuleId so duplicate requests in one ready wave materialise exactly once.
            // File preparation remains parallel inside each module. Module-wave parallelism can
            // return once worklist sessions can commit deterministic deltas concurrently.
            let outcome = {
                let compile_context = DirectoryModuleCompileContext {
                    config,
                    build_profile,
                    project_path_resolver,
                    style_directives,
                    external_packages,
                    builder_surface,
                    provider_store: &provider_store,
                    provider_bindings,
                    source_package_imports,
                    completed_source_packages,
                };
                compile_context.compile(job, generated_store.session())
            };
            match outcome.outcome {
                DirectoryModuleTaskOutcome::Success(compiled) => {
                    let compiled = *compiled;
                    let remap = string_table
                        .merge_delta_from(&compiled.string_table, outcome.string_table_base_len);
                    let ModuleSemanticDraft {
                        mut module,
                        mut generated_worklist_delta,
                        string_table: _,
                        public_interface,
                    } = compiled;
                    if !remap.is_identity() {
                        module.remap_string_ids(&remap);
                        generated_worklist_delta.remap_string_ids(&remap);
                    }
                    let artifact = CompiledModuleArtifact {
                        module,
                        interface: public_interface,
                    };
                    provider_store
                        .publish_success(outcome.module_id, artifact)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    generated_store
                        .publish(generated_worklist_delta)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                }
                DirectoryModuleTaskOutcome::Diagnosed(messages) => {
                    provider_store
                        .mark_diagnosed(outcome.module_id)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    diagnosed.push(FailedModuleCompilation {
                        string_table_base_len: outcome.string_table_base_len,
                        messages,
                    });
                }
                DirectoryModuleTaskOutcome::Infrastructure(error) => {
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
            }
        }
    }

    Ok(GraphCompilationOutcome {
        provider_store,
        generated_store,
        diagnosed,
        blocked,
    })
}

fn aggregate_directory_failures(
    failures: Vec<FailedModuleCompilation>,
    string_table: &mut StringTable,
) -> CompilerMessages {
    let mut aggregated_messages = CompilerMessages::empty(string_table.clone());

    for mut failure in failures {
        let remap = string_table.merge_delta_from(
            &failure.messages.string_table,
            failure.string_table_base_len,
        );

        if !remap.is_identity() {
            failure.messages.remap_string_ids(&remap);
        }

        aggregated_messages.append_messages_preserving_context(failure.messages);
    }

    aggregated_messages.string_table = string_table.clone();
    aggregated_messages
}

fn order_source_package_inventories(
    inventories: Vec<SourcePackageModuleInventory>,
    string_table: &StringTable,
) -> Result<Vec<SourcePackageModuleInventory>, CompilerMessages> {
    let known_prefixes = inventories
        .iter()
        .map(|inventory| inventory.import_prefix.clone())
        .collect::<BTreeSet<_>>();
    let mut remaining = inventories.into_iter().map(Some).collect::<Vec<_>>();
    let mut completed_prefixes = BTreeSet::new();
    let mut ordered = Vec::with_capacity(remaining.len());

    while ordered.len() < remaining.len() {
        let mut next_index = None;

        for (index, inventory) in remaining.iter().enumerate() {
            let Some(inventory) = inventory else {
                continue;
            };
            let dependencies = inventory
                .source_package_imports
                .iter()
                .map(|dependency| dependency.import_prefix.as_str())
                .collect::<BTreeSet<_>>();

            if let Some(unknown) = dependencies
                .iter()
                .find(|dependency| !known_prefixes.contains(**dependency))
            {
                let error = CompilerError::compiler_error(format!(
                    "Source package @{} depends on unindexed source package @{}",
                    inventory.import_prefix, unknown
                ));
                return Err(CompilerMessages::from_error_ref(error, string_table));
            }

            if dependencies
                .iter()
                .all(|dependency| completed_prefixes.contains(*dependency))
            {
                next_index = Some(index);
                break;
            }
        }

        let Some(next_index) = next_index else {
            let blocked = remaining
                .iter()
                .flatten()
                .map(|inventory| format!("@{}", inventory.import_prefix))
                .collect::<Vec<_>>()
                .join(", ");
            let error = CompilerError::compiler_error(format!(
                "Source package dependency cycle detected; no package is ready among {blocked}"
            ));
            return Err(CompilerMessages::from_error_ref(error, string_table));
        };

        let inventory = remaining[next_index]
            .take()
            .expect("selected source-package inventory is present");
        completed_prefixes.insert(inventory.import_prefix.clone());
        ordered.push(inventory);
    }

    Ok(ordered)
}

/// Discover all entry modules in a directory project and compile each one.
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();

    // 1. Setup path resolution based on config settings.
    let path_resolver_start = crate::timing::start_pipeline_timing();
    let mut project_setup = match project_roots::build_project_path_resolver_with_index(
        config,
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
        &builder_surface.binding_packages,
        string_table,
    ) {
        Ok(resolver) => resolver,
        Err(error) => {
            log_stage_timing("stage0.directory.path_resolver", path_resolver_start);
            log_stage_timing("stage0.directory.total", total_start);
            return Err(error);
        }
    };
    log_stage_timing("stage0.directory.path_resolver", path_resolver_start);
    let project_path_resolver = project_setup.resolver;

    // 2. Build every source-package inventory and the project inventory before semantic
    // compilation. Provider-backed discovery may extend the binding registry, so all boundaries
    // finish that serial mutation phase before the registry becomes the immutable frontend view.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    let module_inventory_start = crate::timing::start_pipeline_timing();
    let mut source_package_inventories = Vec::new();
    for (import_prefix, package_index) in project_setup
        .module_namespace_set
        .source_package_boundaries()
    {
        let mut package_graph = ProjectModuleGraph::from_source_tree_index(package_index);
        let package_path_resolver = project_path_resolver.for_source_package_boundary(
            package_index.entry_root().to_path_buf(),
            package_index
                .module_identities()
                .derive_compilation_root_table(),
        );
        let package_resolution = DirectoryImportResolution::package(
            &project_setup.module_namespace_set,
            import_prefix,
            package_index,
        );
        let package_waves = match module_inventory::discover_all_modules_in_package(
            config,
            &package_path_resolver,
            &mut package_graph,
            style_directives,
            &mut external_imports,
            package_resolution,
            string_table,
        ) {
            Ok(module_waves) => module_waves,
            Err(messages) => {
                log_stage_timing("stage0.directory.module_inventory", module_inventory_start);
                log_stage_timing("stage0.directory.total", total_start);
                return Err(messages);
            }
        };
        let root_module_id = package_index
            .module_identities()
            .module_id_for_directory(package_index.entry_root())
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Source package @{import_prefix} has no module rooted at its indexed entry root"
                    )),
                    string_table,
                )
            })?;
        let (module_waves, provider_bindings, source_package_imports) = package_waves.into_parts();

        source_package_inventories.push(SourcePackageModuleInventory {
            import_prefix: import_prefix.to_owned(),
            root_module_id,
            path_resolver: package_path_resolver,
            graph: package_graph,
            module_waves,
            provider_bindings,
            source_package_imports,
        });
    }

    let directory_import_resolution = DirectoryImportResolution::project(
        &project_setup.module_namespace_set,
        &project_setup.source_tree_index,
    );
    let module_waves = match module_inventory::discover_all_modules_in_project(
        config,
        &project_path_resolver,
        &mut project_setup.project_module_graph,
        style_directives,
        &mut external_imports,
        directory_import_resolution,
        string_table,
    ) {
        Ok(module_waves) => module_waves,
        Err(messages) => {
            log_stage_timing("stage0.directory.module_inventory", module_inventory_start);
            log_stage_timing("stage0.directory.total", total_start);
            return Err(messages);
        }
    };
    log_stage_timing("stage0.directory.module_inventory", module_inventory_start);

    let source_package_inventories =
        order_source_package_inventories(source_package_inventories, string_table)?;

    // Share the effective external package registry immutably across all boundary compilations;
    // directory modules may compile in parallel and can safely read the same Arc.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 3. Compile source packages in package-dependency order, then compile the project against
    // their immutable facade interfaces. Each boundary owns independent dense IDs, graphs and
    // provider stores; only the stable public interface crosses into a consuming boundary.
    let module_compile_batch_start = crate::timing::start_pipeline_timing();
    let mut completed_source_packages = Vec::new();
    for inventory in source_package_inventories {
        let SourcePackageModuleInventory {
            import_prefix,
            root_module_id,
            path_resolver,
            graph,
            module_waves,
            provider_bindings,
            source_package_imports,
        } = inventory;
        let outcome = compile_module_waves(
            config,
            build_profile,
            &path_resolver,
            style_directives,
            &external_packages,
            builder_surface,
            &graph,
            module_waves,
            &provider_bindings,
            &source_package_imports,
            &completed_source_packages,
            string_table,
        )?;
        completed_source_packages.push(CompletedSourcePackage {
            import_prefix,
            root_module_id,
            outcome,
        });
    }

    let (project_module_waves, project_provider_bindings, project_source_package_imports) =
        module_waves.into_parts();
    let mut project_outcome = compile_module_waves(
        config,
        build_profile,
        &project_path_resolver,
        style_directives,
        &external_packages,
        builder_surface,
        &project_setup.project_module_graph,
        project_module_waves,
        &project_provider_bindings,
        &project_source_package_imports,
        &completed_source_packages,
        string_table,
    )?;
    log_stage_timing(
        "stage0.directory.module_compile_batch",
        module_compile_batch_start,
    );

    // Project modules remain first so existing entry assembly indexes stay project-local.
    // Source-package executable artefacts follow for stable cross-module call resolution, but
    // their facade interfaces crossed the semantic boundary before this flattening handoff.
    let mut diagnosed = Vec::new();
    let mut blocked_count = project_outcome.blocked.len();
    for package in &mut completed_source_packages {
        diagnosed.append(&mut package.outcome.diagnosed);
        blocked_count += package.outcome.blocked.len();
    }
    diagnosed.append(&mut project_outcome.diagnosed);

    if !diagnosed.is_empty() {
        return Err(aggregate_directory_failures(diagnosed, string_table));
    }
    if blocked_count != 0 {
        let error = CompilerError::compiler_error(format!(
            "Directory graph retained {blocked_count} blocked modules without a diagnosed provider"
        ));
        return Err(CompilerMessages::from_error_ref(error, string_table));
    }

    let GraphCompilationOutcome {
        provider_store: project_provider_store,
        generated_store: project_generated_store,
        diagnosed: _,
        blocked: _,
    } = project_outcome;

    let mut compiled_modules = Vec::new();
    for artifact in project_provider_store.into_artifacts() {
        compiled_modules.push(artifact.module);
    }
    let mut generated_modules = project_generated_store.into_sidecars();
    for package in completed_source_packages {
        for artifact in package.outcome.provider_store.into_artifacts() {
            let mut module = artifact.module;

            // Source-package roots participate in semantic compilation and may provide
            // executable functions, but they are never project entry candidates. Their
            // dormant root activity belongs to the package boundary and must not create
            // a page when the executable lane is joined to the project link store.
            module.metadata.root_activity = Default::default();
            compiled_modules.push(module);
        }
        generated_modules.extend(package.outcome.generated_store.into_sidecars());
    }

    log_stage_timing("stage0.directory.total", total_start);

    Ok(ProjectFrontendCompilation::new(
        compiled_modules,
        generated_modules,
    ))
}
