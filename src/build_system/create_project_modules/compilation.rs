//! Single-file and directory frontend compilation.
//!
//! WHAT: compiles project modules through the frontend pipeline for single-file and directory entries.
//! WHY: separating the two flows keeps each path readable as orchestration over named steps.

use crate::build_system::build::{CompiledModuleResult, Module};

use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringTable, StringTableForkSource};
use rustc_hash::FxHashMap;

use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::source_packages::root_file::file_name_is_hash_root_file;
use crate::projects::settings::{Config, LANGUAGE_SOURCE_EXTENSION};

use rayon::prelude::*;

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::collision_detection::validate_source_package_tree_collisions;
use super::frontend_orchestration::{
    FrontendModuleBuildContext, ModuleCompilationOutcome, ModulePreparationContext,
    module_timing_label, record_module_input_counters,
};
use super::module_inventory;
use super::project_roots;
use super::project_structure_diagnostics::non_utf8_filesystem_name_error;
use super::reachable_file_discovery;
use super::root_validation::validate_source_package_roots;
use super::source_package_discovery::prepare_source_package_roots;
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
) -> Result<Vec<Module>, CompilerMessages> {
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
    let prepared_source_package_roots =
        match prepare_source_package_roots(&builder_surface.source_packages, string_table) {
            Ok(roots) => roots,
            Err(messages) => {
                log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
                log_stage_timing("stage0.single_file.total", total_start);
                return Err(messages);
            }
        };
    if let Err(messages) =
        validate_source_package_roots(&prepared_source_package_roots, string_table)
    {
        log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
        log_stage_timing("stage0.single_file.total", total_start);
        return Err(messages);
    }

    if let Err(messages) =
        validate_source_package_tree_collisions(&builder_surface.source_packages, string_table)
    {
        log_stage_timing("stage0.single_file.path_resolver", path_resolver_start);
        log_stage_timing("stage0.single_file.total", total_start);
        return Err(messages);
    }

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
    let mut external_imports = reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    let reachable_files_start = crate::timing::start_pipeline_timing();
    let input_files = match reachable_file_discovery::collect_reachable_input_files(
        &entry_path,
        &project_path_resolver,
        style_directives,
        &mut external_imports,
        None,
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
        super::frontend_orchestration::ModuleOriginInput::Synthetic(stable_origin),
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
    let compile_context = FrontendModuleBuildContext {
        config,
        build_profile,
        project_path_resolver: Some(project_path_resolver),
        style_directives,
        external_packages: Arc::clone(&external_packages),
        external_import_resolution_table: &builder_surface.external_import_resolution_table,
        builder_runtime_packages: &builder_surface.builder_runtime_packages,
    };

    let result = match compile_context.compile_module_semantic(prepared, &entry_path, module_label)
    {
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
    // The transient `CompiledModuleResult` also carries the aggregate public-interface draft
    // for the next graph/interface slice. The legacy flat `Vec<Module>` handoff drops it here
    // at the migration boundary; the accepted three-lane `Module` does not store it.
    let CompiledModuleResult {
        mut module,
        string_table: _,
        public_interface_draft,
    } = result;
    // Explicit drop keeps production ownership honest until the graph consumer lands.
    drop(public_interface_draft);
    // The validated generic-template store is a body-artefact checkpoint for the future
    // generated sidecar worklist (R3). The legacy flat `Vec<Module>` handoff discards it here
    // before string-table remap because the retained `FunctionSignature` carries donor-local
    // `StringId`s whose remap owner is not in scope for this slice. Discarding before remap
    // keeps stale local `StringId`s from reaching backends.
    module.metadata.discard_validated_generic_templates();
    if !remap.is_identity() {
        module.remap_string_ids(&remap);
    }
    log_stage_timing("stage0.single_file.merge_delta", merge_delta_start);

    log_stage_timing("stage0.single_file.total", total_start);

    Ok(vec![module])
}

// -------------------------
//  Directory Compilation
// -------------------------

/// Module compilation result plus the fork marker needed at merge time.
///
/// Preparation still returns the build-boundary `CompilerMessages`, so the per-module task keeps
/// the build/render-boundary `CompilerMessages` form for failures. The typed semantic result from
/// `compile_module_semantic` is packaged into this form once, here: a `Diagnosed` module becomes
/// its `ModuleDiagnostics::into_messages` inverse, and an infrastructure `CompilerError` becomes
/// an infrastructure `CompilerMessages` through `CompilerMessages::from_error`. The directory
/// aggregation and renderer then consume `CompilerMessages` without re-classifying it.
struct DirectoryModuleTaskResult {
    entry_path: PathBuf,
    string_table_base_len: usize,
    result: Result<CompiledModuleResult, CompilerMessages>,
}

struct SuccessfulModuleCompilation {
    string_table_base_len: usize,
    compiled: CompiledModuleResult,
}

struct FailedModuleCompilation {
    string_table_base_len: usize,
    messages: CompilerMessages,
}

struct DirectoryModuleCompileContext<'a> {
    string_table_fork_source: &'a StringTableForkSource,
    config: &'a Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    external_packages: &'a Arc<ExternalPackageRegistry>,
    builder_surface: &'a BuilderSurface,
    source_origin_lookup: &'a FxHashMap<std::path::PathBuf, StableModuleOriginIdentity>,
}

impl DirectoryModuleCompileContext<'_> {
    fn compile(&self, discovered: module_inventory::DiscoveredModule) -> DirectoryModuleTaskResult {
        let string_table_fork = self.string_table_fork_source.fork_for_module();
        let (local_table, base_len) = string_table_fork.into_parts();
        let module_inventory::DiscoveredModule {
            stable_origin,
            entry_point,
            input_files,
        } = discovered;
        let input_files = &input_files;

        // Record module-input counters and the per-module timing label before preparation so
        // the frontend module total can be attributed even when preparation fails.
        let source_byte_count = record_module_input_counters(input_files);
        let module_label_text =
            module_timing_label(&entry_point, input_files.len(), source_byte_count);
        let module_label: Option<&str> = Some(&module_label_text);

        // Record the total frontend time for this module (success or error).
        let module_total_start = crate::timing::start_pipeline_timing();

        // Preparation is provider-independent: it owns no external package registry, import
        // resolution table or builder runtime packages. Construct it before the semantic context
        // so Phase 5 can schedule provider binding between the two calls.
        let preparation_context = ModulePreparationContext {
            style_directives: self.style_directives,
            project_path_resolver: Some(self.project_path_resolver.clone()),
        };

        let prepared = match preparation_context.prepare_module(
            super::frontend_orchestration::ModuleOriginInput::Graph {
                stable_origin,
                origin_by_canonical_path: self.source_origin_lookup,
            },
            input_files,
            &entry_point,
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
                return DirectoryModuleTaskResult {
                    entry_path: entry_point,
                    string_table_base_len: base_len,
                    result: Err(messages),
                };
            }
        };

        // Semantic compilation is provider-dependent: it binds retained `PreparedHeaderSyntax`
        // against provider interfaces, then resolves dependencies, builds AST, lowers HIR and
        // runs borrow validation.
        let compile_context = FrontendModuleBuildContext {
            config: self.config,
            build_profile: self.build_profile,
            project_path_resolver: Some(self.project_path_resolver.clone()),
            style_directives: self.style_directives,
            external_packages: Arc::clone(self.external_packages),
            external_import_resolution_table: &self
                .builder_surface
                .external_import_resolution_table,
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
        let result =
            match compile_context.compile_module_semantic(prepared, &entry_point, module_label) {
                Ok(ModuleCompilationOutcome::Success(compiled)) => Ok(*compiled),
                Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
                    Err(diagnostics.into_messages())
                }
                Err(error) => {
                    let module_string_table = self
                        .string_table_fork_source
                        .fork_for_module()
                        .into_parts()
                        .0;
                    Err(CompilerMessages::from_error(error, module_string_table))
                }
            };
        crate::timing::record_started_pipeline_timing_with_label(
            "frontend.module.total",
            module_total_start,
            module_label,
        );

        DirectoryModuleTaskResult {
            entry_path: entry_point,
            string_table_base_len: base_len,
            result,
        }
    }
}

