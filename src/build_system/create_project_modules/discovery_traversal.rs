//! Reachable-source traversal for the synthetic single-file Stage 0 lane.
//!
//! WHAT: `discover_reachable_source_files` runs the shared BFS over dependency paths. The
//!       provider-capable path owns external-provider resolution; the shared BFS owns queue
//!       handling, canonicalization, source preparation and local queuing.
//! WHY:  directory projects use indexed discovery and never run this BFS. They reuse the
//!       provider-capable decision through `resolve_structural_provider_reference`, which maps
//!       `DependencyPolicyAction` to `StructuralProviderAction`.
use super::discovery_finalization::finalize_reachable_files;
use super::discovery_provider_imports::{
    ProviderBackedImportRequest, resolve_provider_backed_import, unsupported_builder_package_error,
    unsupported_external_extension_error,
};
use super::*;
use crate::builder_surface::external_import_providers::provider_sources::ProviderSourceRegistrar;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DependencyClauseKind;
use crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    project_config, token_lexical,
};
use crate::compiler_frontend::paths::path_normalization::{
    is_relative_dependency_path, join_and_normalize_path,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{
    NonUtf8PathComponent, PathId, PathInternerFork,
};
/// Resolve provider-backed and binding-backed dependency classes before indexed source resolution.
///
/// Directory module scheduling calls this with provider references retained by header syntax.
#[allow(
    clippy::too_many_arguments,
    reason = "structural provider resolution keeps the retained path, clause kind, declaring file, resolver, and mutable path/external-import/resolution/string state as separate borrows"
)]
pub(crate) fn resolve_structural_provider_reference(
    provider: &RetainedDependencyPath,
    clause_kind: DependencyClauseKind,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    path_fork: &mut PathInternerFork,
    provider_sources: &mut dyn ProviderSourceRegistrar,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    directory_dependency_resolution: DirectoryDependencyResolution<'_>,
    string_table: &mut StringTable,
) -> Result<StructuralProviderAction, SourceDiscoveryError> {
    match handle_provider_capable_dependency(
        ProviderCapableDependencyInput {
            dependency_path: provider.path,
            dependency_span: Some(provider.span),
            clause_kind,
            target: &provider.target,
            provider_target: provider.provider_target.as_ref(),
            canonical_file,
            project_path_resolver,
            path_fork,
            directory_dependency_resolution: Some(directory_dependency_resolution),
            string_table,
        },
        provider_sources,
        external_imports,
    )? {
        DependencyPolicyAction::QueueLocal => Ok(StructuralProviderAction::ResolveSource),
        DependencyPolicyAction::Skip => Ok(StructuralProviderAction::Handled),
    }
}
// -------------------------
//  Reachable Discovery
// -------------------------

/// Action a traversal policy wants the shared BFS to take for one dependency path.
enum DependencyPolicyAction {
    /// Do not follow this dependency.
    Skip,
    /// Resolve and queue the dependency as a normal local Moth dependency.
    QueueLocal,
}

/// Stage 0 dependency policy that customizes the shared reachable-file traversal.
///
/// WHAT: the provider-capable path owns external-provider resolution while the shared BFS owns queue
///       handling, canonicalization, source preparation and local queuing.
///
/// The policy only decides dependency actions for the synthetic single-file traversal. Directory
/// projects use indexed discovery and prepare each owned `SourceId` directly in the module queue;
/// the synthetic traversal alone retains its isolated local scan cache until input assembly.
enum DependencyPolicy<'a, 'b> {
    /// Full provider-capable path. Mutates provider cache and resolution tables.
    Capable {
        external_imports: &'a mut ExternalImportDiscoveryState<'b>,
    },
}
struct ProviderCapableDependencyInput<'a> {
    dependency_path: PathId,
    dependency_span: Option<SourceSpan>,
    clause_kind: DependencyClauseKind,
    target: &'a DependencyTargetKind,
    provider_target: Option<&'a CheckedExternalProviderTarget>,
    canonical_file: &'a Path,
    project_path_resolver: &'a ProjectPathResolver,
    path_fork: &'a mut PathInternerFork,
    directory_dependency_resolution: Option<DirectoryDependencyResolution<'a>>,
    string_table: &'a mut StringTable,
}

