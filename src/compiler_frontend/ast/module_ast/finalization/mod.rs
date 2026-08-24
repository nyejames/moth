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
pub(in crate::compiler_frontend::ast) mod finalizer;
pub(super) mod normalize_ast;
pub(super) mod public_const_templates;
pub(super) mod reactive_templates;
pub(super) mod static_if_specialization;
pub(super) mod template_helpers;
pub(super) mod validate_types;

pub(in crate::compiler_frontend::ast) use finalizer::AstFinalizer;
