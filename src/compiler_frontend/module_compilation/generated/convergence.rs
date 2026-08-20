//! HIR-derived generated-summary convergence for one module compilation.
//!
//! WHAT: builds the transient call-dependency model and runs the monotone dirty queue that
//!       propagates exact base and generated borrow summaries through materialised sidecars, plus
//!       the exact-summary installation each pass depends on.
//! WHY: validated HIR owns executable call topology, so convergence reads it rather than becoming
//!       a second dependency owner. Reaching this fixed point mutates base and generated HIR
//!       summaries and reruns borrow analysis, which is compiler semantics: the build system's
//!       generated store never performs either.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparationBuilder;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerMessages, merge_stage_messages,
};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::public_call_summary::validate_public_call_summary_transition;
use crate::compiler_frontend::public_call_summary::{
    PublicCallSummary, PublicCallSummaryTransition,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::timed_stage_attributed;

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedFunctionTransaction;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ConvergenceNode {
    BaseModule,
    Generated(Box<GeneratedFunctionIdentity>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ConvergenceNodeId(pub(crate) usize);

impl ConvergenceNodeId {
    pub(crate) fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConvergenceNodeRecord {
    node: ConvergenceNode,
    generated_callees: Vec<GeneratedFunctionIdentity>,
    active_public_callees: Vec<OriginFunctionId>,
    module_private_callees: Vec<ModulePrivateExecutableIdentity>,
}

/// Construction-only reverse-call model for one module convergence observation.
///
/// WHAT: assigns dense deterministic nodes to the base module and newly materialised local
///       sidecars, then stores sorted reverse callers for each node.
/// WHY: convergence scheduling needs one inspectable dependency model derived from validated HIR
///      link facts. It is never retained in an artefact and owns only this module's transient
///      queue inputs.
#[derive(Debug)]
pub(crate) struct ConvergenceModel {
    nodes: Vec<ConvergenceNodeRecord>,
    callers: Vec<Vec<ConvergenceNodeId>>,
}

impl ConvergenceModel {
    #[cfg(test)]
    pub(crate) fn from_link_facts<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
    ) -> Result<Self, CompilerError> {
        Self::build(base, generated, &FxHashSet::default(), None)
    }

    pub(crate) fn from_link_facts_for_base_callees<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: &FxHashSet<ModulePrivateExecutableIdentity>,
    ) -> Result<Self, CompilerError> {
        Self::build(
            base,
            generated,
            base_public_origins,
            Some(base_private_identities),
        )
    }

    fn build<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: Option<&FxHashSet<ModulePrivateExecutableIdentity>>,
    ) -> Result<Self, CompilerError> {
        let mut generated = generated.into_iter().collect::<Vec<_>>();
        generated.sort_by_key(|(identity, _)| *identity);
        for pair in generated.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(CompilerError::compiler_error(format!(
                    "convergence model received duplicate generated identity {:?}",
                    pair[0].0
                )));
            }
        }

        let mut nodes = vec![ConvergenceNodeRecord {
            node: ConvergenceNode::BaseModule,
            generated_callees: Vec::new(),
            active_public_callees: Vec::new(),
            module_private_callees: Vec::new(),
        }];
        nodes.extend(generated.iter().map(|(identity, _)| ConvergenceNodeRecord {
            node: ConvergenceNode::Generated(Box::new((*identity).clone())),
            generated_callees: Vec::new(),
            active_public_callees: Vec::new(),
            module_private_callees: Vec::new(),
        }));

        let ids_by_generated = generated
            .iter()
            .enumerate()
            .map(|(index, (identity, _))| ((*identity).clone(), ConvergenceNodeId(index + 1)))
            .collect::<FxHashMap<_, _>>();
        let mut callers = vec![Vec::new(); nodes.len()];

        add_model_edges(
            &mut callers,
            &ids_by_generated,
            &mut nodes[0],
            ConvergenceNodeId(0),
            base_public_origins,
            base_private_identities,
            base.direct_call_targets(),
        );
        for (identity, link_facts) in generated {
            let caller = ids_by_generated[identity];
            add_model_edges(
                &mut callers,
                &ids_by_generated,
                &mut nodes[caller.index()],
                caller,
                base_public_origins,
                base_private_identities,
                link_facts.direct_call_targets(),
            );
        }

        for record in &mut nodes {
            record.generated_callees.sort_unstable();
            record.generated_callees.dedup();
            record.active_public_callees.sort_unstable();
            record.active_public_callees.dedup();
            record.module_private_callees.sort_unstable();
            record.module_private_callees.dedup();
        }
        for caller_ids in &mut callers {
            caller_ids.sort_unstable();
            caller_ids.dedup();
        }

        Ok(Self { nodes, callers })
    }

    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn node(&self, id: ConvergenceNodeId) -> Option<&ConvergenceNode> {
        self.nodes.get(id.index()).map(|record| &record.node)
    }

    #[cfg(test)]
    pub(crate) fn node_id(&self, node: &ConvergenceNode) -> Option<ConvergenceNodeId> {
        self.nodes
            .iter()
            .position(|record| &record.node == node)
            .map(ConvergenceNodeId)
    }

    pub(crate) fn callers(&self, node: ConvergenceNodeId) -> Option<&[ConvergenceNodeId]> {
        self.callers.get(node.index()).map(Vec::as_slice)
    }

    pub(crate) fn all_node_ids(&self) -> impl Iterator<Item = ConvergenceNodeId> + '_ {
        (0..self.nodes.len()).map(ConvergenceNodeId)
    }

    pub(crate) fn generated_callees(
        &self,
        node: ConvergenceNodeId,
    ) -> Option<&[GeneratedFunctionIdentity]> {
        self.nodes
            .get(node.index())
            .map(|record| record.generated_callees.as_slice())
    }

    pub(crate) fn module_private_callees(
        &self,
        node: ConvergenceNodeId,
    ) -> Option<&[ModulePrivateExecutableIdentity]> {
        self.nodes
            .get(node.index())
            .map(|record| record.module_private_callees.as_slice())
    }

    pub(crate) fn active_public_callees(
        &self,
        node: ConvergenceNodeId,
    ) -> Option<&[OriginFunctionId]> {
        self.nodes
            .get(node.index())
            .map(|record| record.active_public_callees.as_slice())
    }

    pub(crate) fn generated_node_ids(&self) -> impl Iterator<Item = ConvergenceNodeId> + '_ {
        (1..self.nodes.len()).map(ConvergenceNodeId)
    }

    /// Return the changed nodes and every reverse-reachable caller in dense ID order.
    #[cfg(test)]
    pub(crate) fn dirty_nodes(
        &self,
        changed_nodes: impl IntoIterator<Item = ConvergenceNodeId>,
    ) -> Vec<ConvergenceNodeId> {
        let mut dirty = vec![false; self.nodes.len()];
        let mut queue = VecDeque::new();
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
    ids_by_generated: &FxHashMap<GeneratedFunctionIdentity, ConvergenceNodeId>,
    record: &mut ConvergenceNodeRecord,
    caller: ConvergenceNodeId,
    base_public_origins: &FxHashSet<OriginFunctionId>,
    base_private_identities: Option<&FxHashSet<ModulePrivateExecutableIdentity>>,
    targets: Vec<(crate::compiler_frontend::hir::ids::FunctionId, CallTarget)>,
) {
    for (_, target) in targets {
        let callee = match target {
            CallTarget::Local(_) | CallTarget::External(_) => None,
            CallTarget::CrossModule(origin) => {
                if !base_public_origins.contains(&origin) {
                    None
                } else {
                    record.active_public_callees.push(origin);
                    match caller.index() {
                        0 => None,
                        _ => Some(ConvergenceNodeId(0)),
                    }
                }
            }
            CallTarget::ModulePrivate(identity) => {
                let is_base_private =
                    base_private_identities.is_none_or(|identities| identities.contains(&identity));
                if !is_base_private {
                    None
                } else {
                    record.module_private_callees.push(identity);
                    match caller.index() {
                        0 => None,
                        _ => Some(ConvergenceNodeId(0)),
                    }
                }
            }
            CallTarget::Generated(identity) => {
                record.generated_callees.push(identity.clone());
                ids_by_generated.get(&identity).copied()
            }
        };
        let Some(callee) = callee else {
            continue;
        };
        callers[callee.index()].push(caller);
    }
}

