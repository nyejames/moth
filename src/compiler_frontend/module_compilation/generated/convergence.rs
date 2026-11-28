//! HIR-derived generated-summary convergence for one module compilation.
//!
//! WHAT: builds the transient call-dependency model and runs the monotone dirty queue that
//!       propagates exact base and generated call summaries through materialised sidecars, plus
//!       the exact-summary installation each pass depends on. Immutable semantic failure facts
//!       join the same queue; only private no-slot functions infer an escaping builtin-failure bit.
//! WHY: validated HIR owns executable call topology, and the generated store never analyses it.
//!       Allocation and failure facts settle before diagnostic transfer; subsequent mutation
//!       widening reuses those transfer facts rather than checking borrows again.
//! Failure boundary diagnostics run before lane installation. An explicit Error! slot consumes
//! implicit builtin failure, a custom error slot requires local handling, and exported no-slot
//! functions may not publish an inferred contract.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    WitnessCandidates, retain_bounded_witness_hops, select_failure_witness,
};
use crate::compiler_frontend::ast::generic_functions::{
    ModuleMaterialisationPreparedBuilder, StableFunctionTarget,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::invalid_fallible_handling;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::invalid_trait_conformance;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, DiagnosticBag, InvalidFallibleHandlingReason,
    InvalidTraitConformanceReason, PreparedFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::constraint_verification::{
    ConstraintFailure, ConstraintModuleSite, ConstraintUniverse,
};
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
    collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::utils::for_each_terminator_target;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::artefact::Module;
use crate::compiler_frontend::module_compilation::context::ModuleCompilationContext;
use crate::compiler_frontend::module_compilation::external_imports::{
    collect_external_import_candidates_for_packages, collect_external_import_package_ids,
};
use crate::compiler_frontend::public_call_summary::{
    PublicCallMutationEffect, PublicCallSummary, PublicCallSummaryTransition,
    same_typed_allocation_transfer_as, validate_public_call_summary_transition,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};

use crate::compiler_frontend::hir::ids::FunctionId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::sync::Arc;

use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedFunctionTransaction;
use crate::compiler_frontend::module_compilation::stages::{
    check_borrows, refresh_call_allocations, refresh_call_mutation_effects,
};
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
    ) -> Self {
        Self::build(base, generated, &FxHashSet::default(), None)
    }

    pub(crate) fn from_link_facts_for_base_callees<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: &FxHashSet<ModulePrivateExecutableIdentity>,
    ) -> Self {
        Self::build(
            base,
            generated,
            base_public_origins,
            Some(base_private_identities),
        )
    }

    fn for_active_transaction(
        base: &HirModule,
        base_link_facts: &HirModuleLinkFacts,
        transaction: &GeneratedFunctionTransaction<'_>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: &FxHashSet<ModulePrivateExecutableIdentity>,
    ) -> Self {
        let mut model = Self::from_link_facts_for_base_callees(
            base_link_facts,
            transaction.completed_link_facts(),
            base_public_origins,
            base_private_identities,
        );
        model.include_failure_dependencies(
            base,
            transaction,
            base_public_origins,
            base_private_identities,
        );
        model
    }

    fn build<'a>(
        base: &HirModuleLinkFacts,
        generated: impl IntoIterator<Item = (&'a GeneratedFunctionIdentity, &'a HirModuleLinkFacts)>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: Option<&FxHashSet<ModulePrivateExecutableIdentity>>,
    ) -> Self {
        let mut generated = generated.into_iter().collect::<Vec<_>>();
        generated.sort_by_key(|(identity, _)| *identity);
        for pair in generated.windows(2) {
            if pair[0].0 == pair[1].0 {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "convergence model received duplicate generated identity {:?}",
                    pair[0].0
                );
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

        Self {
            nodes,
            callers,
            ids_by_generated,
        }
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

    /// Include failure dependencies not yet linked in the runtime CFG. In particular, the
    /// installer follows a newly routed catch immediately and needs current lane decisions for
    /// any statement calls it encounters there.
    fn include_failure_dependencies(
        &mut self,
        base: &HirModule,
        transaction: &GeneratedFunctionTransaction<'_>,
        base_public_origins: &FxHashSet<OriginFunctionId>,
        base_private_identities: &FxHashSet<ModulePrivateExecutableIdentity>,
    ) {
        for index in 0..self.nodes.len() {
            let hir = match &self.nodes[index].node {
                ConvergenceNode::BaseModule => base,
                ConvergenceNode::Generated(identity) => {
                    &transaction.sidecar(identity).module.executable.hir
                }
            };
            let mut targets = deferred_validation_call_targets(hir);
            append_latent_handler_calls(hir, &mut targets);
            // Retained contributors and latent statement calls must resolve even if there is
            // no runtime edge yet. Assertion-only calls may lack a summary and diagnose at the
            // boundary instead of forcing an executable dependency.
            targets.retain(|target| {
                let CallTarget::Generated(identity) = &target.target else {
                    return true;
                };
                if transaction.summary(identity).is_some() {
                    return true;
                }
                if matches!(target.summary_requirement, SummaryRequirement::Required) {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "required failure-lane dependency has no materialised generated summary for {identity:?}"
                    );
                }
                false
            });
            add_model_edges(
                &mut self.callers,
                &self.ids_by_generated,
                &mut self.nodes[index],
                ConvergenceNodeId(index),
                base_public_origins,
                Some(base_private_identities),
                targets.into_iter().map(|target| target.target),
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
    }
}

/// Whether convergence must find a materialised summary for one collected call target.
///
/// WHAT: runtime contributors and catch candidates require an exact summary; assertion-message
///       calls use one only when it already exists.
/// WHY: naming the two cases keeps collection and availability filtering from depending on an
///      opaque boolean literal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SummaryRequirement {
    Required,
    Tolerated,
}

struct DeferredValidationTarget {
    target: CallTarget,
    summary_requirement: SummaryRequirement,
}

/// Call targets whose summaries convergence needs before it can judge failure.
///
/// WHAT: runtime contributors and deferred catch candidates require a materialised summary, while
///       assertion-message calls keep the existing tolerance for an absent one.
/// WHY: the first two decide an escaping-failure verdict; the third only validates an inert
///      message and must not invent a runtime dependency just to make its validation possible.
fn deferred_validation_call_targets(hir: &HirModule) -> Vec<DeferredValidationTarget> {
    let mut targets = Vec::new();
    let mut push = |contributor: &HirBuiltinFailureContributor, summary_requirement| {
        if let HirBuiltinFailureSource::Call(target) = &contributor.source {
            targets.push(DeferredValidationTarget {
                target: target.clone(),
                summary_requirement,
            });
        }
    };
    for facts in hir.function_failure_facts.values() {
        for contributor in &facts.contributors {
            push(contributor, SummaryRequirement::Required);
        }
        for check in &facts.deferred_custom_catches {
            for candidate in &check.candidates {
                push(candidate, SummaryRequirement::Required);
            }
        }
        for contributor in &facts.assertion_message_calls {
            push(contributor, SummaryRequirement::Tolerated);
        }
    }
    targets
}

fn append_latent_handler_calls(hir: &HirModule, targets: &mut Vec<DeferredValidationTarget>) {
    // Protected-call edges are not linked until lane installation. Overlapping handlers share
    // one walk, and only statement calls need their lane contract before the rewrite.
    let mut pending = hir
        .catch_handlers
        .iter()
        .map(|record| record.handler.block)
        .collect::<Vec<_>>();
    let mut visited = FxHashSet::default();
    while let Some(block_id) = pending.pop() {
        if !visited.insert(block_id) {
            continue;
        }
        let block = hir.block(block_id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "latent catch handler reaches unknown HIR block {block_id:?}"
            )
        });
        for statement in &block.statements {
            if let HirStatementKind::Call { target, .. } = &statement.kind
                && !matches!(target, CallTarget::Local(_) | CallTarget::External(_))
            {
                targets.push(DeferredValidationTarget {
                    target: target.clone(),
                    summary_requirement: SummaryRequirement::Required,
                });
            }
        }
        for_each_terminator_target(&block.terminator, |target| pending.push(target));
    }
}

