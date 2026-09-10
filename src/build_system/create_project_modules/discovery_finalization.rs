use super::discovery_identity_rebind::{
    SourceIdentityMap, build_source_identity_map, rebind_discovery_diagnostics,
    rebind_resolved_file_reference_diagnostics, rebind_source_discovery_failure,
};
use super::discovery_missing_source_load::load_missing_sources;
use super::*;
/// Finish only retained work on an aborted walk. Queued sources remain unloaded.
///
/// The final source owner already contains every known snapshot and original span builder. This
/// boundary only normalizes retained preparation facts and then finishes that owner so any
/// terminal rebinding failure still carries the finalized source context.
///
/// WHAT: rebinds retained warnings and the terminal diagnostic to final identities, moves the
///       live local table into a diagnosed batch via `mem::take`, and finishes the source
///       owner before the finalized failure escapes.
/// WHY: no intermediate vessel may carry diagnostics; the parent final boundary owns the
///      single conversion and attaches the finished database. Infrastructure terminals stay
///      typed and drop companion warnings, matching the premerge lane contract.
pub(super) fn finalize_failed_discovery(
    mut files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    source_builder: SourceDatabaseBuilder,
    source_ids: &SourceIdentityMap,
    failure: Option<TraversalFailure>,
    string_table: &mut StringTable,
) -> SourceDiscoveryError {
    files.sort_by_key(|file| {
        source_builder
            .sources()
            .get_by_canonical_path(&file.path)
            .expect("known discovery source has a final registration")
            .id
    });
    let mut warnings = Vec::new();
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
            let error = CompilerError::compiler_error(format!(
                "retained discovery source {} has no final registration",
                file.path.display()
            ));
            return finish_discovery_source_owner(
                PremergeFailure::Infrastructure(error),
                source_builder,
            );
        }
        let SourcePreparationDelta {
            file_id: _,
            span_builder: _,
            result,
        } = prepared.prepared_output;
        match result {
            Ok(mut output) => {
                if let Err(error) = rebind_discovery_diagnostics(&mut output.warnings, source_ids) {
                    return finish_discovery_source_owner(
                        PremergeFailure::Infrastructure(error),
                        source_builder,
                    );
                }
                warnings.append(&mut output.warnings);
            }

            Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
                if let Err(remap_error) =
                    rebind_discovery_diagnostics(&mut error.warnings, source_ids)
                {
                    return finish_discovery_source_owner(
                        PremergeFailure::Infrastructure(remap_error),
                        source_builder,
                    );
                }
                warnings.extend(error.warnings);
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
                PremergeFailure::Infrastructure(error),
                source_builder,
            );
        }
    };
    let terminal_failure = match terminal_error {
        SourceDiscoveryError::Diagnostic(diagnostic) => {
            let table = std::mem::take(string_table);
            let mut diagnostics = warnings;
            diagnostics.push(diagnostic);
            PremergeFailure::Diagnosed(PremergeDiagnosticBatch::from_diagnostics(
                diagnostics,
                table,
            ))
        }
        SourceDiscoveryError::Premerge(mut failure) => {
            match &mut failure {
                PremergeFailure::Diagnosed(batch) | PremergeFailure::Mixed { batch, .. } => {
                    batch.prepend_diagnostics(warnings);
                }
                PremergeFailure::Infrastructure(_) => {}
            }
            failure
        }
        SourceDiscoveryError::Infrastructure(error) => PremergeFailure::Infrastructure(error),
        SourceDiscoveryError::Finalized(_) => {
            unreachable!("discovery failure cannot already carry a finalized source owner")
        }
    };
    finish_discovery_source_owner(terminal_failure, source_builder)
}

/// Finish the final source owner beside a typed failure.
///
/// Every known source snapshot and original builder has already moved into `source_builder`.
/// Finishing here is the only path that can publish those tables after a post-registration error.
///
/// WHAT: finishes the builder, then pairs the finished database with the typed failure.
///       A builder-finish failure supersedes the prior failure as typed infrastructure,
///       matching the package-boundary `?` precedent; there is no finished database to
///       preserve in that case.
/// WHY: the finalized failure must never drop the finished owner, and no intermediate
///      vessel may be constructed before the parent final boundary.
pub(super) fn finish_discovery_source_owner(
    failure: PremergeFailure,
    source_builder: SourceDatabaseBuilder,
) -> SourceDiscoveryError {
    match source_builder.finish() {
        Ok(source_database) => SourceDiscoveryError::finalized(failure, source_database),
        Err(error) => SourceDiscoveryError::Infrastructure(error),
    }
}