impl<'a, 'b> DependencyPolicy<'a, 'b> {
    /// Decide how to handle one dependency path.
    fn handle_dependency(
        &mut self,
        input: ProviderCapableDependencyInput<'_>,
        provider_sources: &mut dyn ProviderSourceRegistrar,
    ) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
        match self {
            DependencyPolicy::Capable { external_imports } => {
                handle_provider_capable_dependency(input, provider_sources, external_imports)
            }
        }
    }
}

/// Read/preparation accounting for one `.moth` file during traversal.
///
/// The complete `PreparedDiscoverySource` remains in `local_source_cache`; this result deliberately
/// carries no detached clause vector because a retained clause range is meaningful only with its
/// owning file selection table.
struct ScannedMothSource {
    fresh_read: bool,
    source_byte_count: usize,
}

#[allow(
    clippy::too_many_arguments,
    reason = "scanning keeps the canonical file, directives, resolver, entry path, and mutable database/path/cache/string state as separate borrows"
)]
fn scan_and_cache_local_moth_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &ProjectPathResolver,
    entry_file_path: &Path,
    source_files: &mut SourceDatabase,
    path_fork: &mut PathInternerFork,
    local_source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if local_source_cache.contains_key(canonical_file) {
        return Ok(ScannedMothSource {
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = prepare_discovery_source(
        canonical_file,
        style_directives,
        &Some(project_path_resolver.clone()),
        entry_file_path,
        source_files,
        path_fork,
        string_table,
    )?;
    let source_byte_count = scanned.source_byte_len;
    local_source_cache.insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        fresh_read: true,
        source_byte_count,
    })
}
#[allow(
    clippy::too_many_arguments,
    reason = "template scanning keeps the canonical file, directives, resolver, entry path, and mutable database/path/cache/string state as separate borrows"
)]
fn scan_and_cache_local_moth_template_source(
    canonical_file: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &ProjectPathResolver,
    entry_file_path: &Path,
    source_files: &mut SourceDatabase,
    path_fork: &mut PathInternerFork,
    local_source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    string_table: &mut StringTable,
) -> Result<ScannedMothSource, SourceDiscoveryError> {
    if local_source_cache.contains_key(canonical_file) {
        return Ok(ScannedMothSource {
            fresh_read: false,
            source_byte_count: 0,
        });
    }

    let scanned = prepare_discovery_template_source(
        canonical_file,
        style_directives,
        &Some(project_path_resolver.clone()),
        entry_file_path,
        source_files,
        path_fork,
        string_table,
    )?;
    let source_byte_count = scanned.source_byte_len;
    local_source_cache.insert(canonical_file.to_path_buf(), scanned);

    Ok(ScannedMothSource {
        fresh_read: true,
        source_byte_count,
    })
}

