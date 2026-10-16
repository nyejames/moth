//! Receiving-site parser entrypoint for value-producing control flow.
//!
//! WHAT: consumes `if` at closed receiver sites and routes through the shared
//! `if_headers` classifier to inline bool, inline single-predicate match, block
//! bool, block single-predicate match, or full match forms.
//! WHY: this is the only place where `if` is permitted in expression position;
//! general expression parsing continues to reject bare `if` everywhere else.
//!
//! This module must not make value blocks general expressions.

use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    ExpressionParseSeed, create_expression_until, create_expression_until_with_seed,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::numeric_construction::{
    StatementExpression, deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_if_statement_condition, if_condition_is_missing,
};
use crate::compiler_frontend::ast::statements::if_headers::{IfHeaderShape, classify_if_header};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ParsedReceiverValue, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

mod block_body;
mod block_if;
mod block_match;
mod full_match;
mod inline_if;
mod inline_match;
mod inline_then_else;
mod result_type;
mod retained;
mod single_predicate;

pub(crate) use retained::{DeferredValueBlock, RetainedValueBlock, complete_deferred_value_block};

use inline_then_else::same_logical_line;
use single_predicate::SinglePredicateParseOutcome;

/// Forwards accumulated parser warnings into the outer scope.
///
/// WHAT: drains a local warning vec and emits each warning through the scope context.
/// WHY: branch-local parsing (e.g. `function_body_to_ast`) may produce warnings
/// that belong to the enclosing receiver site.
pub(super) fn emit_collected_warnings(context: &ScopeContext, warnings: Vec<CompilerDiagnostic>) {
    for warning in warnings {
        context.emit_warning(warning);
    }
}

/// Shared input for inline and block value-if parsers.
///
/// WHAT: bundles the common state needed after the condition has been parsed.
pub(super) struct ValueIfParseInput<'a, 'b, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) target: ActiveValueProductionTarget,
    pub(super) string_table: &'a mut StringTable,
    pub(super) condition: StatementExpression,
    pub(super) span: Option<SourceSpan>,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Attempts to parse a value-producing block when the current token is `if` at a
/// closed receiving site.
///
/// WHAT: returns `None` if the current token is not `If`, otherwise parses the value
/// block and returns the resulting expression. The error preserves authored diagnostics and
/// retained-data infrastructure failures until the enclosing AST emission boundary.
#[allow(
    clippy::too_many_arguments,
    reason = "receiver parsing keeps the cursor, scope, expected types, kind and mutable state separate"
)]
pub fn try_parse_value_block_at_receiver(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expected_result_type_ids: &[TypeId],
    receiver_kind: ValueReceiverKind,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Option<Result<ParsedReceiverValue, ExpressionParseError>> {
    try_parse_value_block_at_receiver_with_target(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        ActiveValueProductionTarget::known(expected_result_type_ids.to_vec(), receiver_kind),
        string_table,
        path_fork,
    )
    .map(|parsed| {
        parsed.map(|value| match value {
            ParsedReceiverValue::NeedsSlotInference { .. } => {
                unreachable!(
                    "known receivers must wrap a finished expression; mixed-slot parse results belong to multi-bind"
                )
            }
            value => value,
        })
    })
}

/// Parses a value-producing `if` at a closed receiver using an explicit production target.
///
/// WHAT: the shared structural dispatcher for known and mixed inferred slots.
#[allow(
    clippy::too_many_arguments,
    reason = "shared dispatch keeps the cursor, scope, target and mutable state separate"
)]
pub fn try_parse_value_block_at_receiver_with_target(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    target: ActiveValueProductionTarget,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Option<Result<ParsedReceiverValue, ExpressionParseError>> {
    if token_stream.current_tag() != TokenTag::IF {
        return None;
    }

    let header_index = token_stream.position();
    let span = Some(token_stream.current_span());
    token_stream.advance();

    let classification = classify_if_header(token_stream);

    match single_predicate::unsupported_optional_single_predicate_reason(
        token_stream,
        context,
        type_interner.environment(),
        string_table,
        classification,
    ) {
        Ok(Some(reason)) => {
            return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
                reason,
                Some(token_stream.current_span()),
            )
            .into()));
        }
        Ok(None) => {}
        Err(error) => return Some(Err(error)),
    }

    match classification.shape {
        IfHeaderShape::FullMatch => Some(full_match::parse_value_match_at_receiver(
            full_match::ValueMatchParseInput {
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                target,
                string_table,
                span,
                path_fork,
            },
        )),

        IfHeaderShape::PotentialInlineSinglePredicate => {
            let seed = match inline_match::try_parse_inline_single_predicate_value_match(
                inline_match::InlineSinglePredicateParseInput {
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    target: target.clone(),
                    string_table,
                    header_index,
                    span,
                    classification,
                    path_fork,
                },
            ) {
                SinglePredicateParseOutcome::Decided(result) => return Some(result),
                SinglePredicateParseOutcome::OrdinarySubject(seed) => Some(seed),
                SinglePredicateParseOutcome::Absent => None,
            };

            Some(parse_bool_value_if_after_condition(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                target,
                string_table,
                header_index,
                span,
                seed,
                path_fork,
            ))
        }

        IfHeaderShape::PotentialBlockSinglePredicate => {
            let seed = match block_match::try_parse_block_single_predicate_value_match(
                block_match::BlockSinglePredicateParseInput {
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    target: target.clone(),
                    string_table,
                    span,
                    classification,
                    path_fork,
                },
            ) {
                SinglePredicateParseOutcome::Decided(result) => return Some(result),
                SinglePredicateParseOutcome::OrdinarySubject(seed) => Some(seed),
                SinglePredicateParseOutcome::Absent => None,
            };

            Some(parse_bool_value_if_after_condition(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                target,
                string_table,
                header_index,
                span,
                seed,
                path_fork,
            ))
        }

        IfHeaderShape::OrdinaryBool => Some(parse_bool_value_if_after_condition(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            target,
            string_table,
            header_index,
            span,
            None,
            path_fork,
        )),
    }
}

