//! HIR-derived generated-summary convergence for one module compilation.
//!
//! WHAT: builds the transient call-dependency model and runs the monotone dirty queue that
//!       propagates exact base and generated call summaries through materialised sidecars, plus
//!       the exact-summary installation each pass depends on. Immutable semantic failure facts
//!       join the same queue; only private no-slot functions infer an escaping builtin-failure bit.
//! WHY: validated HIR owns executable call topology, so convergence reads it rather than becoming
//!       a second dependency owner. Reaching this fixed point mutates base and generated HIR
//!       summaries and reruns borrow analysis, which is compiler semantics: the build system's
//!       generated store never performs either.
//! Function boundary diagnostics run only after this fixed point. An explicit Error! slot consumes
//! implicit builtin failure, a custom error slot requires local handling, and exported no-slot
//! functions may not publish an inferred contract.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    WitnessCandidates, retain_bounded_witness_hops, select_failure_witness,
};
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationPreparationBuilder;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::compiler_errors::RenderTypeContext;
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, CompilerDiagnostic,
    InvalidFallibleHandlingReason, PremergeDiagnosticBatch, PremergeFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::failure_facts::{
    HirBuiltinFailureBoundary, HirBuiltinFailureContributor, HirBuiltinFailureSource,
    HirFunctionFailureFacts,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::private_failure_lane::{
    PrivateFailureLaneInstallation, install_private_failure_lanes,
};
use crate::compiler_frontend::hir::reachability::{
    HirModuleLinkFacts, collect_module_function_link_facts,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::public_call_summary::validate_public_call_summary_transition;
use crate::compiler_frontend::public_call_summary::{
    PublicCallSummary, PublicCallSummaryTransition,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};

use crate::compiler_frontend::hir::ids::FunctionId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedFunctionTransaction;
use crate::compiler_frontend::module_compilation::stages::check_borrows;
use crate::timed_stage_attributed;

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
    ids_by_generated: FxHashMap<GeneratedFunctionIdentity, ConvergenceNodeId>,
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

        Ok(Self {
            nodes,
            callers,
            ids_by_generated,
        })
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

    /// Keep summary-dependent source checks in the existing dependency model, including work
    /// that has no runtime edge. Validation edges alone do not imply execution or escaping failure.
    fn include_deferred_validation_dependencies(
        &mut self,
        base: &HirModule,
        transaction: &mut GeneratedFunctionTransaction<'_>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: &FxHashSet<ModulePrivateExecutableIdentity>,
    ) -> Result<(), CompilerError> {
        for index in 0..self.nodes.len() {
            let mut targets = match &self.nodes[index].node {
                ConvergenceNode::BaseModule => deferred_validation_call_targets(base),
                ConvergenceNode::Generated(identity) => deferred_validation_call_targets(
                    &transaction.sidecar_mut(identity)?.module.executable.hir,
                ),
            };
            // An absent assertion-only summary diagnoses after convergence, rather than becoming
            // an invented false summary or an executable generated-call dependency.
            targets.retain(|(_, target)| {
                !matches!(target, CallTarget::Generated(identity) if transaction.summary(identity).is_none())
            });
            add_model_edges(
                &mut self.callers,
                &self.ids_by_generated,
                &mut self.nodes[index],
                ConvergenceNodeId(index),
                base_public_origins,
                Some(base_private_identities),
                targets,
            );
        }
        for record in &mut self.nodes {
            record.generated_callees.sort_unstable();
            record.generated_callees.dedup();
            record.active_public_callees.sort_unstable();
            record.active_public_callees.dedup();
            record.module_private_callees.sort_unstable();
            record.module_private_callees.dedup();
        }
        for callers in &mut self.callers {
            callers.sort_unstable();
            callers.dedup();
        }
        Ok(())
    }
}

fn deferred_validation_call_targets(hir: &HirModule) -> Vec<(FunctionId, CallTarget)> {
    let mut targets = Vec::new();
    for (function, facts) in &hir.function_failure_facts {
        let contributors = facts.assertion_message_calls.iter().chain(
            facts
                .deferred_custom_catches
                .iter()
                .flat_map(|check| &check.candidates),
        );
        for contributor in contributors {
            if let HirBuiltinFailureSource::Call(target) = &contributor.source {
                targets.push((*function, target.clone()));
            }
        }
    }
    targets
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
#[allow(
    clippy::too_many_arguments,
    reason = "convergence keeps the frontend, base HIR, link facts, generated transaction, bootstrap borrow report, mutable type environment, warnings and the timers-only context as separate borrows"
)]
pub(in crate::compiler_frontend::module_compilation) fn run_generated_summary_convergence(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &mut HirModule,
    function_link_facts: &mut HirModuleLinkFacts,
    generated_transaction: &mut GeneratedFunctionTransaction<'_>,
    bootstrap_borrow_analysis: BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    warnings: &[CompilerDiagnostic],
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<BorrowCheckReport, PremergeFailure> {
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
    let mut convergence_model = ConvergenceModel::from_link_facts_for_base_callees(
        function_link_facts,
        generated_transaction.completed_link_facts(),
        &base_public_origins,
        &base_private_identities,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    convergence_model.include_deferred_validation_dependencies(
        hir_module,
        generated_transaction,
        &base_public_origins,
        &base_private_identities,
    )?;
    let mut convergence_queue = VecDeque::new();
    let mut queued_nodes = vec![false; convergence_model.node_count()];
    for node_id in convergence_model.all_node_ids() {
        convergence_queue.push_back(node_id);
        queued_nodes[node_id.index()] = true;
    }

    let mut borrow_analysis = bootstrap_borrow_analysis;
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
        let direct_summaries = direct_convergence_summaries(
            &convergence_model,
            node_id,
            generated_transaction,
            hir_module,
            &borrow_analysis,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

        match node {
            ConvergenceNode::BaseModule => {
                let summaries_changed = install_convergence_summaries(
                    hir_module,
                    &direct_summaries.generated,
                    &direct_summaries.active_public,
                    &direct_summaries.module_private,
                );
                // The transition check needs only the previous summaries, not the whole report.
                let previous_summaries = borrow_analysis.analysis.public_call_summaries.clone();
                // The current report already analysed this exact HIR and summary set.
                if summaries_changed {
                    increment_convergence_counter(FrontendCounter::ConvergenceBaseBorrowPasses);
                    borrow_analysis = timed_stage_attributed!(
                        crate::timing::TimingMetric::FrontendBorrowConverge,
                        timing_context,
                        check_borrows(compiler, hir_module, warnings, None)
                    )?;
                }
                // A reused bootstrap report still carries unset failure bits, so inference runs
                // whether or not borrow analysis was repeated.
                infer_builtin_failure_summaries(hir_module, &mut borrow_analysis)?;
                let summary_changes = base_summary_changes(
                    hir_module,
                    &previous_summaries,
                    &borrow_analysis.analysis.public_call_summaries,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
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
                let source_identity_handle = FrozenIdentityHandle::for_domain(
                    identity.declaration().module_origin().package().clone(),
                );
                let summary = {
                    let sidecar =
                        generated_transaction
                            .sidecar_mut(&identity)
                            .map_err(|error| {
                                CompilerMessages::from_error_ref(error, &compiler.string_table)
                            })?;
                    let summaries_changed = install_convergence_summaries(
                        &mut sidecar.module.executable.hir,
                        &direct_summaries.generated,
                        &direct_summaries.active_public,
                        &direct_summaries.module_private,
                    );
                    // Materialisation or an earlier visit already analysed this exact HIR and
                    // summary set.
                    if summaries_changed {
                        increment_convergence_counter(
                            FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses,
                        );
                        sidecar.module.executable.borrow_analysis = timed_stage_attributed!(
                            crate::timing::TimingMetric::FrontendGeneratedBorrowRecheck,
                            timing_context,
                            check_borrows(
                                compiler,
                                &sidecar.module.executable.hir,
                                &sidecar.module.metadata.warnings,
                                Some(&source_identity_handle),
                            )
                        )?;
                    }
                    // Materialisation reports carry unset failure bits, so inference always runs.
                    infer_builtin_failure_summaries(
                        &sidecar.module.executable.hir,
                        &mut sidecar.module.executable.borrow_analysis,
                    )?;
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

    validate_builtin_failure_boundaries(
        compiler,
        hir_module,
        &borrow_analysis,
        type_environment,
        warnings,
        None,
    )?;
    for node_id in convergence_model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = convergence_model.node(node_id) else {
            return Err(CompilerError::compiler_error(
                "generated convergence node has no generated identity",
            )
            .into());
        };
        let sidecar = generated_transaction.sidecar_mut(identity)?;
        let source_owner = FrozenIdentityHandle::for_domain(
            identity.declaration().module_origin().package().clone(),
        );
        validate_builtin_failure_boundaries(
            compiler,
            &sidecar.module.executable.hir,
            &sidecar.module.executable.borrow_analysis,
            &sidecar.module.executable.type_environment,
            &sidecar.module.metadata.warnings,
            Some(&source_owner),
        )?;
    }
    // Unchanged base HIR keeps the link facts the service collected before convergence.
    let refreshed = refresh_private_failure_lanes(
        compiler,
        hir_module,
        &borrow_analysis,
        type_environment,
        warnings,
        None,
    )?;
    if let Some(report) = refreshed {
        *function_link_facts = collect_module_function_link_facts(hir_module)
            .map_err(PremergeFailure::Infrastructure)?;
        borrow_analysis = report;
    }
    for node_id in convergence_model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = convergence_model.node(node_id) else {
            return Err(CompilerError::compiler_error(
                "generated convergence node has no generated identity",
            )
            .into());
        };
        let identity = identity.clone();
        let source_owner = FrozenIdentityHandle::for_domain(
            identity.declaration().module_origin().package().clone(),
        );
        let sidecar = generated_transaction.sidecar_mut(&identity)?;
        let refreshed = refresh_private_failure_lanes(
            compiler,
            &mut sidecar.module.executable.hir,
            &sidecar.module.executable.borrow_analysis,
            &mut sidecar.module.executable.type_environment,
            &sidecar.module.metadata.warnings,
            Some(&source_owner),
        )?;
        // Unchanged sidecar HIR keeps the link facts materialisation collected from it.
        if let Some(report) = refreshed {
            sidecar.module.link_facts.functions =
                collect_module_function_link_facts(&sidecar.module.executable.hir)
                    .map_err(PremergeFailure::Infrastructure)?;
            sidecar.module.executable.borrow_analysis = report;
        }
        // The unchanged lane shares this barrier: every newly completed sidecar
        // releases spare construction capacity before the transaction publishes it.
        sidecar.module.executable.hir.expressions.freeze();
    }
    Ok(borrow_analysis)
}

/// Install private failure lanes and re-run analysis of the rewritten HIR.
///
/// Returns `None` when installation left the HIR unchanged, so the converged report stands.
fn refresh_private_failure_lanes(
    compiler: &mut CompilerFrontend<'_>,
    hir: &mut HirModule,
    report: &BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    warnings: &[CompilerDiagnostic],
    source_owner: Option<&FrozenIdentityHandle>,
) -> Result<Option<BorrowCheckReport>, PremergeFailure> {
    let installation =
        install_private_failure_lanes(hir, report, type_environment).map_err(|failure| {
            let messages = match failure {
                HirConstructionFailure::Diagnosed(diagnostic) => {
                    CompilerMessages::from_diagnostic_with_warnings(
                        diagnostic,
                        warnings.to_vec(),
                        &compiler.string_table,
                    )
                    .with_type_context_for_all_diagnostics(type_environment.clone())
                }
                HirConstructionFailure::Infrastructure(error) => {
                    CompilerMessages::from_error_ref(error, &compiler.string_table)
                }
            };
            let mut failure = PremergeFailure::from(messages);
            if let Some(owner) = source_owner {
                failure.set_frozen_identity_handle_if_missing(owner.clone());
            }
            failure
        })?;
    if installation == PrivateFailureLaneInstallation::Unchanged {
        return Ok(None);
    }
    let mut refreshed = check_borrows(compiler, hir, warnings, source_owner)?;
    infer_builtin_failure_summaries(hir, &mut refreshed)
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    Ok(Some(refreshed))
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

/// Install one node's exact direct-call summaries, reporting whether any summary changed.
///
/// WHY: borrow analysis is a pure function of the HIR and these summaries, so the queue may
///      reuse the node's current report when installation changed nothing.
pub(crate) fn install_convergence_summaries(
    hir: &mut HirModule,
    generated_summaries: &[(GeneratedFunctionIdentity, PublicCallSummary)],
    active_public_summaries: &[(OriginFunctionId, PublicCallSummary)],
    private_summaries: &[(ModulePrivateExecutableIdentity, PublicCallSummary)],
) -> bool {
    let generated_unchanged = hir.generated_call_summaries.len() == generated_summaries.len()
        && generated_summaries
            .iter()
            .all(|(identity, summary)| hir.generated_call_summaries.get(identity) == Some(summary));
    if !generated_unchanged {
        hir.generated_call_summaries.clear();
        for (identity, summary) in generated_summaries {
            hir.generated_call_summaries
                .insert(identity.clone(), summary.clone());
        }
    }
    let mut changed = !generated_unchanged;

    // Active-base public calls are represented as CrossModule targets in generated HIR. Update
    // only those stable origins; provider and cross-boundary imports remain fixed bootstrap
    // leaves in the same imported-summary map.
    for (origin, summary) in active_public_summaries {
        changed |= hir
            .imported_call_summaries
            .insert(origin.clone(), summary.clone())
            .as_ref()
            != Some(summary);
    }

    // Provider-private summaries remain fixed bootstrap leaves. Active-base private identities
    // receive exact replacements, while no complete private map is rebuilt or retained.
    for (identity, summary) in private_summaries {
        changed |= hir
            .module_private_call_summaries
            .insert(identity.clone(), summary.clone())
            .as_ref()
            != Some(summary);
    }
    changed
}

/// Resolve a semantic pending-call contributor against an exact retained summary.
/// Missing summaries are infrastructure failures, never evidence that a call is infallible.
fn retained_call_summary<'a>(
    hir: &'a HirModule,
    report: &'a BorrowCheckReport,
    target: &CallTarget,
) -> Result<Option<&'a PublicCallSummary>, CompilerError> {
    Ok(match target {
        CallTarget::Local(function) => report.analysis.public_call_summaries.get(function),
        CallTarget::CrossModule(origin) => hir.imported_call_summaries.get(origin),
        CallTarget::ModulePrivate(identity) => hir.module_private_call_summaries.get(identity),
        CallTarget::Generated(identity) => hir.generated_call_summaries.get(identity),
        CallTarget::External(_) => {
            return Err(CompilerError::compiler_error(
                "external call cannot contribute an implicit builtin failure",
            ));
        }
    })
}

fn call_escapes_builtin_failure(
    hir: &HirModule,
    report: &BorrowCheckReport,
    target: &CallTarget,
) -> Result<bool, CompilerError> {
    let summary = retained_call_summary(hir, report, target)?.ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "implicit builtin failure contributor has no exact call summary for {target:?}"
        ))
    })?;
    Ok(summary.escapes_builtin_failure)
}

