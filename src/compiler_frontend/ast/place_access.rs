//! Shared receiver-place classification without constructing temporary place records.
//!
//! Parser and call validation borrow the issuing expression owner. Field access inherits
//! root mutability, so diagnostics name the binding that actually controls write access.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Source state consumed by source methods and collection/map receiver validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReceiverSourceState {
    Temporary,
    ImmutablePlace { binding_name: Option<StringId> },
    MutablePlace { binding_name: Option<StringId> },
}

pub(crate) fn classify_receiver_source_state(
    node: &AstNode,
    expressions: &AstExpressionBuilder,
    path_fork: &PathInternerFork,
) -> Result<ReceiverSourceState, InfrastructureFailure> {
    let NodeKind::ExpressionStatement(expression) = &node.kind else {
        return Ok(ReceiverSourceState::Temporary);
    };
    Ok(classify_expression_source_state(
        expressions.view(*expression)?,
        path_fork,
    ))
}

pub(crate) fn classify_expression_source_state(
    expression: ExpressionView<'_>,
    path_fork: &PathInternerFork,
) -> ReceiverSourceState {
    let mut root = expression;
    loop {
        match root.operation() {
            ExpressionOperationView::Reference(path) => {
                let binding_name = path_fork.component(path);
                return if root.value_mode().is_mutable() {
                    ReceiverSourceState::MutablePlace { binding_name }
                } else {
                    ReceiverSourceState::ImmutablePlace { binding_name }
                };
            }
            ExpressionOperationView::FieldAccess { base, .. } => root = base,
            _ => return ReceiverSourceState::Temporary,
        }
    }
}
