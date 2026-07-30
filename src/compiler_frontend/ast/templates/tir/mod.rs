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
//! | `node.rs` | TIR roots and structural node kinds |
//! | `summary.rs` | Cheap shape and capacity metadata |
//! | `builder.rs` | Narrow parser-facing TIR emission facade |
//! | `parser_builder_state.rs` | In-progress parser emission state |
//! | `construction_context.rs` | Parser-local construction context over the shared store |
//! | `control_flow_roots.rs` | Install and resolve composed control-flow roots |
//! | `render_unit.rs` | Construct branch and aggregate render-unit roots |
//! | `formatter_view.rs` | Adapt formatter input/output to TIR views |
//! | `slot_composition/` | Compose head chains and route slot contributions |
//! | `slot_plan.rs` | Store-owned runtime slot site and source plans |
//! | `wrapper_sets.rs` | Reuse wrapper references and build wrapper contexts |
//! | `contribution_shape.rs` | Share child-contribution shape decisions |
//! | `copy_state.rs` and `subtree_copy.rs` | Copy module-local derived subtrees for runtime slot planning |
//! | `classification.rs` | Answer narrow TIR shape queries used before final reduction |
//! | `expression_payload_walker.rs` | Walk expression payloads through exact TIR views |
//! | `preparation.rs` | Sole exhaustive semantic preparation owner |
//! | `fold.rs` | Implement `fold_prepared_template`, the sole prepared fold entry |
//! | `fold_cache.rs` | Cache exact-view prepared fold emissions |
//! | `handoff_materialization.rs` | Build prepared owned runtime-handoff payloads |
//! | `tests/` | Focused TIR invariant tests |
//!
//! Only this module selects the narrow `pub(crate)` surface used by the AST
//! template stages.

// -------------------------
//  Submodules
// -------------------------

mod classification;
mod contribution_shape;
mod copy_state;
mod expression_payload_walker;
mod ids;
mod subtree_copy;

mod builder;
mod construction_context;
mod control_flow_roots;
mod fold;
mod fold_cache;
mod formatter_view;
mod handoff_materialization;
mod node;
mod overlays;
mod parser_builder_state;
mod preparation;
pub(crate) mod refs;
mod render_unit;
mod slot_composition;
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

pub(crate) use expression_payload_walker::walk_tir_view_expression_payloads;
pub(crate) use expression_payload_walker::{
    collect_effective_tir_expression_overlay_payloads,
    collect_effective_tir_expression_overlay_payloads_with_phase,
    walk_expression_payloads_with_nested_tir_views,
};

pub(crate) use node::TemplateLoopHeaderExpressionSites;
pub(crate) use node::{
    TemplateIr, TemplateIrBranch, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder,
};
pub(crate) use store::TemplateIrStore;
#[cfg(test)]
pub(crate) use summary::TemplateIrSummary;
pub(crate) use summary::summarize_existing_root;

pub(crate) use refs::{TemplateTirReference, TemplateWrapperReference};
pub(crate) use view::{TirView, TirViewIdentity};
pub(crate) use wrapper_sets::{attach_wrapper_context_overlay, wrapper_reference_for_template};

pub(crate) use overlays::{TemplateViewContext, TirExpressionOverlay};
#[cfg(test)]
pub(crate) use overlays::{
    TirExpressionOverlayId, TirSlotResolution, TirSlotResolutionOverlay, TirWrapperContext,
    TirWrapperContextOverlay,
};
#[cfg(test)]
pub(crate) use store::TemplateWrapperSet;

pub(crate) use builder::TemplateIrBuilder;

pub(crate) use control_flow_roots::{
    ControlFlowBodyKind, replace_control_flow_body_tir_root,
    replace_loop_aggregate_wrapper_tir_root,
};

pub(crate) use contribution_shape::{ContributionShape, classify_tir_contribution_node};

pub(crate) use slot_composition::{
    RoutedTirSlotContributions, TirSlotContributions, TirSlotSchema,
    collect_tir_slot_placeholders_in_order, collect_tir_slot_schema, compose_tir_head_chain,
    compose_tir_head_chain_with_overlays, merge_tir_slot_resolution_contexts,
    wrap_tir_node_in_wrappers,
};

pub(crate) use construction_context::TemplateConstructionContext;

pub(crate) use copy_state::{TirCopyState, record_tir_copy_counters};
pub(crate) use subtree_copy::copy_tir_subtree_with_active_slot_plan;

pub(crate) use classification::{
    TirTemplateClassification, classify_effective_tir_view_template,
    refresh_kind_from_classification, tir_node_is_const_evaluable_value,
    tir_subtree_has_unresolved_slots,
};

pub(crate) use fold::{
    FoldedConstTemplatePiece, fold_prepared_const_template_pattern, fold_prepared_template,
};
pub(crate) use handoff_materialization::{
    owned_runtime_slot_handoff_for_prepared_view, owned_runtime_template_handoff_for_prepared_view,
};
pub(crate) use preparation::{
    PreparedRuntime, PreparedTemplate, RuntimeTemplateReason, TemplateHelperKind,
    TemplatePreparationMode, prepare_tir_view,
};

pub(crate) use fold_cache::TirFoldCache;

#[cfg(test)]
pub(crate) use formatter_view::format_tir_template;

pub(in crate::compiler_frontend::ast::templates) use render_unit::{
    apply_inherited_child_wrappers_to_body_root, build_branch_body_candidate_from_tir_nodes,
    format_tir_body_root, head_prefix_tir_nodes, prepare_loop_aggregate_wrapper,
    run_tir_formatter_with_warnings, sequence_children,
    trim_whitespace_before_loop_control_boundary,
};

pub(crate) use slot_plan::{
    TemplateSlotContributionSourcePlan, TemplateSlotPlan, TemplateSlotSitePlan,
    TemplateSlotSiteRenderPiece, TemplateSlotSiteRenderPlan, convert_tir_tree_to_active_slot_plan,
};

pub(crate) use view::{TemplateTirPhase, finalized_tir_view_for_template};