/// BFS over the synthetic single-file compilation's dependency clauses.
///
/// WHAT: follows each Moth file's declared dependencies, resolves them to canonical typed source
///       files and finalizes the complete source closure before returning its prepared inputs.
/// WHY: directory projects use indexed header-owned discovery; this filesystem traversal exists
///      only for a file invoked directly as one synthetic module.
///
/// The traversal-local source database is consumed before this value is constructed. Every source
/// ID in the returned inputs and database therefore belongs to the final logical-order domain,
/// while the returned owner remains live for semantic span producers.
pub(crate) struct ReachableTraversalOutcome {
    pub(super) source_files: SourceDatabaseBuilder,
    pub(super) input_files: Vec<PreparedSourceInput>,
    pub(super) resolved_file_references: Vec<SingleFileResolvedReference>,
    pub(super) source_package_dependencies: Vec<ResolvedSourcePackageDependency>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "reachable traversal keeps entry paths, namespace, resolver, directives and source-kind registry, mutable dependency policy and resource registry and mutable string/path tables separate"
)]
fn traverse_reachable_source_files(
    entry_paths: &[PathBuf],
    namespace_set: &ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    policy: &mut DependencyPolicy<'_, '_>,
    provider_sources: &mut ProviderOwnedSourceStaging,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let canonical_entry_path = fs::canonicalize(&entry_paths[0]).map_err(|error| {
        InfrastructureFailure::filesystem(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            &entry_paths[0],
            format!("Failed to canonicalize entry file path: {error}"),
        )
    })?;
    let root_directory = canonical_entry_path.parent().ok_or_else(|| {
        InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            "canonical synthetic entry path has no containing module root",
        )
    })?;
    let mut file_reference_resolver = SingleFileReferenceResolver::new(
        root_directory.to_path_buf(),
        source_file_kinds,
        resource_inputs,
    );
    let context = DiscoveryWalkContext {
        canonical_entry_path: &canonical_entry_path,
        project_path_resolver,
        style_directives,
        source_file_kinds,
    };
    let mut inventory = ReachableSourceInventory {
        reachable: BTreeSet::new(),
        queue: entry_paths
            .iter()
            .map(|path| ReachableSourceFile {
                path: path.clone(),
                kind: SourceFileKind::Moth,
            })
            .collect(),
        local_source_cache: FxHashMap::default(),
        traversal_source_files: SourceDatabase::empty(),
        resolved_file_references: Vec::new(),
        source_package_dependencies: Vec::new(),
    };

    // The walk only borrows source ownership, so every terminal path reaches this barrier.
    let outcome = walk_reachable_sources(
        &mut inventory,
        namespace_set,
        &context,
        policy,
        provider_sources,
        &mut file_reference_resolver,
        path_fork,
        string_table,
    );
    let failure = match outcome {
        Ok(DiscoveryWalkOutcome::Complete | DiscoveryWalkOutcome::PreparationFailed) => None,
        Err(error) => Some(TraversalFailure { error }),
    };
    let ReachableSourceInventory {
        mut reachable,
        queue,
        local_source_cache,
        traversal_source_files,
        mut resolved_file_references,
        mut source_package_dependencies,
        ..
    } = inventory;
    reachable.extend(queue);
    drop(traversal_source_files);
    let (source_files, input_files) = finalize_reachable_files(
        reachable.into_iter().collect(),
        local_source_cache,
        &canonical_entry_path,
        project_path_resolver,
        string_table,
        path_fork,
        provider_sources,
        failure,
        &mut resolved_file_references,
        &mut source_package_dependencies,
    )?;
    Ok(ReachableTraversalOutcome {
        source_files,
        input_files,
        resolved_file_references,
        source_package_dependencies,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the walk keeps its shared context separate from each independently borrowed mutable owner: inventory, dependency policy, provider-source staging, reference resolver and path/string tables"
)]
fn walk_reachable_sources(
    inventory: &mut ReachableSourceInventory,
    namespace_set: &ModuleNamespaceSet,
    context: &DiscoveryWalkContext<'_>,
    policy: &mut DependencyPolicy<'_, '_>,
    provider_sources: &mut ProviderOwnedSourceStaging,
    file_reference_resolver: &mut SingleFileReferenceResolver<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<DiscoveryWalkOutcome, SourceDiscoveryError> {
    let ReachableSourceInventory {
        reachable,
        queue,
        local_source_cache,
        traversal_source_files,
        resolved_file_references,
        source_package_dependencies,
    } = inventory;
    let DiscoveryWalkContext {
        canonical_entry_path,
        project_path_resolver,
        style_directives,
        source_file_kinds,
    } = *context;
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let mut dependency_clauses_scanned: usize = 0;
    while let Some(next_file) = queue.pop_front() {
        let canonical_file = fs::canonicalize(&next_file.path).map_err(|error| {
            InfrastructureFailure::filesystem(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                &next_file.path,
                format!("Failed to canonicalize module file path: {error}"),
            )
        })?;
        let reachable_file = ReachableSourceFile {
            path: canonical_file.clone(),
            kind: next_file.kind,
        };

        if !reachable.insert(reachable_file.clone()) {
            continue;
        }

        if next_file.kind == SourceFileKind::PlainMarkdown {
            // Markdown files are importless content assets. They are carried forward for
            // header-stage preparation but are never scanned for dependencies.
            continue;
        }

        let scan_result = match next_file.kind {
            SourceFileKind::Moth => scan_and_cache_local_moth_source(
                &canonical_file,
                style_directives,
                project_path_resolver,
                canonical_entry_path,
                traversal_source_files,
                path_fork,
                local_source_cache,
                string_table,
            ),
            SourceFileKind::MothTemplate => scan_and_cache_local_moth_template_source(
                &canonical_file,
                style_directives,
                project_path_resolver,
                canonical_entry_path,
                traversal_source_files,
                path_fork,
                local_source_cache,
                string_table,
            ),
            SourceFileKind::PlainMarkdown => crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "PlainMarkdown sources continue before scan dispatch; scan_and_cache is only for Moth and MothTemplate"
            ),
        };
        let scanned = match scan_result {
            Ok(scanned) => scanned,
            Err(error) => {
                return Err(error);
            }
        };

        if scanned.fresh_read {
            add_frontend_counter(
                FrontendCounter::Stage0SourceBytesLoaded,
                scanned.source_byte_count,
            );
        }

        let prepared = &local_source_cache
            .get(&canonical_file)
            .expect("scanned source remains owned by traversal")
            .prepared_output
            .result;
        let Ok(prepared) = prepared else {
            return Ok(DiscoveryWalkOutcome::PreparationFailed);
        };
        let dependency_clauses = &prepared.file_dependency_clauses;
        #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
        {
            dependency_clauses_scanned += dependency_clauses.len();
        }

        {
            // Clauses and expression-qualified occurrences share the same synthetic provider
            // closure: a per-file resolution cache keys on the retained path so a second
            // occurrence of one provider cannot repeat provider invocation or filesystem work.
            let mut reachable_queue = ReachableQueue { reachable, queue };
            let mut resolved_by_path = FxHashMap::default();
            let mut context = ReachableDependencyContext {
                policy,
                provider_sources,
                namespace_set,
                canonical_file: &canonical_file,
                project_path_resolver,
            };
            for clause in dependency_clauses {
                process_reachable_authored_dependency(
                    &mut context,
                    &clause.dependency,
                    clause.binding.clause_kind(),
                    string_table,
                    path_fork,
                    &mut reachable_queue,
                    source_package_dependencies,
                    &mut resolved_by_path,
                )
                .map_err(|error| {
                    with_provider_dependency_error(error, Some(clause.dependency.span))
                })?;
            }
            for occurrence in &prepared.qualified_references {
                process_reachable_authored_dependency(
                    &mut context,
                    &occurrence.dependency,
                    DependencyClauseKind::DirectSelection,
                    string_table,
                    path_fork,
                    &mut reachable_queue,
                    source_package_dependencies,
                    &mut resolved_by_path,
                )
                .map_err(|error| {
                    with_provider_dependency_error(error, Some(occurrence.dependency.span))
                })?;
            }
        }

        // Structural file references are already classified by preparation. Resolve every
        // occurrence through the same physical resolver used by directory modules, then queue
        // supported content targets so discovery reaches the complete content closure.
        let prepared_path_syntax = &prepared.path_syntax;
        let structural_file_references = prepared.structural_file_references.references();
        for reference in structural_file_references {
            let resolved = file_reference_resolver
                .resolve(
                    &canonical_file,
                    prepared_path_syntax.table(),
                    reference,
                    string_table,
                    path_fork,
                )
                .map_err(SourceDiscoveryError::from)?;
            if let SingleFileReferenceOutcome::Source { canonical } = &resolved.outcome
                && resolved.class == PreparedFileReferenceClass::ContentSource
            {
                let extension = canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default();
                let Some(kind) = source_file_kinds.kind_for_extension(extension) else {
                    // A symlinked reference can resolve to a target whose extension names no
                    // source kind, so this is project layout, not a compiler invariant.
                    return Err(SourceDiscoveryError::from(InfrastructureFailure::filesystem(
                        crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                        canonical,
                        "content reference resolves to a file whose extension has no registered source kind",
                    )));
                };
                let content_file = ReachableSourceFile {
                    path: canonical.clone(),
                    kind,
                };
                if !reachable.contains(&content_file) {
                    queue.push_back(content_file);
                }
            }
            resolved_file_references.push(resolved);
        }
    }

    // Record concise counters for the completed traversal. Counters are only
    // recorded when `benchmark_counters` is active, and reach stdout only when
    // `MOTH_COUNTERS=summary` requests it.
    counter_observation!(
        "stage0.reachable_discovery.reachable_files",
        reachable.len() as f64,
    );
    counter_observation!(
        "stage0.reachable_discovery.dependency_clauses_scanned",
        dependency_clauses_scanned as f64,
    );

    Ok(DiscoveryWalkOutcome::Complete)
}