/// Run monotone summary convergence for one base HIR and its completed local sidecars.
pub(crate) fn run_generated_summary_convergence(
    compiler: &CompilerFrontend,
    hir_module: &mut HirModule,
    function_link_facts: &HirModuleLinkFacts,
    generated_transaction: &mut GeneratedFunctionTransaction<'_>,
    bootstrap_borrow_analysis: BorrowCheckReport,
    warnings: &[CompilerDiagnostic],
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<BorrowCheckReport, CompilerMessages> {
    let base_public_origins = hir_module
        .function_ids_by_origin
        .keys()
        .cloned()
        .collect::<FxHashSet<_>>();
    let base_private_identities = hir_module
        .function_ids_by_private_origin
        .keys()
        .cloned()
        .collect::<FxHashSet<_>>();
    let convergence_model = ConvergenceModel::from_link_facts_for_base_callees(
        function_link_facts,
        generated_transaction.completed_link_facts(),
        &base_public_origins,
        &base_private_identities,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    let mut convergence_queue = VecDeque::new();
    let mut queued_nodes = vec![false; convergence_model.node_count()];
    for node_id in convergence_model.all_node_ids() {
        convergence_queue.push_back(node_id);
        queued_nodes[node_id.index()] = true;
    }

    let mut borrow_analysis = Some(bootstrap_borrow_analysis);
    while let Some(node_id) = convergence_queue.pop_front() {
        queued_nodes[node_id.index()] = false;
        let node = convergence_model.node(node_id).cloned().ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "convergence queue received unknown node {node_id:?}"
                )),
                &compiler.string_table,
            )
        })?;
        let current_borrow_analysis = borrow_analysis.as_ref().ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "Convergence queue lost the current base borrow analysis",
                ),
                &compiler.string_table,
            )
        })?;
        let direct_summaries = direct_convergence_summaries(
            &convergence_model,
            node_id,
            generated_transaction,
            hir_module,
            current_borrow_analysis,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

        match node {
            ConvergenceNode::BaseModule => {
                install_convergence_summaries(
                    hir_module,
                    &direct_summaries.generated,
                    &direct_summaries.active_public,
                    &direct_summaries.module_private,
                );
                increment_convergence_counter(FrontendCounter::ConvergenceBaseBorrowPasses);
                let report = timed_stage_attributed!(
                    crate::timing::TimingMetric::FrontendBorrowConverge,
                    timing_context,
                    check_borrows_with_warnings(compiler, hir_module, warnings)
                )?;
                let summary_changes =
                    base_summary_changes(hir_module, current_borrow_analysis, &report).map_err(
                        |error| CompilerMessages::from_error_ref(error, &compiler.string_table),
                    )?;
                borrow_analysis = Some(report);
                if !summary_changes.is_empty() {
                    enqueue_base_dependents(
                        &convergence_model,
                        &summary_changes,
                        &mut convergence_queue,
                        &mut queued_nodes,
                    )
                    .map_err(|error| {
                        CompilerMessages::from_error_ref(error, &compiler.string_table)
                    })?;
                }
            }
            ConvergenceNode::Generated(identity) => {
                let identity = *identity;
                let summary = {
                    let sidecar =
                        generated_transaction
                            .sidecar_mut(&identity)
                            .map_err(|error| {
                                CompilerMessages::from_error_ref(error, &compiler.string_table)
                            })?;
                    install_convergence_summaries(
                        &mut sidecar.module.executable.hir,
                        &direct_summaries.generated,
                        &direct_summaries.active_public,
                        &direct_summaries.module_private,
                    );
                    increment_convergence_counter(
                        FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses,
                    );
                    let report = timed_stage_attributed!(
                        crate::timing::TimingMetric::FrontendGeneratedBorrowRecheck,
                        timing_context,
                        check_borrows_with_warnings(
                            compiler,
                            &sidecar.module.executable.hir,
                            &sidecar.module.metadata.warnings,
                        )
                    )?;
                    sidecar.module.executable.borrow_analysis = report;
                    exact_generated_sidecar_summary(&identity, &sidecar.module).map_err(
                        |error| CompilerMessages::from_error_ref(error, &compiler.string_table),
                    )?
                };
                if update_generated_summary(generated_transaction, &identity, summary).map_err(
                    |error| CompilerMessages::from_error_ref(error, &compiler.string_table),
                )? {
                    enqueue_convergence_callers(
                        &convergence_model,
                        node_id,
                        &mut convergence_queue,
                        &mut queued_nodes,
                    )
                    .map_err(|error| {
                        CompilerMessages::from_error_ref(error, &compiler.string_table)
                    })?;
                }
            }
        }
    }

    borrow_analysis.ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error("Convergence queue did not analyze the base module"),
            &compiler.string_table,
        )
    })
}

