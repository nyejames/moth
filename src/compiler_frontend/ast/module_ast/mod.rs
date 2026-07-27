//! AST module construction phases and scope-context helpers.
//!
//! WHAT: maps the AST stage into three explicit owners:
//! `AstModuleEnvironmentBuilder`, `AstEmitter`, and `AstFinalizer`.
//! WHY: the old pass accumulator made field validity depend on implicit ordering. Keeping
//! environment, emission, and finalization state separate makes stage ownership reviewable.
//!
//! ## Phase ownership
//!
//! - `environment` consumes header-built visibility and resolves declarations, signatures, and receiver data.
//! - `emission` lowers executable bodies and const templates into AST-owned output state.
//! - `finalization` normalizes HIR-boundary templates/constants and assembles [`Ast`].
//!
//! The entry point and final assembly live in [`crate::compiler_frontend::ast::Ast::new`].

pub(in crate::compiler_frontend::ast) mod build_context;
pub(in crate::compiler_frontend::ast) mod emission;
pub(in crate::compiler_frontend::ast) mod environment;
pub(in crate::compiler_frontend::ast) mod finalization;
pub(crate) mod scope_context;

// Internal re-exports so `ast/mod.rs` can surface the minimal public API.
//
// `Ast` and `AstBuildContext` live in `ast/mod.rs` (the strict module entry point).
// The types below are re-exported here only so `ast/mod.rs` can re-export them;
// callers should import through `ast::` directly.
#[cfg(test)]
pub(crate) use scope_context::{ReceiverMethodCatalog, ReceiverMethodEntry};

// --------------------------
//  Tests
// --------------------------

#[cfg(test)]
#[path = "../tests/module_ast_receiver_method_tests.rs"]
mod module_ast_receiver_method_tests;

#[cfg(test)]
#[path = "../tests/declaration_table_tests.rs"]
mod declaration_table_tests;

#[cfg(test)]
#[path = "../tests/choice_expression_tests.rs"]
mod choice_expression_tests;

#[cfg(test)]
#[path = "../tests/scope_context_tests.rs"]
mod scope_context_tests;

#[cfg(test)]
#[path = "../tests/resolved_public_type_roots_tests.rs"]
mod resolved_public_type_roots_tests;

#[cfg(test)]
#[path = "../tests/resolved_public_trait_roots_tests.rs"]
mod resolved_public_trait_roots_tests;

#[cfg(test)]
#[path = "../tests/finalizer_tests.rs"]
mod finalizer_tests;

#[cfg(test)]
#[path = "../tests/import_projection_tests.rs"]
mod import_projection_tests;