fn add_model_edges(
    callers: &mut [Vec<ConvergenceNodeId>],
    ids_by_generated: &FxHashMap<GeneratedFunctionIdentity, ConvergenceNodeId>,
    record: &mut ConvergenceNodeRecord,
    caller: ConvergenceNodeId,
    base_public_origins: &FxHashSet<OriginFunctionId>,
    base_private_identities: Option<&FxHashSet<ModulePrivateExecutableIdentity>>,
    targets: impl IntoIterator<Item = CallTarget>,
) {
    for target in targets {
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
                let callee = ids_by_generated.get(identity.as_ref()).copied();
                record.generated_callees.push(*identity);
                callee
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
    reason = "convergence keeps shared compilation context and independently mutable stage owners as separate borrows"
)]
pub(in crate::compiler_frontend::module_compilation) fn run_generated_summary_convergence(
    context: &ModuleCompilationContext<'_>,
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &mut HirModule,
    function_link_facts: &mut HirModuleLinkFacts,
    generated_transaction: &mut GeneratedFunctionTransaction<'_>,
    bootstrap_borrow_analysis: BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    warnings: &mut DiagnosticBag,
    base_projection: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext<'_>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<BorrowCheckReport, PreparedFailure> {
    let projection_registry = std::sync::Arc::clone(compiler.external_package_registry);
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
    let mut convergence_model = ConvergenceModel::for_active_transaction(
        hir_module,
        function_link_facts,
        generated_transaction,
        &base_public_origins,
        &base_private_identities,
    );
    let mut convergence_queue = VecDeque::new();
    let mut queued_nodes = vec![false; convergence_model.node_count()];
    for node_id in convergence_model.all_node_ids() {
        convergence_queue.push_back(node_id);
        queued_nodes[node_id.index()] = true;
    }

    let mut borrow_analysis = bootstrap_borrow_analysis;
    let mut phase = ConvergencePhase::Failure;
    let mut first_visits = convergence_model.node_count();
    loop {
        let Some(node_id) = convergence_queue.pop_front() else {
            phase = match phase {
                ConvergencePhase::Failure => {
                    if install_convergence_lanes(
                        compiler,
                        hir_module,
                        function_link_facts,
                        generated_transaction,
                        &borrow_analysis,
                        type_environment,
                        warnings,
                        &convergence_model,
                        #[cfg(feature = "timers")]
                        timing_context,
                    )? {
                        // Newly routed handlers may add executable dependencies. Node
                        // identities stay stable because installation doesn't add sidecars.
                        convergence_model = ConvergenceModel::for_active_transaction(
                            hir_module,
                            function_link_facts,
                            generated_transaction,
                            &base_public_origins,
                            &base_private_identities,
                        );
                    }
                    // Declaration contracts and escape proofs survive the rewrite. Body
                    // allocation facts are still pending, so no pre-rewrite CFG facts enter
                    // the final-HIR fixed point, even across base/sidecar recursion.
                    ConvergencePhase::Allocation
                }
                ConvergencePhase::Allocation => ConvergencePhase::Diagnostics,
                ConvergencePhase::Diagnostics => break,
            };
            first_visits = convergence_model.node_count();
            for node_id in convergence_model.all_node_ids() {
                enqueue_convergence_node(node_id, &mut convergence_queue, &mut queued_nodes);
            }
            continue;
        };
        queued_nodes[node_id.index()] = false;
        let first_visit = first_visits != 0;
        first_visits = first_visits.saturating_sub(1);
        let node = convergence_model.node(node_id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "convergence queue received unknown node {node_id:?}"
            )
        });
        let direct_summaries = direct_convergence_summaries(
            &convergence_model,
            node_id,
            generated_transaction,
            hir_module,
            &borrow_analysis,
        )
        .map_err(PreparedFailure::from)?;

        match node {
            ConvergenceNode::BaseModule => {
                let input_changes = install_convergence_summaries(
                    hir_module,
                    &direct_summaries.generated,
                    &direct_summaries.active_public,
                    &direct_summaries.module_private,
                );
                let Some((visit, changed_callers)) =
                    BorrowVisit::select(phase, first_visit, &input_changes, function_link_facts)
                else {
                    continue;
                };
                if let BorrowVisit::Check = visit {
                    increment_convergence_counter(
                        FrontendCounter::ConvergenceInitialBaseBorrowPasses,
                    );
                } else if matches!(visit, BorrowVisit::Summarize) && !first_visit {
                    increment_convergence_counter(FrontendCounter::ConvergenceBaseBorrowPasses);
                }
                let previous_summaries = timed_stage_attributed!(
                    crate::timing::TimingMetric::FrontendBorrowConverge,
                    timing_context,
                    visit.update(
                        compiler,
                        hir_module,
                        type_environment,
                        warnings,
                        None,
                        &mut borrow_analysis,
                        (function_link_facts, &changed_callers, base_projection),
                        !matches!(visit, BorrowVisit::Summarize)
                            || convergence_model.node_count() != 1,
                    )
                )?;
                let Some(previous_summaries) = previous_summaries else {
                    // The local solver validated every transition and there are no sidecars.
                    continue;
                };
                let summary_changes = base_summary_changes(
                    hir_module,
                    &previous_summaries,
                    &borrow_analysis.analysis.public_call_summaries,
                )
                .map_err(PreparedFailure::from)?;
                if !summary_changes.is_empty() {
                    enqueue_base_dependents(
                        &convergence_model,
                        &summary_changes,
                        &mut convergence_queue,
                        &mut queued_nodes,
                    );
                }
            }
            ConvergenceNode::Generated(identity) => {
                let identity = identity.as_ref();
                let source_identity_handle = FrozenIdentityHandle::for_domain(
                    identity.declaration().module_origin().package().clone(),
                );
                let summary = {
                    let (sidecar, sidecar_warnings) =
                        generated_transaction.sidecar_and_warnings_mut(identity);
                    let input_changes = install_convergence_summaries(
                        &mut sidecar.module.executable.hir,
                        &direct_summaries.generated,
                        &direct_summaries.active_public,
                        &direct_summaries.module_private,
                    );
                    let links = &sidecar.module.link_facts.functions;
                    let Some((visit, changed_callers)) =
                        BorrowVisit::select(phase, first_visit, &input_changes, links)
                    else {
                        continue;
                    };
                    if matches!(visit, BorrowVisit::Check)
                        || (matches!(visit, BorrowVisit::Summarize) && !first_visit)
                    {
                        increment_convergence_counter(
                            FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses,
                        );
                    }
                    let executable = &mut sidecar.module.executable;
                    timed_stage_attributed!(
                        crate::timing::TimingMetric::FrontendGeneratedBorrowRecheck,
                        timing_context,
                        crate::compiler_frontend::ast::generic_functions::with_registered_type_projection(
                            &executable.type_environment,
                            &projection_registry,
                            |projection| visit.update(
                                compiler,
                                &executable.hir,
                                &executable.type_environment,
                                sidecar_warnings,
                                Some(&source_identity_handle),
                                &mut executable.borrow_analysis,
                                (links, &changed_callers, projection),
                                false,
                            ),
                        )
                    )?;
                    let summary = generated_root_summary(identity, &sidecar.module);
                    if (first_visit
                        && matches!(visit, BorrowVisit::InferFailure)
                        && !summary.escapes_builtin_failure)
                        || (matches!(visit, BorrowVisit::Check)
                            && summary.parameters.iter().all(|parameter| {
                                parameter.mutation == PublicCallMutationEffect::NoWrite
                            }))
                    {
                        // The first failure/diagnostic visit can only set its own component.
                        continue;
                    }
                    summary.clone()
                };
                if update_generated_summary(generated_transaction, identity, summary)
                    .map_err(PreparedFailure::from)?
                {
                    enqueue_convergence_callers(
                        &convergence_model,
                        node_id,
                        &mut convergence_queue,
                        &mut queued_nodes,
                    );
                }
            }
        }
    }

    discard_validated_failure_captures(hir_module);
    for node_id in convergence_model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = convergence_model.node(node_id) else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "generated convergence node has no generated identity"
            );
        };
        discard_validated_failure_captures(
            &mut generated_transaction
                .sidecar_mut(identity)
                .module
                .executable
                .hir,
        );
    }

    // Phase B owns the proof bits. Run it only after allocation, diagnostic transfer, and
    // mutation effects are settled, and do not rebuild a summary afterwards.
    crate::compiler_frontend::module_compilation::generated::constraint_convergence::
        converge_constraint_summaries(
            hir_module,
            &mut borrow_analysis,
            generated_transaction,
            compiler.external_package_registry.as_ref(),
        )
        .map_err(PreparedFailure::from)?;

    // Verification runs immediately after phase B, base module first and then each completed
    // sidecar in sorted identity order. It reads the final summaries and final HIR, never the
    // backend proof table, and the first module with diagnostics owns the prepared failure. A
    // module set with no source-constraint boundary skips the snapshot entirely: phase B has
    // already published the bits, so verification would have nothing to check.
    let verification = ConstraintUniverse::build(
        hir_module,
        &borrow_analysis,
        generated_transaction,
        compiler.external_package_registry.as_ref(),
    )
    .map_err(PreparedFailure::from)?
    .and_then(|universe| {
        universe.verify(
            compiler.source_files.as_ref(),
            &mut compiler.string_table,
            &compiler.path_fork,
        )
    });
    if let Some(failure) = verification {
        let ConstraintFailure { site, diagnostics } = failure;
        let environment = match &site {
            ConstraintModuleSite::Base => Some(&*type_environment),
            ConstraintModuleSite::Sidecar(identity) => Some(
                &generated_transaction
                    .sidecar(identity)
                    .module
                    .executable
                    .type_environment,
            ),
        };
        return Err(PreparedFailure::capture_diagnosed(
            diagnostics,
            compiler.string_table.take_for_handoff(),
            environment,
            Some(Arc::new(compiler.path_fork.shared_table())),
        ));
    }

    for node_id in convergence_model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = convergence_model.node(node_id) else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "generated convergence node has no generated identity"
            );
        };
        let identity = identity.clone();
        let sidecar = generated_transaction.sidecar_mut(&identity);
        retain_live_call_summaries(
            &mut sidecar.module.executable.hir,
            &sidecar.module.link_facts.functions,
        );
        // The publication owner computes the proof table once, from the final HIR after every
        // rewrite and summary refresh above, so the paired facts match what gets published.
        sidecar.module.executable.numeric_proofs = analyse_numeric_proofs(
            &sidecar.module.executable.hir,
            &sidecar.module.executable.type_environment,
        );
        // The unchanged lane shares this barrier: every newly completed sidecar
        // releases spare construction capacity before the transaction publishes it.
        sidecar.module.executable.hir.expressions.freeze();
    }
    retain_live_call_summaries(hir_module, function_link_facts);
    let reachable_sidecars =
        reachable_generated_sidecars(function_link_facts, generated_transaction);
    generated_transaction.retain_reachable_sidecars(&reachable_sidecars);
    // Only executable sidecars survive this transaction. Rebuild their availability facts from
    // the final selected root, after failure-lane rewrites and validation-only demand removal.
    for node_id in convergence_model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = convergence_model.node(node_id) else {
            continue;
        };
        if !reachable_sidecars.contains(identity.as_ref()) {
            continue;
        }
        let sidecar = generated_transaction.sidecar_mut(identity);
        let root = sidecar
            .module
            .executable
            .hir
            .function_ids_by_generated
            .get(identity.as_ref())
            .copied()
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated sidecar {identity:?} lost its selected root before publication"
                )
            });
        let reachability = collect_reachability_from_function_link_facts(
            &sidecar.module.link_facts.functions,
            &[root],
        )?;
        let registry = &sidecar.module.link_facts.external_package_registry;
        let package_ids = collect_external_import_package_ids(
            reachability.reachable_external_calls.iter(),
            registry,
        );
        sidecar.module.link_facts.external_import_candidates =
            collect_external_import_candidates_for_packages(
                &package_ids,
                context.external_dependency_resolution_table,
                context.builder_runtime_packages,
                registry,
            )?;
    }
    Ok(borrow_analysis)
}

