//! Assertion-message escape policy.
//!
//! Message construction executes on the assertion's terminal failure edge and must not exit the
//! enclosing function. General failure classification and AST/TIR/handoff traversal belong to
//! failure_classification; this module owns the assertion-specific policy and diagnostic only.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::failure_classification::{
    EnclosingExitEffect, ExitClassification, classify_enclosing_exit_effect,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};

/// The only assertion whose checked message publishes no executable work.
pub(crate) fn assertion_condition_is_statically_true(condition: &Expression) -> bool {
    matches!(&condition.kind, ExpressionKind::Bool(true))
}

pub(crate) fn classify_assertion_message_effect(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    classify_enclosing_exit_effect(message, template_ir_store, ExitClassification::AssertionMessage)
}

pub(crate) fn assert_message_escape_diagnostic(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    Ok(classify_assertion_message_effect(message, template_ir_store)?.map(|effect| {
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
            effect.span(),
        )
    }))
}
