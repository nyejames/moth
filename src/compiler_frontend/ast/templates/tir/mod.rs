//! AST-local Template IR for one module-scoped template store.
//!
//! `TemplateIrStore` owns all TIR arenas, overlay payloads, and module-local
//! occurrence counters. Typed IDs and thin durable references index that store.
//! TIR is dropped before the completed AST leaves the frontend, so HIR and
//! backends receive only folded strings or neutral owned runtime-handoff data.
//!
//! ## View contract
//!
//! `TirViewIdentity` is the complete read identity:
//!
//! ```text
//! root + phase + TemplateViewContext
//! ```
//!
//! `TemplateViewContext` carries `expression_overlay`, `slot_resolution`, and
//! `wrapper_context` by value. `TirView` is the sole structural read surface
//! and owns structural-child, wrapper, resolved-source, helper, and
//! nested-value transitions.
//!
//! ## Phase and final reducers
//!
//! ```text
//! Parsed -> Composed -> Formatted -> Finalized
//! ```
//!
//! `preparation.rs` performs the exhaustive semantic preparation for an exact
//! view. `fold_prepared_template` is the sole prepared constant-fold entry.
//! `handoff_materialization.rs` builds prepared owned runtime handoffs for the
//! neutral `runtime_handoff` payloads consumed by HIR.
//!
//! ## Module map
//!
//! | Module | Responsibility |
//! |---|---|
//! | `ids.rs` | Typed module-local IDs for TIR arenas and occurrences |
//! | `refs.rs` | Thin durable root/phase/context references |
//! | `overlays.rs` | Value-carried view context and overlay payloads |
//! | `view.rs` | Exact `TirView` identity, reads, and structural/nested-value transitions |
//! | `store.rs` | One module-scoped store for TIR arenas, overlays, and side tables |
//! | `store/control_flow.rs` | Checked control-flow lookup and body mutation |
//! | `store/slot_plans.rs` | Slot-plan reservation, commit validation and lookup |
//! | `store/overlays.rs` | Checked overlay allocation |
//! | `node.rs` | TIR roots and structural node kinds |
//! | `summary.rs` | Cheap shape and capacity metadata |
//! | `construction_context.rs` | Parser-facing TIR emission into the shared store |
//! | `render_unit.rs` | Construct branch and aggregate render-unit roots |
//! | `formatter_view.rs` | Adapt formatter input/output to TIR views |
//! | `slot_layout.rs` | One cycle-guarded slot-schema and placeholder-occurrence walk |
//! | `slot_composition/` | Compose head chains and route slot contributions |
//! | `slot_plan.rs` | Store-owned runtime slot site and source plans |
//! | `wrapper_sets.rs` | Reuse wrapper references and build wrapper contexts |
//! | `contribution_shape.rs` | Share child-contribution shape decisions |
//! | `copy_state.rs` and `subtree_copy.rs` | Copy module-local derived subtrees for runtime slot planning |
//! | `expression_constness.rs` | Shared expression constness, exact branch/loop overlay payload selection and the narrow runtime-contribution structural query |
//! | `expression_sites.rs` | Walk expression payloads through exact TIR views and nested expression values |
//! | `expression_overlays.rs` | Collect structural/effective expression overlays and precedence |
//! | `preparation.rs` | Sole exhaustive semantic preparation owner |
//! | `fold/` | Prepared fold entries, the insertion-aware reducer, control flow, wrappers and estimates |
//! | `handoff_materialization.rs` | Build prepared owned runtime-handoff payloads |
//! | `tests/` | Focused TIR invariant tests |
//!
//! Only this module selects the narrow `pub(crate)` surface used by the AST
//! template stages.

// -------------------------
//  Submodules
// -------------------------

mod contribution_shape;
mod copy_state;
mod expression_constness;
mod expression_overlays;
mod expression_sites;
mod ids;
mod subtree_copy;

