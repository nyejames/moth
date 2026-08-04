//! Build-owned generated-function request scheduling and sidecar storage.
//!
//! WHAT: owns one deterministic request worklist per project or package compilation boundary,
//! dense request IDs, requester/dependency records, exact completed summaries and the separate
//! generated-sidecar lane.
//! WHY: generic call inference belongs to AST, but aggregation, deduplication, fixed-point
//! scheduling and sidecar placement belong to the build boundary. A module compiles against a
//! transactional session and publishes its delta only after the module succeeds.

use crate::build_system::build::GeneratedFunctionSidecar;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, StableModuleOriginIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct GeneratedRequestId(usize);

impl GeneratedRequestId {
    pub(super) fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GeneratedRequester {
    Module(StableModuleOriginIdentity),
    Generated(GeneratedRequestId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeneratedRequestState {
    Pending,
    Materialising,
    Complete,
}

struct GeneratedRequestRecord {
    identity: GeneratedFunctionIdentity,
    display_name: String,
    diagnostic_location: SourceLocation,
    requesters: Vec<GeneratedRequester>,
    dependencies: Vec<GeneratedRequestId>,
    state: GeneratedRequestState,
}

/// One generated request as authored by AST, carrying the facts diagnostics need.
#[derive(Clone, Debug)]
pub(super) struct GeneratedRequestFacts {
    pub(super) identity: GeneratedFunctionIdentity,
    pub(super) display_name: String,
    pub(super) diagnostic_location: SourceLocation,
}

/// Result of attempting to enter one request during depth-first fixed-point materialisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GeneratedRequestEntry {
    Materialise,
    Complete,
    Recursive,
}

/// Dense index of one completed generated function inside a boundary store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GeneratedFunctionId(usize);

impl GeneratedFunctionId {
    fn index(self) -> usize {
        self.0
    }
}

/// One completed generated function with its exact identity, summary and sidecar.
///
/// WHAT: keeps the three facts one coherent record so a boundary can never align a summary with
///       the wrong sidecar or leave one of them orphaned.
/// WHY: generated summaries and sidecars are published transactionally; storing them as one row
///      removes separate publication paths and later reconstruction of the generated owner.
pub(crate) struct CompletedGeneratedFunction {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) summary: PublicCallSummary,
    pub(crate) sidecar: GeneratedFunctionSidecar,
}

/// Transactional request worklist for one module compilation.
///
/// Existing boundary summaries seed the session, while newly produced sidecars stay local until
/// the containing module has completed and its string IDs have been merged.
pub(super) struct GeneratedFunctionWorklist<'a> {
    known: &'a BoundaryGeneratedFunctionStore,
    imported: &'a CompletedGeneratedFunctionView<'a>,
    records: Vec<GeneratedRequestRecord>,
    ids_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedRequestId>,
    completed_records: Vec<CompletedGeneratedFunction>,
    completed_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl<'a> GeneratedFunctionWorklist<'a> {
    fn new(
        known: &'a BoundaryGeneratedFunctionStore,
        imported: &'a CompletedGeneratedFunctionView<'a>,
    ) -> Self {
        Self {
            known,
            imported,
            records: Vec::new(),
            ids_by_identity: FxHashMap::default(),
            completed_records: Vec::new(),
            completed_by_identity: FxHashMap::default(),
        }
    }

    pub(super) fn register_module_requests(
        &mut self,
        requester: &StableModuleOriginIdentity,
        requests: impl IntoIterator<Item = GeneratedRequestFacts>,
    ) -> Vec<GeneratedRequestId> {
        self.register_requests(GeneratedRequester::Module(requester.clone()), requests)
    }

    pub(super) fn register_generated_requests(
        &mut self,
        requester: GeneratedRequestId,
        requests: impl IntoIterator<Item = GeneratedRequestFacts>,
    ) -> Vec<GeneratedRequestId> {
        let dependency_ids =
            self.register_requests(GeneratedRequester::Generated(requester), requests);
        let record = &mut self.records[requester.index()];
        for dependency_id in &dependency_ids {
            if !record.dependencies.contains(dependency_id) {
                record.dependencies.push(*dependency_id);
            }
        }
        record.dependencies.sort_unstable();
        dependency_ids
    }

    fn register_requests(
        &mut self,
        requester: GeneratedRequester,
        requests: impl IntoIterator<Item = GeneratedRequestFacts>,
    ) -> Vec<GeneratedRequestId> {
        let mut requests = requests.into_iter().collect::<Vec<_>>();
        requests.sort_by(|left, right| left.identity.cmp(&right.identity));
        requests.dedup_by(|left, right| left.identity == right.identity);

        let mut request_ids = Vec::with_capacity(requests.len());
        for request in requests {
            if self.known.by_identity.contains_key(&request.identity)
                || self.imported.by_identity.contains_key(&request.identity)
                || self.completed_by_identity.contains_key(&request.identity)
            {
                continue;
            }

            let request_id = if let Some(request_id) = self.ids_by_identity.get(&request.identity) {
                *request_id
            } else {
                let request_id = GeneratedRequestId(self.records.len());
                self.ids_by_identity
                    .insert(request.identity.clone(), request_id);
                self.records.push(GeneratedRequestRecord {
                    identity: request.identity,
                    display_name: request.display_name,
                    diagnostic_location: request.diagnostic_location,
                    requesters: Vec::new(),
                    dependencies: Vec::new(),
                    state: GeneratedRequestState::Pending,
                });
                request_id
            };
            let record = &mut self.records[request_id.index()];
            if !record.requesters.contains(&requester) {
                record.requesters.push(requester.clone());
            }
            request_ids.push(request_id);
        }
        request_ids
    }

    pub(super) fn identity(
        &self,
        request_id: GeneratedRequestId,
    ) -> Result<&GeneratedFunctionIdentity, CompilerError> {
        self.records
            .get(request_id.index())
            .map(|record| &record.identity)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated worklist received out-of-range request id {}",
                    request_id.index()
                ))
            })
    }

    /// The display facts one request record owns for diagnostics.
    pub(super) fn request_facts(
        &self,
        request_id: GeneratedRequestId,
    ) -> Result<(String, SourceLocation), CompilerError> {
        self.records
            .get(request_id.index())
            .map(|record| {
                (
                    record.display_name.clone(),
                    record.diagnostic_location.clone(),
                )
            })
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated worklist received out-of-range request id {}",
                    request_id.index()
                ))
            })
    }

    pub(super) fn enter(
        &mut self,
        request_id: GeneratedRequestId,
    ) -> Result<GeneratedRequestEntry, CompilerError> {
        let record = self.records.get_mut(request_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated worklist received out-of-range request id {}",
                request_id.index()
            ))
        })?;
        match record.state {
            GeneratedRequestState::Pending => {
                record.state = GeneratedRequestState::Materialising;
                Ok(GeneratedRequestEntry::Materialise)
            }
            GeneratedRequestState::Materialising => Ok(GeneratedRequestEntry::Recursive),
            GeneratedRequestState::Complete => Ok(GeneratedRequestEntry::Complete),
        }
    }

    pub(super) fn complete(
        &mut self,
        request_id: GeneratedRequestId,
        summary: PublicCallSummary,
        sidecar: GeneratedFunctionSidecar,
    ) -> Result<(), CompilerError> {
        let record = self.records.get_mut(request_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated worklist received out-of-range request id {}",
                request_id.index()
            ))
        })?;
        if record.state != GeneratedRequestState::Materialising {
            return Err(CompilerError::compiler_error(format!(
                "Generated request {:?} completed from an invalid worklist state",
                record.identity
            )));
        }
        if sidecar.identity != record.identity {
            return Err(CompilerError::compiler_error(
                "Generated sidecar identity disagrees with its worklist request",
            ));
        }
        if self.completed_by_identity.contains_key(&record.identity) {
            return Err(CompilerError::compiler_error(format!(
                "Generated request {:?} completed more than once",
                record.identity
            )));
        }
        record.state = GeneratedRequestState::Complete;
        let identity = record.identity.clone();
        let generated_id = GeneratedFunctionId(self.completed_records.len());
        self.completed_records.push(CompletedGeneratedFunction {
            identity,
            summary,
            sidecar,
        });
        self.completed_by_identity
            .insert(record.identity.clone(), generated_id);
        Ok(())
    }

    pub(super) fn summary(
        &self,
        identity: &GeneratedFunctionIdentity,
    ) -> Option<&PublicCallSummary> {
        self.completed_by_identity
            .get(identity)
            .and_then(|id| self.completed_records.get(id.index()))
            .map(|record| &record.summary)
            .or_else(|| self.known.summary(identity))
            .or_else(|| self.imported.summary(identity))
    }

    pub(super) fn completed_summaries(
        &self,
    ) -> FxHashMap<GeneratedFunctionIdentity, PublicCallSummary> {
        let mut summaries = FxHashMap::default();
        for record in &self.completed_records {
            summaries.insert(record.identity.clone(), record.summary.clone());
        }
        for record in &self.known.records {
            summaries.insert(record.identity.clone(), record.summary.clone());
        }
        for record in &self.imported.records {
            summaries.insert(record.identity.clone(), record.summary.clone());
        }
        summaries
    }

    pub(super) fn sidecars_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut GeneratedFunctionSidecar> + '_ {
        self.completed_records
            .iter_mut()
            .map(|record| &mut record.sidecar)
    }

    pub(super) fn sidecar_count(&self) -> usize {
        self.completed_records.len()
    }

    pub(super) fn remap_sidecars_from(&mut self, first_sidecar: usize, remap: &StringIdRemap) {
        for record in &mut self.completed_records[first_sidecar..] {
            record.sidecar.remap_string_ids(remap);
        }
    }

    pub(super) fn update_summary(
        &mut self,
        identity: &GeneratedFunctionIdentity,
        summary: PublicCallSummary,
    ) -> Result<bool, CompilerError> {
        let record_id = self.completed_by_identity.get(identity).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated worklist cannot update unknown completed request {identity:?}"
            ))
        })?;
        let current = &mut self.completed_records[record_id.index()].summary;
        let changed = *current != summary;
        *current = summary;
        Ok(changed)
    }

    pub(super) fn summaries_for(
        &self,
        identities: impl IntoIterator<Item = GeneratedFunctionIdentity>,
    ) -> Result<FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>, CompilerError> {
        let mut summaries = FxHashMap::default();
        for identity in identities {
            let summary = self.summary(&identity).cloned().ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated request {identity:?} has no exact completed summary"
                ))
            })?;
            summaries.insert(identity, summary);
        }
        Ok(summaries)
    }

    pub(super) fn finish(self) -> Result<GeneratedFunctionWorklistDelta, CompilerError> {
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.state != GeneratedRequestState::Complete)
        {
            return Err(CompilerError::compiler_error(format!(
                "Generated worklist stopped before request {:?} completed",
                record.identity
            )));
        }
        Ok(GeneratedFunctionWorklistDelta {
            records: self.completed_records,
        })
    }
}

