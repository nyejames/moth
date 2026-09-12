use super::discovery_finalization::finalize_reachable_files;
use super::discovery_provider_imports::{
    ProviderBackedImportRequest, resolve_provider_backed_import, unsupported_builder_package_error,
    unsupported_external_extension_error,
};
use super::*;
use crate::compiler_frontend::symbols::interned_path::NonUtf8PathComponent;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::paths::path_normalization::{
    is_relative_dependency_path, join_and_normalize_path,
};
/// Resolve provider-backed and binding-backed dependency classes before indexed source resolution.
///
/// Directory module scheduling calls this with provider references retained by header syntax.
pub(crate) fn resolve_structural_provider_reference(
    provider: &RetainedDependencyPath,
    clause_kind: DependencyClauseKind,
    canonical_file: &Path,
    project_path_resolver: &ProjectPathResolver,
    path_fork: &PathInternerFork,
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
            canonical_file,
            project_path_resolver,
            path_fork,
            directory_dependency_resolution: Some(directory_dependency_resolution),
            string_table,
        },
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
    canonical_file: &'a Path,
    project_path_resolver: &'a ProjectPathResolver,
    path_fork: &'a PathInternerFork,
    directory_dependency_resolution: Option<DirectoryDependencyResolution<'a>>,
    string_table: &'a mut StringTable,
}