/// Drop source-validation captures after convergence has checked them; runtime contributors remain.
fn discard_validated_failure_captures(hir: &mut HirModule) {
    for facts in hir.function_failure_facts.values_mut() {
        facts.assertion_message_calls.clear();
        facts.deferred_custom_catches.clear();
    }
}

fn retain_live_call_summaries(hir: &mut HirModule, link_facts: &HirModuleLinkFacts) {
    let mut live_imported = FxHashSet::default();
    let mut live_private = FxHashSet::default();
    let mut live_generated = FxHashSet::default();
    for target in link_facts.direct_call_targets() {
        match target {
            CallTarget::CrossModule(identity) => {
                live_imported.insert(identity);
            }
            CallTarget::ModulePrivate(identity) => {
                live_private.insert(identity);
            }
            CallTarget::Generated(identity) => {
                live_generated.insert(*identity);
            }
            CallTarget::Local(_) | CallTarget::External(_) => {}
        }
    }
    hir.imported_call_summaries
        .retain(|identity, _| live_imported.contains(identity));
    hir.module_private_call_summaries
        .retain(|identity, _| live_private.contains(identity));
    hir.generated_call_summaries
        .retain(|identity, _| live_generated.contains(identity));
}

fn reachable_generated_sidecars(
    base_link_facts: &HirModuleLinkFacts,
    transaction: &GeneratedFunctionTransaction<'_>,
) -> FxHashSet<GeneratedFunctionIdentity> {
    let generated_callees = transaction
        .completed_link_facts()
        .map(|(identity, link_facts)| {
            let callees = link_facts
                .direct_call_targets()
                .filter_map(|target| match target {
                    CallTarget::Generated(identity) => Some(*identity),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (identity.clone(), callees)
        })
        .collect::<FxHashMap<_, _>>();
    let mut pending = base_link_facts
        .direct_call_targets()
        .filter_map(|target| match target {
            CallTarget::Generated(identity) => Some(*identity),
            _ => None,
        })
        .collect::<VecDeque<_>>();
    let mut reachable = FxHashSet::default();
    while let Some(identity) = pending.pop_front() {
        if !reachable.insert(identity.clone()) {
            continue;
        }
        if let Some(callees) = generated_callees.get(&identity) {
            pending.extend(callees.iter().cloned());
        }
    }
    reachable
}

/// Validate inferred failure boundaries, then install lanes once and refresh rewritten link facts.
#[allow(
    clippy::too_many_arguments,
    reason = "lane installation needs the existing convergence owners without taking ownership"
)]
fn install_convergence_lanes(
    compiler: &mut CompilerFrontend<'_>,
    hir: &mut HirModule,
    link_facts: &mut HirModuleLinkFacts,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    report: &BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    warnings: &mut DiagnosticBag,
    model: &ConvergenceModel,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<bool, PreparedFailure> {
    validate_builtin_failure_boundaries(compiler, hir, report, type_environment, warnings, None)?;
    for node_id in model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = model.node(node_id) else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "generated convergence node has no generated identity"
            );
        };
        let (sidecar, sidecar_warnings) = transaction.sidecar_and_warnings_mut(identity);
        let source_owner = FrozenIdentityHandle::for_domain(
            identity.declaration().module_origin().package().clone(),
        );
        let executable = &sidecar.module.executable;
        validate_builtin_failure_boundaries(
            compiler,
            &executable.hir,
            &executable.borrow_analysis,
            &executable.type_environment,
            sidecar_warnings,
            Some(&source_owner),
        )?;
    }

    let mut changed = install_validated_failure_lanes(
        compiler,
        hir,
        link_facts,
        report,
        type_environment,
        None,
        #[cfg(feature = "timers")]
        timing_context,
    )?;
    for node_id in model.generated_node_ids() {
        let Some(ConvergenceNode::Generated(identity)) = model.node(node_id) else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "generated convergence node has no generated identity"
            );
        };
        let source_owner = FrozenIdentityHandle::for_domain(
            identity.declaration().module_origin().package().clone(),
        );
        let sidecar = transaction.sidecar_mut(identity);
        let executable = &mut sidecar.module.executable;
        changed |= install_validated_failure_lanes(
            compiler,
            &mut executable.hir,
            &mut sidecar.module.link_facts.functions,
            &executable.borrow_analysis,
            &mut executable.type_environment,
            Some(&source_owner),
            #[cfg(feature = "timers")]
            timing_context,
        )?;
    }
    Ok(changed)
}