/// The receiver parser is the local join between expression parsing and recursive body parsing.
/// It therefore carries the shared two-lane error type instead of collapsing an internal frozen
/// The receiver parser preserves infrastructure failures from recursive expression/body parsing.
type ReceiverResult<T> = Result<T, ExpressionParseError>;
#[allow(
    clippy::too_many_arguments,
    reason = "receiver value parsing keeps the token stream, scope, mutable interner/string/path state, the value target, header position, and span as separate borrows"
)]
fn parse_bool_value_if_after_condition(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    target: ActiveValueProductionTarget,
    string_table: &mut StringTable,
    header_index: usize,
    span: Option<SourceSpan>,
    seed: Option<StatementExpression>,
    path_fork: &mut PathInternerFork,
) -> ReceiverResult<ParsedReceiverValue> {
    if seed.is_none() && if_condition_is_missing(token_stream) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut condition_type = ExpectedType::Infer;
    let condition_context =
        context.new_child_control_flow(ContextKind::Condition, string_table, path_fork);
    let mut cast_target_context = CastTargetContext::None;
    // An ordinary single-predicate subject already parsed exactly once; resume the central parser
    // from its seed instead of relexing it, so forward producers and generic requests stay issued
    // once. A completed subject contributes its ID, a retained one its ordered fragment.
    let seed = seed.map(|seed| match seed {
        StatementExpression::Built(id) => {
            ExpressionParseSeed::Completed(expression_builder.own(id))
        }
        StatementExpression::Retained(fragment) => ExpressionParseSeed::Fragment(fragment),
    });
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &condition_context,
        type_interner,
        expression_builder,
        const_values,
        expected_type: &mut condition_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
        path_fork,
    })
    .with_numeric_deferral_owner();
    let condition = match match seed {
        Some(seed) => {
            create_expression_until_with_seed(input, &[TokenTag::THEN, TokenTag::COLON], seed)
                .map(|expression| expression.id())
        }
        None => create_expression_until(input, &[TokenTag::THEN, TokenTag::COLON])
            .map(|expression| expression.id()),
    } {
        Ok(expression) => {
            ensure_if_statement_condition(
                expression_builder.view(expression)?,
                type_interner.environment(),
            )?;
            StatementExpression::Built(expression)
        }
        // A pending origin keeps the condition authored as `Bool` for the enclosing body root,
        // matching statement `if` headers. Only an owning deferral can be retained.
        Err(error @ ExpressionParseError::NumericDeferral(_)) => {
            let deferral = error.take_numeric_deferral("an if condition")?;
            StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                ExpectedType::Known(type_interner.environment().builtins().bool),
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            ))
        }
        Err(error) => return Err(error),
    };

    if token_stream.current_tag() == TokenTag::THEN {
        if !same_logical_line(token_stream, header_index, token_stream.position()) {
            return Err(CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::InlineValueIfMultiline,
                Some(token_stream.current_span()),
            )
            .into());
        }

        return inline_if::parse_inline_value_if(ValueIfParseInput {
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            target,
            string_table,
            condition,
            span,
            path_fork,
        });
    }

    if token_stream.current_tag() == TokenTag::COLON {
        return block_if::parse_block_value_if(ValueIfParseInput {
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            target,
            string_table,
            condition,
            span,
            path_fork,
        });
    }

    Err(CompilerDiagnostic::invalid_control_flow_statement(
        InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
        Some(token_stream.current_span()),
    )
    .into())
}