fn check_borrows_with_warnings(
    compiler: &CompilerFrontend,
    hir_module: &HirModule,
    warnings: &[CompilerDiagnostic],
) -> Result<BorrowCheckReport, CompilerMessages> {
    compiler
        .check_borrows(hir_module)
        .map_err(|messages| merge_stage_messages(messages, warnings, &compiler.string_table))
}

/// Stable base identities whose exact summaries widened during one borrow pass.
#[derive(Debug)]
pub(crate) struct BaseSummaryChanges {
    pub(crate) public: Vec<OriginFunctionId>,
    pub(crate) module_private: Vec<ModulePrivateExecutableIdentity>,
}

impl BaseSummaryChanges {
    pub(crate) fn is_empty(&self) -> bool {
        self.public.is_empty() && self.module_private.is_empty()
    }
}

/// Exact direct-call summaries needed to analyze one convergence node.
pub(crate) struct DirectConvergenceSummaries {
    pub(crate) generated: Vec<(GeneratedFunctionIdentity, PublicCallSummary)>,
    pub(crate) active_public: Vec<(OriginFunctionId, PublicCallSummary)>,
    pub(crate) module_private: Vec<(ModulePrivateExecutableIdentity, PublicCallSummary)>,
}

pub(crate) fn direct_convergence_summaries(
    model: &ConvergenceModel,
    node_id: ConvergenceNodeId,
    transaction: &GeneratedFunctionTransaction<'_>,
    base_hir: &HirModule,
    base_borrow_analysis: &BorrowCheckReport,
) -> Result<DirectConvergenceSummaries, CompilerError> {
    let generated_callees = model.generated_callees(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "convergence model is missing generated callees for node {node_id:?}"
        ))
    })?;
    let generated_summaries = generated_callees
        .iter()
        .map(|identity| {
            transaction
                .summary(identity)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "convergence node {node_id:?} has no exact generated summary for {identity:?}"
                    ))
                })
                .map(|summary| (identity.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let active_public_callees = model.active_public_callees(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "convergence model is missing active public callees for node {node_id:?}"
        ))
    })?;
    let active_public_summaries = active_public_callees
        .iter()
        .map(|origin| {
            let function_id = base_hir
                .function_ids_by_origin
                .get(origin)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "active base public origin {origin:?} has no HIR function identity"
                    ))
                })?;
            let summary = base_borrow_analysis
                .analysis
                .public_call_summaries
                .get(&function_id)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "active base public function {function_id:?} has no exact borrow summary"
                    ))
                })?;
            Ok::<_, CompilerError>((origin.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let private_callees = model.module_private_callees(node_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "convergence model is missing module-private callees for node {node_id:?}"
        ))
    })?;
    let private_summaries = private_callees
        .iter()
        .map(|identity| {
            let function_id = base_hir
                .function_ids_by_private_origin
                .get(identity)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "active base private identity {identity:?} has no HIR function identity"
                    ))
                })?;
            let summary = base_borrow_analysis
                .analysis
                .public_call_summaries
                .get(&function_id)
                .cloned()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "active base private function {function_id:?} has no exact borrow summary"
                    ))
                })?;
            Ok::<_, CompilerError>((identity.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(DirectConvergenceSummaries {
        generated: generated_summaries,
        active_public: active_public_summaries,
        module_private: private_summaries,
    })
}

pub(crate) fn install_convergence_summaries(
    hir: &mut HirModule,
    generated_summaries: &[(GeneratedFunctionIdentity, PublicCallSummary)],
    active_public_summaries: &[(OriginFunctionId, PublicCallSummary)],
    private_summaries: &[(ModulePrivateExecutableIdentity, PublicCallSummary)],
) {
    hir.generated_call_summaries.clear();
    for (identity, summary) in generated_summaries {
        hir.generated_call_summaries
            .insert(identity.clone(), summary.clone());
    }

    // Active-base public calls are represented as CrossModule targets in generated HIR. Update
    // only those stable origins; provider and cross-boundary imports remain fixed bootstrap
    // leaves in the same imported-summary map.
    for (origin, summary) in active_public_summaries {
        hir.imported_call_summaries
            .insert(origin.clone(), summary.clone());
    }

    // Provider-private summaries remain fixed bootstrap leaves. Active-base private identities
    // receive exact replacements, while no complete private map is rebuilt or retained.
    for (identity, summary) in private_summaries {
        hir.module_private_call_summaries
            .insert(identity.clone(), summary.clone());
    }
}

pub(crate) fn base_summary_changes(
    hir: &HirModule,
    previous: &BorrowCheckReport,
    next: &BorrowCheckReport,
) -> Result<BaseSummaryChanges, CompilerError> {
    let mut widened_functions = FxHashSet::default();
    for function in &hir.functions {
        let previous_summary = previous
            .analysis
            .public_call_summaries
            .get(&function.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "previous base borrow report is missing summary for {:?}",
                    function.id
                ))
            })?;
        let next_summary = next
            .analysis
            .public_call_summaries
            .get(&function.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "next base borrow report is missing summary for {:?}",
                    function.id
                ))
            })?;
        if validate_public_call_summary_transition(previous_summary, next_summary)?
            == PublicCallSummaryTransition::Widened
        {
            widened_functions.insert(function.id);
        }
    }

    let mut changes = BaseSummaryChanges {
        public: hir
            .function_ids_by_origin
            .iter()
            .filter_map(|(origin, function_id)| {
                widened_functions
                    .contains(function_id)
                    .then_some(origin.clone())
            })
            .collect(),
        module_private: hir
            .function_ids_by_private_origin
            .iter()
            .filter_map(|(identity, function_id)| {
                widened_functions
                    .contains(function_id)
                    .then_some(identity.clone())
            })
            .collect(),
    };
    changes.public.sort_unstable();
    changes.module_private.sort_unstable();
    Ok(changes)
}