fn install_validated_failure_lanes(
    compiler: &mut CompilerFrontend<'_>,
    hir: &mut HirModule,
    link_facts: &mut HirModuleLinkFacts,
    report: &BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    source_owner: Option<&FrozenIdentityHandle>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<bool, PreparedFailure> {
    let installation = timed_stage_attributed!(
        if source_owner.is_some() {
            crate::timing::TimingMetric::FrontendGeneratedBorrowRecheck
        } else {
            crate::timing::TimingMetric::FrontendBorrowConverge
        },
        timing_context,
        install_private_failure_lanes(
            hir,
            report,
            type_environment,
            compiler.external_package_registry.as_ref(),
        )
    )
    .map_err(|failure| match failure {
        HirConstructionFailure::Diagnosed(mut diagnostic) => {
            if let Some(owner) = source_owner {
                diagnostic.attach_frozen_identity_handle_if_missing(owner.clone());
            }
            PreparedFailure::capture_diagnosed(
                DiagnosticBag::from(diagnostic),
                compiler.string_table.take_for_handoff(),
                Some(type_environment),
                Some(Arc::new(compiler.path_fork.shared_table())),
            )
        }
        HirConstructionFailure::Infrastructure(error) => PreparedFailure::from(error),
    })?;
    if installation == PrivateFailureLaneInstallation::Unchanged {
        return Ok(false);
    }
    *link_facts = collect_module_function_link_facts(hir).map_err(PreparedFailure::from)?;
    Ok(installation == PrivateFailureLaneInstallation::Rewritten)
}

#[derive(Clone, Copy)]
enum ConvergencePhase {
    Failure,
    Allocation,
    Diagnostics,
}

#[derive(Clone, Copy)]
enum BorrowVisit {
    Summarize,
    InferFailure,
    Check,
    RefreshMutation,
}

impl BorrowVisit {
    fn select(
        phase: ConvergencePhase,
        first_visit: bool,
        changes: &CallInputChanges<'_>,
        links: &HirModuleLinkFacts,
    ) -> Option<(Self, FxHashSet<FunctionId>)> {
        let mut callers = FxHashSet::default();
        let visit = match phase {
            ConvergencePhase::Failure if first_visit || changes.failure => Self::InferFailure,
            ConvergencePhase::Allocation => {
                if !first_visit {
                    callers = changes.allocation_callers(links);
                    if callers.is_empty() {
                        return None;
                    }
                }
                Self::Summarize
            }
            ConvergencePhase::Diagnostics if first_visit => Self::Check,
            ConvergencePhase::Diagnostics if changes.mutation => Self::RefreshMutation,
            _ => return None,
        };
        Some((visit, callers))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "convergence shares the same update for base and generated reports"
    )]
    fn update(
        self,
        compiler: &mut CompilerFrontend<'_>,
        hir: &HirModule,
        types: &TypeEnvironment,
        warnings: &mut DiagnosticBag,
        owner: Option<&FrozenIdentityHandle>,
        report: &mut BorrowCheckReport,
        allocation_inputs: (
            &HirModuleLinkFacts,
            &FxHashSet<FunctionId>,
            &crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext<'_>,
        ),
        compare_base_summaries: bool,
    ) -> Result<Option<FxHashMap<FunctionId, PublicCallSummary>>, PreparedFailure> {
        let previous =
            compare_base_summaries.then(|| report.analysis.public_call_summaries.clone());
        match self {
            Self::Summarize => {
                refresh_call_allocations(
                    compiler,
                    hir,
                    types,
                    warnings,
                    owner,
                    allocation_inputs,
                    report,
                )?;
            }
            Self::InferFailure => infer_builtin_failure_summaries(hir, report),
            Self::Check => check_borrows(
                compiler,
                hir,
                types,
                warnings,
                owner,
                report,
                allocation_inputs.2,
            )?,
            Self::RefreshMutation => {
                refresh_call_mutation_effects(compiler, hir, types, warnings, owner, report)?;
            }
        }
        Ok(previous)
    }
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
) -> Result<DirectConvergenceSummaries, InfrastructureFailure> {
    let generated_callees = model.generated_callees(node_id).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "convergence model is missing generated callees for node {node_id:?}"
        )
    });
    let generated_summaries = generated_callees
        .iter()
        .map(|identity| {
            transaction
                .summary(identity)
                .cloned()
                .ok_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "convergence node {node_id:?} has no exact generated summary for {identity:?}"
                    )
                })
                .map(|summary| (identity.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let active_public_callees = model.active_public_callees(node_id).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "convergence model is missing active public callees for node {node_id:?}"
        )
    });
    let active_public_summaries = active_public_callees
        .iter()
        .map(|origin| {
            let function_id = base_hir
                .function_ids_by_origin
                .get(origin)
                .copied()
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "active base public origin {origin:?} has no HIR function identity"
                    )
                });
            let summary = base_borrow_analysis
                .analysis
                .public_call_summaries
                .get(&function_id)
                .cloned()
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "active base public function {function_id:?} has no exact borrow summary"
                    )
                });
            Ok::<_, InfrastructureFailure>((origin.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let private_callees = model.module_private_callees(node_id).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "convergence model is missing module-private callees for node {node_id:?}"
        )
    });
    let private_summaries = private_callees
        .iter()
        .map(|identity| {
            let function_id = base_hir
                .function_ids_by_private_origin
                .get(identity)
                .copied()
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "active base private identity {identity:?} has no HIR function identity"
                    )
                });
            let summary = base_borrow_analysis
                .analysis
                .public_call_summaries
                .get(&function_id)
                .cloned()
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "active base private function {function_id:?} has no exact borrow summary"
                    )
                });
            Ok::<_, InfrastructureFailure>((identity.clone(), summary))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(DirectConvergenceSummaries {
        generated: generated_summaries,
        active_public: active_public_summaries,
        module_private: private_summaries,
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CallInputChanges<'a> {
    generated: FxHashSet<&'a GeneratedFunctionIdentity>,
    public: FxHashSet<&'a OriginFunctionId>,
    private: FxHashSet<&'a ModulePrivateExecutableIdentity>,
    failure: bool,
    mutation: bool,
}

impl CallInputChanges<'_> {
    fn allocations_changed(&self) -> bool {
        !self.generated.is_empty() || !self.public.is_empty() || !self.private.is_empty()
    }

    fn allocation_callers(&self, links: &HirModuleLinkFacts) -> FxHashSet<FunctionId> {
        if !self.allocations_changed() {
            return FxHashSet::default();
        }
        links.callers_of(&self.generated, &self.public, &self.private)
    }
}

