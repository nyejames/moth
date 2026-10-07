//! Single-file frontend compilation internals.
//!
//! The parent module retains the command-facing entry contracts; this child owns the synthetic
//! module discovery, preparation and semantic compilation pipeline.

use crate::compiler_frontend::paths::module_roots::ModuleRootTable;
use crate::{timing_scope, timing_scope_attributed};

#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::ModuleCompilationContext;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::compile_module_for_boracle;

use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, PremergeDiagnosticBatch, PremergeFailure, SourceSpanCapacityResource,
};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source_packages::root_file::file_name_is_normal_module_root_file;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathInternError, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::profile::NumericProfile;

use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::FrontendCompilationMode;
use super::super::compiled_boundary::ProjectFrontendCompilation;
use super::super::config_boundary;
use super::super::file_reference_resolution::{
    SingleFileReferenceOutcome, SingleFileResolvedReference,
};
#[cfg(feature = "boracle")]
use super::super::generated_store::BoundaryGeneratedFunctionStore;
#[cfg(feature = "boracle")]
use super::super::module_artifact_store::ModuleArtifactStore;
use super::super::module_inventory::ModuleCompilationJob;
use super::super::module_namespace::ModuleNamespaceSet;
use super::super::module_preparation::{
    ModulePreparationContext, record_module_input_counters, source_is_moth_template,
};
use super::super::prepared_module::PreparedModule;
use super::super::project_module_graph::ProjectModuleGraph;
use super::super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::super::resource_inputs::ResourceInputRegistry;
use super::super::source_discovery;
use super::super::source_package_discovery::build_source_package_boundary_indexes;
use super::super::source_tree_index::SourceTreeIndex;
use super::{
    BoundaryPremergeFailure, append_finish_failure, canonical, compile_source_package_inventories,
    discover_source_package_inventories, prepare_source_package_check_only_jobs,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_single_file_frontend_with_inputs(
    config: &Config,
    build_profile: FrontendBuildProfile,
    numeric_profile: NumericProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    project_source_files: &mut Option<Arc<SourceDatabase>>,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    let result = match compile_single_file_frontend_with_target(
        config,
        build_profile,
        numeric_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        project_source_files,
        build_config_inputs,
        mode,
        SingleFileFrontendTarget::Normal,
    ) {
        Ok(result) => result,
        Err(failure) => {
            if let Some(package_source) = failure.package_source {
                return Err((*failure.failure)
                    .into_messages_with_source(string_table, Arc::new(*package_source)));
            }
            let mut messages = (*failure.failure).into_messages(string_table);
            if messages.source_database_for_diagnostic(0).is_none()
                && let Some(source_database) = project_source_files.as_ref()
            {
                messages.set_source_database(Arc::clone(source_database));
            }
            return Err(messages);
        }
    };

    match result {
        SingleFileFrontendResult::Project(compilation) => Ok(*compilation),
        #[cfg(feature = "boracle")]
        SingleFileFrontendResult::Boracle(_) => Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "normal single-file compilation unexpectedly returned a Boracle payload",
            ),
            string_table,
        )),
    }
}
#[cfg(feature = "boracle")]
pub(crate) fn compile_single_file_boracle_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    numeric_profile: NumericProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<BoracleModuleInput, CompilerMessages> {
    let mut project_source_files = None;
    let result = match compile_single_file_frontend_with_target(
        config,
        build_profile,
        numeric_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        &mut project_source_files,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
        SingleFileFrontendTarget::Boracle,
    ) {
        Ok(result) => result,
        Err(failure) => {
            if let Some(package_source) = failure.package_source {
                return Err((*failure.failure)
                    .into_messages_with_source(string_table, Arc::new(*package_source)));
            }
            let mut messages = (*failure.failure).into_messages(string_table);
            if messages.source_database_for_diagnostic(0).is_none()
                && let Some(source_database) = project_source_files.as_ref()
            {
                messages.set_source_database(Arc::clone(source_database));
            }
            return Err(messages);
        }
    };

    match result {
        SingleFileFrontendResult::Boracle(input) => Ok(*input),
        SingleFileFrontendResult::Project(compilation) => {
            if !compilation.has_diagnosed_or_blocked() {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Boracle source compilation unexpectedly returned a clean project payload",
                    ),
                    string_table,
                ));
            }
            // The canonical scheduler retains failed package reports and blocks their consumer.
            // Render only after the synthetic source owner has finalised its span tables.
            let messages = compilation
                .into_render_messages_with_frozen_identity(string_table, project_source_files, None)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            Err(messages)
        }
    }
}
enum SingleFileFrontendResult {
    Project(Box<ProjectFrontendCompilation>),
    #[cfg(feature = "boracle")]
    Boracle(Box<BoracleModuleInput>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleFileFrontendTarget {
    Normal,
    #[cfg(feature = "boracle")]
    Boracle,
}

#[allow(clippy::too_many_arguments)]
fn compile_single_file_frontend_with_target(
    config: &Config,
    build_profile: FrontendBuildProfile,
    numeric_profile: NumericProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    project_source_files: &mut Option<Arc<SourceDatabase>>,
    build_config_inputs: &BuildConfigInputSet,
    // Only the synthetic consumer has no unselected sources. Packages retain ordinary Check policy.
    mode: FrontendCompilationMode,
    target: SingleFileFrontendTarget,
) -> Result<SingleFileFrontendResult, BoundaryPremergeFailure> {
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut discovery_path_fork = PathInternerFork::empty();
    // 1. Verify standard Moth file extension.
    //
    // A non-UTF-8 extension is an unrepresentable filesystem input. Reject it before
    // any lossy conversion can collapse it into the empty extension.
    let extension_text = match extension.to_str() {
        Some(text) => text,
        None => {
            return Err(CompilerError::file_error(
                &config.entry_dir,
                "Entry file extension is not valid UTF-8".to_owned(),
            )
            .into());
        }
    };
    if extension_text != LANGUAGE_SOURCE_EXTENSION {
        let interned_path = match discovery_path_fork
            .try_intern_filesystem_path(&config.entry_dir, string_table)
        {
            Ok(path) => path,
            Err(PathInternError::NonUtf8(non_utf8)) => {
                return Err(BoundaryPremergeFailure::from(
                    non_utf8_filesystem_name_error(&non_utf8.path, "single-file entry path"),
                ));
            }
            Err(PathInternError::TableFull) => {
                // Authored exhaustion of the compact path table: the typed capacity diagnostic
                // travels without a path snapshot, because the exhausted table owns no further
                // PathId to render.
                return Err(
                    PremergeFailure::Diagnosed(PremergeDiagnosticBatch::from_diagnostic(
                        CompilerDiagnostic::source_table_capacity(
                            SourceSpanCapacityResource::LogicalPath,
                        ),
                        std::mem::take(string_table),
                    ))
                    .into(),
                );
            }
            Err(PathInternError::BaseMismatch { .. }) => {
                return Err(PremergeFailure::Infrastructure(
                    CompilerError::compiler_error(
                        "logical path merge base is not a structural prefix of the destination table",
                    ),
                ).into());
            }
        };
        let extension = string_table.intern(extension_text);
        let diagnostic =
            CompilerDiagnostic::invalid_source_file_entry(interned_path, extension, None);

        // Move the local string table and path identity into the diagnosed lane. Discovery aborts
        // before a source database exists, so the path snapshot must travel with the diagnostic.
        let table = std::mem::take(string_table);
        let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, table);
        if let Err(error) =
            batch.attach_path_table_if_missing(Arc::new(discovery_path_fork.snapshot_table()))
        {
            return Err(PremergeFailure::Infrastructure(error).into());
        }
        return Err(PremergeFailure::Diagnosed(batch).into());
    }

    timing_scope!(
        timing_guard_stage0_single_file_total,
        crate::timing::TimingMetric::Stage0SingleFileTotal
    );

    // 2. Resolve canonical entry path.
    let entry_path = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            return Err(CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to resolve entry file path: {error}"),
            )
            .into());
        }
    };
    let source_root = entry_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // 3. Initialize dependency-path resolution.
    // Build one independent source-package boundary index per registered package. The traversal
    // owns direct root discovery and sibling collision checks, so the resolver view is derived
    // from indexed facts and no separate package-root or package-tree scan remains.
    let source_package_indexes = build_source_package_boundary_indexes(
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
        &builder_surface.external_import_providers,
        string_table,
    )?;
    let prepared_source_package_roots = source_package_indexes.prepared_source_package_roots();
    let entry_file_name = match entry_path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            return Err(BoundaryPremergeFailure::from(
                non_utf8_filesystem_name_error(&entry_path, "single-file entry name"),
            ));
        }
    };

    let module_roots = if file_name_is_normal_module_root_file(entry_file_name) {
        SourceTreeIndex::bounded_module_roots_for_single_file(
            &entry_path,
            config,
            &builder_surface.source_packages,
            &builder_surface.source_file_kinds,
            &builder_surface.external_import_providers,
            string_table,
        )?
    } else {
        ModuleRootTable::empty()
    };
    let project_path_resolver = ProjectPathResolver::new_with_module_roots(
        source_root.clone(),
        source_root.clone(),
        prepared_source_package_roots,
        &builder_surface.source_file_kinds,
        module_roots,
    )?;
    let namespace_set = ModuleNamespaceSet::for_single_file(
        source_package_indexes,
        &builder_surface.binding_packages,
        &project_path_resolver,
    )?;
    // 4. Discover all transitively reachable files.
    // Register the synthetic main-project boundary before its inventory so the human summary can
    // attribute reachable discovery and module compilation as accumulated boundary work.
    #[cfg(feature = "timers")]
    let timing_boundary = crate::timing::register_timing_boundary(
        crate::timing::TimingBoundaryKind::MainProject,
        || config.project_name.clone(),
    );
    timing_scope_attributed!(
        timing_guard_build_boundary_inventory,
        crate::timing::TimingMetric::BoundaryInventory,
        Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
    );
    let collection_result = {
        let mut external_imports = source_discovery::ExternalImportDiscoveryState {
            external_packages: &mut builder_surface.binding_packages,
            providers: &builder_surface.external_import_providers,
            cache: &mut builder_surface.external_import_cache,
            resolution_table: &mut builder_surface.external_dependency_resolution_table,
        };
        source_discovery::collect_reachable_input_files(
            &entry_path,
            &namespace_set,
            &project_path_resolver,
            style_directives,
            &mut external_imports,
            &builder_surface.source_file_kinds,
            &mut resource_inputs,
            &mut discovery_path_fork,
            string_table,
        )
    };
    let collected = match collection_result {
        Ok(collected) => collected,
        Err(error) => {
            let (failure, source_database) = error.into_parts();
            if let Some(source_database) = source_database {
                *project_source_files = Some(Arc::new(source_database));
            }
            return Err(failure.into());
        }
    };
    let source_package_dependencies = collected.source_package_dependencies;
    let mut source_owner = collected.source_files;
    let input_files = collected.input_files;
    let resolved_file_references = collected.resolved_file_references;
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_inventory.finish();
    // Split the discovery builder once: preparation and semantic compilation borrow the
    // immutable database beside the retained span builders, and the builder stays the exclusive
    // owner that finalizes every table after the last span producer returns.
    let (source_files, mut source_spans) = source_owner.split();
    let mut path_interner = source_files.clone_path_builder();
    // Every stage after the split runs inside one scoped pipeline. Its early returns funnel
    // through the single finalization tail below, so a diagnosed or infrastructure failure can
    // never drop the builder with live span owners or an unfinalized table.
    let result = (|| -> Result<SingleFileFrontendResult, BoundaryPremergeFailure> {
        // Stage 0 indexed package roots above for namespace and collision checks. The shared
        // inventory path materialises every registered source package when entered, so skip it
        // only when the consumer retained no source-package edges and no selected Moth template
        // can receive an implicit package from this builder.
        let requires_source_package_inventories = !source_package_dependencies.is_empty()
            || (!builder_surface
                .implicit_template_scope_source_packages
                .is_empty()
                && input_files
                    .iter()
                    .any(|input| source_is_moth_template(source_files, input.source_id())));
        let source_package_inventories = {
            if requires_source_package_inventories {
                let mut external_imports = source_discovery::ExternalImportDiscoveryState {
                    external_packages: &mut builder_surface.binding_packages,
                    providers: &builder_surface.external_import_providers,
                    cache: &mut builder_surface.external_import_cache,
                    resolution_table: &mut builder_surface.external_dependency_resolution_table,
                };
                let mut inventories = discover_source_package_inventories(
                    config,
                    numeric_profile,
                    &namespace_set,
                    &project_path_resolver,
                    style_directives,
                    &mut external_imports,
                    &mut resource_inputs,
                    string_table,
                    mode,
                )?;
                if mode.includes_check_only() {
                    prepare_source_package_check_only_jobs(
                        &mut inventories,
                        &namespace_set,
                        style_directives,
                        &mut external_imports,
                        string_table,
                    )?;
                }
                inventories
            } else {
                Vec::new()
            }
        };

        // Share the effective external package registry only after Stage 0 has finished every
        // provider-discovery operation that mutates it.
        let external_packages = Arc::new(builder_surface.binding_packages.clone());
        let (completed_source_packages, transient_batches) = compile_source_package_inventories(
            config,
            build_profile,
            numeric_profile,
            style_directives,
            builder_surface,
            &external_packages,
            source_package_inventories,
            &mut resource_inputs,
            string_table,
        )?;

        let string_table_fork = string_table.fork_for_module();
        let (local_table, base_len) = string_table_fork.into_parts();
        let path_fork = path_interner.fork_source().fork_for_module();
        let path_base_len = path_fork.base_len();

        timing_scope_attributed!(
            timing_guard_boundary_compile,
            crate::timing::TimingMetric::BoundaryCompile,
            Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
        );

        // Record module-input counters before preparation so the frontend module total can be
        // attributed even when preparation fails. A source-load failure is replayed here from the
        // final database rather than being replaced with a fabricated zero byte length.
        let source_byte_count = record_module_input_counters(&input_files, source_files)?;

        // Register the single synthetic module with its portable logical identity and source facts.
        // The empty path is this mode's fixed entry-root logical spelling, matching the origin
        // constructed below.
        #[cfg(feature = "timers")]
        let timing_module_key = crate::timing::register_timing_module(
            timing_boundary,
            0,
            "",
            input_files.len() as u64,
            source_byte_count as u64,
        );
        #[cfg(feature = "timers")]
        let timing_module_context =
            Some(crate::timing::TimingContext::for_module(timing_module_key));

        // Single-file compilation is a separate synthetic-module mode: it builds one deterministic
        // normal-module origin from the configured project identity, the empty logical module path
        // and `ModuleRootRole::Normal`. The empty path is the entry-root spelling and is always valid,
        // so construction failure is a proven internal invariant surfaced as a typed
        // `CompilerError` rather than a panic. The origin travels through
        // preparation into semantic compilation so the single-file module receives the same canonical
        // identity contract as a directory-discovered module.
        let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
            StablePackageIdentity::project_local(&config.project_name),
            Path::new(""),
            ModuleRootRole::Normal,
        )?;
        let preparation_context = ModulePreparationContext {
            source_files,
            style_directives,
        };

        let graph_stable_origin = stable_origin.clone();
        #[cfg(feature = "timers")]
        let prepare_result = preparation_context.prepare_module(
            stable_origin,
            input_files,
            &mut source_spans,
            &entry_path,
            local_table,
            path_fork,
            source_byte_count,
            timing_module_context,
        );
        #[cfg(not(feature = "timers"))]
        let prepare_result = preparation_context.prepare_module(
            stable_origin,
            input_files,
            &mut source_spans,
            &entry_path,
            local_table,
            path_fork,
            source_byte_count,
        );
        let mut prepared = prepare_result?;
        attach_single_file_resolved_references(
            &mut prepared,
            source_files,
            resolved_file_references,
            string_table,
        )?;

        let source_facts = config_boundary::source_contract_facts_from_prepared(
            &prepared,
            string_table,
            base_len,
            numeric_profile,
        )?;
        let effective_project_fields =
            config_boundary::effective_project_fields(config, string_table, numeric_profile)?;
        let fixed_project_facts =
            config_boundary::fixed_project_contract_facts(&effective_project_fields);
        let direct_project_facts = config_boundary::input_contract_facts(&effective_project_fields);
        let project_globals = config_boundary::build_project_globals_interface(
            config,
            &effective_project_fields,
            string_table,
        )?;
        let fallback_span = None;
        let build_config_values = config_boundary::resolve_boundary_build_config(
            &source_facts,
            &fixed_project_facts,
            &direct_project_facts,
            build_config_inputs,
            builder_surface.config_globals(),
            fallback_span,
            string_table,
            numeric_profile,
        )?;
        let graph = ProjectModuleGraph::from_normal_roots(vec![(
            graph_stable_origin.clone(),
            source_root,
            entry_path,
        )]);
        let module_id = graph.entry_modules()[0];
        let boundary_context = canonical::BoundaryCompilationContext::new(
            config,
            build_profile,
            numeric_profile,
            &project_path_resolver,
            Arc::clone(source_files),
            style_directives,
            &external_packages,
            builder_surface,
            &completed_source_packages,
            build_config_values,
            source_facts,
            build_config_inputs.clone(),
            builder_surface.config_globals().clone(),
            fixed_project_facts,
            direct_project_facts,
            project_globals.as_ref(),
        );
        #[cfg(feature = "boracle")]
        if target == SingleFileFrontendTarget::Boracle
            && !completed_source_packages.has_diagnosed_or_blocked()
            && !transient_batches
                .iter()
                .any(|transient| transient.batch.has_errors())
        {
            let modules = ModuleArtifactStore::new(1);
            let generated_store = BoundaryGeneratedFunctionStore::default();
            let provider_materialisations =
                super::seed_completed_package_materialisations(&completed_source_packages)?;
            let provider_binding_index = super::build_provider_binding_index(&[])?;
            let package_binding_index = super::build_source_package_dependency_index(
                &provider_binding_index,
                &source_package_dependencies,
            )?;
            let module_context = canonical::DirectoryModuleCompileContext::new(
                &boundary_context,
                &modules,
                &provider_materialisations,
                &[],
                &provider_binding_index,
                &source_package_dependencies,
                &package_binding_index,
                string_table,
            );
            let source_provider_dependencies = module_context
                .build_source_provider_dependencies(module_id, &prepared, None, None)?;
            let compile_context = ModuleCompilationContext {
                options: config.frontend_options(numeric_profile),
                build_profile,
                root_role_override: None,
                project_path_resolver: Some(&project_path_resolver),
                source_files,
                style_directives,
                global_string_table: Some(string_table),
                global_path_table: Some(path_interner.paths()),
                external_packages: Arc::clone(&external_packages),
                build_config_values: boundary_context.build_config_values(),
                external_dependency_resolution_table: &builder_surface
                    .external_dependency_resolution_table,
                source_provider_dependencies: &source_provider_dependencies,
                provider_materialisations: &provider_materialisations,
                builder_runtime_packages: &builder_surface.builder_runtime_packages,
            };
            let boracle_result = compile_module_for_boracle(
                &compile_context,
                prepared.semantic,
                generated_store.known_generated(),
                #[cfg(feature = "timers")]
                timing_module_context,
            );
            return boracle_result
                .map(|input| SingleFileFrontendResult::Boracle(Box::new(input)))
                .map_err(BoundaryPremergeFailure::from);
        }
        #[cfg(not(feature = "boracle"))]
        let _ = target;
        let job = ModuleCompilationJob {
            module_id,
            #[cfg(test)]
            stable_origin: graph_stable_origin,
            string_table_base_len: base_len,
            path_base_len,
            prepared,
            #[cfg(feature = "timers")]
            timing_module_key,
        };
        // The same readiness and publication owner handles package failures and binds their
        // immutable facades, while the synthetic graph still contains exactly one consumer.
        let (boundary, project_batches) = canonical::compile_module_waves_in_premerge_lane(
            boundary_context,
            graph,
            vec![vec![job]],
            Vec::new(),
            &[],
            &source_package_dependencies,
            &mut resource_inputs,
            string_table,
            &mut path_interner,
        )?;
        debug_assert!(
            project_batches.is_empty(),
            "synthetic consumer has no check-only sources"
        );
        ProjectFrontendCompilation::new(
            boundary,
            completed_source_packages,
            resource_inputs,
            transient_batches,
        )
        .map(|compilation| SingleFileFrontendResult::Project(Box::new(compilation)))
        .map_err(BoundaryPremergeFailure::from)
    })();
    // Finalize the source owner beside the semantic result. A finished source keeps
    // current attachment behavior; a failed finish keeps the semantic failure
    // authoritative and chains the finish failure beside it instead of replacing it.
    // A successful result with a failed finish surfaces only the finish error.
    source_owner.sources_mut().adopt_path_builder(path_interner);
    let finish_outcome = source_owner.finish();
    let (result, finalized) = match (result, finish_outcome) {
        (result, Ok(finished)) => (result, Arc::new(finished)),
        (Ok(_), Err(finish_error)) => {
            return Err(PremergeFailure::Infrastructure(finish_error).into());
        }
        (Err(failure), Err(finish_error)) => {
            return Err(BoundaryPremergeFailure {
                failure: Box::new(append_finish_failure(*failure.failure, finish_error)),
                package_source: failure.package_source,
            });
        }
    };
    *project_source_files = Some(finalized);
    match result {
        Ok(SingleFileFrontendResult::Project(compilation)) => {
            Ok(SingleFileFrontendResult::Project(compilation))
        }
        #[cfg(feature = "boracle")]
        Ok(SingleFileFrontendResult::Boracle(input)) => {
            Ok(SingleFileFrontendResult::Boracle(input))
        }
        Err(failure) => Err(failure),
    }
}

