//! WHAT: prepares one exact TIR view into exclusive Foldable, Runtime, or Helper facts.
//! WHY: this is the sole final-value semantic owner; folding emits values and
//!      handoff owns runtime materialization after consuming the prepared result.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{TemplateConstValueKind, TemplateType};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader, collect_option_capture_binding_path,
    loop_body_const_evaluation_bindings,
};
use crate::compiler_frontend::ast::templates::tir::expression_constness::{
    classify_expression_const_evaluable_with_nested_template, effective_branch_selector_for_view,
    effective_loop_header_for_view,
};
use crate::compiler_frontend::ast::templates::tir::ids::{
    TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId, TemplateWrapperSetId,
};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrNodeKind, TirSlotPlaceholder};
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TemplateViewContext, TirSlotResolutionKind,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirReference;

use crate::compiler_frontend::ast::templates::tir::view::{
    TemplateTirPhase, TirView, TirViewIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use std::collections::HashSet;

type PreparationNodeKey = (TemplateIrNodeId, TirViewIdentity);
type PreparationSlotPlanKey = (TemplateSlotPlanId, TirViewIdentity);
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeTemplateReason {
    InheritedWrapperApplication,
    RuntimeSlotPlan,
    WrapperApplication,
    ReactiveContent,
    SlotResolution,
    SlotWrapperApplication,
    SlotContribution,
    RuntimeSlotSite,
    AggregateOutput,
    RuntimeExpression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateHelperKind {
    LoopControl,
    SlotInsert,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplatePreparationMode {
    Value,
    ConstRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TemplatePreparationFacts {
    pub(crate) is_const_evaluable_shape: bool,
    pub(crate) has_unresolved_slot_occurrences: bool,
    pub(crate) has_resolved_slot_sources: bool,
    pub(crate) has_escaped_insert_helpers: bool,
    pub(crate) wrapper_foldable: bool,
    pub(crate) has_runtime_slot_plan: bool,
    pub(crate) has_runtime_slot_sites: bool,
    pub(crate) has_reactive_dependence: bool,
    pub(crate) final_value_kind: TemplateConstValueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplatePreparationOutcome {
    Foldable,
    Runtime(RuntimeTemplateReason),
    Helper(TemplateHelperKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TemplatePreparation {
    pub(crate) identity: TirViewIdentity,
    pub(crate) facts: TemplatePreparationFacts,
    pub(crate) outcome: TemplatePreparationOutcome,
}

/// Refreshes a parser-owned generic template kind from complete preparation facts.
pub(crate) fn refresh_kind_from_preparation(
    kind: &mut TemplateType,
    facts: &TemplatePreparationFacts,
) {
    if matches!(
        *kind,
        TemplateType::SlotInsert(_) | TemplateType::SlotDefinition(_) | TemplateType::Comment(_)
    ) {
        return;
    }

    *kind = if facts.is_const_evaluable_shape && !facts.has_escaped_insert_helpers {
        TemplateType::String
    } else {
        TemplateType::StringFunction
    };
}

/// Mutable state for the one exhaustive preparation traversal.
struct PreparationWalk {
    visiting_templates: HashSet<TirViewIdentity>,
    visiting_nodes: HashSet<PreparationNodeKey>,
    visiting_slot_plans: HashSet<PreparationSlotPlanKey>,
    runtime_reason: Option<RuntimeTemplateReason>,
    mode: TemplatePreparationMode,
    const_diagnostic: Option<Box<CompilerDiagnostic>>,
}

struct PreparationFacts {
    const_evaluable: bool,
    has_unresolved_slots: bool,
    has_resolved_slot_sources: bool,
    has_slot_insertions: bool,
    wrapper_foldable: bool,
    has_runtime_slot_plan: bool,
    has_runtime_slot_sites: bool,
    has_reactive_dependence: bool,
}

struct WrapperSetFacts {
    foldable: bool,
    contains_slot_insert: bool,
}

impl Default for PreparationFacts {
    fn default() -> Self {
        Self {
            const_evaluable: false,
            has_unresolved_slots: false,
            has_resolved_slot_sources: false,
            has_slot_insertions: false,
            wrapper_foldable: true,
            has_runtime_slot_plan: false,
            has_runtime_slot_sites: false,
            has_reactive_dependence: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct PreparationTraversalRole {
    virtual_wrapper: bool,
    in_aggregate_wrapper: bool,
    active_slot_plan: Option<TemplateSlotPlanId>,
}

impl PreparationTraversalRole {
    fn virtual_wrapper(in_aggregate_wrapper: bool) -> Self {
        Self {
            virtual_wrapper: true,
            in_aggregate_wrapper,
            active_slot_plan: None,
        }
    }
}

impl PreparationFacts {
    fn const_value() -> Self {
        Self {
            const_evaluable: true,
            ..Self::default()
        }
    }

    fn merge(&mut self, other: Self) {
        self.const_evaluable &= other.const_evaluable;
        self.has_unresolved_slots |= other.has_unresolved_slots;
        self.has_resolved_slot_sources |= other.has_resolved_slot_sources;
        self.has_slot_insertions |= other.has_slot_insertions;
        self.wrapper_foldable &= other.wrapper_foldable;
        self.has_runtime_slot_plan |= other.has_runtime_slot_plan;
        self.has_runtime_slot_sites |= other.has_runtime_slot_sites;
        self.has_reactive_dependence |= other.has_reactive_dependence;
    }
}

impl PreparationWalk {
    fn new(mode: TemplatePreparationMode) -> Self {
        Self {
            visiting_templates: HashSet::new(),
            visiting_nodes: HashSet::new(),
            visiting_slot_plans: HashSet::new(),
            runtime_reason: None,
            mode,
            const_diagnostic: None,
        }
    }

    fn record_runtime(&mut self, reason: RuntimeTemplateReason) {
        if self.runtime_reason.is_none() {
            self.runtime_reason = Some(reason);
        }
    }

    fn record_role_runtime(
        &mut self,
        facts: &mut PreparationFacts,
        role: &PreparationTraversalRole,
        reason: RuntimeTemplateReason,
    ) {
        if role.virtual_wrapper {
            facts.wrapper_foldable = false;
        } else {
            self.record_runtime(reason);
        }
    }

    fn record_const_diagnostic(&mut self, diagnostic: CompilerDiagnostic) {
        if self.const_diagnostic.is_none() {
            self.const_diagnostic = Some(Box::new(diagnostic));
        }
    }
}

/// Prepares one exact view for folding or runtime handoff.
pub(crate) fn prepare_tir_view(
    view: &TirView<'_>,
    mode: TemplatePreparationMode,
) -> Result<TemplatePreparation, TemplateError> {
    if !view.phase().is_at_least(TemplateTirPhase::Composed) {
        return Err(CompilerError::compiler_error(format!(
            "TIR preparation requires Composed-or-later TIR, but root {} is at phase {}.",
            view.root_ref(),
            view.phase()
        ))
        .into());
    }

    increment_ast_counter(AstCounter::TirPreparationAttempts);
    let mut walk = PreparationWalk::new(mode);
    let facts = walk.walk_template(
        view.root_ref(),
        view,
        &[],
        PreparationTraversalRole::default(),
    )?;
    if let Some(diagnostic) = walk.const_diagnostic {
        return Err(TemplateError::Diagnostic(diagnostic));
    }
    let template = view
        .store()
        .get_template(view.root_ref())
        .ok_or_else(|| missing_template_error(view.root_ref()))?;
    let const_value_kind = if matches!(
        view.store().get_node(template.root).map(|node| &node.kind),
        Some(TemplateIrNodeKind::LoopControl { .. })
    ) {
        TemplateConstValueKind::LoopControlSignal
    } else if !facts.const_evaluable {
        TemplateConstValueKind::NonConst
    } else if matches!(template.kind, TemplateType::SlotInsert(_)) {
        if facts.has_slot_insertions {
            TemplateConstValueKind::NonConst
        } else {
            TemplateConstValueKind::SlotInsertHelper
        }
    } else if matches!(template.kind, TemplateType::SlotDefinition(_)) {
        TemplateConstValueKind::NonConst
    } else if facts.has_unresolved_slots || facts.has_resolved_slot_sources {
        TemplateConstValueKind::WrapperTemplate
    } else if facts.has_slot_insertions {
        TemplateConstValueKind::NonConst
    } else {
        TemplateConstValueKind::RenderableString
    };

    let outcome = match const_value_kind {
        TemplateConstValueKind::LoopControlSignal => {
            TemplatePreparationOutcome::Helper(TemplateHelperKind::LoopControl)
        }
        TemplateConstValueKind::SlotInsertHelper => {
            TemplatePreparationOutcome::Helper(TemplateHelperKind::SlotInsert)
        }
        TemplateConstValueKind::RenderableString | TemplateConstValueKind::WrapperTemplate => {
            if let Some(reason) = walk.runtime_reason
                && !matches!(reason, RuntimeTemplateReason::SlotResolution)
            {
                TemplatePreparationOutcome::Runtime(reason)
            } else {
                TemplatePreparationOutcome::Foldable
            }
        }
        TemplateConstValueKind::NonConst => {
            TemplatePreparationOutcome::Runtime(if facts.has_slot_insertions {
                RuntimeTemplateReason::SlotContribution
            } else {
                walk.runtime_reason
                    .unwrap_or(RuntimeTemplateReason::RuntimeExpression)
            })
        }
    };

    Ok(TemplatePreparation {
        identity: view.identity(),
        facts: TemplatePreparationFacts {
            is_const_evaluable_shape: facts.const_evaluable,
            has_unresolved_slot_occurrences: facts.has_unresolved_slots,
            has_resolved_slot_sources: facts.has_resolved_slot_sources,
            has_escaped_insert_helpers: facts.has_slot_insertions,
            wrapper_foldable: facts.wrapper_foldable,
            has_runtime_slot_plan: facts.has_runtime_slot_plan,
            has_runtime_slot_sites: facts.has_runtime_slot_sites,
            has_reactive_dependence: facts.has_reactive_dependence,
            final_value_kind: const_value_kind,
        },
        outcome,
    })
}

impl PreparationWalk {
    fn walk_template(
        &mut self,
        template_id: TemplateIrId,
        view: &TirView<'_>,
        loop_binding_paths: &[InternedPath],
        role: PreparationTraversalRole,
    ) -> Result<PreparationFacts, TemplateError> {
        let traversal_key = view.identity();
        if !self.visiting_templates.insert(traversal_key) {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: exact view {traversal_key:?} re-entered while still active."
            ))
            .into());
        }
        let result = (|| {
            let mut wrapper_foldable = true;
            let store = view.store();
            let (root, kind, conditional_child_wrapper_set, runtime_slot_plan) = store
                .get_template(template_id)
                .map(|template| {
                    (
                        template.root,
                        template.kind.clone(),
                        template.conditional_child_wrapper_set,
                        template.runtime_slot_plan,
                    )
                })
                .ok_or_else(|| missing_template_error(template_id))?;

            self.validate_template_identity(template_id, root, view)?;
            if let Some(wrapper_set_id) = conditional_child_wrapper_set {
                let wrapper_facts =
                    self.walk_wrapper_set(wrapper_set_id, view, role.in_aggregate_wrapper)?;
                if !wrapper_facts.foldable {
                    let reason = if wrapper_facts.contains_slot_insert {
                        RuntimeTemplateReason::SlotContribution
                    } else {
                        RuntimeTemplateReason::WrapperApplication
                    };
                    wrapper_foldable = false;
                    if !role.virtual_wrapper {
                        self.record_runtime(reason);
                    }
                }
            }
            let mut node_role = role.clone();
            let runtime_slot_plan_facts = if let Some(slot_plan_id) = runtime_slot_plan {
                node_role.active_slot_plan = Some(slot_plan_id);
                let facts = self.walk_slot_plan(slot_plan_id, view, &node_role)?;
                wrapper_foldable = false;

                if !role.virtual_wrapper {
                    self.record_runtime(RuntimeTemplateReason::RuntimeSlotPlan);
                }

                facts
            } else {
                PreparationFacts::const_value()
            };
            let mut facts = self.walk_node(root, view, loop_binding_paths, &node_role)?;
            facts.merge(runtime_slot_plan_facts);
            facts.has_runtime_slot_plan |= runtime_slot_plan.is_some();

            if matches!(
                kind,
                crate::compiler_frontend::ast::templates::template::TemplateType::SlotInsert(_)
            ) {
                self.record_role_runtime(
                    &mut facts,
                    &role,
                    RuntimeTemplateReason::SlotContribution,
                );
            }

            facts.wrapper_foldable &= wrapper_foldable;

            if role.virtual_wrapper && !facts.const_evaluable {
                facts.wrapper_foldable = false;
            }

            Ok(facts)
        })();
        self.visiting_templates.remove(&traversal_key);
        result
    }

    fn validate_template_identity(
        &self,
        template_id: TemplateIrId,
        root_node_id: TemplateIrNodeId,
        view: &TirView<'_>,
    ) -> Result<(), CompilerError> {
        if view.root_ref() != template_id {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: view root {} does not match walked template {}.",
                view.root_ref(),
                template_id
            )));
        }
        let view_template = view.root_template()?;
        if view_template.root != root_node_id {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: view root {} does not match supplied template root node {}.",
                view.root_ref(),
                root_node_id
            )));
        }
        validate_view_context_dimensions(view)?;
        Ok(())
    }

    fn walk_node(
        &mut self,
        node_id: TemplateIrNodeId,
        view: &TirView<'_>,
        loop_binding_paths: &[InternedPath],
        role: &PreparationTraversalRole,
    ) -> Result<PreparationFacts, TemplateError> {
        let store = view.store();
        increment_ast_counter(AstCounter::TirPreparationNodesVisited);
        let traversal_key = (node_id, view.identity());
        if !self.visiting_nodes.insert(traversal_key) {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: node {} is recursively referenced directly.",
                node_id
            ))
            .into());
        }
        let result = (|| {
            let node = view
                .effective_node(node_id)
                .map_err(|_| missing_node_error(node_id))?;

            match &node.kind {
                TemplateIrNodeKind::Sequence { children } => {
                    let mut facts = PreparationFacts::const_value();
                    for child in children {
                        facts.merge(self.walk_node(*child, view, loop_binding_paths, role)?);
                    }
                    Ok(facts)
                }

                TemplateIrNodeKind::ChildTemplate {
                    reference,
                    occurrence_id,
                } => {
                    let mut facts = PreparationFacts::const_value();
                    if let Some(context) = view.effective_wrapper_context(*occurrence_id)?
                        && let Some(wrapper_set_ref) = context.inherited_wrapper_set
                    {
                        let wrapper_facts = self.walk_wrapper_set(
                            wrapper_set_ref,
                            view,
                            role.in_aggregate_wrapper,
                        )?;
                        if !context.skip_parent_child_wrappers && !wrapper_facts.foldable {
                            let reason = if wrapper_facts.contains_slot_insert {
                                RuntimeTemplateReason::SlotContribution
                            } else {
                                RuntimeTemplateReason::InheritedWrapperApplication
                            };
                            self.record_role_runtime(&mut facts, role, reason);
                        }
                    }
                    let child_template_id = reference.root;
                    let child_view = view.structural_child(*reference)?;
                    let child_facts = self.walk_template(
                        child_template_id,
                        &child_view,
                        loop_binding_paths,
                        role.clone(),
                    )?;
                    facts.merge(child_facts);
                    if store
                        .get_template(child_template_id)
                        .is_some_and(|template| {
                            matches!(template.kind, TemplateType::SlotInsert(_))
                        })
                    {
                        facts.has_slot_insertions = true;
                    }
                    Ok(facts)
                }

                TemplateIrNodeKind::Slot { placeholder } => {
                    let mut facts = PreparationFacts::const_value();
                    facts.has_unresolved_slots = true;
                    for wrapper_set_id in [
                        placeholder.applied_child_wrapper_set,
                        placeholder.child_wrapper_set,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let wrapper_facts =
                            self.walk_wrapper_set(wrapper_set_id, view, role.in_aggregate_wrapper)?;
                        if !wrapper_facts.foldable {
                            self.record_role_runtime(
                                &mut facts,
                                role,
                                if wrapper_facts.contains_slot_insert {
                                    RuntimeTemplateReason::SlotContribution
                                } else {
                                    RuntimeTemplateReason::WrapperApplication
                                },
                            );
                        }
                    }
                    if let Some(resolution) =
                        view.effective_slot_resolution(placeholder.occurrence_id)?
                        && let TirSlotResolutionKind::Resolved { sources } = &resolution.kind
                    {
                        facts.has_resolved_slot_sources = true;
                        for source in sources {
                            let source_view = view.resolved_slot_source(*source)?;
                            facts.merge(self.walk_template(
                                *source,
                                &source_view,
                                loop_binding_paths,
                                role.clone(),
                            )?);
                        }
                    }
                    let slot_resolution_missing = view.slot_resolution_overlay()?.is_none();
                    if role.virtual_wrapper
                        && (role.in_aggregate_wrapper
                            || slot_placeholder_has_wrapper_context(placeholder))
                    {
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::SlotWrapperApplication,
                        );
                    } else if !role.virtual_wrapper && slot_resolution_missing {
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::SlotResolution,
                        );
                    }
                    Ok(facts)
                }
                TemplateIrNodeKind::InsertContribution { template } => {
                    let helper_view = view.structural_helper(*template)?;
                    let mut facts = self.walk_template(
                        *template,
                        &helper_view,
                        loop_binding_paths,
                        role.clone(),
                    )?;
                    facts.has_slot_insertions = true;
                    self.record_role_runtime(
                        &mut facts,
                        role,
                        RuntimeTemplateReason::SlotContribution,
                    );
                    Ok(facts)
                }
                TemplateIrNodeKind::BranchChain { branches, fallback } => {
                    let mut facts = PreparationFacts::const_value();
                    for branch in branches {
                        let branch_selector = effective_branch_selector_for_view(
                            view,
                            &branch.selector,
                            branch.selector_site_id,
                        )?;
                        let (branch_binding_paths, selector_const, selector_facts) = self
                            .const_required_branch_selector(
                                view,
                                &branch_selector,
                                &node.location,
                                loop_binding_paths,
                                role,
                            )?;
                        facts.merge(selector_facts);
                        if !selector_const {
                            facts.const_evaluable = false;
                            self.record_role_runtime(
                                &mut facts,
                                role,
                                RuntimeTemplateReason::RuntimeExpression,
                            );
                        }
                        let branch_facts =
                            self.walk_node(branch.body, view, &branch_binding_paths, role)?;
                        if !branch_facts.const_evaluable {
                            self.record_role_runtime(
                                &mut facts,
                                role,
                                RuntimeTemplateReason::RuntimeExpression,
                            );
                            if matches!(self.mode, TemplatePreparationMode::ConstRequired) {
                                self.record_const_diagnostic(
                                    CompilerDiagnostic::invalid_template_structure(
                                        InvalidTemplateStructureReason::TemplateIfBranchNotConst,
                                        node.location.clone(),
                                    ),
                                );
                            }
                        }
                        facts.merge(branch_facts);
                    }
                    if let Some(fallback) = fallback {
                        let fallback_facts =
                            self.walk_node(*fallback, view, loop_binding_paths, role)?;
                        if !fallback_facts.const_evaluable {
                            self.record_role_runtime(
                                &mut facts,
                                role,
                                RuntimeTemplateReason::RuntimeExpression,
                            );
                            if matches!(self.mode, TemplatePreparationMode::ConstRequired) {
                                self.record_const_diagnostic(
                                    CompilerDiagnostic::invalid_template_structure(
                                        InvalidTemplateStructureReason::TemplateIfBranchNotConst,
                                        node.location.clone(),
                                    ),
                                );
                            }
                        }
                        facts.merge(fallback_facts);
                    }
                    Ok(facts)
                }
                TemplateIrNodeKind::Loop {
                    header,
                    header_sites,
                    body,
                    aggregate_wrapper,
                    ..
                } => {
                    let effective_header =
                        effective_loop_header_for_view(view, header, *header_sites)?;
                    let mut facts =
                        self.walk_loop_header(view, &effective_header, &node.location, role)?;
                    let body_binding_paths =
                        loop_body_const_evaluation_bindings(&effective_header, loop_binding_paths);
                    let body_facts = self.walk_node(*body, view, &body_binding_paths, role)?;
                    if !body_facts.const_evaluable
                        && matches!(self.mode, TemplatePreparationMode::ConstRequired)
                    {
                        self.record_const_diagnostic(
                            CompilerDiagnostic::invalid_template_structure(
                                InvalidTemplateStructureReason::TemplateLoopBodyNotConst,
                                node.location.clone(),
                            ),
                        );
                    }
                    facts.merge(body_facts);
                    if let Some(aggregate_wrapper) = aggregate_wrapper {
                        let aggregate_role = PreparationTraversalRole {
                            in_aggregate_wrapper: true,
                            ..role.clone()
                        };
                        facts.merge(self.walk_node(
                            *aggregate_wrapper,
                            view,
                            loop_binding_paths,
                            &aggregate_role,
                        )?);
                    }
                    Ok(facts)
                }
                TemplateIrNodeKind::RuntimeSlotSite { plan, site } => {
                    self.validate_runtime_slot_site(*plan, *site, view, role)?;
                    let mut facts = self.walk_slot_plan(*plan, view, role)?;
                    facts.const_evaluable = false;
                    facts.has_runtime_slot_sites = true;
                    self.record_role_runtime(
                        &mut facts,
                        role,
                        RuntimeTemplateReason::RuntimeSlotSite,
                    );
                    Ok(facts)
                }
                TemplateIrNodeKind::RuntimeSlotContributionSource { plan, source } => {
                    self.validate_contribution_source_marker(*plan, *source, view, role)?;
                    let mut facts = PreparationFacts::default();
                    self.record_role_runtime(
                        &mut facts,
                        role,
                        RuntimeTemplateReason::RuntimeSlotSite,
                    );
                    Ok(facts)
                }
                TemplateIrNodeKind::DynamicExpression {
                    expression,
                    site_id,
                    reactive_subscription,
                    ..
                } => {
                    let effective_expression = view
                        .effective_expression_for_site(*site_id)?
                        .unwrap_or(expression.as_ref());
                    let mut facts =
                        self.walk_expression(view, effective_expression, loop_binding_paths, role)?;
                    if reactive_subscription.is_some() {
                        facts.has_reactive_dependence = true;
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::ReactiveContent,
                        );
                    }
                    Ok(facts)
                }

                TemplateIrNodeKind::Text { .. } => {
                    if store.node_reactive_subscription(node_id)?.is_some() {
                        let mut facts = PreparationFacts {
                            has_reactive_dependence: true,
                            ..PreparationFacts::default()
                        };
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::ReactiveContent,
                        );
                        Ok(facts)
                    } else {
                        Ok(PreparationFacts::const_value())
                    }
                }
                TemplateIrNodeKind::AggregateOutput => {
                    if role.in_aggregate_wrapper {
                        Ok(PreparationFacts::const_value())
                    } else {
                        let mut facts = PreparationFacts::default();
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::AggregateOutput,
                        );
                        Ok(facts)
                    }
                }
                TemplateIrNodeKind::LoopControl { .. } => {
                    if role.virtual_wrapper {
                        let mut facts = PreparationFacts::default();
                        self.record_role_runtime(
                            &mut facts,
                            role,
                            RuntimeTemplateReason::WrapperApplication,
                        );
                        Ok(facts)
                    } else {
                        Ok(PreparationFacts::const_value())
                    }
                }
            }
        })();

        self.visiting_nodes.remove(&traversal_key);
        result
    }

    fn walk_slot_plan(
        &mut self,
        slot_plan_id: TemplateSlotPlanId,
        view: &TirView<'_>,
        role: &PreparationTraversalRole,
    ) -> Result<PreparationFacts, TemplateError> {
        let store = view.store();
        let traversal_key = (slot_plan_id, view.identity());
        if !self.visiting_slot_plans.insert(traversal_key) {
            return Ok(PreparationFacts::const_value());
        }
        let result = (|| {
            let slot_plan = store
                .get_slot_plan(slot_plan_id)
                .ok_or_else(|| missing_slot_plan_error(slot_plan_id))?;

            for (index, source) in slot_plan.contribution_sources.iter().enumerate() {
                if source.source.0 != index {
                    return Err(CompilerError::compiler_error(format!(
                        "TIR preparation: contribution source {} in plan {} does not match its index {}.",
                        source.source.0, slot_plan_id, index
                    ))
                    .into());
                }
            }
            for (index, site) in slot_plan.slot_sites.iter().enumerate() {
                if site.site.0 != index {
                    return Err(CompilerError::compiler_error(format!(
                        "TIR preparation: slot site {} in plan {} does not match its index {}.",
                        site.site.0, slot_plan_id, index
                    ))
                    .into());
                }
            }

            let mut plan_role = role.clone();
            plan_role.active_slot_plan = Some(slot_plan_id);
            let mut facts = PreparationFacts::const_value();
            for source in &slot_plan.contribution_sources {
                facts.merge(self.walk_node(source.render_root, view, &[], &plan_role)?);
            }
            for site in &slot_plan.slot_sites {
                facts.merge(self.walk_node(site.render_root, view, &[], &plan_role)?);
            }
            Ok(facts)
        })();

        self.visiting_slot_plans.remove(&traversal_key);
        result
    }

    fn validate_contribution_source_marker(
        &self,
        plan: TemplateSlotPlanId,
        source: crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotContributionSourceId,
        view: &TirView<'_>,
        role: &PreparationTraversalRole,
    ) -> Result<(), CompilerError> {
        let Some(active_plan) = role.active_slot_plan else {
            return Err(CompilerError::compiler_error(
                "TIR preparation: runtime slot contribution marker appeared outside its owning plan.",
            ));
        };
        if plan != active_plan {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: runtime slot contribution marker plan {} differs from the active plan {}.",
                plan, active_plan
            )));
        }
        let slot_plan = view
            .store()
            .get_slot_plan(active_plan)
            .ok_or_else(|| missing_slot_plan_error(active_plan))?;
        let Some(stored) = slot_plan.contribution_sources.get(source.0) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: runtime slot contribution source {} is outside plan {}.",
                source.0, active_plan
            )));
        };
        if stored.source != source {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: contribution source {} in plan {} does not match its stored identity {}.",
                source.0, active_plan, stored.source.0
            )));
        }
        Ok(())
    }

    fn validate_runtime_slot_site(
        &self,
        plan: TemplateSlotPlanId,
        site: crate::compiler_frontend::ast::templates::template_slots::RuntimeSlotSiteId,
        view: &TirView<'_>,
        role: &PreparationTraversalRole,
    ) -> Result<(), CompilerError> {
        let Some(active_plan) = role.active_slot_plan else {
            return Err(CompilerError::compiler_error(
                "TIR preparation: runtime slot site appeared outside its owning plan.",
            ));
        };
        if plan != active_plan {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: runtime slot site plan {} differs from the active plan {}.",
                plan, active_plan
            )));
        }

        let slot_plan = view
            .store()
            .get_slot_plan(active_plan)
            .ok_or_else(|| missing_slot_plan_error(active_plan))?;
        let Some(stored) = slot_plan.slot_sites.get(site.0) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: runtime slot site {} is outside plan {}.",
                site.0, active_plan
            )));
        };
        if stored.site != site {
            return Err(CompilerError::compiler_error(format!(
                "TIR preparation: slot site {} in plan {} does not match its stored identity {}.",
                site.0, active_plan, stored.site.0
            )));
        }

        Ok(())
    }

    fn walk_wrapper_set(
        &mut self,
        wrapper_set_id: TemplateWrapperSetId,
        view: &TirView<'_>,
        in_aggregate_wrapper: bool,
    ) -> Result<WrapperSetFacts, TemplateError> {
        let wrapper_set = view
            .store()
            .get_wrapper_set(wrapper_set_id)
            .ok_or_else(|| missing_wrapper_set_error(wrapper_set_id))?;
        let mut facts = WrapperSetFacts {
            foldable: true,
            contains_slot_insert: false,
        };
        for wrapper in &wrapper_set.wrappers {
            let wrapper_view = view.wrapper(*wrapper)?;
            let wrapper_facts = self.walk_template(
                wrapper.root,
                &wrapper_view,
                &[],
                PreparationTraversalRole::virtual_wrapper(in_aggregate_wrapper),
            )?;
            facts.contains_slot_insert |= wrapper_facts.has_slot_insertions
                || view
                    .store()
                    .get_template(wrapper.root)
                    .is_some_and(|template| matches!(template.kind, TemplateType::SlotInsert(_)));
            if !wrapper_facts.wrapper_foldable {
                facts.foldable = false;
            }
        }
        Ok(facts)
    }

    /// Walks one expression payload and re-enters nested template values
    /// through the same exact-view preparation traversal.
    fn walk_expression(
        &mut self,
        view: &TirView<'_>,
        expression: &Expression,
        loop_binding_paths: &[InternedPath],
        role: &PreparationTraversalRole,
    ) -> Result<PreparationFacts, TemplateError> {
        let mut nested_facts = PreparationFacts::const_value();
        let mut visit_nested_template =
            |reference: TemplateTirReference, nested_binding_paths: &[InternedPath]| {
                let nested_view = view.nested_template_value(reference)?;
                let facts = self.walk_template(
                    reference.root,
                    &nested_view,
                    nested_binding_paths,
                    role.clone(),
                )?;
                let const_evaluable = facts.const_evaluable;
                nested_facts.merge(facts);
                Ok(const_evaluable)
            };

        let expression_const = classify_expression_const_evaluable_with_nested_template(
            expression,
            loop_binding_paths,
            &mut visit_nested_template,
        )?;
        nested_facts.const_evaluable &= expression_const;
        if !expression_const {
            self.record_role_runtime(
                &mut nested_facts,
                role,
                RuntimeTemplateReason::RuntimeExpression,
            );
        }
        Ok(nested_facts)
    }

    fn const_required_branch_selector(
        &mut self,
        view: &TirView<'_>,
        selector: &TemplateBranchSelector,
        fallback_location: &SourceLocation,
        loop_binding_paths: &[InternedPath],
        role: &PreparationTraversalRole,
    ) -> Result<(Vec<InternedPath>, bool, PreparationFacts), TemplateError> {
        let mut branch_binding_paths = loop_binding_paths.to_owned();
        let mut selector_facts = PreparationFacts::const_value();
        let result = match selector {
            TemplateBranchSelector::Bool(condition) => {
                let condition_facts =
                    self.walk_expression(view, condition, loop_binding_paths, role)?;
                let is_const = condition_facts.const_evaluable;
                selector_facts.merge(condition_facts);
                if is_const {
                    Ok(())
                } else {
                    Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateIfConditionNotConst,
                        condition.location.clone(),
                    ))
                }
            }
            TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
                let is_const = match &scrutinee.kind {
                    ExpressionKind::OptionNone => true,
                    ExpressionKind::Reference(path) => {
                        loop_binding_paths.iter().any(|known| known == path)
                    }
                    ExpressionKind::Coerced { value, .. } => {
                        let value_facts =
                            self.walk_expression(view, value, loop_binding_paths, role)?;
                        let is_const = value_facts.const_evaluable;
                        selector_facts.merge(value_facts);
                        is_const
                    }
                    _ => {
                        let scrutinee_facts =
                            self.walk_expression(view, scrutinee, loop_binding_paths, role)?;
                        selector_facts.merge(scrutinee_facts);
                        false
                    }
                };
                if is_const {
                    collect_option_capture_binding_path(pattern, &mut branch_binding_paths);
                    Ok(())
                } else {
                    let location = if scrutinee.location == SourceLocation::default() {
                        fallback_location.clone()
                    } else {
                        scrutinee.location.clone()
                    };
                    Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
                        location,
                    ))
                }
            }
        };
        let selector_const = result.is_ok();
        if !selector_const {
            self.record_role_runtime(
                &mut selector_facts,
                role,
                RuntimeTemplateReason::RuntimeExpression,
            );
        }
        if matches!(self.mode, TemplatePreparationMode::ConstRequired)
            && let Err(diagnostic) = result
        {
            self.record_const_diagnostic(diagnostic);
        }
        Ok((branch_binding_paths, selector_const, selector_facts))
    }

    fn walk_loop_header(
        &mut self,
        view: &TirView<'_>,
        header: &TemplateLoopHeader,
        loop_location: &SourceLocation,
        role: &PreparationTraversalRole,
    ) -> Result<PreparationFacts, TemplateError> {
        let mut facts = PreparationFacts::const_value();
        let diagnostic = match header {
            TemplateLoopHeader::Conditional { condition } => {
                let condition_facts = self.walk_expression(view, condition, &[], role)?;
                facts.merge(condition_facts);

                let mut diagnostic_condition = condition.as_ref();
                while let ExpressionKind::Coerced { value, .. } = &diagnostic_condition.kind {
                    diagnostic_condition = value;
                }

                match &diagnostic_condition.kind {
                    ExpressionKind::Bool(false) => None,
                    ExpressionKind::Bool(true) => {
                        Some(CompilerDiagnostic::invalid_template_structure(
                            InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
                            condition_location_or_loop_location(
                                diagnostic_condition,
                                loop_location,
                            ),
                        ))
                    }
                    _ => Some(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopConditionNotConst,
                        condition_location_or_loop_location(diagnostic_condition, loop_location),
                    )),
                }
            }
            TemplateLoopHeader::Range { range, .. } => {
                let start_facts = self.walk_expression(view, &range.start, &[], role)?;
                let end_facts = self.walk_expression(view, &range.end, &[], role)?;
                let mut header_const = start_facts.const_evaluable;
                header_const &= end_facts.const_evaluable;
                facts.merge(start_facts);
                facts.merge(end_facts);
                if let Some(step) = &range.step {
                    let step_facts = self.walk_expression(view, step, &[], role)?;
                    header_const &= step_facts.const_evaluable;
                    facts.merge(step_facts);
                }
                (!header_const).then(|| {
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst,
                        loop_location.clone(),
                    )
                })
            }
            TemplateLoopHeader::Collection { iterable, .. } => {
                let iterable_facts = self.walk_expression(view, iterable, &[], role)?;
                let header_const = iterable_facts.const_evaluable;
                facts.merge(iterable_facts);
                (!header_const).then(|| {
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateLoopSourceNotConst,
                        iterable.location.clone(),
                    )
                })
            }
        };

        if matches!(self.mode, TemplatePreparationMode::ConstRequired)
            && let Some(diagnostic) = diagnostic
        {
            self.record_const_diagnostic(diagnostic);
        }
        Ok(facts)
    }
}

