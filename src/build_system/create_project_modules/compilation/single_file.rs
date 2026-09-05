//! Single-file frontend compilation internals.
//!
//! The parent module retains the command-facing entry contracts; this child owns the synthetic
//! module discovery, preparation and semantic compilation pipeline.

use crate::{timing_scope, timing_scope_attributed};

#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::BoracleModuleInput;
#[cfg(feature = "boracle")]
use crate::compiler_frontend::module_compilation::compile_module_for_boracle;
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, ModuleCompilationContext, ModuleCompilationOutcome,
    ModuleSemanticResult, ProviderMaterialisationRegistry, compile_module,
};

use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ModuleDiagnostics};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::public_interface::SourceProviderDependencySet;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source_packages::root_file::file_name_is_normal_module_root_file;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::FrontendCompilationMode;
use super::super::compiled_boundary::{
    CompiledGraphBoundary, CompletedSourcePackageRegistry, DiagnosedModule,
    ProjectFrontendCompilation,
};
use super::super::config_boundary;
use super::super::file_reference_resolution::{
    SingleFileReferenceOutcome, SingleFileResolvedReference,
};
use super::super::generated_store::BoundaryGeneratedFunctionStore;
use super::super::module_artifact_store::ModuleArtifactStore;
use super::super::module_preparation::{ModulePreparationContext, record_module_input_counters};
use super::super::prepared_module::PreparedModule;
use super::super::prepared_source::PreparedSourceInput;
use super::super::project_module_graph::ProjectModuleGraph;
use super::super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::super::resource_inputs::ResourceInputRegistry;
use super::super::source_discovery;
use super::super::source_package_discovery::build_source_package_boundary_indexes;
use super::super::source_tree_index::SourceTreeIndex;
use super::{ModuleBoundaryPublication, publish_module_and_generated};

