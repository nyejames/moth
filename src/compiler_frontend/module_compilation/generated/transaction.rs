//! Per-module generated-function transaction.
//!
//! WHAT: the request state machine one module compilation runs to reach its local generated fixed
//!       point — request registration and deduplication against already published work, recursion
//!       detection, completed record storage and the final delta.
//! WHY:  reaching that fixed point is compiler semantics: it interleaves materialisation, HIR
//!       lowering, borrow analysis and summary convergence. The boundary store owns what happens
//!       afterwards. A transaction publishes nothing: a diagnosed module simply drops it.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::generated::artefacts::{
    CompletedGeneratedFunction, GeneratedFunctionDelta, GeneratedFunctionId,
    GeneratedFunctionSidecar,
};
use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::source::SourceSpan;

use rustc_hash::{FxHashMap, FxHashSet};

/// Dense index of one request inside this transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GeneratedRequestId(usize);

impl GeneratedRequestId {
    fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GeneratedRequestState {
    Pending,
    Materialising,
    Complete,
}

struct GeneratedRequestRecord {
    identity: GeneratedFunctionIdentity,
    display_name: String,
    call_span: Option<SourceSpan>,
    state: GeneratedRequestState,
}

/// One generated request as authored by AST, carrying the call span diagnostics need.
#[derive(Clone, Debug)]
pub(crate) struct GeneratedRequestFacts {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) display_name: String,
    pub(crate) call_span: Option<SourceSpan>,
}

/// Result of attempting to enter one request during depth-first fixed-point materialisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeneratedRequestEntry {
    Materialise,
    Complete,
    Recursive,
}

/// Transactional request state for one module compilation.
///
/// Already published boundary summaries seed the transaction through an immutable view, while
/// newly produced sidecars stay local until the module has completed and its string IDs have been
/// merged.
pub(crate) struct GeneratedFunctionTransaction<'a> {
    known: KnownGeneratedFunctions<'a>,
    records: Vec<GeneratedRequestRecord>,
    ids_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedRequestId>,
    completed_records: Vec<CompletedGeneratedFunction>,
    completed_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
    /// Transient warnings keyed separately from warning-free sidecar payloads through convergence.
    completed_warnings: FxHashMap<GeneratedFunctionIdentity, DiagnosticBag>,
}

impl<'a> GeneratedFunctionTransaction<'a> {
    pub(crate) fn new(known: KnownGeneratedFunctions<'a>) -> Self {
        Self {
            known,
            records: Vec::new(),
            ids_by_identity: FxHashMap::default(),
            completed_records: Vec::new(),
            completed_by_identity: FxHashMap::default(),
            completed_warnings: FxHashMap::default(),
        }
    }