/// Install exact inputs for the two analyses that consume the call contract. Escape bits are
/// installed too, but only failure inference reads them before lane delivery is rewritten.
pub(crate) fn install_convergence_summaries<'a>(
    hir: &mut HirModule,
    generated_summaries: &'a [(GeneratedFunctionIdentity, PublicCallSummary)],
    active_public_summaries: &'a [(OriginFunctionId, PublicCallSummary)],
    private_summaries: &'a [(ModulePrivateExecutableIdentity, PublicCallSummary)],
) -> CallInputChanges<'a> {
    let mut failure = false;
    let mut mutation = false;
    // Keep latent and validation-only inputs until link pruning. The lists contain only
    // direct generated dependencies and active-base origins; fixed provider leaves stay put.
    let generated = install_call_inputs(
        &mut hir.generated_call_summaries,
        generated_summaries,
        &mut failure,
        &mut mutation,
    );
    let public = install_call_inputs(
        &mut hir.imported_call_summaries,
        active_public_summaries,
        &mut failure,
        &mut mutation,
    );
    let private = install_call_inputs(
        &mut hir.module_private_call_summaries,
        private_summaries,
        &mut failure,
        &mut mutation,
    );
    CallInputChanges {
        generated,
        public,
        private,
        failure,
        mutation,
    }
}