/// Rebind synthetic Stage 0 file-reference rows to the boundary's source identities.
///
/// Synthetic discovery resolves paths before the complete source closure is known, so it retains
/// canonical target paths and joins them here against the database discovery finalized once that
/// closure was known. This helper performs that one identity join and publishes the same resolved
/// table consumed by directory modules; it does not probe the filesystem or reinterpret path
/// syntax.
fn attach_single_file_resolved_references(
    prepared: &mut PreparedModule,
    source_files: &SourceDatabase,
    references: Vec<SingleFileResolvedReference>,
    _string_table: &StringTable,
) -> Result<(), CompilerError> {
    let mut resolved_table = ResolvedFileReferenceTable::new();

    for reference in references {
        let source_file = source_files
            .get_by_canonical_path(&reference.source_path)
            .map(|identity| identity.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "synthetic file-reference owner {:?} is absent from the boundary source database",
                    reference.source_path
                ))
            })?;

        let outcome = match reference.outcome {
            SingleFileReferenceOutcome::NoPhysicalTarget => {
                ResolvedFileReferenceOutcome::NoPhysicalTarget
            }
            SingleFileReferenceOutcome::Diagnostic(diagnostic) => {
                ResolvedFileReferenceOutcome::Diagnostic(diagnostic)
            }
            SingleFileReferenceOutcome::Resource {
                source,
                owner_relative_path,
            } => {
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ResourceSource {
                    source,
                    owner_relative_path,
                })
            }
            SingleFileReferenceOutcome::Source { canonical } => {
                let target_file = source_files
                    .get_by_canonical_path(&canonical)
                    .map(|identity| identity.id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "synthetic file-reference target {:?} is absent from the boundary source database",
                            canonical
                        ))
                    })?;
                if reference.class != PreparedFileReferenceClass::ContentSource {
                    return Err(CompilerError::compiler_error(
                        "synthetic physical file-reference outcome has an incompatible class",
                    ));
                }
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                    source: target_file,
                })
            }
            SingleFileReferenceOutcome::IdentifiedSourceKind => {
                if reference.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                    return Err(CompilerError::compiler_error(
                        "synthetic identified-source outcome has an incompatible class",
                    ));
                }
                ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::IdentifiedSourceKind,
                )
            }
        };

        resolved_table.push(ResolvedFileReference {
            source_file,
            path_syntax: reference.path_syntax,
            class: reference.class,
            outcome,
        })?;
    }

    prepared.semantic.resolved_file_references = resolved_table;
    Ok(())
}
