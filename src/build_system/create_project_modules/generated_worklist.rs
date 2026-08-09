//! Build-owned generated-function request scheduling and sidecar storage.
//!
//! WHAT: owns one deterministic request worklist per project or package compilation boundary,
//! dense request IDs, exact completed summaries and the separate generated-sidecar lane.
//! WHY: generic call inference belongs to AST, but aggregation, deduplication, fixed-point
//! scheduling and sidecar placement belong to the owning build boundary. A session reuses only
//! its own boundary's completed store and transactional delta; equal generated identities may
//! coexist in unrelated boundaries and are never suppressed or resolved across them. Materialising
//! request history is intentionally not retained as a second dependency topology; convergence
//! derives its observation model from validated HIR call facts.

use crate::build_system::build::GeneratedFunctionSidecar;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::public_call_summary::{
    PublicCallSummary, PublicCallSummaryTransition, validate_public_call_summary_transition,
};
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct GeneratedRequestId(usize);

impl GeneratedRequestId {
    pub(super) fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum ConvergenceNode {
    BaseModule,
    Generated(Box<GeneratedFunctionIdentity>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct ConvergenceNodeId(usize);

impl ConvergenceNodeId {
    pub(super) fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConvergenceNodeRecord {
    node: ConvergenceNode,
}

/// Construction-only reverse-call model for one module convergence observation.
///
/// WHAT: assigns dense deterministic nodes to the base module and newly materialised local
///       sidecars, then stores sorted reverse callers for each node.
/// WHY: R5C6A needs an inspectable dependency model before scheduling changes. It is derived from
///      validated HIR link facts, is never retained in an artefact, and never replaces the broad
///      recheck loop in this slice.
#[derive(Debug)]
pub(super) struct ConvergenceModel {
    nodes: Vec<ConvergenceNodeRecord>,
    callers: Vec<Vec<ConvergenceNodeId>>,
}

impl ConvergenceModel {
    pub(super) fn from_link_facts<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
    ) -> Result<Self, CompilerError> {
        let mut generated = generated.into_iter().collect::<Vec<_>>();
        generated.sort_by_key(|(left, _)| *left);

        let mut nodes = vec![ConvergenceNodeRecord {
            node: ConvergenceNode::BaseModule,
        }];
        for (identity, _) in &generated {
            let node = ConvergenceNode::Generated(Box::new((*identity).clone()));
            if nodes.iter().any(|record| record.node == node) {
                return Err(CompilerError::compiler_error(format!(
                    "convergence model received duplicate generated identity {identity:?}"
                )));
            }
            nodes.push(ConvergenceNodeRecord { node });
        }

        let ids_by_node = nodes
            .iter()
            .enumerate()
            .map(|(index, record)| (record.node.clone(), ConvergenceNodeId(index)))
            .collect::<FxHashMap<_, _>>();
        let mut callers = vec![Vec::new(); nodes.len()];

        add_model_edges(
            &mut callers,
            &ids_by_node,
            ConvergenceNodeId(0),
            base.direct_call_targets(),
        );
        for (identity, link_facts) in generated {
            let caller = ids_by_node[&ConvergenceNode::Generated(Box::new(identity.clone()))];
            add_model_edges(
                &mut callers,
                &ids_by_node,
                caller,
                link_facts.direct_call_targets(),
            );
        }

        for caller_ids in &mut callers {
            caller_ids.sort_unstable();
            caller_ids.dedup();
        }

        Ok(Self { nodes, callers })
    }

    #[cfg(test)]
    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[cfg(test)]
    pub(super) fn node(&self, id: ConvergenceNodeId) -> Option<&ConvergenceNode> {
        self.nodes.get(id.index()).map(|record| &record.node)
    }

    pub(super) fn node_id(&self, node: &ConvergenceNode) -> Option<ConvergenceNodeId> {
        self.nodes
            .iter()
            .position(|record| &record.node == node)
            .map(ConvergenceNodeId)
    }

    #[cfg(test)]
    pub(super) fn callers(&self, node: ConvergenceNodeId) -> Option<&[ConvergenceNodeId]> {
        self.callers.get(node.index()).map(Vec::as_slice)
    }

    pub(super) fn all_node_ids(&self) -> Vec<ConvergenceNodeId> {
        (0..self.nodes.len()).map(ConvergenceNodeId).collect()
    }

    /// Return the changed nodes and every reverse-reachable caller in dense ID order.
    pub(super) fn dirty_nodes(
        &self,
        changed_nodes: impl IntoIterator<Item = ConvergenceNodeId>,
    ) -> Vec<ConvergenceNodeId> {
        let mut dirty = vec![false; self.nodes.len()];
        let mut queue = std::collections::VecDeque::new();
        for node in changed_nodes {
            if node.index() < dirty.len() && !dirty[node.index()] {
                dirty[node.index()] = true;
                queue.push_back(node);
            }
        }

        while let Some(changed) = queue.pop_front() {
            for caller in &self.callers[changed.index()] {
                if !dirty[caller.index()] {
                    dirty[caller.index()] = true;
                    queue.push_back(*caller);
                }
            }
        }

        dirty
            .into_iter()
            .enumerate()
            .filter_map(|(index, is_dirty)| is_dirty.then_some(ConvergenceNodeId(index)))
            .collect()
    }
}

fn add_model_edges(
    callers: &mut [Vec<ConvergenceNodeId>],
    ids_by_node: &FxHashMap<ConvergenceNode, ConvergenceNodeId>,
    caller: ConvergenceNodeId,
    targets: Vec<(crate::compiler_frontend::hir::ids::FunctionId, CallTarget)>,
) {
    for (_, target) in targets {
        let callee = match target {
            CallTarget::Local(_) | CallTarget::CrossModule(_) | CallTarget::External(_) => None,
            CallTarget::ModulePrivate(_) => match caller.index() {
                0 => None,
                _ => Some(ConvergenceNodeId(0)),
            },
            CallTarget::Generated(identity) => ids_by_node
                .get(&ConvergenceNode::Generated(Box::new(identity)))
                .copied(),
        };
        let Some(callee) = callee else {
            continue;
        };
        callers[callee.index()].push(caller);
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
    records: Vec<GeneratedRequestRecord>,
    ids_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedRequestId>,
    completed_records: Vec<CompletedGeneratedFunction>,
    completed_by_identity: FxHashMap<GeneratedFunctionIdentity, GeneratedFunctionId>,
}

impl<'a> GeneratedFunctionWorklist<'a> {
    fn new(known: &'a BoundaryGeneratedFunctionStore) -> Self {
        Self {
            known,
            records: Vec::new(),
            ids_by_identity: FxHashMap::default(),
            completed_records: Vec::new(),
            completed_by_identity: FxHashMap::default(),
        }
    }

    pub(super) fn register_requests(
        &mut self,
        requests: impl IntoIterator<Item = GeneratedRequestFacts>,
    ) -> Vec<GeneratedRequestId> {
        let mut requests = requests.into_iter().collect::<Vec<_>>();
        requests.sort_by(|left, right| left.identity.cmp(&right.identity));
        requests.dedup_by(|left, right| left.identity == right.identity);

        let mut request_ids = Vec::with_capacity(requests.len());
        for request in requests {
            if self.known.by_identity.contains_key(&request.identity)
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
    }

    pub(super) fn completed_summaries(
        &self,
    ) -> FxHashMap<GeneratedFunctionIdentity, PublicCallSummary> {
        increment_frontend_counter(FrontendCounter::ConvergenceCompleteGeneratedSummaryMapBuilds);
        let mut summaries = FxHashMap::default();
        for record in &self.completed_records {
            summaries.insert(record.identity.clone(), record.summary.clone());
        }
        for record in &self.known.records {
            summaries.insert(record.identity.clone(), record.summary.clone());
        }
        summaries
    }

    pub(super) fn convergence_model(
        &self,
        base_link_facts: &HirModuleLinkFacts,
    ) -> Result<ConvergenceModel, CompilerError> {
        ConvergenceModel::from_link_facts(
            base_link_facts,
            self.completed_records.iter().map(|record| {
                (
                    &record.identity,
                    &record.sidecar.module.link_facts.functions,
                )
            }),
        )
    }

    pub(super) fn sidecar_node_id(
        &self,
        model: &ConvergenceModel,
        identity: &GeneratedFunctionIdentity,
    ) -> Result<ConvergenceNodeId, CompilerError> {
        model
            .node_id(&ConvergenceNode::Generated(Box::new(identity.clone())))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "convergence model is missing generated sidecar {identity:?}"
                ))
            })
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
        add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
        let current = &self.completed_records[record_id.index()].summary;
        let transition = validate_public_call_summary_transition(current, &summary)?;
        if transition == PublicCallSummaryTransition::Widened {
            add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
            self.completed_records[record_id.index()].summary = summary;
            Ok(true)
        } else {
            Ok(false)
        }
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

    pub(super) fn session<'a>(&'a self) -> GeneratedFunctionWorklist<'a> {
        GeneratedFunctionWorklist::new(self)
    }

