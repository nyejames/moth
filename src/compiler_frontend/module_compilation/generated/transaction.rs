//! Per-module generated-function transaction.
//!
//! WHAT: the request state machine one module compilation runs to reach its local generated fixed
//!       point — request registration and deduplication against already published work, recursion
//!       detection, completed record storage and the final delta.
//! WHY:  reaching that fixed point is compiler semantics: it interleaves materialisation, HIR
//!       lowering, borrow analysis and summary convergence. The boundary store owns what happens
//!       afterwards. A transaction publishes nothing: a diagnosed module simply drops it.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironmentRemapCache;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::module_compilation::generated::artefacts::{
    CompletedGeneratedFunction, GeneratedFunctionDelta, GeneratedFunctionId,
    GeneratedFunctionSidecar,
};
use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;

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
    diagnostic_location: SourceLocation,
    state: GeneratedRequestState,
}

/// One generated request as authored by AST, carrying the facts diagnostics need.
#[derive(Clone, Debug)]
pub(crate) struct GeneratedRequestFacts {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) display_name: String,
    pub(crate) diagnostic_location: SourceLocation,
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
}

impl<'a> GeneratedFunctionTransaction<'a> {
    pub(crate) fn new(known: KnownGeneratedFunctions<'a>) -> Self {
        Self {
            known,
            records: Vec::new(),
            ids_by_identity: FxHashMap::default(),
            completed_records: Vec::new(),
            completed_by_identity: FxHashMap::default(),
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
                    diagnostic_location: request.diagnostic_location,
                    state: GeneratedRequestState::Pending,
                });
                request_id
            };
            request_ids.push(request_id);
        }
        request_ids
    }

    pub(crate) fn identity(
        &self,
        request_id: GeneratedRequestId,
    ) -> Result<&GeneratedFunctionIdentity, CompilerError> {
        self.records
            .get(request_id.index())
            .map(|record| &record.identity)
            .ok_or_else(|| out_of_range(request_id))
    }

    /// The display facts one request record owns for diagnostics.
    pub(crate) fn request_facts(
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
            .ok_or_else(|| out_of_range(request_id))
    }

    pub(crate) fn enter(
        &mut self,
        request_id: GeneratedRequestId,
    ) -> Result<GeneratedRequestEntry, CompilerError> {
        let record = self
            .records
            .get_mut(request_id.index())
            .ok_or_else(|| out_of_range(request_id))?;
        match record.state {
            GeneratedRequestState::Pending => {
                record.state = GeneratedRequestState::Materialising;
                Ok(GeneratedRequestEntry::Materialise)
            }
            GeneratedRequestState::Materialising => Ok(GeneratedRequestEntry::Recursive),
            GeneratedRequestState::Complete => Ok(GeneratedRequestEntry::Complete),
        }
    }

    pub(crate) fn complete(
        &mut self,
        request_id: GeneratedRequestId,
        summary: PublicCallSummary,
        sidecar: GeneratedFunctionSidecar,
    ) -> Result<(), CompilerError> {
        let record = self
            .records
            .get_mut(request_id.index())
            .ok_or_else(|| out_of_range(request_id))?;
        if record.state != GeneratedRequestState::Materialising {
            return Err(CompilerError::compiler_error(format!(
                "Generated request {:?} completed from an invalid transaction state",
                record.identity
            )));
        }
        if sidecar.identity != record.identity {
            return Err(CompilerError::compiler_error(
                "Generated sidecar identity disagrees with its request",
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
        let generated_id = GeneratedFunctionId::new(self.completed_records.len());
        self.completed_by_identity
            .insert(identity.clone(), generated_id);
        self.completed_records.push(CompletedGeneratedFunction {
            identity,
            summary,
            sidecar,
        });
        Ok(())
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
    ) -> Result<&mut GeneratedFunctionSidecar, CompilerError> {
        let record_id = self.completed_by_identity.get(identity).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated transaction cannot find sidecar {identity:?}"
            ))
        })?;
        self.completed_records
            .get_mut(record_id.index())
            .map(|record| &mut record.sidecar)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated transaction sidecar index for {identity:?} is out of range"
                ))
            })
    }

    pub(crate) fn summary_mut(
        &mut self,
        identity: &GeneratedFunctionIdentity,
    ) -> Result<&mut PublicCallSummary, CompilerError> {
        let record_id = self.completed_by_identity.get(identity).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated transaction cannot update unknown completed request {identity:?}"
            ))
        })?;
        self.completed_records
            .get_mut(record_id.index())
            .map(|record| &mut record.summary)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated transaction summary index for {identity:?} is out of range"
                ))
            })
    }

    pub(crate) fn sidecar_count(&self) -> usize {
        self.completed_records.len()
    }

    /// Remap sidecars completed after `first_sidecar` into a merged string domain.
    ///
    /// WHY: nested requests materialise against their own local string table, so the sidecars they
    ///      produced must follow the same merge as their requester's generated module.
    pub(crate) fn remap_sidecars_and_module_from(
        &mut self,
        first_sidecar: usize,
        module: &mut Module,
        remap: &StringIdRemap,
    ) {
        let mut type_environment_cache = TypeEnvironmentRemapCache::default();
        for record in &mut self.completed_records[first_sidecar..] {
            record
                .sidecar
                .remap_string_ids_with_type_environment_cache(remap, &mut type_environment_cache);
        }
        module.remap_string_ids_with_type_environment_cache(remap, &mut type_environment_cache);
    }

    /// Close the transaction and hand back everything it completed.
    pub(crate) fn finish(self) -> Result<GeneratedFunctionDelta, CompilerError> {
        if let Some(record) = self
            .records
            .iter()
            .find(|record| record.state != GeneratedRequestState::Complete)
        {
            return Err(CompilerError::compiler_error(format!(
                "Generated transaction stopped before request {:?} completed",
                record.identity
            )));
        }
        Ok(GeneratedFunctionDelta::from_records(self.completed_records))
    }
}

fn out_of_range(request_id: GeneratedRequestId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Generated transaction received out-of-range request id {}",
        request_id.index()
    ))
}

#[cfg(test)]
#[path = "tests/transaction_tests.rs"]
mod tests;
