//! TIR slot-plan handoff helpers.
//!
//! WHAT: owns the TIR-side representation of AST-prepared runtime slot
//! application plans.
//!
//! WHY: slot routing still belongs to AST template planning. TIR should carry
//! the already-routed source and site plans behind a typed side-table ID so
//! later folding and HIR handoff can consume slot applications without
//! rediscovering or re-routing slots.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::tir::copy_state::TirCopyState;
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrNodeId, TemplateSlotPlanId};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrNode, TemplateIrNodeKind};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// TIR side-table entry for a slot-routing plan.
///
/// WHAT: stores TIR-rendered contribution source plans and slot-site plans for
/// a runtime slot application. The wrapper plan itself is the owning
/// `TemplateIr` root that references this side-table entry through
/// `TemplateIr::runtime_slot_plan`.
/// WHY: this is the first TIR-owned slot-plan handoff. TIR no longer carries a
/// raw runtime-slot planner object; later HIR handoff can consume this route
/// view without re-running AST slot routing.
#[derive(Clone, Debug)]
pub(crate) struct TemplateSlotPlan {
    /// Source location for invariant reporting at the AST/HIR handoff.
    pub(crate) location: SourceLocation,

    /// TIR-rendered contribution source plans, one per runtime contribution.
    pub(crate) contribution_sources: Vec<TemplateSlotContributionSourcePlan>,

    /// TIR-rendered slot-site plans, one per wrapper placeholder occurrence.
    pub(crate) slot_sites: Vec<TemplateSlotSitePlan>,
}

/// TIR-side plan for one runtime slot contribution source.
///
/// WHAT: stores the source accumulator metadata plus a TIR root for the render
/// pieces that fill that accumulator.
/// WHY: source rendering is the next consumer-facing unit after routing. Keeping
/// it in TIR lets later HIR handoff avoid rebuilding render plans from AST
/// pieces.
#[derive(Clone, Debug)]
pub(crate) struct TemplateSlotContributionSourcePlan {
    pub(crate) source: RuntimeSlotContributionSourceId,
    pub(crate) target: SlotKey,
    pub(crate) render_root: TemplateIrNodeId,
    pub(crate) renders_wrapper_unconditionally: bool,
    pub(crate) location: SourceLocation,
}

/// TIR-side plan for one concrete runtime slot site.
///
/// WHAT: stores the site identity, slot key and a single TIR render root.
/// WHY: contribution splices are TIR nodes, including when they sit directly
///      in a site, so a parallel piece list would be a second representation.
#[derive(Clone, Debug)]
pub(crate) struct TemplateSlotSitePlan {
    pub(crate) site: RuntimeSlotSiteId,
    pub(crate) key: SlotKey,
    pub(crate) render_root: TemplateIrNodeId,
    pub(crate) location: SourceLocation,
}

pub(crate) fn runtime_slot_plan_roots(
    store: &TemplateIrStore,
    slot_plan_id: TemplateSlotPlanId,
) -> Result<(Vec<TemplateIrNodeId>, Vec<TemplateIrNodeId>), CompilerError> {
    let slot_plan = store.get_slot_plan(slot_plan_id).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR runtime slot-plan root lookup referenced a missing slot plan.",
        )
    })?;

    let contribution_roots = slot_plan
        .contribution_sources
        .iter()
        .map(|source| source.render_root)
        .collect();

    let site_render_roots = slot_plan
        .slot_sites
        .iter()
        .map(|site| site.render_root)
        .collect();

    Ok((contribution_roots, site_render_roots))
}

pub(crate) fn runtime_slot_plan_site_render_root(
    store: &TemplateIrStore,
    slot_plan_id: TemplateSlotPlanId,
    site_id: RuntimeSlotSiteId,
) -> Result<TemplateIrNodeId, CompilerError> {
    let slot_plan = store.get_slot_plan(slot_plan_id).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR runtime slot-plan site lookup referenced a missing slot plan.",
        )
    })?;

    slot_plan
        .slot_sites
        .iter()
        .find(|site| site.site == site_id)
        .map(|site| site.render_root)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR runtime slot-plan site lookup referenced missing site {:?} in plan {}.",
                site_id, slot_plan_id
            ))
        })
}

pub(super) fn convert_runtime_slot_site(
    plan: TemplateSlotPlanId,
    site: RuntimeSlotSiteId,
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
    location: &SourceLocation,
) -> TemplateIrNodeId {
    copy_state.record_runtime_slot_site(plan, site);

    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotSite { plan, site },
        location.clone(),
    ))
}

