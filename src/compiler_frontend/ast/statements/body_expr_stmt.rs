//! Function-body expression statement filtering.
//!
//! WHAT: parses expression candidates in statement position and enforces the subset that can
//! stand alone as statements.
//! WHY: expression parsing is broader than statement grammar, so this module centralizes
//! statement-position filtering and targeted diagnostics.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, ExpressionStatementContinuation, NumericContinuation, StatementExpression,
    deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidFallibleHandlingReason, InvalidStandaloneStatementReason,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;
/// Returns `true` if the given expression is valid in statement position.
///
/// Direct calls and handled fallible calls are always valid statements.
/// Resolved operation expressions are valid only when an operand contains a call-like node,
/// since a computation without calls has no side effects.
pub(crate) fn is_expression_statement(expression: ExpressionView<'_>) -> bool {
    use ExpressionOperationView as Operation;
    match expression.operation() {
        Operation::Call(_)
        | Operation::MethodCall { .. }
        | Operation::CollectionBuiltin { .. }
        | Operation::MapBuiltin { .. }
        | Operation::HandleFallible { .. } => true,
        Operation::ValueBlock(ValueBlock::Catch(block)) => block.result_type_ids.is_empty(),
        Operation::ValueBlock(ValueBlock::LexicalScope(block)) => block.result_type_ids.is_empty(),
        Operation::NumericUnary { operand, .. } | Operation::LogicalNot { operand } => {
            is_expression_statement(operand)
        }
        Operation::NumericBinary { left, right, .. }
        | Operation::LogicalBinary { left, right, .. }
        | Operation::Compare { left, right, .. }
        | Operation::Range {
            start: left,
            end: right,
            ..
        } => is_expression_statement(left) || is_expression_statement(right),
        _ => false,
    }
}

/// Validate an expression that numeric completion resumed in statement position.
///
/// WHAT: applies the same two statement-position rules the parser applies before retention
///       (no discarded fallible success value, and at least one side-effecting call).
/// WHY: a statement retained until the body root must not skip validation, and the resumed
///      expression has no cursor to report against, so the authored span carries the diagnostic.
pub(crate) fn validate_resumed_statement_expression(
    expression: ExpressionView<'_>,
) -> Result<(), ExpressionParseError> {
    if let Some(diagnostic) = rejects_discarded_fallible_success(expression) {
        return Err(diagnostic.into());
    }
    if !is_expression_statement(expression) {
        return Err(choices_control_flow::invalid_standalone_statement(
            InvalidStandaloneStatementReason::Expression,
            expression.span(),
        )
        .into());
    }
    Ok(())
}

/// Checks whether a fallible expression's success value is being discarded.
///
/// Returns a diagnostic when a handled-fallible expression with a non-`none`
/// return type is used in statement position, since the caller is silently
/// dropping a potentially meaningful success value.
fn rejects_discarded_fallible_success(expression: ExpressionView<'_>) -> Option<DiagnosticDraft> {
    let is_handled_fallible = match expression.operation() {
        ExpressionOperationView::HandleFallible { .. } => true,
        ExpressionOperationView::Call(call) | ExpressionOperationView::MethodCall { call, .. } => {
            call.handling().is_some()
        }
        _ => false,
    };

    if is_handled_fallible && !expression.success_types().is_empty() {
        return Some(choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::SuccessValueDiscarded,
            expression.span(),
        ));
    }

    None
}

/// Parses an expression and validates that it is valid in statement position.
///
/// WHAT: statement positions own their complete fragment, so an unresolvable numeric origin
///       raises a retainable deferral instead of an escaped sub-fragment error.
/// WHY: the resumed statement is validated exactly where it was authored, and a fragment that
///      swallowed enclosing structure stays rejected. The expected-type and cast-target
///      temporaries are owned by this frame because the parse input borrows them.
///
/// Rejects expressions whose success value would be silently discarded,
/// and rejects non-call expressions that have no side effects as statements.
fn parse_and_validate_statement_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExprId, ExpressionParseError> {
    let mut inferred = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expression_builder,
            const_values,
            expected_type: &mut inferred,
            cast_target_context: &mut cast_target_context,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
            path_fork,
        },
        false,
    )
    .with_numeric_deferral_owner();
    let expression = create_expression_with_trailing_newline_policy(input)?;
    validate_statement_expression(
        expression_builder.view_owned(expression)?,
        token_stream,
        string_table,
    )?;
    Ok(expression.id())
}

/// Apply the statement-position rules to one parsed expression.
fn validate_statement_expression(
    expression: ExpressionView<'_>,
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    if let Some(diagnostic) = rejects_discarded_fallible_success(expression) {
        return Err(diagnostic.into());
    }

    if !is_expression_statement(expression) {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                InfrastructureFailure::malformed_token_projection(
                    "statement expression diagnostic",
                    error,
                )
            })?
            .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
        return Err(token_lexical::unexpected_token_from_tag(
            found,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

/// Parse one statement-position expression and place it in the body draft.
///
/// WHAT: a statement whose expression still holds construction-only numeric items records one
///       retained continuation in the authored position; the body root resumes it exactly once.
/// WHY: token stream has moved past a retained fragment, so the statement must be resumable from
///      retained structure rather than re-parsed, and its position must stay authored order.
#[allow(
    clippy::too_many_arguments,
    reason = "statement expression keeps stream, body draft, scope and mutable parse resources apart"
)]
pub(crate) fn push_statement_expression(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    match parse_and_validate_statement_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    ) {
        Ok(expression) => {
            let span = expression_builder.view(expression)?.span();
            draft.push_node(AstNode {
                kind: NodeKind::ExpressionStatement(expression),
                span,
                scope: context.scope,
            });
            Ok(())
        }
        Err(error) => {
            let deferral = error.take_numeric_deferral("a statement expression")?;
            let span = deferral.span;
            let expression = StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                deferral.expected,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            ));
            let continuation = context.register_numeric_continuation(
                NumericContinuation::ExpressionStatement(ExpressionStatementContinuation {
                    expression,
                    span,
                    scope: context.scope,
                }),
            )?;
            draft.push_pending(continuation);
            Ok(())
        }
    }
}