fn active_builtin_failure_contributor<'a>(
    hir: &'a HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
) -> Result<Option<&'a HirBuiltinFailureContributor>, CompilerError> {
    let facts = function_failure_facts(hir, function)?;
    let contributors = facts.contributors.iter().filter_map(|contributor| {
        match builtin_failure_contributor_is_active(hir, report, contributor) {
            Ok(active) => active.then_some(Ok(contributor)),
            Err(error) => Some(Err(error)),
        }
    });
    select_failure_witness(contributors, HirBuiltinFailureContributor::witness_site)
}

fn function_failure_facts(
    hir: &HirModule,
    function: FunctionId,
) -> Result<&HirFunctionFailureFacts, CompilerError> {
    hir.function_failure_facts.get(&function).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "implicit builtin failure analysis is missing semantic facts for {function:?}"
        ))
    })
}

/// Numeric contributors are always active once their code set is validated; calls are active
/// while their exact summary escapes builtin failure.
fn builtin_failure_contributor_is_active(
    hir: &HirModule,
    report: &BorrowCheckReport,
    contributor: &HirBuiltinFailureContributor,
) -> Result<bool, CompilerError> {
    match &contributor.source {
        HirBuiltinFailureSource::NumericOperation
        | HirBuiltinFailureSource::CompoundWriteBack { .. } => {
            if contributor.codes.is_empty()
                || contributor
                    .codes
                    .iter()
                    .any(|code| !code.is_implicit_failure())
            {
                return Err(CompilerError::compiler_error(
                    "implicit numeric failure contributor must carry nonempty implicit builtin failure codes",
                ));
            }
            Ok(true)
        }
        HirBuiltinFailureSource::Call(target) => call_escapes_builtin_failure(hir, report, target),
    }
}