fn install_call_inputs<'a, K: Clone + Eq + std::hash::Hash>(
    installed: &mut FxHashMap<K, PublicCallSummary>,
    incoming: &'a [(K, PublicCallSummary)],
    failure: &mut bool,
    mutation: &mut bool,
) -> FxHashSet<&'a K> {
    let mut allocations = FxHashSet::default();
    for (target, summary) in incoming {
        let Some(previous) = installed.get_mut(target) else {
            installed.insert(target.clone(), summary.clone());
            allocations.insert(target);
            *failure |= summary.escapes_builtin_failure;
            *mutation = true;
            continue;
        };
        if !same_typed_allocation_transfer_as(
            previous.return_alias.facts(),
            summary.return_alias.facts(),
        ) {
            previous.return_alias.clone_from(&summary.return_alias);
            allocations.insert(target);
        } else if std::mem::discriminant(&previous.return_alias)
            != std::mem::discriminant(&summary.return_alias)
        {
            // Equal transfer does not erase the difference between pending and proved bottom.
            previous.return_alias.clone_from(&summary.return_alias);
        }
        if previous.parameters != summary.parameters {
            if previous.parameters.len() != summary.parameters.len()
                || previous
                    .parameters
                    .iter()
                    .zip(&summary.parameters)
                    .any(|(old, new)| {
                        old.access != new.access || old.transfer_effect != new.transfer_effect
                    })
            {
                allocations.insert(target);
            }
            *mutation = true;
            previous.parameters.clone_from(&summary.parameters);
        }
        *failure |= previous.escapes_builtin_failure != summary.escapes_builtin_failure;
        previous.escapes_builtin_failure = summary.escapes_builtin_failure;
        previous.uses_fast_semantics = summary.uses_fast_semantics;
        previous.may_trap = summary.may_trap;
        previous.may_fail = summary.may_fail;
    }
    allocations
}

/// Resolve a semantic pending-call contributor against its retained summary.
///
/// A missing summary is left to each caller's contract: a required summary is a compiler bug,
/// while a deferred assertion message diagnoses when it cannot prove non-fallibility.
fn retained_call_summary<'a>(
    hir: &'a HirModule,
    report: &'a BorrowCheckReport,
    target: &CallTarget,
) -> Option<&'a PublicCallSummary> {
    match target {
        CallTarget::Local(function) => report.analysis.public_call_summaries.get(function),
        CallTarget::CrossModule(origin) => hir.imported_call_summaries.get(origin),
        CallTarget::ModulePrivate(identity) => hir.module_private_call_summaries.get(identity),
        CallTarget::Generated(identity) => hir.generated_call_summaries.get(identity.as_ref()),
        CallTarget::External(_) => {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "external call cannot contribute an implicit builtin failure"
            );
        }
    }
}

fn call_escapes_builtin_failure(
    hir: &HirModule,
    report: &BorrowCheckReport,
    target: &CallTarget,
) -> bool {
    retained_call_summary(hir, report, target)
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "implicit builtin failure contributor has no exact call summary for {target:?}"
            )
        })
        .escapes_builtin_failure
}

fn active_builtin_failure_contributor<'a>(
    hir: &'a HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
) -> Option<&'a HirBuiltinFailureContributor> {
    let facts = function_failure_facts(hir, function);
    let contributors = facts
        .contributors
        .iter()
        .filter(|contributor| builtin_failure_contributor_is_active(hir, report, contributor))
        .map(Ok::<_, std::convert::Infallible>);
    select_failure_witness(contributors, HirBuiltinFailureContributor::witness_site)
        .unwrap_or_else(|never| match never {})
}

fn function_failure_facts(hir: &HirModule, function: FunctionId) -> &HirFunctionFailureFacts {
    hir.function_failure_facts
        .get(&function)
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "implicit builtin failure analysis is missing semantic facts for {function:?}"
            )
        })
}

/// Arithmetic and cast contributors are active once validated; calls are active while their
/// exact summary escapes builtin failure.
fn builtin_failure_contributor_is_active(
    hir: &HirModule,
    report: &BorrowCheckReport,
    contributor: &HirBuiltinFailureContributor,
) -> bool {
    match &contributor.source {
        HirBuiltinFailureSource::NumericOperation => {
            if contributor.codes.is_empty()
                || contributor
                    .codes
                    .iter()
                    .any(|code| !code.is_implicit_numeric_or_range_failure())
            {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "implicit numeric failure contributor must carry nonempty implicit builtin failure codes"
                );
            }
            true
        }
        HirBuiltinFailureSource::CompoundWriteBack { .. } => {
            if contributor.codes.is_empty()
                || contributor
                    .codes
                    .iter()
                    .any(|code| !code.is_implicit_cast_failure())
            {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "implicit compound write-back failure contributor must carry nonempty cast failure codes"
                );
            }
            true
        }
        HirBuiltinFailureSource::AuthoredCastConversion => {
            if contributor
                .codes
                .iter()
                .any(|code| !code.is_implicit_cast_failure())
            {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "implicit cast failure contributor carries a non-implicit builtin failure code"
                );
            }
            true
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
) -> BuiltinFailureWitness {
    let mut witness = BuiltinFailureWitness {
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
                HirBuiltinFailureSource::NumericOperation
                | HirBuiltinFailureSource::AuthoredCastConversion => {
                    witness.origin_span = contributor.span;
                    witness.origin = BuiltinFailureOriginKind::Operation;
                    break;
                }
                HirBuiltinFailureSource::CompoundWriteBack { target, .. } => {
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
                CallTarget::Generated(identity) => hir
                    .function_ids_by_generated
                    .get(identity.as_ref())
                    .copied(),
                CallTarget::CrossModule(_) | CallTarget::External(_) => None,
            };
            let Some(callee) = callee else {
                hops.push(contributor.span);
                break;
            };
            if entered.insert(callee) {
                hops.push(contributor.span);
                let facts = function_failure_facts(hir, callee);
                stack.push(WitnessCandidates::new(&facts.contributors));
            }
        }

        let Some(candidates) = stack.last_mut() else {
            break;
        };
        next = candidates
            .next_candidate(
                |candidate| {
                    Ok::<_, std::convert::Infallible>(builtin_failure_contributor_is_active(
                        hir, report, candidate,
                    ))
                },
                HirBuiltinFailureContributor::witness_site,
            )
            .unwrap_or_else(|never| match never {});
        if next.is_none() {
            stack.pop();
            hops.pop();
        }
    }

    retain_bounded_witness_hops(&mut witness, hops);
    witness
}

