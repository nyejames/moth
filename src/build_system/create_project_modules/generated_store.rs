//! Build-owned generated-function storage and transactional publication.
//!
//! WHAT: one boundary store per project or package compilation, holding every completed generated
//!       record in deterministic publication order, plus the preflight and commit that publish one
//!       module's delta atomically.
//! WHY: aggregation, deduplication against already published work, storage and placement belong to
//!      the owning build boundary. Materialising a request, validating its HIR, borrow-checking it
//!      and converging its summaries do not: the compiler completes that work inside one module
//!      transaction and hands back a finished delta. Equal generated identities may coexist in
//!      unrelated boundaries and are never suppressed or resolved across them.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::module_compilation::{
    CompletedGeneratedFunction, GeneratedFunctionDelta, GeneratedFunctionId,
    GeneratedFunctionSidecar, KnownGeneratedFunctions, validate_completed_generated_record,
};
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;

use rustc_hash::{FxHashMap, FxHashSet};

/// Preflight receipt proving one delta may be committed to a boundary store.
#[derive(Debug)]
pub(crate) struct GeneratedFunctionPublication {
    record_count: usize,
}

/// One project/package boundary's exact generated summaries and explicit sidecar lane.
#[derive(Default)]
pub(crate) struct BoundaryGeneratedFunctionStore {
    records: Vec<CompletedGeneratedFunction>,
    by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl BoundaryGeneratedFunctionStore {
    /// Lend an immutable view of everything this boundary has already published.
    ///
    /// WHY: a module transaction reuses published generated work but must never write through to
    ///      the store. The view carries exactly the membership and summary lookups it needs.
    pub(crate) fn known_generated(&self) -> KnownGeneratedFunctions<'_> {
        KnownGeneratedFunctions::new(&self.records, &self.by_identity)
    }

    pub(crate) fn preflight(
        &self,
        delta: &GeneratedFunctionDelta,
    ) -> Result<GeneratedFunctionPublication, CompilerError> {
        // Preflight the complete delta before mutation: identity/sidecar agreement, executable
        // record shape, duplicate identities inside the delta and duplicates against retained
        // state must all pass before any row is appended.
        let mut delta_identities = FxHashSet::default();
        for record in delta.records() {
            if record.identity != record.sidecar.identity {
                return Err(CompilerError::compiler_error(format!(
                    "Generated sidecar identity {:?} disagrees with its record identity {:?}",
                    record.sidecar.identity, record.identity
                )));
            }
            validate_completed_generated_record(record)?;
            if !delta_identities.insert(record.identity.clone()) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated identity {:?} is duplicated inside one publication delta",
                    record.identity
                )));
            }
            if self.by_identity.contains_key(&record.identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated identity {:?} was published more than once in one compilation boundary",
                    record.identity
                )));
            }
        }

        Ok(GeneratedFunctionPublication {
            record_count: delta.records().len(),
        })
    }

    pub(crate) fn commit(
        &mut self,
        publication: GeneratedFunctionPublication,
        delta: GeneratedFunctionDelta,
    ) {
        let records = delta.into_records();
        debug_assert_eq!(publication.record_count, records.len());
        for record in records {
            let record_id = GeneratedFunctionId::new(self.records.len());
            self.by_identity.insert(record.identity.clone(), record_id);
            self.records.push(record);
        }
    }

    pub(crate) fn reserve_commit(&mut self, publication: &GeneratedFunctionPublication) {
        self.records.reserve(publication.record_count);
        self.by_identity.reserve(publication.record_count);
    }

    #[cfg(test)]
    pub(crate) fn publish(&mut self, delta: GeneratedFunctionDelta) -> Result<(), CompilerError> {
        let publication = self.preflight(&delta)?;
        self.reserve_commit(&publication);
        self.commit(publication, delta);
        Ok(())
    }

    /// Borrow this boundary's completed sidecars in deterministic publication order.
    pub(crate) fn sidecars(&self) -> impl Iterator<Item = &GeneratedFunctionSidecar> + '_ {
        self.records.iter().map(|record| &record.sidecar)
    }

    /// Resolve one completed sidecar by its dense publication index.
    pub(crate) fn sidecar_at(
        &self,
        index: usize,
    ) -> Result<&GeneratedFunctionSidecar, CompilerError> {
        self.records
            .get(index)
            .map(|record| &record.sidecar)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated sidecar index {index} is out of range for this boundary"
                ))
            })
    }

    /// Append one completed record for focused tests that build real boundary payloads.
    #[cfg(test)]
    pub(crate) fn push_completed_for_test(&mut self, record: CompletedGeneratedFunction) {
        let record_id = GeneratedFunctionId::new(self.records.len());
        self.by_identity.insert(record.identity.clone(), record_id);
        self.records.push(record);
    }
}

#[cfg(test)]
#[path = "../tests/generated_store_tests.rs"]
mod tests;
