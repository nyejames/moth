//! Reactive template metadata propagation inside the finalized AST.
//!
//! WHAT: computes and attaches value-level metadata for template-backed `String` values on an
//! active-flow AST candidate before template normalization and on the durable specialised AST.
//! WHY: top-level function signatures are resolved before bodies are parsed, so return metadata
//! must be derived from executable flow rather than declaration syntax. A specialised candidate
//! retains its exact annotated TIR contexts through runtime handoff construction and becomes
//! durable only after the untouched authored AST has passed full validation. The durable pass then
//! annotates the completed AST. Assignments and references carry expression metadata, functions
//! substitute direct call arguments, runtime handoffs expose normalized template structure and
//! ordinary string operations remain plain snapshots.
//!
//! The submodule layout mirrors the pass phases:
//! - `flow`: builds a per-function return-metadata flow map and runs the fixed-point refresh.
//! - `collector`: looks up metadata for expressions, templates, render plans, and runtime slot
//!   plans using the current flow map and value environment.
//! - `annotation`: mutates the AST, templates, and expressions to attach the computed metadata.

mod annotation;
mod collector;
mod flow;
mod types;

use super::finalizer::AstFinalizer;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;

impl AstFinalizer<'_, '_> {
    /// Propagate direct reactive-template value metadata through the emitted AST.
    pub(super) fn propagate_reactive_template_metadata(
        &self,
        ast: &mut [AstNode],
    ) -> Result<(), CompilerError> {
        // Borrow the module-scoped TIR store mutably so annotation can update
        // same-store finalized body roots directly. Later normalization then
        // observes the same annotated expressions instead of stale body content.
        let mut store = self.context.template_ir_store.borrow_mut();
        propagate_reactive_template_metadata_in_ast(ast, &mut store)
    }
}

pub(crate) fn propagate_reactive_template_metadata_in_ast(
    ast: &mut [AstNode],
    store: &mut TemplateIrStore,
) -> Result<(), CompilerError> {
    let mut function_flows = flow::initialize_function_template_flows(ast);

    // Function return metadata can depend on direct calls to other functions. A tiny fixed point
    // handles helper chains without introducing a general expression dependency graph.
    loop {
        let mut next_flows = function_flows.clone();
        flow::refresh_function_template_flows(ast, &function_flows, &mut next_flows, &*store)?;

        if next_flows == function_flows {
            break;
        }

        function_flows = next_flows;
    }

    let mut value_environment = types::ReactiveTemplateValueEnvironment::default();
    annotation::annotate_nodes(ast, &function_flows, &mut value_environment, store)
}

#[cfg(test)]
#[path = "../tests/reactive_templates_tests.rs"]
mod reactive_templates_tests;
