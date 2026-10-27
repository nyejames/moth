//! Final source ownership and failure publication for private discovery traversals.

use super::super::source_loading::{
    SourceReadError, classify_source_read_error, registered_source_read_error,
    retained_source_error,
};
use super::discovery_identity_rebind::{
    SourceIdentityMap, build_source_identity_map, rebind_resolved_file_reference_diagnostics,
    rebind_source_discovery_failure,
};
use super::discovery_missing_source_load::load_missing_sources;
use super::*;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use std::sync::Arc;
/// Finish only retained work on an aborted walk. Queued sources remain unloaded.
///
/// The final source owner already contains every known snapshot and original span builder. This
/// boundary only normalizes retained preparation facts and then finishes that owner so any
/// terminal rebinding failure still carries the finalized source context.
///
/// WHAT: rebinds retained warnings and the terminal diagnostic to final identities, moves the
///       live local table into a prepared batch via `mem::take`, and finishes the source
///       owner before the finalized failure escapes.
/// WHY: no intermediate vessel may carry diagnostics; the parent final boundary owns the
///      single render conversion. Infrastructure terminals stay typed and drop companion
///      warnings, matching the premerge lane contract.
pub(super) fn finalize_failed_discovery(
    mut files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    source_builder: SourceDatabaseBuilder,
    source_ids: &SourceIdentityMap,
    failure: Option<TraversalFailure>,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> SourceDiscoveryError {
    files.sort_by_key(|file| {
        source_builder
            .sources()
            .get_by_canonical_path(&file.path)
            .expect("known discovery source has a final registration")
            .id
    });
    let mut preparation_failure = None;

    for file in files {
        let Some(prepared) = source_cache.remove(&file.path) else {
            continue;
        };
        if source_builder
            .sources()
            .get_by_canonical_path(&file.path)
            .is_none()
        {
            let error = InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "retained discovery source {} has no final registration",
                    file.path.display()
                ),
            );
            return finish_discovery_source_owner(
                PreparedFailure::Infrastructure(error.into()),
                source_builder,
            );
        }
        let SourcePreparationDelta {
            file_id: _,
            span_builder: _,
            result,
        } = prepared.prepared_output;
        match result {
            Ok(_output) => {}

            Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
                preparation_failure = Some(SourceDiscoveryError::Diagnostic(error.diagnostic));
            }

            Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
                preparation_failure = Some(SourceDiscoveryError::Infrastructure(error));
            }
        }
    }

    let terminal_error = if let Some(failure) = failure {
        failure.error
    } else {
        preparation_failure.expect("an aborted discovery owns its terminal preparation failure")
    };
    let terminal_error = match rebind_source_discovery_failure(terminal_error, source_ids) {
        Ok(error) => error,
        Err(error) => {
            return finish_discovery_source_owner(
                PreparedFailure::Infrastructure(error.into()),
                source_builder,
            );
        }
    };
    let terminal_failure = match terminal_error {
        SourceDiscoveryError::Diagnostic(diagnostic) => {
            let table = string_table.take_for_handoff();
            let batch = match PreparedDiagnosticBatch::capture_with_required_paths(
                DiagnosticBag::from(diagnostic),
                table,
                None,
                || Some(Arc::new(path_fork.shared_table())),
            ) {
                Ok(batch) => batch,
                Err(error) => {
                    return finish_discovery_source_owner(
                        PreparedFailure::Infrastructure(error.into()),
                        source_builder,
                    );
                }
            };
            PreparedFailure::Diagnosed(batch)
        }
        SourceDiscoveryError::Premerge(failure) => failure,
        SourceDiscoveryError::Infrastructure(error) => {
            PreparedFailure::Infrastructure(error.into())
        }
        SourceDiscoveryError::Finalized(_) => {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "discovery failure cannot already carry a finalized source owner"
            )
        }
    };
    finish_discovery_source_owner(terminal_failure, source_builder)
}

/// Finalize every retained source at the terminal aborted-discovery outcome.
///
/// No later stage can produce spans after an aborted discovery. Loaded records without a retained
/// producer receive an empty table; each known source builder is finalized before the database
/// owner is consumed.
///
/// WHAT: finalizes the loaded source identities, checks the completed builder, and pairs the
///       resulting database with the typed failure.
/// WHY: the finalized failure must never drop the source owner, and terminal validation must not
///      perform a delayed freeze pass for span tables.
pub(super) fn finish_discovery_source_owner(
    mut failure: PreparedFailure,
    mut source_builder: SourceDatabaseBuilder,
) -> SourceDiscoveryError {
    let loaded_source_ids: Vec<_> = source_builder
        .sources()
        .iter()
        .filter_map(|slot| {
            source_builder
                .sources()
                .retained_text(slot.id)
                .map(|_| slot.id)
        })
        .collect();
    for source_id in loaded_source_ids {
        match source_builder.is_source_finalized(source_id) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                failure = failure.with_infrastructure_cause(error);
                continue;
            }
        }
        if let Err(error) = source_builder.finish_source(source_id) {
            failure = failure.with_infrastructure_cause(error);
        }
    }
    let finish_result = source_builder
        .assemble_finalized_tables()
        .and_then(|()| source_builder.finish());
    match finish_result {
        Ok(source_database) => SourceDiscoveryError::finalized(failure, source_database),
        Err(error) => SourceDiscoveryError::Premerge(failure.with_infrastructure_cause(error)),
    }
}