mod construction_context;
mod fold;
mod formatter_view;
mod handoff_materialization;
mod node;
mod overlays;
mod preparation;
pub(crate) mod refs;
mod render_unit;
mod slot_composition;
mod slot_layout;
mod slot_plan;
mod store;
mod summary;
mod view;
mod wrapper_sets;

#[cfg(test)]
mod tests;

// -------------------------
//  Re-exports
// -------------------------

pub(crate) use ids::{
    ExpressionSiteId, SlotOccurrenceId, TemplateIrId, TemplateIrNodeId, TemplateSlotPlanId,
    TemplateWrapperSetId,
};

pub(crate) use expression_overlays::{
    collect_effective_tir_expression_overlay_payloads, replace_expression_overlay_entries,
};
pub(crate) use expression_sites::{
    walk_expression_payloads_with_nested_tir_views, walk_tir_view_expression_payloads,
};

pub(crate) use node::TemplateLoopHeaderExpressionSites;
pub(crate) use node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
#[cfg(test)]
pub(crate) use store::MalformedTirStore;
pub(crate) use store::{
    ControlFlowBodyKind, DerivedCount, DerivedTemplateMetadata, TemplateIrStore,
};
#[cfg(test)]
pub(crate) use summary::TemplateIrSummary;
pub(crate) use summary::summarize_existing_root;

pub(crate) use refs::{TemplateTirReference, TemplateWrapperReference};
pub(crate) use view::{TirView, TirViewIdentity};
pub(crate) use wrapper_sets::{attach_wrapper_context_overlay, wrapper_reference_for_template};

pub(crate) use overlays::TemplateViewContext;
#[cfg(test)]
pub(crate) use overlays::{
    TirExpressionOverlay, TirExpressionOverlayId, TirSlotResolution, TirSlotResolutionOverlay,
    TirWrapperApplicationMode, TirWrapperContext, TirWrapperContextOverlay,
};
#[cfg(test)]
pub(crate) use store::TemplateWrapperSet;

#[cfg(test)]
pub(crate) use tests::builder::TemplateIrBuilder;

pub(crate) use contribution_shape::{ContributionShape, classify_tir_contribution_node};

pub(crate) use slot_layout::{
    TirSlotPlaceholderRef, TirSlotSchema, collect_tir_slot_layout,
    collect_tir_slot_layout_from_root, collect_tir_slot_schema,
};

pub(crate) use slot_composition::{
    TirSlotContributions, compose_tir_head_chain_from_root, stored_insert_contribution_templates,
};

pub(crate) use construction_context::TemplateConstructionContext;

pub(crate) use copy_state::{TirCopyState, record_tir_copy_counters};
pub(crate) use subtree_copy::copy_tir_subtree_with_active_slot_plan;

pub(crate) use expression_constness::tir_node_is_const_evaluable_value;

pub(crate) use fold::{
    FoldedConstTemplatePiece, fold_prepared_const_template_pattern, fold_prepared_template,
};
pub(crate) use handoff_materialization::{
    owned_runtime_slot_handoff_for_prepared_view, owned_runtime_template_handoff_for_prepared_view,
};
pub(crate) use preparation::{
    RuntimeTemplateReason, TemplateHelperKind, TemplatePreparation, TemplatePreparationFacts,
    TemplatePreparationMode, TemplatePreparationOutcome, prepare_tir_view,
    refresh_kind_from_preparation,
};

#[cfg(test)]
pub(crate) use formatter_view::format_tir_template;

pub(in crate::compiler_frontend::ast::templates) use render_unit::{
    build_branch_body_candidate_root_from_tir_nodes, format_tir_body_root, head_prefix_tir_nodes,
    prepare_loop_aggregate_wrapper, run_tir_formatter_with_warnings, sequence_children,
    trim_whitespace_before_loop_control_boundary,
};

pub(crate) use slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
    convert_tir_tree_to_active_slot_plan, push_runtime_slot_contribution_source,
    runtime_slot_plan_roots, runtime_slot_plan_site_render_root,
};

pub(crate) use view::{TemplateTirPhase, finalized_tir_view_for_template};