/// Move source snapshots carried by synthetic discovery into their registered database slots.
///
/// This runs while the database is still uniquely owned. Once preparation starts, every module
/// borrows the immutable database and no later `Arc::get_mut` handoff is possible.
fn retain_single_file_source_texts(
    input_files: &mut [PreparedSourceInput],
    source_files: &mut SourceDatabase,
) -> Result<(), CompilerError> {
    for input in input_files {
        let source_path = input.source_path().to_owned();
        let source_id = source_files
            .get_by_canonical_path(&source_path)
            .map(|record| record.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "single-file source path {} has no registered source identity",
                    source_path.display()
                ))
            })?;
        let source_code = input.take_source_code().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "single-file source path {} has no loaded source text",
                source_path.display()
            ))
        })?;
        source_files.retain_text(source_id, source_code)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_single_file_frontend_with_inputs(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    build_config_inputs: &BuildConfigInputSet,
    mode: FrontendCompilationMode,
) -> Result<ProjectFrontendCompilation, CompilerMessages> {
    match compile_single_file_frontend_with_target(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        build_config_inputs,
        mode,
        SingleFileFrontendTarget::Normal,
    )? {
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
pub(super) fn compile_single_file_boracle_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
) -> Result<BoracleModuleInput, CompilerMessages> {
    match compile_single_file_frontend_with_target(
        config,
        build_profile,
        style_directives,
        builder_surface,
        extension,
        string_table,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
        SingleFileFrontendTarget::Boracle,
    )? {
        SingleFileFrontendResult::Boracle(input) => Ok(*input),
        SingleFileFrontendResult::Project(_) => Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "Boracle source compilation unexpectedly returned a complete project payload",
            ),
            string_table,
        )),
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
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    extension: &OsStr,
    string_table: &mut StringTable,
    build_config_inputs: &BuildConfigInputSet,
    _mode: FrontendCompilationMode,
    target: SingleFileFrontendTarget,
) -> Result<SingleFileFrontendResult, CompilerMessages> {
    let mut resource_inputs = ResourceInputRegistry::new();
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

    timing_scope!(
        timing_guard_stage0_single_file_total,
        crate::timing::TimingMetric::Stage0SingleFileTotal
    );

    // 2. Resolve canonical entry path.
    let entry_path = match fs::canonicalize(&config.entry_dir) {
        Ok(path) => path,
        Err(error) => {
            let file_error = CompilerError::file_error(
                &config.entry_dir,
                format!("Failed to resolve entry file path: {error}"),
                string_table,
            );

            return Err(CompilerMessages::from_error_ref(file_error, string_table));
        }
    };
    let source_root = entry_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // 3. Initialize dependency-path resolution.
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
            return Err(messages);
        }
    };

    let entry_file_name = match entry_path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name,
        None => {
            let messages =
                non_utf8_filesystem_name_error(&entry_path, "single-file entry name", string_table);
            return Err(messages);
        }
    };

    let module_roots = if file_name_is_normal_module_root_file(entry_file_name) {
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
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    // 4. Discover all transitively reachable files.
    let mut external_imports = source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_dependency_resolution_table,
    };

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
    let collected = match source_discovery::collect_reachable_input_files(
        &entry_path,
        &project_path_resolver,
        style_directives,
        &mut external_imports,
        &builder_surface.source_file_kinds,
        &mut resource_inputs,
        string_table,
    ) {
        Ok(collected) => collected,
        Err(messages) => {
            return Err(messages);
        }
    };
    let mut input_files = collected.input_files;
    let resolved_file_references = collected.resolved_file_references;
    #[cfg(feature = "timers")]
    timing_guard_build_boundary_inventory.finish();
    // Share the effective external package registry immutably for the rest of the frontend
    // pipeline so each stage does not need its own deep clone.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 5. Run the module compilation pipeline with a local string-table delta.
    add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, 1);

    let string_table_fork = string_table.fork_for_module();
    let (local_table, base_len) = string_table_fork.into_parts();

    timing_scope_attributed!(
        timing_guard_boundary_compile,
        crate::timing::TimingMetric::BoundaryCompile,
        Some(crate::timing::TimingContext::for_boundary(timing_boundary)),
    );

    // Record module-input counters before preparation so the frontend module
    // total can be attributed even when preparation fails.
    let source_byte_count = record_module_input_counters(&input_files);

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
    let timing_module_context = Some(crate::timing::TimingContext::for_module(timing_module_key));

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
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    // Preparation retains the existing synthetic temporary-table lifecycle: register the complete
    // reachable candidate set before rebinding prepared outputs, then discard this table with the
    // single-file compilation boundary.
    let mut source_files = SourceDatabase::build(
        input_files.iter().map(PreparedSourceInput::source_path),
        &entry_path,
        Some(&project_path_resolver),
        string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    retain_single_file_source_texts(&mut input_files, &mut source_files)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let source_files = Arc::new(source_files);
    let preparation_context = ModulePreparationContext {
        source_files: &source_files,
        style_directives,
        project_path_resolver: Some(project_path_resolver.clone()),
    };

    let graph_stable_origin = stable_origin.clone();
    #[cfg(feature = "timers")]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        input_files,
        &entry_path,
        local_table,
        source_byte_count,
        timing_module_context,
    );
    #[cfg(not(feature = "timers"))]
    let prepare_result = preparation_context.prepare_module(
        stable_origin,
        input_files,
        &entry_path,
        local_table,
        source_byte_count,
    );
    let mut prepared = match prepare_result {
        Ok(prepared) => prepared,
        Err(messages) => {
            return Err(messages);
        }
    };
    attach_single_file_resolved_references(
        &mut prepared,
        &source_files,
        resolved_file_references,
        string_table,
    )?;

    let source_facts =
        config_boundary::source_contract_facts_from_prepared(&prepared, string_table, base_len);
    let effective_project_fields = config_boundary::effective_project_fields(config, string_table)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let fixed_project_facts =
        config_boundary::fixed_project_contract_facts(&effective_project_fields);
    let direct_project_facts =
        config_boundary::direct_project_contract_facts(&effective_project_fields);
    let fallback_location = SourceLocation::from_path(&config.entry_dir, string_table);
    let build_config_values = config_boundary::resolve_boundary_build_config(
        &source_facts,
        &fixed_project_facts,
        &direct_project_facts,
        build_config_inputs,
        builder_surface.config_globals(),
        fallback_location,
        string_table,
    )?;
    // Semantic compilation is one compiler service call. A synthetic single-file module has no
    // completed providers, so it binds against empty provider and materialisation views.
    let source_provider_dependencies = SourceProviderDependencySet::default();
    let mut provider_materialisations = ProviderMaterialisationRegistry::default();
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    let compile_context = ModuleCompilationContext {
        options: config.frontend_options(),
        build_profile,
        root_role_override: None,
        project_path_resolver: Some(project_path_resolver),
        source_files: &source_files,
        style_directives,
        external_packages: Arc::clone(&external_packages),
        build_config_values: Arc::new(build_config_values),
        external_dependency_resolution_table: &builder_surface.external_dependency_resolution_table,
        source_provider_dependencies: &source_provider_dependencies,
        provider_materialisations: &provider_materialisations,
        builder_runtime_packages: &builder_surface.builder_runtime_packages,
    };
    let semantic = prepared.semantic;

    timing_scope_attributed!(
        timing_guard_frontend_module_semantic_total,
        crate::timing::TimingMetric::FrontendModuleSemanticTotal,
        timing_module_context,
    );
    #[cfg(feature = "boracle")]
    if target == SingleFileFrontendTarget::Boracle {
        #[cfg(feature = "timers")]
        let boracle_result = compile_module_for_boracle(
            &compile_context,
            semantic,
            generated_store.known_generated(),
            timing_module_context,
        );
        #[cfg(not(feature = "timers"))]
        let boracle_result = compile_module_for_boracle(
            &compile_context,
            semantic,
            generated_store.known_generated(),
        );
        #[cfg(feature = "timers")]
        timing_guard_frontend_module_semantic_total.finish();
        #[cfg(feature = "timers")]
        timing_guard_boundary_compile.finish();
        return boracle_result.map(|input| SingleFileFrontendResult::Boracle(Box::new(input)));
    }
    #[cfg(not(feature = "boracle"))]
    let _ = target;
    #[cfg(feature = "timers")]
    let semantic_result = compile_module(
        &compile_context,
        semantic,
        generated_store.known_generated(),
        timing_module_context,
    );
    #[cfg(not(feature = "timers"))]
    let semantic_result = compile_module(
        &compile_context,
        semantic,
        generated_store.known_generated(),
    );
    #[cfg(feature = "timers")]
    timing_guard_frontend_module_semantic_total.finish();
    let result = match semantic_result {
        Ok(ModuleCompilationOutcome::Success(compiled)) => *compiled,
        Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
            let mut messages = diagnostics.into_messages();
            let remap = string_table.merge_delta_from(&messages.string_table, base_len);
            if !remap.is_identity() {
                messages.remap_string_ids(&remap);
            }
            let diagnosed = ModuleDiagnostics::from_messages(messages)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            let graph = ProjectModuleGraph::from_normal_roots(vec![(
                graph_stable_origin,
                source_root,
                entry_path,
            )]);
            let module_id = graph
                .entry_modules()
                .first()
                .copied()
                .expect("a single-module graph has one normal entry");
            let mut modules = ModuleArtifactStore::new(1);
            modules
                .mark_diagnosed(module_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            let boundary = CompiledGraphBoundary {
                structure: graph,
                modules,
                generated: generated_store,
                diagnosed: vec![DiagnosedModule {
                    module_id,
                    diagnostics: diagnosed,
                }],
                blocked: Vec::new(),
            };
            return ProjectFrontendCompilation::new(
                boundary
                    .finish()
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                CompletedSourcePackageRegistry::new(),
                resource_inputs,
            )
            .map(|compilation| SingleFileFrontendResult::Project(Box::new(compilation)))
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table));
        }
        Err(error) => {
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };
    #[cfg(feature = "timers")]
    timing_guard_boundary_compile.finish();

    // 6. Merge local results back into the global build context.
    let remap = string_table.merge_delta_from(&result.string_table, base_len);
    let ModuleSemanticResult {
        mut module,
        mut generated_delta,
        resource_source_associations,
        string_table: _,
        public_interface,
    } = result;
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
        generated_delta.remap_string_ids(&remap);
    }
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        graph_stable_origin.clone(),
        source_root,
        entry_path,
    )]);
    let module_id = graph
        .entry_modules()
        .first()
        .copied()
        .expect("a single-module graph has one normal entry");
    let mut modules = ModuleArtifactStore::new(1);
    let artifact = CompiledModuleArtifact {
        module,
        interface: public_interface,
    };
    publish_module_and_generated(ModuleBoundaryPublication {
        modules: &mut modules,
        generated: &mut generated_store,
        materialisations: &mut provider_materialisations,
        resource_inputs: &mut resource_inputs,
        module_id,
        expected_origin: &graph_stable_origin,
        artifact,
        generated_delta,
        resource_source_associations,
    })
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules,
        generated: generated_store,
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    ProjectFrontendCompilation::new(
        boundary
            .finish()
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
        CompletedSourcePackageRegistry::new(),
        resource_inputs,
    )
    .map(|compilation| SingleFileFrontendResult::Project(Box::new(compilation)))
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