/// Retain a bounded source witness without changing the converged failure facts or summaries.
///
/// WHAT: depth-first search with an explicit stack of entered bodies, following each body's
///       candidates in policy order. A numeric or write-back candidate is the origin. A callee
///       this HIR does not own is a summary leaf: its hop stays the visible boundary and the
///       search stops without inventing an origin across it. A body that runs out of
///       candidates is popped with its hop.
/// WHY: a recursive call can precede the real producer in source order, so a call into an
///      already-entered body is skipped rather than ending the search. An entered body is
///      either on the current path (a back-edge) or already exhausted. An exhausted body has
///      no origin or leaf reachable without passing through bodies still on the path, so
///      re-entering it can never change the witness. Each body is therefore expanded at most
///      once per search, recorded in the caller's `entered` set, which starts with `function`.
fn builtin_failure_witness_search<'a>(
    hir: &'a HirModule,
    report: &BorrowCheckReport,
    contributor: &'a HirBuiltinFailureContributor,
    entered: &mut FxHashSet<FunctionId>,
) -> Result<BuiltinFailureWitness, CompilerError> {
    let mut witness = BuiltinFailureWitness {
        codes: Vec::new(),
        call_spans: Vec::new(),
        origin_span: None,
        origin: BuiltinFailureOriginKind::Operation,
        elided_call_hops: 0,
    };
    let mut stack: Vec<WitnessCandidates<'a, HirBuiltinFailureContributor>> = Vec::new();
    let mut hops: Vec<Option<SourceSpan>> = Vec::new();
    let mut next = Some(contributor);
    loop {
        if let Some(contributor) = next.take() {
            let target = match &contributor.source {
                HirBuiltinFailureSource::NumericOperation => {
                    witness.codes = contributor.codes.clone();
                    witness.origin_span = contributor.span;
                    witness.origin = BuiltinFailureOriginKind::Operation;
                    break;
                }
                HirBuiltinFailureSource::CompoundWriteBack { target, .. } => {
                    witness.codes = contributor.codes.clone();
                    witness.origin_span = contributor.span;
                    witness.origin =
                        BuiltinFailureOriginKind::CompoundWriteBack { target: *target };
                    break;
                }
                HirBuiltinFailureSource::Call(target) => target,
            };
            // Stable identities resolve only when this HIR owns the callee's semantic facts.
            // Other generated/provider executables remain summary leaves, not donor-local IDs.
            let callee = match target {
                CallTarget::Local(function) => Some(*function),
                CallTarget::ModulePrivate(identity) => {
                    hir.function_ids_by_private_origin.get(identity).copied()
                }
                CallTarget::Generated(identity) => {
                    hir.function_ids_by_generated.get(identity).copied()
                }
                CallTarget::CrossModule(_) | CallTarget::External(_) => None,
            };
            let Some(callee) = callee else {
                hops.push(contributor.span);
                break;
            };
            if entered.insert(callee) {
                hops.push(contributor.span);
                let facts = function_failure_facts(hir, callee)?;
                stack.push(WitnessCandidates::new(&facts.contributors));
            }
        }

        let Some(candidates) = stack.last_mut() else {
            break;
        };
        next = candidates.next_candidate(
            |candidate| builtin_failure_contributor_is_active(hir, report, candidate),
            HirBuiltinFailureContributor::witness_site,
        )?;
        if next.is_none() {
            stack.pop();
            hops.pop();
        }
    }

    retain_bounded_witness_hops(&mut witness, hops);
    Ok(witness)
}

