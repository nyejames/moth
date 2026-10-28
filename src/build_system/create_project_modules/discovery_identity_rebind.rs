//! Provisional source-identity rebinding for Stage 0 discovery.
//!
//! WHAT: `build_source_identity_map` joins traversal-local provisional `SourceId`s to canonical
//!       registrations in the finalized `SourceDatabase`.
//! WHY:  the cached delta is the only owner of each provisional ID. The `rebind_*` helpers
//!       (`rebind_resolved_file_reference_diagnostics`, `rebind_source_discovery_failure`)
//!       apply that map to diagnostics and source-discovery failures.
use super::*;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::source::SourceSpan;
pub(super) type SourceIdentityMap = FxHashMap<SourceId, SourceId>;

/// Join traversal-local preparation identities to the final source registrations.
///
/// Synthetic preparation mints source IDs before the complete closure can be canonically sorted.
/// The cached delta is the only owner that remembers each provisional ID, while the finalized
/// source database supplies the corresponding canonical registration.
pub(super) fn build_source_identity_map(
    source_cache: &FxHashMap<PathBuf, PreparedDiscoverySource>,
    source_files: &SourceDatabase,
) -> Result<SourceIdentityMap, InfrastructureFailure> {
    let mut source_ids = FxHashMap::default();
    source_ids.insert(SourceId::COMPILATION_ROOT, SourceId::COMPILATION_ROOT);

    for (canonical_path, prepared) in source_cache {
        let final_source_id = source_files
            .get_by_canonical_path(canonical_path)
            .map(|source| source.id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "cached discovery source {} has no final registration",
                    canonical_path.display()
                ))
            })?;
        let provisional_source_id = prepared.prepared_output.file_id;
        if let Some(previous_source_id) = source_ids.insert(provisional_source_id, final_source_id)
            && previous_source_id != final_source_id
        {
            return Err(InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "provisional source identity {} maps to final identities {} and {}",
                    provisional_source_id.index(),
                    previous_source_id.index(),
                    final_source_id.index()
                ),
            ));
        }
    }

    Ok(source_ids)
}

pub(super) fn final_source_id(
    provisional_source_id: SourceId,
    source_ids: &SourceIdentityMap,
) -> Result<SourceId, InfrastructureFailure> {
    if provisional_source_id == SourceId::COMPILATION_ROOT {
        return Ok(SourceId::COMPILATION_ROOT);
    }

    source_ids
        .get(&provisional_source_id)
        .copied()
        .ok_or_else(|| {
            InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                "synthetic discovery diagnostic references unknown provisional source identity {}",
                provisional_source_id.index()
            ))
        })
}

fn rebind_discovery_source_span(
    span: &mut SourceSpan,
    source_ids: &SourceIdentityMap,
) -> Result<(), InfrastructureFailure> {
    let source_id = final_source_id(span.source(), source_ids)?;
    *span = SourceSpan::new(source_id, span.local());
    Ok(())
}

pub(super) fn rebind_discovery_diagnostic(
    diagnostic: &mut DiagnosticDraft,
    source_ids: &SourceIdentityMap,
) -> Result<(), InfrastructureFailure> {
    diagnostic.rebind_source_ids(&mut |source_id| final_source_id(source_id, source_ids))
}

pub(super) fn rebind_discovery_batch(
    mut batch: PreparedDiagnosticBatch,
    source_ids: &SourceIdentityMap,
) -> Result<PreparedDiagnosticBatch, InfrastructureFailure> {
    batch.rebind_source_ids(&mut |source_id| final_source_id(source_id, source_ids))?;
    Ok(batch)
}

pub(super) fn rebind_discovery_failure(
    failure: PreparedFailure,
    source_ids: &SourceIdentityMap,
) -> Result<PreparedFailure, InfrastructureFailure> {
    match failure {
        PreparedFailure::Diagnosed(batch) => Ok(PreparedFailure::Diagnosed(
            rebind_discovery_batch(batch, source_ids)?,
        )),
        PreparedFailure::Infrastructure(error) => Ok(PreparedFailure::Infrastructure(error)),
    }
}

pub(super) fn rebind_source_discovery_failure(
    error: SourceDiscoveryError,
    source_ids: &SourceIdentityMap,
) -> Result<SourceDiscoveryError, InfrastructureFailure> {
    match error {
        SourceDiscoveryError::Diagnostic(mut diagnostic) => {
            rebind_discovery_diagnostic(&mut diagnostic, source_ids)?;
            Ok(SourceDiscoveryError::Diagnostic(diagnostic))
        }
        SourceDiscoveryError::Premerge(failure) => Ok(SourceDiscoveryError::Premerge(
            rebind_discovery_failure(failure, source_ids)?,
        )),
        SourceDiscoveryError::Finalized(_) => {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "traversal failure cannot already carry a finalized source owner"
            )
        }
        SourceDiscoveryError::Infrastructure(error) => {
            Ok(SourceDiscoveryError::Infrastructure(error))
        }
    }
}

pub(super) fn rebind_resolved_file_reference_diagnostics(
    references: &mut [SingleFileResolvedReference],
    source_ids: &SourceIdentityMap,
) -> Result<(), InfrastructureFailure> {
    for reference in references {
        if let SingleFileReferenceOutcome::Diagnostic(failure) = &mut reference.outcome {
            rebind_discovery_source_span(failure.span_mut(), source_ids)?;
        }
    }
    Ok(())
}
