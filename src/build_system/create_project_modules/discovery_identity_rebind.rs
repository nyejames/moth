use super::*;
pub(super) type SourceIdentityMap = FxHashMap<SourceId, SourceId>;

/// Join traversal-local preparation identities to the final source registrations.
///
/// Synthetic preparation mints source IDs before the complete closure can be canonically sorted.
/// The cached delta is the only owner that remembers each provisional ID, while the finalized
/// source database supplies the corresponding canonical registration.
pub(super) fn build_source_identity_map(
    source_cache: &FxHashMap<PathBuf, PreparedDiscoverySource>,
    source_files: &SourceDatabase,
) -> Result<SourceIdentityMap, CompilerError> {
    let mut source_ids = FxHashMap::default();
    source_ids.insert(SourceId::COMPILATION_ROOT, SourceId::COMPILATION_ROOT);

    for (canonical_path, prepared) in source_cache {
        let final_source_id = source_files
            .get_by_canonical_path(canonical_path)
            .map(|source| source.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "cached discovery source {} has no final registration",
                    canonical_path.display()
                ))
            })?;
        let provisional_source_id = prepared.prepared_output.file_id;
        if let Some(previous_source_id) = source_ids.insert(provisional_source_id, final_source_id)
            && previous_source_id != final_source_id
        {
            return Err(CompilerError::compiler_error(format!(
                "provisional source identity {} maps to final identities {} and {}",
                provisional_source_id.index(),
                previous_source_id.index(),
                final_source_id.index()
            )));
        }
    }

    Ok(source_ids)
}

pub(super) fn final_source_id(
    provisional_source_id: SourceId,
    source_ids: &SourceIdentityMap,
) -> Result<SourceId, CompilerError> {
    if provisional_source_id == SourceId::COMPILATION_ROOT {
        return Ok(SourceId::COMPILATION_ROOT);
    }

    source_ids
        .get(&provisional_source_id)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "synthetic discovery diagnostic references unknown provisional source identity {}",
                provisional_source_id.index()
            ))
        })
}

pub(super) fn rebind_discovery_span(
    span: &mut Option<SourceSpan>,
    source_ids: &SourceIdentityMap,
) -> Result<(), CompilerError> {
    if let Some(span) = span {
        let source_id = final_source_id(span.source(), source_ids)?;
        *span = SourceSpan::new(source_id, span.local());
    }
    Ok(())
}

pub(super) fn rebind_discovery_diagnostic(
    diagnostic: &mut CompilerDiagnostic,
    source_ids: &SourceIdentityMap,
) -> Result<(), CompilerError> {
    rebind_discovery_span(&mut diagnostic.primary_span, source_ids)?;
    for label in &mut diagnostic.labels {
        rebind_discovery_span(&mut label.span, source_ids)?;
    }
    Ok(())
}

pub(super) fn rebind_discovery_diagnostics(
    diagnostics: &mut [CompilerDiagnostic],
    source_ids: &SourceIdentityMap,
) -> Result<(), CompilerError> {
    for diagnostic in diagnostics {
        rebind_discovery_diagnostic(diagnostic, source_ids)?;
    }
    Ok(())
}

pub(super) fn rebind_discovery_compiler_error(
    error: &mut CompilerError,
    source_ids: &SourceIdentityMap,
) -> Result<(), CompilerError> {
    rebind_discovery_span(&mut error.source_span, source_ids)
}

pub(super) fn rebind_discovery_batch(
    batch: PremergeDiagnosticBatch,
    source_ids: &SourceIdentityMap,
) -> Result<PremergeDiagnosticBatch, CompilerError> {
    let (bag, string_table, render_type_contexts, render_path_contexts) = batch.into_parts();
    let mut diagnostics = bag.into_diagnostics();
    rebind_discovery_diagnostics(&mut diagnostics, source_ids)?;
    Ok(PremergeDiagnosticBatch::from_parts(
        diagnostics,
        string_table,
        render_type_contexts,
        render_path_contexts,
    ))
}

pub(super) fn rebind_discovery_failure(
    failure: PremergeFailure,
    source_ids: &SourceIdentityMap,
) -> Result<PremergeFailure, CompilerError> {
    match failure {
        PremergeFailure::Diagnosed(batch) => Ok(PremergeFailure::Diagnosed(
            rebind_discovery_batch(batch, source_ids)?,
        )),
        PremergeFailure::Infrastructure(mut error) => {
            rebind_discovery_compiler_error(&mut error, source_ids)?;
            Ok(PremergeFailure::Infrastructure(error))
        }
        PremergeFailure::Mixed { batch, mut error } => {
            let batch = rebind_discovery_batch(batch, source_ids)?;
            rebind_discovery_compiler_error(&mut error, source_ids)?;
            Ok(PremergeFailure::Mixed { batch, error })
        }
    }
}

pub(super) fn rebind_source_discovery_failure(
    error: SourceDiscoveryError,
    source_ids: &SourceIdentityMap,
) -> Result<SourceDiscoveryError, CompilerError> {
    match error {
        SourceDiscoveryError::Diagnostic(mut diagnostic) => {
            rebind_discovery_diagnostic(&mut diagnostic, source_ids)?;
            Ok(SourceDiscoveryError::Diagnostic(diagnostic))
        }
        SourceDiscoveryError::Premerge(failure) => Ok(SourceDiscoveryError::Premerge(
            rebind_discovery_failure(failure, source_ids)?,
        )),
        SourceDiscoveryError::Finalized(_) => {
            unreachable!("traversal failure cannot already carry a finalized source owner")
        }
        SourceDiscoveryError::Infrastructure(mut error) => {
            rebind_discovery_compiler_error(&mut error, source_ids)?;
            Ok(SourceDiscoveryError::Infrastructure(error))
        }
    }
}

pub(super) fn rebind_resolved_file_reference_diagnostics(
    references: &mut [SingleFileResolvedReference],
    source_ids: &SourceIdentityMap,
) -> Result<(), CompilerError> {
    for reference in references {
        if let SingleFileReferenceOutcome::Diagnostic(diagnostic) = &mut reference.outcome {
            rebind_discovery_diagnostic(diagnostic, source_ids)?;
        }
    }
    Ok(())
}
