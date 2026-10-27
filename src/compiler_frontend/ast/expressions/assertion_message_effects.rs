//! Assertion-message escape policy.
//!
//! Message construction executes on the assertion's terminal failure edge and must not exit the
//! enclosing function. General failure classification and AST/TIR/handoff traversal belong to
//! failure_classification; this module owns the assertion-specific policy and diagnostic only.

use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::failure_classification::{
    EnclosingExitEffect, ExitClassification, FailureClassificationExpressions,
    classify_enclosing_exit_effect,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;

/// The only assertion whose checked message publishes no executable work.
pub(crate) fn assertion_condition_is_statically_true(
    condition: ExpressionView<'_>,
    const_values: &ConstValueStore,
) -> bool {
    matches!(
        condition.operation(),
        ExpressionOperationView::Constant(id)
            if matches!(const_values.payload(id), Some(ConstValuePayload::Bool(true)))
    )
}

pub(crate) fn classify_assertion_message_effect<'a>(
    message: ExpressionView<'a>,
    template_ir_store: &TemplateIrStore,
    expressions: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<Option<EnclosingExitEffect>, InfrastructureFailure> {
    classify_enclosing_exit_effect(
        message,
        template_ir_store,
        ExitClassification::AssertionMessage,
        expressions,
    )
}

pub(crate) fn assert_message_escape_diagnostic<'a>(
    message: ExpressionView<'a>,
    template_ir_store: &TemplateIrStore,
    expressions: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<Option<DiagnosticDraft>, InfrastructureFailure> {
    Ok(
        classify_assertion_message_effect(message, template_ir_store, expressions)?.map(|effect| {
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
                effect.span(),
            )
        }),
    )
}