pub(crate) fn enqueue_convergence_node(
    node_id: ConvergenceNodeId,
    queue: &mut VecDeque<ConvergenceNodeId>,
    queued_nodes: &mut [bool],
) -> Result<(), CompilerError> {
    let queued = queued_nodes.get_mut(node_id.index()).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "convergence node {node_id:?} is outside the queue bitset"
        ))
    })?;
    if !*queued {
        *queued = true;
        queue.push_back(node_id);
    }
    Ok(())
}

pub(crate) fn enqueue_base_dependents(
    model: &ConvergenceModel,
    changes: &BaseSummaryChanges,
    queue: &mut VecDeque<ConvergenceNodeId>,
    queued_nodes: &mut [bool],
) -> Result<(), CompilerError> {
    for node_id in model.generated_node_ids() {
        let active_public_callees = model.active_public_callees(node_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "convergence model is missing active public callees for node {node_id:?}"
            ))
        })?;
        let private_callees = model.module_private_callees(node_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "convergence model is missing module-private callees for node {node_id:?}"
            ))
        })?;
        let public_changed = changes
            .public
            .iter()
            .any(|origin| active_public_callees.binary_search(origin).is_ok());
        let private_changed = changes
            .module_private
            .iter()
            .any(|identity| private_callees.binary_search(identity).is_ok());
        if public_changed || private_changed {
            enqueue_convergence_node(node_id, queue, queued_nodes)?;
        }
    }
    Ok(())
}