/// Finish the final source owner beside an infrastructure error.
///
/// WHAT: pairs a typed infrastructure error with its finished database; a builder-finish
///       failure appends another cause without replacing the original failure.
/// WHY: retention and identity-rebinding errors occur after registration, so their source
///      snapshots must still reach the final boundary.
pub(super) fn discovery_finalization_error(
    error: InfrastructureFailure,
    source_builder: SourceDatabaseBuilder,
) -> SourceDiscoveryError {
    finish_discovery_source_owner(
        PreparedFailure::Infrastructure(error.into()),
        source_builder,
    )
}

/// Move every known synthetic snapshot and its original span builder into final source ownership.
///
/// Snapshots are retained first because the builder owner accepts a span builder only after its
/// source slot is loaded. Retention continues after the first snapshot error, preserving sibling
/// source facts before the caller publishes the terminal failure; successful snapshots then receive
/// their original builders before any output identity transformation begins.
pub(super) fn retain_known_discovery_sources(
    files: &[ReachableSourceFile],
    source_cache: &mut FxHashMap<PathBuf, PreparedDiscoverySource>,
    source_builder: &mut SourceDatabaseBuilder,
    path_fork: &mut PathInternerFork,
) -> Result<(), SourceDiscoveryError> {
    let mut first_error: Option<SourceDiscoveryError> = None;
    for file in files {
        let Some(prepared) = source_cache.get_mut(&file.path) else {
            continue;
        };
        let Some(source_id) = source_builder
            .sources()
            .get_by_canonical_path(&file.path)
            .map(|source| source.id)
        else {
            first_error.get_or_insert_with(|| {
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "known discovery source {} has no final registration",
                    file.path.display()
                ))
                .into()
            });
            continue;
        };
        let source_code = std::mem::take(&mut prepared.source_code);
        let retained = source_builder
            .sources_mut()
            .retain_text(source_id, source_code);
        match retained {
            Ok(()) => {
                let span_builder = std::mem::take(&mut prepared.prepared_output.span_builder);
                source_builder.retain_span_builder(source_id, span_builder);
            }
            Err(error) => {
                first_error.get_or_insert(retained_source_error(
                    error,
                    source_id,
                    source_builder.sources(),
                    path_fork,
                ));
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Settle every missing-source read in the final owner before output rebinding.
///
/// Successful sibling snapshots are retained before the first error is published. An
/// infrastructure read failure fails its slot. Invalid UTF-8 becomes a `NON_UTF8_SOURCE`
/// diagnostic named by the source's logical path and leaves its slot `Pending`: that diagnostic is
/// the source's terminal outcome, and there is no text or span to record.
///
/// WHAT: retains every successful sibling snapshot, fails infrastructure slots, and publishes the
///       first typed error with its logical path.
/// WHY: the caller pairs this typed error with the aborted-walk finalization that finishes the
///      source owner, so snapshots stay owned until the finalized failure escapes.
pub(super) fn finalize_missing_source_loads(
    files: &[ReachableSourceFile],
    load_results: Vec<MissingSourceLoadResult>,
    source_builder: &mut SourceDatabaseBuilder,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), SourceDiscoveryError> {
    let mut failure_owner: Option<PreparedFailure> = None;

    for result in load_results {
        let input_index = missing_source_load_input_index(&result);
        let Some(source_file) = files.get(input_index) else {
            let error = InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "missing source inventory slot {} is out of range",
                    input_index
                ),
            )
            .into();
            admit_missing_source_error(&mut failure_owner, error, string_table, path_fork);
            continue;
        };
        let Some(source_id) = source_builder
            .sources()
            .get_by_canonical_path(&source_file.path)
            .map(|source| source.id)
        else {
            let error = InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "missing discovery source {} has no final registration",
                    source_file.path.display()
                ),
            )
            .into();
            admit_missing_source_error(&mut failure_owner, error, string_table, path_fork);
            continue;
        };

        match result {
            MissingSourceLoadResult::Loaded(loaded) => {
                add_frontend_counter(
                    FrontendCounter::Stage0SourceBytesLoaded,
                    loaded.source_code.len(),
                );
                let retained = source_builder
                    .sources_mut()
                    .retain_text(source_id, loaded.source_code);
                if let Err(error) = retained {
                    let error = retained_source_error(
                        error,
                        source_id,
                        source_builder.sources(),
                        path_fork,
                    );
                    admit_missing_source_error(&mut failure_owner, error, string_table, path_fork);
                }
            }
            MissingSourceLoadResult::Failed(failure) => {
                match classify_source_read_error(&failure.path, failure.error) {
                    SourceReadError::NonUtf8 => {
                        let error = registered_source_read_error(
                            SourceReadError::NonUtf8,
                            source_id,
                            source_builder.sources(),
                            path_fork,
                        );
                        admit_missing_source_error(
                            &mut failure_owner,
                            error,
                            string_table,
                            path_fork,
                        );
                    }
                    SourceReadError::Infrastructure(error) => {
                        let record_result = source_builder
                            .sources_mut()
                            .record_source_load_error(source_id, error.clone());
                        match record_result {
                            Ok(()) => admit_missing_source_error(
                                &mut failure_owner,
                                error.into(),
                                string_table,
                                path_fork,
                            ),
                            Err(record_error) => admit_missing_source_error(
                                &mut failure_owner,
                                record_error.into(),
                                string_table,
                                path_fork,
                            ),
                        }
                    }
                }
            }
        }
    }

    match failure_owner {
        Some(failure) => Err(SourceDiscoveryError::Premerge(failure)),
        None => Ok(()),
    }
}