impl<'a, 'b> DependencyPolicy<'a, 'b> {
    /// Decide how to handle one dependency path.
    fn handle_dependency(
        &mut self,
        input: ProviderCapableDependencyInput<'_>,
    ) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
        match self {
            DependencyPolicy::Capable {
                external_imports: state,
            } => handle_provider_capable_dependency(input, state),
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
}

#[allow(clippy::too_many_arguments)]
fn traverse_reachable_source_files(
    entry_paths: &[PathBuf],
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    policy: &mut DependencyPolicy<'_, '_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let canonical_entry_path = fs::canonicalize(&entry_paths[0]).map_err(|error| {
        CompilerError::file_error(
            &entry_paths[0],
            format!("Failed to canonicalize entry file path: {error}"),
        )
    })?;
    let root_directory = canonical_entry_path.parent().ok_or_else(|| {
        CompilerError::compiler_error(
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
    };

    // The walk only borrows source ownership, so every terminal path reaches this barrier.
    let outcome = walk_reachable_sources(
        &mut inventory,
        &context,
        policy,
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
        failure,
        &mut resolved_file_references,
    )?;
    Ok(ReachableTraversalOutcome {
        source_files,
        input_files,
        resolved_file_references,
    })
}

fn walk_reachable_sources(
    inventory: &mut ReachableSourceInventory,
    context: &DiscoveryWalkContext<'_>,
    policy: &mut DependencyPolicy<'_, '_>,
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
            CompilerError::file_error(
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

        if next_file.kind == SourceFileKind::MothTemplate {
            // Moth template is a Moth template body with a small compile-time scope, so the
            // same-directory root may supply visible constants. Its retained output is scanned
            // here as well, allowing content references to reach a fixed point without a second
            // parse during module preparation.
            queue_same_directory_root_for_moth_template(
                &canonical_file,
                project_path_resolver,
                reachable,
                queue,
            );
        } else if next_file.kind == SourceFileKind::PlainMarkdown {
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
            SourceFileKind::PlainMarkdown => unreachable!(),
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

        for clause in dependency_clauses {
            // Stage 0 resolves each retained clause root once. Direct selections remain binding
            // facts inside the completed provider surface and never become independent source
            // paths during discovery.
            let provider = &clause.dependency;
            let dependency_path = provider.path;
            let action = policy.handle_dependency(ProviderCapableDependencyInput {
                dependency_path,
                dependency_span: Some(provider.span),
                clause_kind: clause.binding.clause_kind(),
                target: &provider.target,
                canonical_file: &canonical_file,
                project_path_resolver,
                path_fork,
                directory_dependency_resolution: None,
                string_table,
            })?;

            match action {
                DependencyPolicyAction::Skip => continue,
                DependencyPolicyAction::QueueLocal => {
                    let mut reachable_queue = ReachableQueue { reachable, queue };
                    let result = resolve_and_queue_local_dependency(
                        provider,
                        &canonical_file,
                        path_fork,
                        project_path_resolver,
                        string_table,
                        &mut reachable_queue,
                    );
                    result?;
                }
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
                    return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
                        "resolved supported content target has no registered source kind",
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
    // `MOTH_COUNTERS` requests it (summary/full).
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
pub(crate) fn discover_reachable_source_files(
    entry_point: &Path,
    project_path_resolver: &ProjectPathResolver,
    style_directives: &StyleDirectiveRegistry,
    external_imports: &mut ExternalImportDiscoveryState<'_>,
    source_file_kinds: &SourceFileKindRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<ReachableTraversalOutcome, SourceDiscoveryError> {
    let mut policy = DependencyPolicy::Capable { external_imports };

    traverse_reachable_source_files(
        &[entry_point.to_path_buf()],
        project_path_resolver,
        style_directives,
        &mut policy,
        source_file_kinds,
        resource_inputs,
        string_table,
        path_fork,
    )
}

/// Resolve a compiler-semantic Moth dependency and enqueue its indexed or synthetic-file target.
///
/// WHAT: handles cross-module root queuing, implementation-file discovery and direct dependency
///       edge retention for a dependency that is not provider-backed or a virtual package dependency.
/// WHY: one owner keeps indexed resolution, same-module queuing and graph-edge retention aligned.
///      A graph edge is retained only when indexed resolution crosses project module roots.
fn resolve_and_queue_local_dependency(
    provider: &RetainedDependencyPath,
    canonical_file: &Path,
    path_fork: &mut PathInternerFork,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    resolve_and_queue_via_filesystem(
        provider,
        canonical_file,
        path_fork,
        project_path_resolver,
        string_table,
        reachable_queue,
    )
}

/// Resolve a compiler-semantic dependency through the filesystem-backed resolver for single-file
/// synthetic compilation.
///
/// Single-file compilation has no directory source index or project module graph, so ordinary bare
/// source clauses use the prepared owning-module-root table while relative and registered-package
/// paths use the normal filesystem resolver. No dependency edges are collected because there is
/// no project module graph to populate.
fn resolve_and_queue_via_filesystem(
    provider: &RetainedDependencyPath,
    canonical_file: &Path,
    path_fork: &mut PathInternerFork,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    reachable_queue: &mut ReachableQueue<'_>,
) -> Result<(), SourceDiscoveryError> {
    let resolved = if is_relative_dependency_path(provider.path, path_fork, string_table)
        || project_path_resolver
            .source_package_root_for_dependency(provider.path, path_fork, string_table)
            .is_some()
    {
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
    add_frontend_counter(FrontendCounter::ResolvedSourcePackageClauseCount, 1);
    let resolved_source_file = resolved_source_file(&resolved.path, resolved.kind);
    if !reachable_queue.reachable.contains(&resolved_source_file) {
        reachable_queue.queue.push_back(resolved_source_file);
    }

    Ok(())
}

/// Resolve a bare dependency from the retained module-root topology before entry-root lookup.
///
/// WHAT: maps a module path to its prepared root facade, or resolves a bare dependency from the
///      declaring file's owning module root.
/// WHY: synthetic discovery has no indexed namespace, but it must still implement the same
///      module-root-relative source contract as directory discovery without allowing an entry-root
///      namesake to shadow the declaring module's source.
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

    let root_candidate =
        join_and_normalize_path(&module_root, provider, &*path_fork, string_table);
    if let Some(root_file) = project_path_resolver.module_root_file_for_directory(&root_candidate) {
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
            SourceDiscoveryError::from(CompilerError::compiler_error(format!(
                "Owning module root '{}' is outside entry root '{}'",
                module_root.display(),
                project_path_resolver.entry_root().display()
            )))
        })?;
    let module_prefix = path_fork
        .try_intern_filesystem_path(module_prefix, string_table)
        .map_err(|error| match error {
            crate::compiler_frontend::symbols::path_interner::PathInternError::NonUtf8(
                NonUtf8PathComponent { path },
            ) => SourceDiscoveryError::from(CompilerError::file_error(
                &path,
                format!(
                    "Owning module path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
            )),
            crate::compiler_frontend::symbols::path_interner::PathInternError::TableFull => {
                SourceDiscoveryError::from(CompilerError::compiler_error(
                    "path table exhausted while interning owning module path",
                ))
            }
        })?;
    let mut scratch = Vec::new();
    let module_local_provider = path_fork
        .try_join(module_prefix, provider, &mut scratch)
        .ok_or_else(|| {
            SourceDiscoveryError::from(CompilerError::compiler_error(
                "path table exhausted while joining owning module dependency path",
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
    external_imports: &mut ExternalImportDiscoveryState<'_>,
) -> Result<DependencyPolicyAction, SourceDiscoveryError> {
    let ProviderCapableDependencyInput {
        dependency_path,
        dependency_span,
        clause_kind,
        target,
        canonical_file,
        project_path_resolver,
        path_fork,
        directory_dependency_resolution,
        string_table,
    } = input;
    // `@project` is a reserved synthetic provider, not a source/package path. Keep the exact
    // root out of filesystem discovery and external-package registration only for the owning
    // project boundary; source packages must receive the structured reserved-path diagnostic.
    if is_project_globals_namespace(dependency_path, path_fork, string_table) {
        let is_owning_project_root = directory_dependency_resolution
            .is_none_or(|resolution| resolution.is_project_boundary());
        if is_project_globals_dependency(dependency_path, path_fork, string_table)
            && is_owning_project_root
        {
            return Ok(DependencyPolicyAction::Skip);
        }
        return Err(CompilerDiagnostic::invalid_dependency_clause(
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
        if directory_dependency_resolution.is_some_and(|resolution| {
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
            unsupported_builder_package_error(package_path, dependency_span, string_table),
        ));
    }

    // Consume the retained provider classification. Header syntax already identified the
    // first explicit non-source extension, so Stage 0 must not rescan path components.
    if let Some(decoded) = decode_dependency_target(dependency_path, target, path_fork, string_table)
        .map_err(SourceDiscoveryError::from)?
    {
        let prefix_path = decoded.prefix_path_id();
        let prefix_str = path_fork.render_portable(prefix_path, string_table, &mut Vec::new());
        let extension = decoded.extension_spelling().to_owned();
        if let Some(provider) = external_imports.providers.find_by_extension(&extension) {
            let result = resolve_provider_backed_import(
                ProviderBackedImportRequest {
                    consumer_canonical_path: canonical_file,
                    import_path: dependency_path,
                    source_span: dependency_span,
                    prefix_path,
                    raw_prefix: &prefix_str,
                    provider,
                    project_path_resolver,
                    path_fork,
                    directory_dependency_resolution,
                },
                external_imports,
                string_table,
            );
            if let Err(error) = result {
                return Err(with_provider_dependency_error(error, dependency_span));
            }
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
                string_table,
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
    mut diagnostic: CompilerDiagnostic,
    source_span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    diagnostic.primary_span = source_span;
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

fn queue_same_directory_root_for_moth_template(
    moth_template_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    reachable: &BTreeSet<ReachableSourceFile>,
    queue: &mut VecDeque<ReachableSourceFile>,
) {
    let Some(directory) = moth_template_path.parent() else {
        return;
    };

    let Some(root_path) = project_path_resolver.module_root_file_for_directory(directory) else {
        return;
    };

    let root_source_file = ReachableSourceFile {
        path: root_path,
        kind: SourceFileKind::Moth,
    };
    if !reachable.contains(&root_source_file) {
        queue.push_back(root_source_file);
    }
}