fn builtin_failure_witness(
    hir: &HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
    contributor: &HirBuiltinFailureContributor,
) -> BuiltinFailureWitness {
    let mut entered = FxHashSet::from_iter([function]);
    builtin_failure_witness_search(hir, report, contributor, &mut entered)
}

/// Join immutable semantic failure facts into the exact summaries computed for this queue node.
///
/// Local recursion reaches its finite boolean fixed point here; generated and provider calls
/// resolve through the summaries installed by the canonical convergence queue.
pub(crate) fn infer_builtin_failure_summaries(hir: &HirModule, report: &mut BorrowCheckReport) {
    for function in &hir.functions {
        let summary = report
            .analysis
            .public_call_summaries
            .get_mut(&function.id)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "implicit builtin failure analysis is missing call summary for {:?}",
                    function.id
                )
            });
        summary.escapes_builtin_failure = false;
    }
    loop {
        let mut changed = false;
        for function in &hir.functions {
            let active = active_builtin_failure_contributor(hir, report, function.id).is_some();
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
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "implicit builtin failure analysis lost a call summary"
                    )
                });
            if infer && active && !summary.escapes_builtin_failure {
                summary.escapes_builtin_failure = true;
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

fn append_builtin_failure_witness_labels(
    mut draft: DiagnosticDraft,
    call_spans: &[Option<SourceSpan>],
    origin_span: Option<SourceSpan>,
    origin: BuiltinFailureOriginKind,
) -> DiagnosticDraft {
    for span in call_spans.iter().flatten() {
        draft = draft.with_label(Some(*span), Some(LabelDraftMessage::BuiltinFailureCall));
    }
    if let Some(span) = origin_span {
        let message = match origin {
            BuiltinFailureOriginKind::Operation => LabelDraftMessage::BuiltinFailureOrigin,
            BuiltinFailureOriginKind::CompoundWriteBack { target } => {
                LabelDraftMessage::BuiltinWriteBackOrigin {
                    target_type_id: target,
                }
            }
        };
        draft = draft.with_label(Some(span), Some(message));
    }
    draft
}

fn builtin_failure_reason_witness(witness: &BuiltinFailureWitness) -> Box<BuiltinFailureWitness> {
    Box::new(BuiltinFailureWitness {
        call_spans: Vec::new(),
        origin_span: None,
        origin: witness.origin,
        elided_call_hops: witness.elided_call_hops,
    })
}

pub(crate) fn builtin_failure_diagnostic(
    hir: &HirModule,
    report: &BorrowCheckReport,
    function: FunctionId,
) -> Option<DiagnosticDraft> {
    let facts = function_failure_facts(hir, function);
    for check in &facts.deferred_custom_catches {
        let mut eligible = false;
        for candidate in &check.candidates {
            let HirBuiltinFailureSource::Call(target) = &candidate.source else {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "deferred custom catch contributor is not a private call"
                );
            };
            if call_escapes_builtin_failure(hir, report, target) {
                eligible = true;
                if check.eligibility_only {
                    break;
                }
                let reason = InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                    error_type_id: check.error_type_id,
                    typed_producer_span: check.typed_producer_span,
                    implicit_producer_span: candidate.span,
                };
                let mut draft = invalid_fallible_handling(reason, check.catch_span);
                if let Some(span) = check.typed_producer_span {
                    draft = draft.with_label(
                        Some(span),
                        Some(LabelDraftMessage::TypedFailureProducer {
                            error_type_id: check.error_type_id,
                        }),
                    );
                }
                if let Some(span) = candidate.span {
                    draft = draft
                        .with_label(Some(span), Some(LabelDraftMessage::ImplicitFailureProducer));
                }
                return Some(draft);
            }
        }
        if check.eligibility_only && !eligible {
            return Some(invalid_fallible_handling(
                InvalidFallibleHandlingReason::CatchOnNonFallible,
                check.catch_span,
            ));
        }
    }
    for contributor in &facts.assertion_message_calls {
        let HirBuiltinFailureSource::Call(target) = &contributor.source else {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "deferred assertion message contributor is not a call"
            );
        };
        if retained_call_summary(hir, report, target)
            .is_none_or(|summary| summary.escapes_builtin_failure)
        {
            return Some(invalid_fallible_handling(
                InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
                contributor.span,
            ));
        }
    }
    let contributor = active_builtin_failure_contributor(hir, report, function)?;
    // Evidence was selected before lowering. Discharge its promise only against the closed
    // summaries, before private-lane installation can erase an apparently infallible catch.
    if let Some(contract) = &facts.infallible_cast_evidence {
        let witness = builtin_failure_witness(hir, report, function, contributor);
        let draft = invalid_trait_conformance(
            contract.target_name,
            Some(contract.trait_name),
            InvalidTraitConformanceReason::InferredFailureInInfallibleCast {
                requirement_name: contract.requirement_name,
            },
            contract.span,
        );
        let draft = append_builtin_failure_witness_labels(
            draft,
            &witness.call_spans,
            witness.origin_span,
            witness.origin,
        );
        return Some(draft);
    }
    if matches!(
        facts.boundary,
        HirBuiltinFailureBoundary::InferPrivate | HirBuiltinFailureBoundary::BuiltinErrorSlot
    ) {
        return None;
    }
    let witness = builtin_failure_witness(hir, report, function, contributor);
    let reason_witness = builtin_failure_reason_witness(&witness);
    let reason = match facts.boundary {
        HirBuiltinFailureBoundary::CustomErrorSlot(error_type_id) => {
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id,
                witness: reason_witness,
            }
        }
        HirBuiltinFailureBoundary::ExportedNoSlot => {
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness: reason_witness,
            }
        }
        HirBuiltinFailureBoundary::Never => {
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInNeverFunction {
                witness: reason_witness,
            }
        }
        HirBuiltinFailureBoundary::InferPrivate | HirBuiltinFailureBoundary::BuiltinErrorSlot => {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "InferPrivate and BuiltinErrorSlot return None before this match; arm unreachable"
            )
        }
    };
    let draft = invalid_fallible_handling(reason, facts.span);
    let draft = append_builtin_failure_witness_labels(
        draft,
        &witness.call_spans,
        witness.origin_span,
        witness.origin,
    );
    Some(draft)
}