/// Plants one plan-qualified contribution-source marker.
///
/// WHAT: constructs the only TIR splice representation used by runtime slot
///       site planning. The marker always carries the active slot plan.
/// WHY: a marker without a plan cannot be checked against its owning source
///      list, and nested plans may reuse the same local source index.
pub(crate) fn push_runtime_slot_contribution_source(
    store: &mut TemplateIrStore,
    plan: TemplateSlotPlanId,
    source: RuntimeSlotContributionSourceId,
    location: SourceLocation,
) -> TemplateIrNodeId {
    store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::RuntimeSlotContributionSource { plan, source },
        location,
    ))
}

// ---------------------------------------------------------------------------
//  Slot-to-RuntimeSlotSite conversion
// ---------------------------------------------------------------------------

/// Converts `Slot` nodes in a TIR tree into `RuntimeSlotSite` nodes.
///
/// WHAT: walks the TIR tree starting at `root_node_id` in document order and
/// replaces each `Slot` node in-place with a `RuntimeSlotSite` node. The
/// matching site is found via the cursor in `copy_state`, which advances as each
/// slot is converted.
///
/// `ChildTemplate` nodes are recursed into so nested slots inside child
/// templates are converted in the same document-order pass. When a child
/// template has slots converted, its `TemplateIr.summary` is recomputed from
/// the converted tree.
///
/// WHY: after materializing a scratch wrapper tree with `Slot` nodes still
/// intact (for site-draft collection), this conversion replaces them with the
/// resolved `RuntimeSlotSite` nodes in-place, keeping the scratch tree as the
/// single source for the final wrapper tree.
///
/// Returns `true` when at least one `Slot` node was converted in this subtree.
pub(crate) fn convert_tir_tree_to_active_slot_plan(
    root_node_id: TemplateIrNodeId,
    slot_plan_id: TemplateSlotPlanId,
    slot_sites: &[TemplateSlotSitePlan],
    store: &mut TemplateIrStore,
    copy_state: &mut TirCopyState,
) -> Result<bool, TemplateError> {
    let node_kind = store
        .get_node(root_node_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR active-slot conversion: node ID was not present in the store.",
            )
        })?
        .kind
        .clone();

    let converted = match node_kind {
        TemplateIrNodeKind::Slot { placeholder } => {
            let site_id = copy_state
                .next_runtime_slot_site_for_key_in_sites(
                    slot_plan_id,
                    &placeholder.key,
                    slot_sites,
                )
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "TIR active-slot conversion: no matching site found for a slot placeholder.",
                    )
                })?;

            store.node_mut(root_node_id)?.kind = TemplateIrNodeKind::RuntimeSlotSite {
                plan: slot_plan_id,
                site: site_id,
            };

            true
        }

        TemplateIrNodeKind::Sequence { children } => {
            let mut any_converted = false;
            for child_id in children {
                any_converted |= convert_tir_tree_to_active_slot_plan(
                    child_id,
                    slot_plan_id,
                    slot_sites,
                    store,
                    copy_state,
                )?;
            }
            any_converted
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_template_id = reference.root;

            let child_root = store
                .get_template(child_template_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "TIR active-slot conversion: child template ID was not present in the store.",
                    )
                })?
                .root;

            let child_converted = convert_tir_tree_to_active_slot_plan(
                child_root,
                slot_plan_id,
                slot_sites,
                store,
                copy_state,
            )?;

            if child_converted {
                let child_root = store
                    .get_template(child_template_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "TIR active-slot conversion: child template ID was not present in the store.",
                        )
                    })?
                    .root;
                let derived_child_template_id = store.push_structurally_derived_template(
                    child_template_id,
                    child_root,
                    crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(),
                )?;
                store.replace_child_template_reference(root_node_id, derived_child_template_id)?;
            }

            child_converted
        }

        TemplateIrNodeKind::BranchChain { branches, fallback } => {
            let mut any_converted = false;
            for branch in branches {
                any_converted |= convert_tir_tree_to_active_slot_plan(
                    branch.body,
                    slot_plan_id,
                    slot_sites,
                    store,
                    copy_state,
                )?;
            }
            if let Some(fallback_id) = fallback {
                any_converted |= convert_tir_tree_to_active_slot_plan(
                    fallback_id,
                    slot_plan_id,
                    slot_sites,
                    store,
                    copy_state,
                )?;
            }
            any_converted
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            let mut any_converted = convert_tir_tree_to_active_slot_plan(
                body,
                slot_plan_id,
                slot_sites,
                store,
                copy_state,
            )?;
            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                any_converted |= convert_tir_tree_to_active_slot_plan(
                    aggregate_wrapper_id,
                    slot_plan_id,
                    slot_sites,
                    store,
                    copy_state,
                )?;
            }
            any_converted
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::InsertContribution { .. }
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => false,
    };

    Ok(converted)
}