/// Rebind synthetic Stage 0 file-reference rows to the boundary's temporary source identities.
///
/// Synthetic discovery resolves paths before the complete source closure is known, so it retains
/// canonical target paths until the enclosing single-file boundary builds its authoritative
/// `SourceDatabase`. This helper performs that one identity join and publishes the same resolved
/// table consumed by directory modules; it does not probe the filesystem or reinterpret path
/// syntax.
fn attach_single_file_resolved_references(
    prepared: &mut PreparedModule,
    source_files: &SourceDatabase,
    references: Vec<SingleFileResolvedReference>,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let mut resolved_table = ResolvedFileReferenceTable::new();

    for reference in references {
        let source_file = source_files
            .get_by_canonical_path(&reference.source_path)
            .map(|identity| identity.id)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "synthetic file-reference owner {:?} is absent from the boundary source database",
                        reference.source_path
                    )),
                    string_table,
                )
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
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(format!(
                                "synthetic file-reference target {:?} is absent from the boundary source database",
                                canonical
                            )),
                            string_table,
                        )
                    })?;
                if reference.class != PreparedFileReferenceClass::ContentSource {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "synthetic physical file-reference outcome has an incompatible class",
                        ),
                        string_table,
                    ));
                }
                ResolvedFileReferenceOutcome::Target(ResolvedFileReferenceTarget::ContentSource {
                    source: target_file,
                })
            }
            SingleFileReferenceOutcome::IdentifiedSourceKind => {
                if reference.class != PreparedFileReferenceClass::SourceKindNoFileValue {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "synthetic identified-source outcome has an incompatible class",
                        ),
                        string_table,
                    ));
                }
                ResolvedFileReferenceOutcome::Target(
                    ResolvedFileReferenceTarget::IdentifiedSourceKind,
                )
            }
        };

        resolved_table
            .push(ResolvedFileReference {
                source_file,
                path_syntax: reference.path_syntax,
                class: reference.class,
                outcome,
            })
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    }

    prepared.semantic.resolved_file_references = resolved_table;
    Ok(())
}