/// BFS over dependency clauses starting from `entry_point`, preserving source kind.
///
/// WHAT: follows each Moth file's declared dependencies, resolves them to canonical typed source
/// files and returns the completed final source identity domain with prepared inputs.
/// WHY: source kind belongs to Stage 0 input discovery. Builder-supported content assets can be
/// loaded and carried forward without being treated as Moth module roots.
#[allow(
    clippy::too_many_arguments,
    reason = "reachable traversal keeps the entry point, resolver, directives, and mutable external-import/kind/resource/path/string state as separate borrows"
)]
pub(crate) fn discover_reachable_source_files(
    entry_point: &Path,
    namespace_set: &ModuleNamespaceSet,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    provider_sources: &mut ProviderOwnedSourceStaging,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let mut policy = DependencyPolicy::Capable { external_imports };

    traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        namespace_set,
        project_path_resolver,
        style_directives,
        &mut policy,
        provider_sources,
        source_file_kinds,
        resource_inputs,
        string_table,
        path_fork,
    )
}

/// Resolve an ordinary source clause within the bounded synthetic consumer.
/// Registered package clauses have already produced separate provider edges, so this lane
/// only queues consumer-owned source through its retained module roots and filesystem resolver.
/// One synthetic dependency resolution reused by every authored dependency with the same path.
///
/// WHAT: records the provider-policy outcome, the package-surface record or the filesystem source
///       file selected for one retained path.
/// WHY: the single-file traversal has no module graph dedup, so two qualified occurrences of one
///       provider would otherwise repeat provider invocation and filesystem resolution.
#[derive(Clone)]
enum SyntheticDependencyResolution {
    Skipped,
    Queue(ReachableSourceFile),
    PackageSurface {
        consumer_module_id: ModuleId,
        dependency_prefix: String,
    },
    HandledPackage,
}