fn validate_builtin_failure_boundaries(
    compiler: &mut CompilerFrontend<'_>,
    hir: &HirModule,
    report: &BorrowCheckReport,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    source_owner: Option<&FrozenIdentityHandle>,
) -> Result<(), PreparedFailure> {
    for function in &hir.functions {
        let Some(mut diagnostic) = builtin_failure_diagnostic(hir, report, function.id) else {
            continue;
        };
        if let Some(owner) = source_owner {
            for warning in warnings.iter_mut() {
                warning.attach_frozen_identity_handle_if_missing(owner.clone());
            }
            diagnostic.attach_frozen_identity_handle_if_missing(owner.clone());
        }
        warnings.push(diagnostic);
        let batch = PreparedDiagnosticBatch::capture_with_required_paths(
            std::mem::take(warnings),
            compiler.string_table.take_for_handoff(),
            Some(type_environment),
            || Some(Arc::new(compiler.path_fork.shared_table())),
        )
        .map_err(PreparedFailure::from)?;
        return Err(PreparedFailure::Diagnosed(batch));
    }
    Ok(())
}

pub(crate) fn base_summary_changes(
    hir: &HirModule,
    previous: &FxHashMap<FunctionId, PublicCallSummary>,
    next: &FxHashMap<FunctionId, PublicCallSummary>,
) -> Result<BaseSummaryChanges, InfrastructureFailure> {
    let mut widened_functions = FxHashSet::default();
    for function in &hir.functions {
        let previous_summary = previous.get(&function.id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "previous base borrow report is missing summary for {:?}",
                function.id
            )
        });
        let next_summary = next.get(&function.id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "next base borrow report is missing summary for {:?}",
                function.id
            )
        });
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
) {
    let queued = queued_nodes.get_mut(node_id.index()).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "convergence node {node_id:?} is outside the queue bitset"
        )
    });
    if !*queued {
        *queued = true;
        queue.push_back(node_id);
    }
}

pub(crate) fn enqueue_base_dependents(
    model: &ConvergenceModel,
    changes: &BaseSummaryChanges,
    queue: &mut VecDeque<ConvergenceNodeId>,
    queued_nodes: &mut [bool],
) {
    for node_id in model.generated_node_ids() {
        let active_public_callees = model.active_public_callees(node_id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "convergence model is missing active public callees for node {node_id:?}"
            )
        });
        let private_callees = model.module_private_callees(node_id).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "convergence model is missing module-private callees for node {node_id:?}"
            )
        });
        let public_changed = changes
            .public
            .iter()
            .any(|origin| active_public_callees.binary_search(origin).is_ok());
        let private_changed = changes
            .module_private
            .iter()
            .any(|identity| private_callees.binary_search(identity).is_ok());
        if public_changed || private_changed {
            enqueue_convergence_node(node_id, queue, queued_nodes);
        }
    }
}

pub(crate) fn enqueue_convergence_callers(
    model: &ConvergenceModel,
    changed_node: ConvergenceNodeId,
    queue: &mut VecDeque<ConvergenceNodeId>,
    queued_nodes: &mut [bool],
) {
    let callers = model.callers(changed_node).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "convergence model is missing callers for node {changed_node:?}"
        )
    });
    for caller in callers {
        enqueue_convergence_node(*caller, queue, queued_nodes);
    }
}

pub(crate) fn update_generated_summary(
    transaction: &mut GeneratedFunctionTransaction<'_>,
    identity: &GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> Result<bool, InfrastructureFailure> {
    let current = transaction.summary(identity).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "Generated transaction cannot update unknown completed request {identity:?}"
        )
    });
    add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
    let transition = validate_public_call_summary_transition(current, &summary)?;
    if transition != PublicCallSummaryTransition::Widened {
        return Ok(false);
    }

    add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
    *transaction.summary_mut(identity) = summary;
    Ok(true)
}

fn increment_convergence_counter(counter: FrontendCounter) {
    add_frontend_counter(counter, 1);
}

pub(crate) fn generated_root_summary<'a>(
    identity: &GeneratedFunctionIdentity,
    module: &'a Module,
) -> &'a PublicCallSummary {
    let function_id = module
        .executable
        .hir
        .function_ids_by_generated
        .get(identity)
        .copied()
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated sidecar {identity:?} has no generated root function"
            )
        });
    if module.executable.hir.function_ids_by_generated.len() != 1 {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "Generated sidecar {identity:?} contains more than one generated root identity"
        );
    }
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(&function_id)
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "Generated sidecar {identity:?} has no exact root borrow summary"
            )
        })
}

pub(in crate::compiler_frontend) fn install_exact_concrete_call_summaries(
    context: &mut ModuleMaterialisationPreparedBuilder,
    hir: &HirModule,
    borrow_analysis: &BorrowCheckReport,
) -> Result<(), InfrastructureFailure> {
    for index in 0..context.callables().len() {
        let contract = &context.callables()[index];
        let function_id = match &contract.target {
            StableFunctionTarget::Imported(origin) => {
                let Some(function_id) = hir.function_ids_by_origin.get(origin).copied() else {
                    continue;
                };
                function_id
            }
            StableFunctionTarget::ModulePrivate(identity) => hir
                .function_ids_by_private_origin
                .get(identity)
                .copied()
                .unwrap_or_else(|| {
                    crate::compiler_bug!(
                        CompilerStage::ProjectBuildAssembly,
                        "Module materialisation context could not resolve private executable {identity:?}"
                    )
                }),
            StableFunctionTarget::Generated(_) => continue,
        };
        let exact_summary = borrow_analysis
            .analysis
            .public_call_summaries
            .get(&function_id)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "Module materialisation context is missing the exact call summary for {function_id:?}"
                )
            });
        add_frontend_counter(FrontendCounter::ConvergenceSummaryComparisons, 1);
        let transition = validate_public_call_summary_transition(&contract.summary, exact_summary)?;
        if transition == PublicCallSummaryTransition::Widened {
            add_frontend_counter(FrontendCounter::ConvergenceSummaryChanges, 1);
            let local_path = contract.local_path;
            context.update_callable_summary(&local_path, exact_summary);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/convergence_tests.rs"]
mod tests;
