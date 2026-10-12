//! Function-body expression statement filtering.
//!
//! WHAT: parses expression candidates in statement position and enforces the subset that can
//! stand alone as statements.
//! WHY: expression parsing is broader than statement grammar, so this module centralizes
//! statement-position filtering and targeted diagnostics.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
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
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidFallibleHandlingReason,
    InvalidStandaloneStatementReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;
/// Returns `true` if the given expression is valid in statement position.
///
/// Direct calls and handled fallible calls are always valid statements.
/// Runtime expressions are valid only when they contain at least one
/// call-like node, since a runtime block without calls has no side effects.
pub(crate) fn is_expression_statement(expression: &Expression) -> bool {
    match &expression.kind {
        // Direct calls and handled-fallible operations are valid statements.
        ExpressionKind::FunctionCall { .. }
        | ExpressionKind::HandledFallibleFunctionCall { .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { .. }
        | ExpressionKind::HandledFallibleExpression { .. }
        | ExpressionKind::HostFunctionCall { .. }
        | ExpressionKind::MethodCall { .. }
        | ExpressionKind::CollectionBuiltinCall { .. }
        | ExpressionKind::MapBuiltinCall { .. } => true,

        // Error-only `catch` recovery lowers control flow and handler side effects, but it
        // produces no success value. Value-producing catch blocks with result slots remain
        // invalid as standalone statements because that would discard the success value.
        ExpressionKind::ValueBlock { block } => {
            matches!(
                block.as_ref(),
                ValueBlock::Catch(value_catch) if value_catch.result_type_ids.is_empty()
            )
        }

        // Runtime expressions are valid only when they contain at least one
        // call-like node (method, builtin, function, or host call).
        ExpressionKind::Runtime(rpn) => rpn.items.iter().any(|item| {
            match item {
                ExpressionRpnItem::Operand(expression)
                    if matches!(
                        expression.kind,
                        ExpressionKind::MethodCall { .. }
                            | ExpressionKind::CollectionBuiltinCall { .. }
                            | ExpressionKind::MapBuiltinCall { .. }
                            | ExpressionKind::FunctionCall { .. }
                            | ExpressionKind::HostFunctionCall { .. }
                    ) =>
                {
                    true
                }
                ExpressionRpnItem::Operand(_) | ExpressionRpnItem::Operator { .. } => false,
                // Pending syntax never survives evaluation; flag it in debug builds so the
                // invalid boundary is visible instead of silently reporting no statement call.
                ExpressionRpnItem::PendingNumericLiteral { .. }
                | ExpressionRpnItem::PendingGroup { .. }
                | ExpressionRpnItem::PendingNumericBinding { .. }
                | ExpressionRpnItem::PendingNumericCast { .. }
                | ExpressionRpnItem::PendingNumericCall { .. }
                | ExpressionRpnItem::PendingNumericExpression { .. } => {
                    debug_assert!(
                        false,
                        "pending expression syntax reached statement classification"
                    );
                    false
                }
            }
        }),

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
    expression: &Expression,
) -> Result<(), ExpressionParseError> {
    if let Some(diagnostic) = rejects_discarded_fallible_success(expression) {
        return Err(diagnostic.into());
    }
    if !is_expression_statement(expression) {
        return Err(CompilerDiagnostic::invalid_standalone_statement(
            InvalidStandaloneStatementReason::Expression,
            expression.span,
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
fn rejects_discarded_fallible_success(expression: &Expression) -> Option<CompilerDiagnostic> {
    let is_handled_fallible = matches!(
        expression.kind,
        ExpressionKind::HandledFallibleExpression { .. }
            | ExpressionKind::HandledFallibleFunctionCall { .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { .. }
    );

    if is_handled_fallible && expression.type_id != builtin_type_ids::NONE {
        return Some(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::SuccessValueDiscarded,
            expression.span,
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
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let mut inferred = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
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
    validate_statement_expression(&expression, token_stream, string_table)?;
    Ok(expression)
}

/// Apply the statement-position rules to one parsed expression.
fn validate_statement_expression(
    expression: &Expression,
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
                CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "statement expression diagnostic",
                )
            })?
            .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
        return Err(CompilerDiagnostic::unexpected_token_from_tag(
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
pub(crate) fn push_statement_expression(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    match parse_and_validate_statement_expression(
        token_stream,
        context,
        type_interner,
        string_table,
        path_fork,
    ) {
        Ok(expression) => {
            let span = expression.span;
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
