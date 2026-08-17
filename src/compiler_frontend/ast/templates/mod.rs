//! AST-stage template parsing, composition, formatting, folding, and handoff.
//!
//! One module build owns one `TemplateIrStore`. Parser and composition code emit
//! structural TIR into that store, then consume exact `TirView` values through
//! the AST template stages. TIR IDs and references stay module-local and are
//! dropped before the completed AST leaves the frontend.
//!
//! ## Template pipeline
//!
//! ```text
//! tokens
//!   -> head/body parsers and `TemplateConstructionContext`
//!   -> module-local `TemplateIrStore`
//!   -> composition, render-unit construction, and formatting
//!   -> exact `TirView` reads
//!   -> `preparation.rs`
//!      -> `fold_prepared_template` for constants
//!      -> `tir/handoff_materialization.rs` for prepared runtime values
//!   -> folded strings or `runtime_handoff.rs` owned payloads
//!   -> HIR
//! ```
//!
//! `TemplateTirReference` and `TemplateWrapperReference` are thin durable
//! root/phase/context values. `TirViewIdentity` carries the root, phase, and
//! value-carried `TemplateViewContext` (`expression_overlay`, `slot_resolution`,
//! and `wrapper_context`) that determine every effective read. `TirView` owns
//! the structural-child, wrapper, resolved-source, helper, and nested-value
//! transition rules.
//!
//! `preparation.rs` is the sole exhaustive semantic preparation owner. It
//! validates reachable structure and produces the foldable, runtime, or helper
//! result consumed by the final reducer. `fold_prepared_template` is the sole
//! prepared constant-fold entry. Template values cross the AST/HIR boundary
//! only as folded strings or neutral owned payloads from `runtime_handoff.rs`.
//!
//! ## Module map
//!
//! | Module | Responsibility |
//! |---|---|
//! | `create_template_node.rs` | Coordinate head/body parsing and finish parser TIR construction |
//! | `template_head_parser/` | Parse directives, head expressions, slots, and control-flow suffixes |
//! | `template_body_parser.rs` | Parse body text, nested templates, and expression splices |
//! | `template_body_sentinels.rs` | Parse body-only control-flow markers |
//! | `template_build_state.rs` | Hold parser-local template construction state |
//! | `template.rs` | Define the thin `Template` handle and shared template vocabulary |
//! | `template_control_flow/` | Define template `if`/`loop` metadata, validation, and const helpers |
//! | `template_slots/` | Define slot schema and runtime slot plans |
//! | `template_render_units.rs` | Build composed and formatted TIR render-unit roots |
//! | `styles/` | Implement directive-owned formatters |
//! | `formatter_contract.rs` | Define formatter input/output and anchor boundaries |
//! | `template_folding.rs` | Own AST folding context and final value-boundary policy |
//! | `top_level_templates.rs` | Collect top-level constant and documentation fragments |
//! | `reactive_template_metadata/` | Reduce reactive metadata separately for exact TIR views and owned handoffs |
//! | `template_renderability.rs` | Resolve template-head renderability from semantic types |
//! | `runtime_handoff.rs` | Define neutral owned runtime-template and slot payloads for HIR |
//! | `tir/` | Own module-local TIR storage, views, preparation, folding, formatting, and materialization |
//! | `doc_fragments.rs` | Build AST documentation-fragment metadata |
//! | `error.rs` | Define the local template diagnostic/infrastructure boundary |

// -------------------------
//  Public Modules
// -------------------------

pub(crate) mod create_template_node;
pub(crate) mod formatter_contract;
pub(crate) mod runtime_handoff;
pub(crate) mod styles;
pub(crate) mod template;
pub(crate) mod template_body_parser;
mod template_body_sentinels;
mod template_build_state;
pub(crate) mod template_control_flow;
pub(crate) mod template_folding;
pub(crate) mod template_head_parser;
pub(crate) mod template_render_units;
pub(crate) mod template_renderability;
pub(crate) mod template_slots;
pub(crate) mod top_level_templates;

// -------------------------
//  Template IR (TIR)
// -------------------------

pub(crate) mod tir;

// -------------------------
//  AST/HIR Runtime Handoff
// -------------------------

pub(crate) use runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};

// -------------------------
//  Reactive metadata traversal
// -------------------------

pub(crate) mod reactive_template_metadata;

// -------------------------
//  Private Modules
// -------------------------

mod doc_fragments;
pub(crate) mod error;

#[cfg(test)]
mod template_folding_tests;
