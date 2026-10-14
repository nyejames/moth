//! AST finalization sub-modules.
//!
//! WHAT: groups final AST assembly, AST node normalization, shared constant facts, and template
//! projection helpers.
//!
//! WHY: Separates finalization concerns from the entry-point orchestration in
//! `ast/mod.rs`, making the high-level phase sequence and detailed normalization logic easier
//! to understand independently.

pub(in crate::compiler_frontend::ast) mod const_fact_collection;
#[cfg(debug_assertions)]
pub(super) mod debug_type_validation;
#[cfg(feature = "benchmark_counters")]
mod expression_census;
pub(in crate::compiler_frontend::ast) mod finalizer;
pub(in crate::compiler_frontend::ast) mod normalize_ast;
pub(in crate::compiler_frontend::ast) mod public_const_templates;
pub(super) mod static_if_specialization;
pub(super) mod template_helpers;
pub(super) mod validate_types;

#[cfg(test)]
#[path = "tests/collection_chain_fixture.rs"]
mod collection_chain_fixture;

pub(in crate::compiler_frontend::ast) use finalizer::{
    AstFinalizer, MaterialisationContextRetention,
};