    /// Register newly emitted requests, skipping anything already published or completed here.
    pub(crate) fn register_requests(
        &mut self,
        requests: impl IntoIterator<Item = GeneratedRequestFacts>,
    ) -> Vec<GeneratedRequestId> {
        let mut requests = requests.into_iter().collect::<Vec<_>>();
        requests.sort_by(|left, right| left.identity.cmp(&right.identity));
        requests.dedup_by(|left, right| left.identity == right.identity);

        let mut request_ids = Vec::with_capacity(requests.len());
        for request in requests {
            if self.known.contains(&request.identity)
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
                    call_span: request.call_span,
                    state: GeneratedRequestState::Pending,
                });
                request_id
            };
            request_ids.push(request_id);
        }
        request_ids
    }

    pub(crate) fn identity(&self, request_id: GeneratedRequestId) -> &GeneratedFunctionIdentity {
        self.records
            .get(request_id.index())
            .map(|record| &record.identity)
            .unwrap_or_else(|| out_of_range(request_id))
    }

    /// The display facts one request owns for diagnostics.
    pub(crate) fn request_facts(
        &self,
        request_id: GeneratedRequestId,
    ) -> (String, Option<SourceSpan>) {
        self.records
            .get(request_id.index())
            .map(|record| (record.display_name.clone(), record.call_span))
            .unwrap_or_else(|| out_of_range(request_id))
    }

    pub(crate) fn enter(&mut self, request_id: GeneratedRequestId) -> GeneratedRequestEntry {
        let record = self
            .records
            .get_mut(request_id.index())
            .unwrap_or_else(|| out_of_range(request_id));
        match record.state {
            GeneratedRequestState::Pending => {
                record.state = GeneratedRequestState::Materialising;
                GeneratedRequestEntry::Materialise
            }
            GeneratedRequestState::Materialising => GeneratedRequestEntry::Recursive,
            GeneratedRequestState::Complete => GeneratedRequestEntry::Complete,
        }
    }

    pub(crate) fn complete(
        &mut self,
        request_id: GeneratedRequestId,
        summary: PublicCallSummary,
        sidecar: GeneratedFunctionSidecar,
        warnings: DiagnosticBag,
    ) {
        let record = self
            .records
            .get_mut(request_id.index())
            .unwrap_or_else(|| out_of_range(request_id));
        if record.state != GeneratedRequestState::Materialising {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated request {:?} completed from an invalid transaction state",
                record.identity
            );
        }
        if sidecar.identity != record.identity {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated sidecar identity disagrees with its request"
            );
        }
        if self.completed_by_identity.contains_key(&record.identity) {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated request {:?} completed more than once",
                record.identity
            );
        }

        record.state = GeneratedRequestState::Complete;
        let identity = record.identity.clone();
        let generated_id = GeneratedFunctionId::new(self.completed_records.len());
        self.completed_by_identity
            .insert(identity.clone(), generated_id);
        self.completed_records.push(CompletedGeneratedFunction {
            identity: identity.clone(),
            summary,
            sidecar,
        });
        self.completed_warnings.insert(identity, warnings);
    }

    /// The current summary for one generated function, local first and then already published.
    pub(crate) fn summary(
        &self,
        identity: &GeneratedFunctionIdentity,
    ) -> Option<&PublicCallSummary> {
        self.completed_by_identity
            .get(identity)
            .and_then(|id| self.completed_records.get(id.index()))
            .map(|record| &record.summary)
            .or_else(|| self.known.summary(identity))
    }

    pub(crate) fn completed_link_facts(
        &self,
    ) -> impl Iterator<Item = (&GeneratedFunctionIdentity, &HirModuleLinkFacts)> + '_ {
        self.completed_records.iter().map(|record| {
            (
                &record.identity,
                &record.sidecar.module.link_facts.functions,
            )
        })
    }

    pub(crate) fn sidecar_mut(
        &mut self,
        identity: &GeneratedFunctionIdentity,
    ) -> &mut GeneratedFunctionSidecar {
        let record_id = self.completed_by_identity.get(identity).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction cannot find sidecar {identity:?}"
            )
        });
        self.completed_records
            .get_mut(record_id.index())
            .map(|record| &mut record.sidecar)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated transaction sidecar index for {identity:?} is out of range"
                )
            })
    }
    pub(crate) fn sidecar_and_warnings_mut(
        &mut self,
        identity: &GeneratedFunctionIdentity,
    ) -> (&mut GeneratedFunctionSidecar, &mut DiagnosticBag) {
        let record_id = self.completed_by_identity.get(identity).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction cannot find sidecar {identity:?}"
            )
        });
        let sidecar = &mut self
            .completed_records
            .get_mut(record_id.index())
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated transaction sidecar index for {identity:?} is out of range"
                )
            })
            .sidecar;
        let warnings = self
            .completed_warnings
            .get_mut(identity)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated transaction cannot find warnings for sidecar {identity:?}"
                )
            });
        (sidecar, warnings)
    }

    pub(crate) fn summary_mut(
        &mut self,
        identity: &GeneratedFunctionIdentity,
    ) -> &mut PublicCallSummary {
        let record_id = self.completed_by_identity.get(identity).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction cannot update unknown completed request {identity:?}"
            )
        });
        self.completed_records
            .get_mut(record_id.index())
            .map(|record| &mut record.summary)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated transaction summary index for {identity:?} is out of range"
                )
            })
    }

    /// Retain only completed sidecars reachable from the final executable call graph.
    ///
    /// The request and completion IDs are transaction-local, so they are rebuilt together after
    /// convergence has validated every materialised body and unreachable handler work is removed.
    pub(crate) fn retain_reachable_sidecars(
        &mut self,
        reachable: &FxHashSet<GeneratedFunctionIdentity>,
    ) {
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.state != GeneratedRequestState::Complete)
        {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction stopped before request {:?} completed",
                record.identity
            );
        }
        if self.records.len() != self.completed_records.len() {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction request and sidecar records disagree after convergence"
            );
        }
        self.completed_records
            .retain(|record| reachable.contains(&record.identity));
        self.completed_warnings
            .retain(|identity, _| reachable.contains(identity));
        self.completed_by_identity = self
            .completed_records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.identity.clone(), GeneratedFunctionId::new(index)))
            .collect();
        self.records
            .retain(|record| reachable.contains(&record.identity));
        self.ids_by_identity = self
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| (record.identity.clone(), GeneratedRequestId(index)))
            .collect();
    }

    pub(crate) fn finish(mut self) -> (GeneratedFunctionDelta, Vec<DiagnosticBag>) {
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.state != GeneratedRequestState::Complete)
        {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction stopped before request {:?} completed",
                record.identity
            );
        }
        if self
            .completed_records
            .iter()
            .any(|record| !record.sidecar.module.executable.hir.expressions.is_frozen())
        {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction cannot publish a growing HIR expression store"
            );
        }
        let mut warnings = Vec::with_capacity(self.completed_records.len());
        for record in &self.completed_records {
            let Some(record_warnings) = self.completed_warnings.remove(&record.identity) else {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated transaction cannot find warnings for completed sidecar {:?}",
                    record.identity
                );
            };
            warnings.push(record_warnings);
        }
        if !self.completed_warnings.is_empty() {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated transaction retained warnings without completed sidecars"
            );
        }
        (
            GeneratedFunctionDelta::from_records(self.completed_records),
            warnings,
        )
    }
}

fn out_of_range(request_id: GeneratedRequestId) -> ! {
    crate::compiler_bug!(
        CompilerStage::ProjectBuildAssembly,
        "Generated transaction received out-of-range request id {}",
        request_id.index()
    )
}

#[cfg(test)]
#[path = "tests/transaction_tests.rs"]
mod tests;