/// Borrowed services shared by every authored dependency of one reachable source.
struct ReachableDependencyContext<'a, 'state, 'external> {
    policy: &'a mut DependencyPolicy<'state, 'external>,
    provider_sources: &'a mut ProviderOwnedSourceStaging,
    namespace_set: &'a ModuleNamespaceSet,
    canonical_file: &'a Path,
    project_path_resolver: &'a ProjectPathResolver,
}

/// Resolve one authored synthetic dependency (clause or qualified occurrence) exactly once.
///
/// WHAT: runs the provider policy, the local binding-package collision check and filesystem source
///       resolution for one retained provider path, then applies the cached outcome to this
///       authored shell.
/// WHY: direct selections inside clauses and expression-qualified occurrences must reach the same
///       synthetic source/provider/package closure and the same error lane. Only the caller's
///       authored path span distinguishes their diagnostics.
#[allow(
    clippy::too_many_arguments,
    reason = "synthetic dependency resolution keeps the shared walk services, the authored provider path and kind, and the mutable tables/accumulators separate"
)]
fn process_reachable_authored_dependency(
    context: &mut ReachableDependencyContext<'_, '_, '_>,
    provider: &RetainedDependencyPath,
    clause_kind: DependencyClauseKind,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    reachable_queue: &mut ReachableQueue<'_>,
    source_package_dependencies: &mut Vec<ResolvedSourcePackageDependency>,
    resolved_by_path: &mut FxHashMap<PathId, SyntheticDependencyResolution>,
) -> Result<(), SourceDiscoveryError> {
    let resolution = match resolved_by_path.get(&provider.path) {
        Some(resolution) => resolution.clone(),
        None => {
            let action = context.policy.handle_dependency(
                ProviderCapableDependencyInput {
                    dependency_path: provider.path,
                    dependency_span: Some(provider.span),
                    clause_kind,
                    target: &provider.target,
                    provider_target: provider.provider_target.as_ref(),
                    canonical_file: context.canonical_file,
                    project_path_resolver: context.project_path_resolver,
                    path_fork,
                    directory_dependency_resolution: None,
                    string_table,
                },
                context.provider_sources,
            )?;

            // Virtual binding packages also need local collision checks before their policy
            // skips filesystem discovery. Explicit-extension providers retain their own owner.
            let resolution = if matches!(provider.target, DependencyTargetKind::Source)
                && let Some(package) = context
                    .namespace_set
                    .resolve_single_file_package_dependency(
                        provider,
                        context.canonical_file,
                        context.project_path_resolver,
                        string_table,
                        path_fork,
                    )? {
                match package {
                    ResolvedDependency::SourcePackageSurface {
                        consumer_module_id,
                        dependency_prefix,
                        ..
                    } => SyntheticDependencyResolution::PackageSurface {
                        consumer_module_id,
                        dependency_prefix,
                    },
                    _ => SyntheticDependencyResolution::HandledPackage,
                }
            } else {
                match action {
                    DependencyPolicyAction::Skip => SyntheticDependencyResolution::Skipped,
                    DependencyPolicyAction::QueueLocal => {
                        SyntheticDependencyResolution::Queue(resolve_via_filesystem(
                            provider,
                            context.canonical_file,
                            path_fork,
                            context.project_path_resolver,
                            string_table,
                        )?)
                    }
                }
            };
            resolved_by_path.insert(provider.path, resolution.clone());
            resolution
        }
    };

    match resolution {
        SyntheticDependencyResolution::Skipped | SyntheticDependencyResolution::HandledPackage => {}
        SyntheticDependencyResolution::PackageSurface {
            consumer_module_id,
            dependency_prefix,
        } => {
            source_package_dependencies.push(ResolvedSourcePackageDependency {
                consumer_module_id,
                dependency_prefix,
                dependency_shell_id: provider.dependency_shell_id,
            });
            add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
        }
        SyntheticDependencyResolution::Queue(resolved_source_file) => {
            add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
            if !reachable_queue.reachable.contains(&resolved_source_file) {
                reachable_queue.queue.push_back(resolved_source_file);
            }
        }
    }

    Ok(())
}