    pub(super) fn publish(
        &mut self,
        delta: GeneratedFunctionWorklistDelta,
    ) -> Result<(), CompilerError> {
        // Preflight the complete delta before mutation: identity/sidecar agreement, executable
        // record shape, duplicate identities inside the delta and duplicates against retained
        // state must all pass before any row is appended.
        let mut delta_identities = FxHashSet::default();
        for record in &delta.records {
            if record.identity != record.sidecar.identity {
                return Err(CompilerError::compiler_error(format!(
                    "Generated sidecar identity {:?} disagrees with its record identity {:?}",
                    record.sidecar.identity, record.identity
                )));
            }
            Self::validate_completed_generated_record(record)?;
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

        for record in delta.records {
            let record_id = GeneratedFunctionId(self.records.len());
            self.by_identity.insert(record.identity.clone(), record_id);
            self.records.push(record);
        }
        Ok(())
    }

    /// Validate one completed generated record as an executable retained-store row.
    ///
    /// WHAT: proves the sidecar HIR contains exactly one generated root mapping for this
    ///       identity, that the root `FunctionId` is in range, and that the record's summary is
    ///       the exact borrow summary of that generated root.
    /// WHY: generated summaries and sidecars publish together, so the retained store must never
    ///       accept a sidecar whose root identity or summary cannot back the record.
    fn validate_completed_generated_record(
        record: &CompletedGeneratedFunction,
    ) -> Result<(), CompilerError> {
        let hir = &record.sidecar.module.executable.hir;
        let mut roots = hir.function_ids_by_generated.iter();
        let Some((root_identity, function_id)) = roots.next() else {
            return Err(CompilerError::compiler_error(format!(
                "Generated sidecar {:?} has no generated root executable identity",
                record.identity
            )));
        };
        if roots.next().is_some() {
            return Err(CompilerError::compiler_error(format!(
                "Generated sidecar {:?} presents more than one generated root identity",
                record.identity
            )));
        }
        if root_identity != &record.identity {
            return Err(CompilerError::compiler_error(format!(
                "Generated sidecar root identity {:?} disagrees with its record identity {:?}",
                root_identity, record.identity
            )));
        }
        if function_id.0 as usize >= hir.functions.len() {
            return Err(CompilerError::compiler_error(format!(
                "Generated sidecar {:?} references out-of-range HIR FunctionId {}",
                record.identity, function_id.0
            )));
        }
        let exact_summary = record
            .sidecar
            .module
            .executable
            .borrow_analysis
            .analysis
            .public_call_summaries
            .get(function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Generated function {:?} has no exact borrow summary",
                    record.identity
                ))
            })?;
        if exact_summary != &record.summary {
            return Err(CompilerError::compiler_error(format!(
                "Generated function {:?} summary disagrees with its sidecar borrow summary",
                record.identity
            )));
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

#[cfg(test)]
#[path = "../tests/generated_worklist_tests.rs"]
mod tests;