/// Successful new work produced while compiling one module.
pub(crate) struct GeneratedFunctionWorklistDelta {
    records: Vec<CompletedGeneratedFunction>,
}

impl GeneratedFunctionWorklistDelta {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for record in &mut self.records {
            record.sidecar.remap_string_ids(remap);
        }
    }
}

/// One project/package boundary's exact generated summaries and explicit sidecar lane.
#[derive(Default)]
pub(crate) struct BoundaryGeneratedFunctionStore {
    records: Vec<CompletedGeneratedFunction>,
    by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl BoundaryGeneratedFunctionStore {
    fn summary(&self, identity: &GeneratedFunctionIdentity) -> Option<&PublicCallSummary> {
        self.by_identity
            .get(identity)
            .and_then(|id| self.records.get(id.index()))
            .map(|record| &record.summary)
    }

    pub(super) fn session<'a>(
        &'a self,
        imported: &'a CompletedGeneratedFunctionView<'a>,
    ) -> GeneratedFunctionWorklist<'a> {
        GeneratedFunctionWorklist::new(self, imported)
    }

    pub(super) fn publish(
        &mut self,
        delta: GeneratedFunctionWorklistDelta,
    ) -> Result<(), CompilerError> {
        for record in delta.records {
            if self.by_identity.contains_key(&record.identity) {
                return Err(CompilerError::compiler_error(format!(
                    "Generated identity {:?} was published more than once in one compilation boundary",
                    record.identity
                )));
            }
            let record_id = GeneratedFunctionId(self.records.len());
            self.by_identity.insert(record.identity.clone(), record_id);
            self.records.push(record);
        }
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
        let record_id = GeneratedFunctionId(self.records.len());
        self.by_identity.insert(record.identity.clone(), record_id);
        self.records.push(record);
    }
}