/// Finish the final source owner beside an infrastructure error.
///
/// WHAT: pairs a typed infrastructure error with its finished database; a builder-finish
///       failure supersedes as typed infrastructure with no database to preserve.
/// WHY: retention and identity-rebinding errors occur after registration, so their source
///      snapshots must still reach the final boundary.
pub(super) fn discovery_finalization_error(
    error: CompilerError,
    source_builder: SourceDatabaseBuilder,
) -> SourceDiscoveryError {
    finish_discovery_source_owner(PremergeFailure::Infrastructure(error), source_builder)
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
) -> Result<(), CompilerError> {
    let mut first_error = None;
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
                CompilerError::compiler_error(format!(
                    "known discovery source {} has no final registration",
                    file.path.display()
                ))
            });
            continue;
        };
        let source_code = std::mem::take(&mut prepared.source_code);
        match source_builder
            .sources_mut()
            .retain_text(source_id, source_code)
        {
            Ok(()) => {
                let span_builder = std::mem::take(&mut prepared.prepared_output.span_builder);
                source_builder.retain_span_builder(source_id, span_builder);
            }
            Err(error) => {
                first_error.get_or_insert(error);
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
/// Read failures remain infrastructure diagnostics, but their source slots still become terminal
/// failures. Successful siblings are retained before the first error is published, so finalizing
/// a diagnosed discovery cannot leave a pending slot or discard a usable snapshot.
///
/// WHAT: retains every successful sibling snapshot before publishing the first typed
///       infrastructure error; no intermediate vessel is constructed.
/// WHY: the caller pairs this typed error with the aborted-walk finalization that finishes
///      the source owner, so snapshots stay owned until the finalized failure escapes.
pub(super) fn finalize_missing_source_loads(
    files: &[ReachableSourceFile],
    load_results: Vec<MissingSourceLoadResult>,
    source_builder: &mut SourceDatabaseBuilder,
) -> Result<(), CompilerError> {
    let mut first_error = None;

    for result in load_results {
        let input_index = missing_source_load_input_index(&result);
        let Some(source_file) = files.get(input_index) else {
            first_error.get_or_insert_with(|| {
                CompilerError::compiler_error(format!(
                    "missing source inventory slot {} is out of range",
                    input_index
                ))
            });
            continue;
        };
        let Some(source_id) = source_builder
            .sources()
            .get_by_canonical_path(&source_file.path)
            .map(|source| source.id)
        else {
            first_error.get_or_insert_with(|| {
                CompilerError::compiler_error(format!(
                    "missing discovery source {} has no final registration",
                    source_file.path.display()
                ))
            });
            continue;
        };

        match result {
            MissingSourceLoadResult::Loaded(loaded) => {
                add_frontend_counter(
                    FrontendCounter::Stage0SourceBytesLoaded,
                    loaded.source_code.len(),
                );
                match source_builder
                    .sources_mut()
                    .retain_text(source_id, loaded.source_code)
                {
                    Ok(()) => {}
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            MissingSourceLoadResult::Failed(failure) => {
                let error = source_read_error(&failure.path, failure.error);
                let record_result = source_builder
                    .sources_mut()
                    .record_source_load_error(source_id, error.clone());
                match record_result {
                    Ok(()) => {
                        first_error.get_or_insert(error);
                    }
                    Err(record_error) => {
                        first_error.get_or_insert(record_error);
                    }
                }
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Build the final source database and remap every synthetic prepared output before returning it.
///
/// WHAT: registers the complete reachable closure in canonical logical order, moves every known
///       snapshot and original span builder into final ownership, then rebinds traversal-prepared
///       outputs to final IDs.
/// WHY: header preparation needs traversal identities before the closure is complete, but no
///      traversal-domain identity may cross this discovery boundary. Ownership is established
///      before any fallible output transformation so terminal failures retain source context.
pub(super) fn finalize_reachable_files(
    files: Vec<ReachableSourceFile>,
    mut source_cache: FxHashMap<PathBuf, PreparedDiscoverySource>,
    entry_file_path: &Path,
    project_path_resolver: &ProjectPathResolver,
    string_table: &mut StringTable,
    failure: Option<TraversalFailure>,
    resolved_file_references: &mut [SingleFileResolvedReference],
) -> Result<(SourceDatabaseBuilder, Vec<PreparedSourceInput>), SourceDiscoveryError> {
    let registration_index = SourceRegistrationIndex::from_rows(files.iter().map(|source_file| {
        (
            source_file.path.as_path(),
            SourceKind::Compiler(source_file.kind),
        )
    }));
    let source_files = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        entry_file_path,
        Some(project_path_resolver),
        string_table,
    )?;
    let mut source_files = SourceDatabaseBuilder::new(source_files);

    if let Err(error) = retain_known_discovery_sources(&files, &mut source_cache, &mut source_files)
    {
        return Err(discovery_finalization_error(error, source_files));
    }
    let source_ids = match build_source_identity_map(&source_cache, source_files.sources()) {
        Ok(source_ids) => source_ids,
        Err(error) => return Err(discovery_finalization_error(error, source_files)),
    };

    // Every cached preparation delta now has a final source identity, including the root sentinel.
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
            let error = CompilerError::compiler_error(format!(
                "reachable Moth source {} has no prepared traversal output",
                source_file.source_file.path.display()
            ));
            return Err(discovery_finalization_error(error, source_files));
        }
    }
    for source_file in &files {
        if source_cache.contains_key(&source_file.path) {
            add_frontend_counter(FrontendCounter::Stage0SourceCacheHitCount, 1);
        }
    }
    let load_results = load_missing_sources(missing_sources);
    if let Err(error) = finalize_missing_source_loads(&files, load_results, &mut source_files) {
        let failure = TraversalFailure {
            error: SourceDiscoveryError::Infrastructure(error),
        };
        return Err(finalize_failed_discovery(
            files,
            source_cache,
            source_files,
            &source_ids,
            Some(failure),
            string_table,
        ));
    };
    if let Err(error) =
        rebind_resolved_file_reference_diagnostics(resolved_file_references, &source_ids)
    {
        return Err(discovery_finalization_error(error, source_files));
    }

    // All original tables are owned before the final source-kind/input assembly.
    // Cached prepared outputs are rebound to their final source identity immediately before freeze.
    let normalized: Result<_, CompilerError> = (|| {
        let mut input_files = Vec::with_capacity(files.len());
        for source_file in files {
            let final_record = source_files
                .sources()
                .get_by_canonical_path(&source_file.path)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "final source database is missing reachable source {}",
                        source_file.path.display()
                    ))
                })?;
            let final_source_id = final_record.id;
            let source_kind = final_record.kind.ok_or_else(|| {
                CompilerError::compiler_error(format!(
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
                            CompilerError::compiler_error(format!(
                                "cached discovery source identity {} has no final mapping",
                                provisional_source_id.index()
                            ))
                        })?;
                if mapped_source_id != final_source_id {
                    return Err(CompilerError::compiler_error(format!(
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
                    source_files.sources().legacy_logical_path(final_source_id),
                    source_file.path.clone(),
                )?;
                output.freeze_path_syntax(string_table)?;
                match source_kind {
                    SourceKind::Compiler(SourceFileKind::Moth) => {
                        PreparedSourceKind::MothPrepared {
                            output: Box::new(output),
                        }
                    }
                    SourceKind::Compiler(SourceFileKind::MothTemplate) => {
                        PreparedSourceKind::MothTemplatePrepared {
                            output: Box::new(output),
                        }
                    }
                    SourceKind::Compiler(SourceFileKind::PlainMarkdown) => {
                        return Err(CompilerError::compiler_error(
                            "plain Markdown cannot enter the prepared source cache",
                        ));
                    }
                    SourceKind::ProviderOwned => {
                        return Err(CompilerError::compiler_error(format!(
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
                    return Err(CompilerError::compiler_error(format!(
                        "source inventory slot for {} was not loaded",
                        source_file.path.display()
                    )));
                }
                match source_kind {
                    SourceKind::Compiler(SourceFileKind::MothTemplate) => {
                        PreparedSourceKind::MothTemplate
                    }
                    SourceKind::Compiler(SourceFileKind::PlainMarkdown) => {
                        PreparedSourceKind::PlainMarkdown
                    }
                    SourceKind::Compiler(SourceFileKind::Moth) => {
                        unreachable!("Moth sources were handled above")
                    }
                    SourceKind::ProviderOwned => {
                        return Err(CompilerError::compiler_error(format!(
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
            return Err(CompilerError::compiler_error(
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