pub(crate) fn enqueue_convergence_callers(
    model: &ConvergenceModel,
    changed_node: ConvergenceNodeId,
    queue: &mut VecDeque<ConvergenceNodeId>,
    queued_nodes: &mut [bool],
) -> Result<(), CompilerError> {
    let callers = model.callers(changed_node).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "convergence model is missing callers for node {changed_node:?}"
        ))
    })?;
    for caller in callers {
        enqueue_convergence_node(*caller, queue, queued_nodes)?;
    }
    Ok(())
}

fn update_generated_summary(
    transaction: &mut GeneratedFunctionTransaction<'_>,
    identity: &GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> Result<bool, CompilerError> {
    let current = transaction.summary(identity).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "Generated transaction cannot update unknown completed request {identity:?}"
        ))
    })?;
    add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
    let transition = validate_public_call_summary_transition(current, &summary)?;
    if transition != PublicCallSummaryTransition::Widened {
        return Ok(false);
    }

    add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
    *transaction.summary_mut(identity)? = summary;
    Ok(true)
}

fn increment_convergence_counter(counter: FrontendCounter) {
    add_frontend_counter(counter, 1);
}

pub(crate) fn exact_generated_sidecar_summary(
    identity: &GeneratedFunctionIdentity,
    module: &Module,
) -> Result<PublicCallSummary, CompilerError> {
    let function_id = module
        .executable
        .hir
        .function_ids_by_generated
        .get(identity)
        .copied()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated sidecar {identity:?} has no generated root function"
            ))
        })?;
    if module.executable.hir.function_ids_by_generated.len() != 1 {
        return Err(CompilerError::compiler_error(format!(
            "Generated sidecar {identity:?} contains more than one generated root identity"
        )));
    }
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(&function_id)
        .cloned()
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Generated sidecar {identity:?} has no exact root borrow summary"
            ))
        })
}