/// Borrowed fixed-leaf view over every completed generated function outside one boundary.
///
/// WHAT: flattens completed package stores into one direct summary lookup without cloning
///       summaries into each consuming boundary.
/// WHY: imported generated records are immutable leaves; one borrowed view per boundary
///      compilation keeps cross-boundary lookup exact without per-module map copies.
pub(crate) struct CompletedGeneratedFunctionView<'a> {
    records: Vec<&'a CompletedGeneratedFunction>,
    by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl<'a> CompletedGeneratedFunctionView<'a> {
    pub(crate) fn new(
        stores: impl IntoIterator<Item = &'a BoundaryGeneratedFunctionStore>,
    ) -> Result<Self, CompilerError> {
        let mut records = Vec::new();
        let mut by_identity = FxHashMap::default();
        for store in stores {
            for record in &store.records {
                if by_identity.contains_key(&record.identity) {
                    return Err(CompilerError::compiler_error(format!(
                        "Generated identity {:?} is published by more than one completed source-package boundary",
                        record.identity
                    )));
                }
                let record_id = GeneratedFunctionId(records.len());
                by_identity.insert(record.identity.clone(), record_id);
                records.push(record);
            }
        }
        Ok(Self {
            records,
            by_identity,
        })
    }

    fn summary(&self, identity: &GeneratedFunctionIdentity) -> Option<&PublicCallSummary> {
        self.by_identity
            .get(identity)
            .and_then(|id| self.records.get(id.index()))
            .map(|record| &record.summary)
    }
}

#[cfg(test)]
#[path = "../tests/generated_worklist_tests.rs"]
mod tests;
