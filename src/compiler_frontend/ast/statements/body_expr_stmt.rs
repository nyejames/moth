//! Function-body expression statement filtering.
//!
//! WHAT: parses expression candidates in statement position and enforces the subset that can
//! stand alone as statements.
//! WHY: expression parsing is broader than statement grammar, so this module centralizes
//! statement-position filtering and targeted diagnostics.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
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
            matches!(
                item,
                ExpressionRpnItem::Operand(expression)
                    if matches!(
                        expression.kind,
                        ExpressionKind::MethodCall { .. }
                            | ExpressionKind::CollectionBuiltinCall { .. }
                            | ExpressionKind::MapBuiltinCall { .. }
                            | ExpressionKind::FunctionCall { .. }
                            | ExpressionKind::HostFunctionCall { .. }
                    )
            )
        }),

        _ => false,
    }
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
            expression.location.clone(),
        ));
    }

    None
}

/// Parses an expression and validates that it is valid in statement position.
///
/// Rejects expressions whose success value would be silently discarded,
/// and rejects non-call expressions that have no side effects as statements.
fn parse_and_validate_statement_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let mut inferred = ExpectedType::Infer;
    let expression = create_expression(
        token_stream,
        context,
        type_interner,
        &mut inferred,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
    )?;

    if let Some(diagnostic) = rejects_discarded_fallible_success(&expression) {
        return Err(diagnostic.into());
    }

    if !is_expression_statement(&expression) {
        return Err(CompilerDiagnostic::unexpected_token(
            token_stream.current_token_kind().to_owned(),
            token_stream.current_location(),
        )
        .into());
    }

    Ok(expression)
}

pub(crate) fn parse_expression_statement_candidate(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    parse_and_validate_statement_expression(token_stream, context, type_interner, string_table)
}

pub(crate) fn parse_symbol_expression_statement_candidate(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    _symbol_id: StringId,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    parse_and_validate_statement_expression(token_stream, context, type_interner, string_table)
}