fn slot_placeholder_has_wrapper_context(placeholder: &TirSlotPlaceholder) -> bool {
    placeholder.applied_child_wrapper_set.is_some()
        || placeholder.child_wrapper_set.is_some()
        || placeholder.skip_parent_child_wrappers
}

fn validate_view_context_dimensions(view: &TirView<'_>) -> Result<(), CompilerError> {
    let store = view.store();
    let context = view.context();
    if let Some(overlay_id) = context.expression_overlay
        && store.expression_overlay(overlay_id).is_none()
    {
        return Err(missing_overlay_dimension_error(
            context,
            "expression",
            overlay_id,
        ));
    }

    if let Some(overlay_id) = context.slot_resolution
        && store.slot_resolution_overlay(overlay_id).is_none()
    {
        return Err(missing_overlay_dimension_error(
            context,
            "slot-resolution",
            overlay_id,
        ));
    }

    if let Some(overlay_id) = context.wrapper_context
        && store.wrapper_context_overlay(overlay_id).is_none()
    {
        return Err(missing_overlay_dimension_error(
            context,
            "wrapper-context",
            overlay_id,
        ));
    }

    Ok(())
}

fn missing_template_error(template_id: TemplateIrId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR preparation: template {} is not present in the module store.",
        template_id
    ))
}

fn missing_node_error(node_id: TemplateIrNodeId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR preparation: node {} does not exist in the module store.",
        node_id
    ))
}

fn missing_wrapper_set_error(wrapper_set_id: TemplateWrapperSetId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR preparation: wrapper set {} is not present in the module store.",
        wrapper_set_id
    ))
}

fn missing_slot_plan_error(slot_plan_id: TemplateSlotPlanId) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR preparation: slot plan {} is not present in the module store.",
        slot_plan_id
    ))
}

fn missing_overlay_dimension_error(
    context: TemplateViewContext,
    dimension: &str,
    overlay_id: impl std::fmt::Display,
) -> CompilerError {
    CompilerError::compiler_error(format!(
        "TIR preparation: context {:?} references missing {} overlay {}.",
        context, dimension, overlay_id
    ))
}

fn condition_location_or_loop_location(
    condition: &Expression,
    loop_location: &SourceLocation,
) -> SourceLocation {
    if condition.location == SourceLocation::default() {
        loop_location.clone()
    } else {
        condition.location.clone()
    }
}