fn builtin_failure_witness(
    hir: &HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
    contributor: &HirBuiltinFailureContributor,
) -> Result<BuiltinFailureWitness, CompilerError> {
    let mut entered = FxHashSet::from_iter([function]);
    builtin_failure_witness_search(hir, report, contributor, &mut entered)
}

/// Join immutable semantic failure facts into the exact summaries computed for this queue node.
///
/// Local recursion reaches its finite boolean fixed point here; generated and provider calls
/// resolve through the summaries installed by the canonical convergence queue.
pub(crate) fn infer_builtin_failure_summaries(
    hir: &HirModule,
    report: &mut BorrowCheckReport,
) -> Result<(), CompilerError> {
    for function in &hir.functions {
        let summary = report
            .analysis
            .public_call_summaries
            .get_mut(&function.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "implicit builtin failure analysis is missing call summary for {:?}",
                    function.id
                ))
            })?;
        summary.escapes_builtin_failure = false;
    }
    loop {
        let mut changed = false;
        for function in &hir.functions {
            let active = active_builtin_failure_contributor(hir, report, function.id)?.is_some();
            let infer = hir
                .function_failure_facts
                .get(&function.id)
                .is_some_and(|facts| {
                    matches!(facts.boundary, HirBuiltinFailureBoundary::InferPrivate)
                });
            let summary = report
                .analysis
                .public_call_summaries
                .get_mut(&function.id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "implicit builtin failure analysis lost a call summary",
                    )
                })?;
            if infer && active && !summary.escapes_builtin_failure {
                summary.escapes_builtin_failure = true;
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
    }
}

