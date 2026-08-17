//! Reactive template metadata reducers.
//!
//! WHAT: keeps expression-resolution policy and metadata accumulation local
//!       while delegating structural reduction to representation-specific
//!       TIR-view and owned-handoff modules.
//! WHY: finalized TIR and neutral runtime handoffs have different shapes and
//!      authority rules. Splitting their reducers prevents another parallel
//!      representation from being hidden inside one broad traversal module.

mod owned_handoff;
mod tir;

use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::compiler_errors::CompilerError;

pub(super) type ReactiveMetadataResolver<'a> =
    dyn FnMut(&Expression) -> Result<Option<ReactiveTemplateMetadata>, CompilerError> + 'a;

pub(crate) use owned_handoff::{
    metadata_for_owned_runtime_slot_application_handoff,
    metadata_for_owned_runtime_template_handoff,
};
pub(crate) use tir::merge_reactive_template_metadata;

/// Resolves metadata at the representation boundary, falling back to the
/// owned-handoff reducer when the expression carries a neutral handoff shell.
pub(super) fn merge_expression_metadata(
    expression: &Expression,
    metadata: &mut ReactiveTemplateMetadata,
    resolver: &mut ReactiveMetadataResolver<'_>,
) -> Result<(), CompilerError> {
    if let Some(expression_metadata) = resolver(expression)? {
        metadata.merge_from(&expression_metadata);
        return Ok(());
    }

    match &expression.kind {
        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            owned_handoff::merge_owned_runtime_template_handoff_metadata(
                handoff, metadata, resolver,
            )?;
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            owned_handoff::merge_owned_runtime_slot_application_handoff_metadata(
                handoff, metadata, resolver,
            )?;
        }

        _ => {}
    }

    Ok(())
}

#[cfg(test)]
#[path = "../tests/reactive_template_metadata_tests.rs"]
mod reactive_template_metadata_tests;