fn resolve_via_filesystem(
    provider: &RetainedDependencyPath,
    canonical_file: &Path,
    path_fork: &mut PathInternerFork,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<ReachableSourceFile, SourceDiscoveryError> {
    let resolved = if is_relative_dependency_path(provider.path, path_fork, string_table) {
        project_path_resolver
            .resolve_dependency_to_source_file(
                provider.path,
                path_fork,
                canonical_file,
                string_table,
            )
            .map_err(SourceDiscoveryError::from)?
    } else {
        match resolve_module_root_bare_dependency(
            provider.path,
            path_fork,
            canonical_file,
            project_path_resolver,
            string_table,
        )? {
            Some(resolved) => resolved,
            None => project_path_resolver
                .resolve_dependency_to_source_file(
                    provider.path,
                    path_fork,
                    canonical_file,
                    string_table,
                )
                .map_err(SourceDiscoveryError::from)?,
        }
    };

    // Extensionless source clauses bind to a resolved source file in the synthetic traversal.
    Ok(resolved_source_file(&resolved.path, resolved.kind))
}

/// Resolve a bare dependency from the retained module-root topology before entry-root lookup.
///
/// WHAT: maps a module path to its prepared normal public surface, or resolves a bare dependency
///      from the declaring file's owning module root.
/// WHY: synthetic discovery has no indexed namespace, but it must still implement the same
///      module-root-relative source contract as directory discovery without allowing an entry-root
///      namesake to shadow the declaring module's source. A dependency target is importable only
///      through the normal facade: an entry-only child publishes no surface, and the preparation
///      anchor (normal, then support, then entry) carries no import authority.
fn resolve_module_root_bare_dependency(
    provider: PathId,
    path_fork: &mut PathInternerFork,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
) -> Result<Option<ResolvedDependencyFile>, SourceDiscoveryError> {
    let Some(first_component) = path_fork.component(provider) else {
        return Ok(None);
    };
    let first_segment = string_table.resolve(first_component);
    if matches!(first_segment, "." | "..") {
        return Ok(None);
    }

    let module_root = project_path_resolver.module_root_for_file(canonical_file);
    let Some(module_root) = module_root else {
        return Ok(None);
    };

    let root_candidate = join_and_normalize_path(&module_root, provider, &*path_fork, string_table);
    if let Some(root_file) = project_path_resolver
        .module_root_file_for_directory_surface(&root_candidate, ModuleRootRole::Normal)
    {
        return Ok(Some(ResolvedDependencyFile {
            path: root_file,
            kind: SourceFileKind::Moth,
        }));
    }

    if module_root == project_path_resolver.entry_root() {
        return Ok(None);
    }

    let module_prefix = module_root
        .strip_prefix(project_path_resolver.entry_root())
        .ok()
        .ok_or_else(|| {
            SourceDiscoveryError::from(InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "Owning module root '{}' is outside entry root '{}'",
                    module_root.display(),
                    project_path_resolver.entry_root().display()
                ),
            ))
        })?;
    let module_prefix = path_fork
        .try_intern_filesystem_path(module_prefix, string_table)
        .map_err(|error| match error {
            crate::compiler_frontend::symbols::path_interner::PathInternError::NonUtf8(
                NonUtf8PathComponent { path },
            ) => SourceDiscoveryError::from(InfrastructureFailure::filesystem(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, &path, format!(
                "Owning module path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
            ))),
            crate::compiler_frontend::symbols::path_interner::PathInternError::TableFull => {
                SourceDiscoveryError::Diagnostic(token_lexical::source_span_capacity(
                    0,
                    u32::MAX,
                    SourceSpanCapacityResource::LogicalPath,
                    None,
                ))
            }
            crate::compiler_frontend::symbols::path_interner::PathInternError::BaseMismatch {
                ..
            } => SourceDiscoveryError::Infrastructure(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, "logical path merge base is not a structural prefix of the destination table")),
        })?;
    let mut scratch = Vec::new();
    let module_local_provider = path_fork
        .try_join(module_prefix, provider, &mut scratch)
        .ok_or_else(|| {
            SourceDiscoveryError::Diagnostic(token_lexical::source_span_capacity(
                0,
                u32::MAX,
                SourceSpanCapacityResource::LogicalPath,
                None,
            ))
        })?;

    project_path_resolver
        .resolve_dependency_to_source_file(
            module_local_provider,
            &*path_fork,
            canonical_file,
            string_table,
        )
        .map(Some)
        .map_err(SourceDiscoveryError::from)
}
fn handle_provider_capable_dependency(
    input: ProviderCapableDependencyInput<'_>,
    provider_sources: &mut dyn ProviderSourceRegistrar,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
    let ProviderCapableDependencyInput {
        dependency_path,
        dependency_span,
        clause_kind,
        target,
        provider_target,
        canonical_file,
        project_path_resolver,
        path_fork,
        directory_dependency_resolution,
        string_table,
    } = input;

    // Rebind the directory resolution onto this caller-owned discovery fork so every namespace
    // lookup and provider target resolution sees PathIds from the same domain the provider will
    // intern diagnostic paths into.
    let resolution = directory_dependency_resolution.map(|r| r.with_path_fork(&*path_fork));

    // `@project` is a reserved synthetic provider, not a source/package path. Keep the exact
    // root out of filesystem discovery and external-package registration only for the owning
    // project boundary; source packages must receive the structured reserved-path diagnostic.
    if is_project_globals_namespace(dependency_path, path_fork, string_table) {
        let is_owning_project_root = resolution
            .as_ref()
            .is_none_or(|resolution| resolution.is_project_boundary());
        if is_project_globals_dependency(dependency_path, path_fork, string_table)
            && is_owning_project_root
        {
            return Ok(DependencyPolicyAction::Skip);
        }
        return Err(project_config::invalid_dependency_clause(
            clause_kind,
            InvalidDependencyClauseReason::ProjectGlobalsPathReserved,
            dependency_span,
        )
        .into());
    }

    // Skip virtual package dependencies — AST resolution handles those.
    if external_imports
        .external_packages
        .is_virtual_package_dependency(dependency_path, path_fork, string_table)
    {
        if resolution.as_ref().is_some_and(|resolution| {
            resolution.has_binding_package_dependency(dependency_path, string_table)
        }) {
            return Ok(DependencyPolicyAction::QueueLocal);
        }
        // Extensionless binding-package clauses bind through the external package registry.
        add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
        return Ok(DependencyPolicyAction::Skip);
    }

    // Check for unsupported builder-specific core packages.
    if let Some(package_path) = external_imports
        .external_packages
        .unsupported_known_package_dependency(dependency_path, path_fork, string_table)
    {
        return Err(SourceDiscoveryError::from(
            unsupported_builder_package_error(
                package_path,
                dependency_span,
                &mut DiagnosticStringBuilder::new(string_table),
            ),
        ));
    }
    if matches!(target, DependencyTargetKind::ExternalProvider { .. }) {
        let checked = provider_target.ok_or_else(|| {
            SourceDiscoveryError::Infrastructure(InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                "retained provider classification is missing its checked target fact",
            ))
        })?;
        let prefix_path = checked.prefix_path_id();
        let extension = checked
            .extension_spelling(string_table)
            .map_err(SourceDiscoveryError::from)?
            .to_owned();
        if let Some(provider) = external_imports.providers.find_by_extension(&extension) {
            // Directory projects resolve provider-owned targets through the same boundary-aware
            // namespace as compiler-semantic dependencies; the scoped immutable reborrow finishes
            // before the mutable fork is moved into the provider request.
            let directory_target = resolution
                .map(|resolution| {
                    resolution
                        .resolve_provider_target(
                            prefix_path,
                            canonical_file,
                            dependency_span,
                            string_table,
                        )
                        .map_err(SourceDiscoveryError::from)
                })
                .transpose()?;
            let request = ProviderBackedImportRequest {
                consumer_canonical_path: canonical_file,
                import_path: dependency_path,
                source_span: dependency_span,
                prefix_path,
                raw_prefix: checked.raw_prefix(),
                provider,
                project_path_resolver,
                path_fork,
                provider_sources,
            };
            // The single-file traversal has no directory namespace; target resolution falls
            // through to the filesystem lane inside `resolve_provider_backed_import`.
            resolve_provider_backed_import(
                request,
                directory_target,
                external_imports,
                string_table,
            )
            .map_err(|error| with_provider_dependency_error(error, dependency_span))?;
            counter_observation!("stage0.reachable_discovery.provider_imports", 1.0);
            // Explicit-extension registered-provider clauses bind through the provider registry.
            add_frontend_counter(FrontendCounter::ResolvedProviderClauseCount, 1);
            return Ok(DependencyPolicyAction::Skip);
        }

        // No provider registered for this extension — report unsupported extension.
        return Err(SourceDiscoveryError::from(
            unsupported_external_extension_error(
                dependency_path,
                &extension,
                dependency_span,
                &mut DiagnosticStringBuilder::new(string_table),
            ),
        ));
    }

    Ok(DependencyPolicyAction::QueueLocal)
}

/// Attach the retained provider clause's exact authored span to a direct user diagnostic.
///
/// Provider resolution can also return infrastructure failures or a message set owned by the
/// provider itself. Those lanes retain their existing ownership and source context; only the
/// direct diagnostic produced at this dependency boundary is attributed to the clause span.
fn with_provider_dependency_span(
    mut diagnostic: DiagnosticDraft,
    source_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    diagnostic.set_primary_span(source_span);
    diagnostic
}

fn with_provider_dependency_error(
    error: SourceDiscoveryError,
    source_span: Option<SourceSpan>,
) -> SourceDiscoveryError {
    match error {
        SourceDiscoveryError::Diagnostic(mut diagnostic) => {
            diagnostic = with_provider_dependency_span(diagnostic, source_span);
            SourceDiscoveryError::Diagnostic(diagnostic)
        }
        other => other,
    }
}
fn resolved_source_file(path: &Path, kind: SourceFileKind) -> ReachableSourceFile {
    ReachableSourceFile {
        path: path.to_path_buf(),
        kind,
    }
}