pub(crate) fn builtin_failure_diagnostic(
    hir: &HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let facts = hir.function_failure_facts.get(&function).ok_or_else(|| {
        CompilerError::compiler_error("builtin failure boundary lost its semantic facts")
    })?;
    for check in &facts.deferred_custom_catches {
        let mut eligible = false;
        for candidate in &check.candidates {
            let HirBuiltinFailureSource::Call(target) = &candidate.source else {
                return Err(CompilerError::compiler_error(
                    "deferred custom catch contributor is not a private call",
                ));
            };
            if call_escapes_builtin_failure(hir, report, target)? {
                eligible = true;
                if check.eligibility_only {
                    break;
                }
                return Ok(Some(CompilerDiagnostic::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                        error_type_id: check.error_type_id,
                        typed_producer_span: check.typed_producer_span,
                        implicit_producer_span: candidate.span,
                    },
                    check.catch_span,
                )));
            }
        }
        if check.eligibility_only && !eligible {
            return Ok(Some(CompilerDiagnostic::invalid_fallible_handling(
                InvalidFallibleHandlingReason::CatchOnNonFallible,
                check.catch_span,
            )));
        }
    }
    for contributor in &facts.assertion_message_calls {
        let HirBuiltinFailureSource::Call(target) = &contributor.source else {
            return Err(CompilerError::compiler_error(
                "deferred assertion message contributor is not a call",
            ));
        };
        if retained_call_summary(hir, report, target)?
            .is_none_or(|summary| summary.escapes_builtin_failure)
        {
            return Ok(Some(CompilerDiagnostic::invalid_fallible_handling(
                InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
                contributor.span,
            )));
        }
    }
    let Some(contributor) = active_builtin_failure_contributor(hir, report, function)? else {
        return Ok(None);
    };
    let reason = match facts.boundary {
        HirBuiltinFailureBoundary::InferPrivate | HirBuiltinFailureBoundary::BuiltinErrorSlot => {
            return Ok(None);
        }
        HirBuiltinFailureBoundary::CustomErrorSlot(error_type_id) => {
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id,
                witness: Box::new(builtin_failure_witness(hir, report, function, contributor)?),
            }
        }
        HirBuiltinFailureBoundary::ExportedNoSlot => {
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness: Box::new(builtin_failure_witness(hir, report, function, contributor)?),
            }
        }
    };
    Ok(Some(CompilerDiagnostic::invalid_fallible_handling(
        reason, facts.span,
    )))
}