pub(crate) fn install_exact_concrete_call_summaries(
    context: &mut ModuleMaterialisationPreparationBuilder,
    hir: &HirModule,
    borrow_analysis: &BorrowCheckReport,
) -> Result<(), CompilerError> {
    for contract in context.imported_functions_mut().values_mut() {
        let function_id = match &contract.target {
            SourceFunctionTarget::Imported { origin, .. } => {
                let Some(function_id) = hir.function_ids_by_origin.get(origin).copied() else {
                    continue;
                };
                function_id
            }
            SourceFunctionTarget::ModulePrivate { identity, .. } => hir
                .function_ids_by_private_origin
                .get(identity)
                .copied()
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Module materialisation context could not resolve private executable {identity:?}"
                    ))
                })?,
            SourceFunctionTarget::Local(_) | SourceFunctionTarget::Generated { .. } => continue,
        };
        let exact_summary = borrow_analysis
            .analysis
            .public_call_summaries
            .get(&function_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Module materialisation context is missing the exact call summary for {function_id:?}"
                ))
            })?;
        add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
        let transition =
            validate_public_call_summary_transition(&contract.summary, &exact_summary)?;
        if transition == PublicCallSummaryTransition::Widened {
            add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
            contract.summary = exact_summary;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/convergence_tests.rs"]
mod tests;
