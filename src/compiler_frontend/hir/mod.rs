//! High-level IR modules, AST-to-HIR lowering, and invariant validation.
//!
//! WHAT: defines the backend-facing semantic IR, the lowering builder that converts typed AST into
//! explicit blocks/regions/locals, and the internal validator that checks HIR invariants.
//! WHY: HIR is the stable semantic boundary before borrow validation and backend lowering.
//! Compile-time page fragments, template folding, dependency syntax, and source diagnostics should
//! already be resolved before values reach this stage.
//!
//! ## Boundary contract
//!
//! - HIR core data structures use `TypeId` for semantic type identity. They do not carry
//!   parse-era type syntax values as executable semantic state.
//! - HIR validation uses `CompilerError` with `ErrorType::HirTransformation` for invariant
//!   failures only. It must not construct `CompilerDiagnostic`.
//! - HIR lowering uses `CompilerError` with `ErrorType::HirTransformation` for transformation
//!   invariants. Function-body terminality is validated by AST; fallthrough reaching HIR lowering
//!   is an internal compiler invariant failure.
//! - Borrow analysis facts are side-table metadata keyed by HIR IDs. HIR is not mutated to
//!   encode borrow or ownership state.
//!
//! `hir_builder` owns lowering orchestration and mutable construction state. `validation`
//! checks compiler invariants only; it must not become a user diagnostic layer.

pub(crate) mod blocks;
pub(crate) mod const_facts;
pub(crate) mod constants;
pub(crate) mod expression_rewrite;
pub(crate) mod expressions;
pub(crate) mod functions;
pub(crate) mod ids;
pub(crate) mod module;
pub(crate) mod numeric;
pub(crate) mod operators;
pub(crate) mod patterns;
pub(crate) mod places;
pub(crate) mod reachability;
pub(crate) mod reactivity;
pub(crate) mod regions;
pub(crate) mod statements;
pub(crate) mod structs;
pub(crate) mod terminators;

pub(crate) mod hir_builder;
pub(crate) mod hir_datatypes;
pub(crate) mod hir_side_table;

// Private lowering/validation implementation owners.
pub(crate) mod hir_display;
mod hir_expression;
mod hir_statement;
mod hir_structs;
pub(crate) mod utils;
mod validation;

#[cfg(test)]
mod tests;