/// Discover all entry modules in a directory project and compile each one.
pub(crate) fn compile_directory_frontend(
    config: &Config,
    build_profile: FrontendBuildProfile,
    style_directives: &StyleDirectiveRegistry,
    builder_surface: &mut BuilderSurface,
    string_table: &mut StringTable,
) -> Result<Vec<Module>, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();

    // 1. Setup path resolution based on config settings.
    let path_resolver_start = crate::timing::start_pipeline_timing();
    let mut project_setup = match project_roots::build_project_path_resolver_with_index(
        config,
        &builder_surface.source_packages,
        &builder_surface.source_file_kinds,
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

    // 2. Scan the directory for entry modules and their reachable files.
    let mut external_imports = reachable_file_discovery::ExternalImportDiscoveryState {
        external_packages: &mut builder_surface.binding_packages,
        providers: &builder_surface.external_import_providers,
        cache: &mut builder_surface.external_import_cache,
        resolution_table: &mut builder_surface.external_import_resolution_table,
    };

    let module_inventory_start = crate::timing::start_pipeline_timing();
    let module_waves = match module_inventory::discover_all_modules_in_project(
        config,
        &project_path_resolver,
        &mut project_setup.project_module_graph,
        style_directives,
        &mut external_imports,
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

    // Build the immutable source-origin lookup from the graph's owned source sets so each
    // directory module's preparation can resolve every prepared source file to its owning
    // stable module origin. The lookup is shared across all module compilations and is a direct
    // projection of the graph ownership authority, not a filesystem scan or longest-prefix guess.
    let source_origin_lookup = match project_setup
        .project_module_graph
        .build_source_origin_lookup()
    {
        Ok(lookup) => lookup,
        Err(error) => {
            log_stage_timing("stage0.directory.total", total_start);
            return Err(CompilerMessages::from_error_ref(error, string_table));
        }
    };

    // Share the effective external package registry immutably across all module compilations;
    // directory modules may compile in parallel and can safely read the same Arc.
    let external_packages = Arc::new(builder_surface.binding_packages.clone());

    // 3. Compile modules one compile wave at a time, each with its own local string-table delta.
    //
    // The fork source owns one shared base snapshot for the whole project. Individual module forks
    // then keep only strings introduced during that module's frontend pipeline. Phase 5c schedules
    // the temporary normal-entry jobs in graph wave order: waves are consumed sequentially so a
    // provider's entry job finishes before its consumers' entry jobs start, and within a ready
    // wave Rayon parallelism is used only when the wave has multiple entry jobs. This preserves
    // wave boundaries only; current entry-closure payload semantics are unchanged and no immutable
    // provider interface is produced or consumed yet, which remains a Phase 5d concern.
    let string_table_fork_source = string_table.fork_source();
    let compile_context = DirectoryModuleCompileContext {
        string_table_fork_source: &string_table_fork_source,
        config,
        build_profile,
        project_path_resolver: &project_path_resolver,
        style_directives,
        external_packages: &external_packages,
        builder_surface,
        source_origin_lookup: &source_origin_lookup,
    };

    // Record frontend counters per wave: multi-job waves contribute to the parallel-task count and
    // singleton waves contribute to the serial count. This avoids claiming cross-wave tasks as
    // parallel when only intra-wave jobs run concurrently.
    for wave in module_waves.waves() {
        if wave.len() > 1 {
            add_frontend_counter(
                FrontendCounter::ModuleCompilationParallelTaskCount,
                wave.len(),
            );
        } else {
            add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, wave.len());
        }
    }

    let module_compile_batch_start = crate::timing::start_pipeline_timing();

    // Compile one wave at a time. Results are appended in deterministic graph order so the
    // subsequent entry-path sort and string-table/diagnostic aggregation are independent of
    // worker completion order. A wave with 0 or 1 entry jobs compiles serially; a wave with
    // multiple ready jobs uses the existing Rayon indexed parallel iterator. Each wave finishes
    // before the next starts.
    let mut results: Vec<DirectoryModuleTaskResult> = Vec::new();
    for wave in module_waves.into_waves() {
        let wave_results = if wave.len() > 1 {
            wave.into_par_iter()
                .map(|discovered| compile_context.compile(discovered))
                .collect::<Vec<DirectoryModuleTaskResult>>()
        } else {
            wave.into_iter()
                .map(|discovered| compile_context.compile(discovered))
                .collect::<Vec<DirectoryModuleTaskResult>>()
        };
        results.extend(wave_results);
    }
    log_stage_timing(
        "stage0.directory.module_compile_batch",
        module_compile_batch_start,
    );

    // 4. Deterministic ordering by entry path.
    let result_sort_start = crate::timing::start_pipeline_timing();
    results.sort_by(|a, b| a.entry_path.cmp(&b.entry_path));
    log_stage_timing("stage0.directory.result_sort", result_sort_start);

    // 5. Partition into successes and failures.
    let mut successes = Vec::with_capacity(results.len());
    let mut failures = Vec::new();

    for outcome in results {
        match outcome.result {
            Ok(compiled) => successes.push(SuccessfulModuleCompilation {
                string_table_base_len: outcome.string_table_base_len,
                compiled,
            }),
            Err(messages) => failures.push(FailedModuleCompilation {
                string_table_base_len: outcome.string_table_base_len,
                messages,
            }),
        }
    }

    // 6. If any module failed, aggregate all diagnostics deterministically and exit.
    if !failures.is_empty() {
        let failure_aggregation_start = crate::timing::start_pipeline_timing();
        let aggregation_fork = string_table_fork_source.fork_for_module();
        let (mut aggregated_table, _) = aggregation_fork.into_parts();
        let mut aggregated_messages = CompilerMessages::empty(aggregated_table.clone());

        for mut failure in failures {
            let remap = aggregated_table.merge_delta_from(
                &failure.messages.string_table,
                failure.string_table_base_len,
            );

            if !remap.is_identity() {
                failure.messages.remap_string_ids(&remap);
            }

            aggregated_messages.append_messages_preserving_context(failure.messages);
        }

        aggregated_messages.string_table = aggregated_table;
        log_stage_timing(
            "stage0.directory.failure_aggregation",
            failure_aggregation_start,
        );
        log_stage_timing("stage0.directory.total", total_start);
        return Err(aggregated_messages);
    }

    // 7. All succeeded: merge each local table into the build table and remap.
    let success_merge_start = crate::timing::start_pipeline_timing();
    let mut compiled_modules = Vec::with_capacity(successes.len());

    for success in successes {
        let remap = string_table.merge_delta_from(
            &success.compiled.string_table,
            success.string_table_base_len,
        );
        // The transient `CompiledModuleResult` also carries the aggregate public-interface
        // draft for the next graph/interface slice. The legacy flat `Vec<Module>` handoff
        // drops it here at the migration boundary; the accepted three-lane `Module` does not
        // store it.
        let CompiledModuleResult {
            mut module,
            string_table: _,
            public_interface_draft,
        } = success.compiled;
        // Explicit drop keeps production ownership honest until the graph consumer lands.
        drop(public_interface_draft);
        // The validated generic-template store is a body-artefact checkpoint for the future
        // generated sidecar worklist (R3). The legacy flat `Vec<Module>` handoff discards it here
        // before string-table remap because the retained `FunctionSignature` carries donor-local
        // `StringId`s whose remap owner is not in scope for this slice. Discarding before remap
        // keeps stale local `StringId`s from reaching backends.
        module.metadata.discard_validated_generic_templates();
        if !remap.is_identity() {
            module.remap_string_ids(&remap);
        }
        compiled_modules.push(module);
    }
    log_stage_timing("stage0.directory.success_merge", success_merge_start);

    log_stage_timing("stage0.directory.total", total_start);

    Ok(compiled_modules)
}