fn validate_builtin_failure_boundaries(
    compiler: &mut CompilerFrontend<'_>,
    hir: &HirModule,
    report: &BorrowCheckReport,
    type_environment: &TypeEnvironment,
    warnings: &[CompilerDiagnostic],
    source_owner: Option<&FrozenIdentityHandle>,
) -> Result<(), PremergeFailure> {
    for function in &hir.functions {
        let Some(mut diagnostic) = builtin_failure_diagnostic(hir, report, function.id)? else {
            continue;
        };
        if let Some(owner) = source_owner {
            diagnostic.attach_frozen_identity_handle_if_missing(owner.clone());
        }
        let mut batch = PremergeDiagnosticBatch::from_parts(
            vec![diagnostic],
            std::mem::take(&mut compiler.string_table),
            vec![RenderTypeContext {
                diagnostic_range: 0..1,
                type_environment: type_environment.clone(),
            }],
            vec![],
        );
        batch.prepend_diagnostics(warnings.iter().cloned().map(|mut warning| {
            if let Some(owner) = source_owner {
                warning.attach_frozen_identity_handle_if_missing(owner.clone());
            }
            warning
        }));
        return Err(batch.into());
    }
    Ok(())
}

pub(crate) fn base_summary_changes(
    hir: &HirModule,
    previous: &FxHashMap<FunctionId, PublicCallSummary>,
    next: &FxHashMap<FunctionId, PublicCallSummary>,
) -> Result<BaseSummaryChanges, CompilerError> {
    let mut widened_functions = FxHashSet::default();
    for function in &hir.functions {
        let previous_summary = previous.get(&function.id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "previous base borrow report is missing summary for {:?}",
                function.id
            ))
        })?;
        let next_summary = next.get(&function.id).ok_or_else(|| {
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

pub(in crate::compiler_frontend) fn install_exact_concrete_call_summaries(
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