fn admit_missing_source_error(
    failure_owner: &mut Option<PreparedFailure>,
    error: SourceDiscoveryError,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) {
    *failure_owner = Some(error.admit_into_owner(failure_owner.take(), string_table, path_fork));
}

/// Build the final source database and remap every synthetic prepared output before returning it.
///
/// WHAT: registers the complete reachable closure in canonical logical order, moves every known
///       snapshot and original span builder into final ownership, then rebinds traversal-prepared
///       outputs to final IDs.
/// WHY: header preparation needs traversal identities before the closure is complete, but no
///      traversal-domain identity may cross this discovery boundary. Ownership is established
///      before any fallible output transformation so terminal failures retain source context.
#[allow(
    clippy::too_many_arguments,
    reason = "finalization keeps reachable files, the source cache, entry and resolver state, mutable string/path forks, the deferred failure, and resolved references as separate borrows"
)]
pub(super) fn finalize_reachable_files(
    files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    entry_file_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    failure: Option<TraversalFailure>,
    resolved_file_references: &mut [SingleFileResolvedReference],
    source_package_dependencies: &mut [ResolvedSourcePackageDependency],
) -> Result<(SourceDatabaseBuilder, Vec<PreparedSourceInput>), SourceDiscoveryError> {
    let registration_index = SourceRegistrationIndex::from_rows(files.iter().map(|source_file| {
        (
            source_file.path.as_path(),
            SourceKind::Compiler(source_file.kind),
        )
    }));
    let source_files =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &registration_index,
            entry_file_path,
            Some(project_path_resolver),
            string_table,
            path_fork.fork_path_builder(),
        )?;
    let mut source_files = SourceDatabaseBuilder::new(source_files);

    if let Err(error) =
        retain_known_discovery_sources(&files, &mut source_cache, &mut source_files, path_fork)
    {
        let failure = error.into_failure(string_table, Arc::new(path_fork.shared_table()));
        return Err(finish_discovery_source_owner(failure, source_files));
    }
    let source_ids = match build_source_identity_map(&source_cache, source_files.sources()) {
        Ok(source_ids) => source_ids,
        Err(error) => return Err(discovery_finalization_error(error, source_files)),
    };

    // Every cached preparation delta has a final source identity, including the root sentinel.
    if failure.is_some()
        || source_cache
            .values()
            .any(|source| source.prepared_output.result.is_err())
    {
        return Err(finalize_failed_discovery(
            files,
            source_cache,
            source_files,
            &source_ids,
            failure,
            string_table,
            path_fork,
        ));
    }

    let missing_sources = files
        .iter()
        .enumerate()
        .filter(|(_, source_file)| !source_cache.contains_key(&source_file.path))
        .map(|(input_index, source_file)| MissingSourceFile {
            input_index,
            source_file: source_file.clone(),
        })
        .collect::<Vec<_>>();
    for source_file in &missing_sources {
        add_frontend_counter(FrontendCounter::Stage0SourceCacheMissCount, 1);
        if source_file.source_file.kind == SourceFileKind::Moth {
            let error = InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "reachable Moth source {} has no prepared traversal output",
                    source_file.source_file.path.display()
                ),
            );
            return Err(discovery_finalization_error(error, source_files));
        }
    }
    for source_file in &files {
        if source_cache.contains_key(&source_file.path) {
            add_frontend_counter(FrontendCounter::Stage0SourceCacheHitCount, 1);
        }
    }
    let load_results = load_missing_sources(missing_sources);
    if let Err(error) = finalize_missing_source_loads(
        &files,
        load_results,
        &mut source_files,
        string_table,
        path_fork,
    ) {
        let failure = TraversalFailure { error };
        return Err(finalize_failed_discovery(
            files,
            source_cache,
            source_files,
            &source_ids,
            Some(failure),
            string_table,
            path_fork,
        ));
    };
    if let Err(error) =
        rebind_resolved_file_reference_diagnostics(resolved_file_references, &source_ids)
    {
        return Err(discovery_finalization_error(error, source_files));
    }

    for dependency in source_package_dependencies {
        let shell = &mut dependency.dependency_shell_id;
        shell.source =
            match super::discovery_identity_rebind::final_source_id(shell.source, &source_ids) {
                Ok(source) => source,
                Err(error) => return Err(discovery_finalization_error(error, source_files)),
            };
    }

    // All original tables are owned before the final source-kind/input assembly.
    // Cached prepared outputs are rebound to their final source identity immediately before freeze.
    let normalized: Result<_, InfrastructureFailure> = (|| {
        let mut input_files = Vec::with_capacity(files.len());
        for source_file in files {
            let final_record = source_files
                .sources()
                .get_by_canonical_path(&source_file.path)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                        "final source database is missing reachable source {}",
                        source_file.path.display()
                    ))
                })?;
            let final_source_id = final_record.id;
            let source_kind = final_record.kind.ok_or_else(|| {
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "final source identity {} has no lexical kind",
                    final_source_id.index()
                ))
            })?;

            let source = if let Some(scanned_source) = source_cache.remove(&source_file.path) {
                let provisional_source_id = scanned_source.prepared_output.file_id;
                let mapped_source_id =
                    source_ids
                        .get(&provisional_source_id)
                        .copied()
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                                "cached discovery source identity {} has no final mapping",
                                provisional_source_id.index()
                            ))
                        })?;
                if mapped_source_id != final_source_id {
                    return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                        "cached discovery source identity {} maps to final identity {}, expected {}",
                        provisional_source_id.index(),
                        mapped_source_id.index(),
                        final_source_id.index()
                    )));
                }
                let mut output = scanned_source
                    .prepared_output
                    .result
                    .expect("diagnosed preparation exits through failed discovery finalization");
                output.rebind_source_identity(
                    final_source_id,
                    final_record.logical_path,
                    path_fork,
                )?;
                output.freeze_path_syntax(string_table, path_fork)?;
                match source_kind {
                    SourceKind::Compiler(SourceFileKind::Moth)
                    | SourceKind::Compiler(SourceFileKind::MothTemplate) => {
                        PreparedSourceKind::MothPrepared {
                            output: Box::new(output),
                        }
                    }
                    SourceKind::Compiler(SourceFileKind::PlainMarkdown) => {
                        return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, "plain Markdown cannot enter the prepared source cache"));
                    }
                    SourceKind::ProviderOwned => {
                        return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                            "final source identity {} is provider-owned and cannot be compiled",
                            final_source_id.index()
                        )));
                    }
                }
            } else {
                if source_files
                    .sources()
                    .retained_text(final_source_id)
                    .is_none()
                {
                    return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                        "source inventory slot for {} was not loaded",
                        source_file.path.display()
                    )));
                }
                match source_kind {
                    SourceKind::Compiler(
                        SourceFileKind::MothTemplate | SourceFileKind::PlainMarkdown,
                    ) => PreparedSourceKind::Deferred,
                    SourceKind::Compiler(SourceFileKind::Moth) => {
                        crate::compiler_bug!(
                            CompilerStage::ProjectBuildAssembly,
                            "Moth sources are always present in the selected cache before finalization; only MothTemplate and PlainMarkdown may be deferred without a cache hit"
                        )
                    }
                    SourceKind::ProviderOwned => {
                        return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                            "final source identity {} is provider-owned and cannot be compiled",
                            final_source_id.index()
                        )));
                    }
                }
            };
            input_files.push(PreparedSourceInput {
                source_id: final_source_id,
                source,
            });
        }
        if !source_cache.is_empty() {
            return Err(InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                "synthetic source cache contains an unreachable prepared source",
            ));
        }
        Ok(input_files)
    })();

    match normalized {
        Ok(input_files) => Ok((source_files, input_files)),
        Err(error) => Err(discovery_finalization_error(error, source_files)),
    }
}
