//! Type-aware single-predicate header parsing for value receivers.
//!
//! WHAT: parses the scrutinee once, confirms option or choice eligibility, consumes
//! `is`, and returns the shared pattern plus capture scope.
//! WHY: inline and block match bodies must not rescan the header. Syntax
//! classification stays in `if_headers.rs`; this file owns receiver-only pattern
//! eligibility and option `none`/literal diagnostics.

use super::token_checkpoint::TokenCheckpoint;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_headers::{
    parse_scrutinee_until_is, parse_single_predicate_match_pattern,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Shared facts after a committed single-predicate header.
///
/// WHAT: carries the parsed scrutinee, pattern, capture scope and the delimiter
/// that starts the authored body.
/// WHY: inline and block match parsers own only their body grammar after this
/// point.
pub(super) struct ParsedSinglePredicateHeader {
    pub(super) scrutinee: Expression,
    pub(super) pattern: MatchPattern,
    pub(super) then_context: ScopeContext,
    pub(super) body_delimiter: IfHeaderDelimiter,
}

pub(super) struct SinglePredicateHeaderInput<'a, 'b> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'b>,
    pub(super) string_table: &'a mut StringTable,
    pub(super) classification: IfHeaderClassification,
}

/// Attempts to parse a type-eligible single-predicate header after `if`.
///
/// Returns `None` only when an authored diagnostic still allows Bool fallback
/// before the match shape is committed. Infrastructure errors never fall back.
pub(super) fn try_parse_single_predicate_header(
    input: SinglePredicateHeaderInput<'_, '_>,
) -> Option<Result<ParsedSinglePredicateHeader, ExpressionParseError>> {
    let SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
    } = input;

    let checkpoint = TokenCheckpoint::capture(token_stream);
    let scrutinee_context = context.new_child_control_flow(ContextKind::Condition, string_table);
    let scrutinee = match parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
    ) {
        Ok(expression) => expression,
        Err(ExpressionParseError::Diagnostic(_)) => {
            checkpoint.restore(token_stream);
            return None;
        }
        Err(error @ ExpressionParseError::Infrastructure(_)) => return Some(Err(error)),
    };

    if token_stream.current_token_kind() != &TokenKind::Is {
        checkpoint.restore(token_stream);
        return None;
    }

    if !scrutinee_is_single_predicate_eligible(
        token_stream,
        type_interner,
        &scrutinee,
        classification,
    ) {
        checkpoint.restore(token_stream);
        return None;
    }

    token_stream.advance(); // consume `is`

    // Capture bindings such as `|name|` belong on the matched arm, not the
    // receiving declaration or the else branch.
    let match_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    let parsed_pattern = match parse_single_predicate_match_pattern(
        &scrutinee,
        token_stream,
        &match_context,
        type_interner,
        string_table,
    ) {
        Ok(pattern) => pattern,
        Err(error) => return Some(Err(error)),
    };

    checkpoint.commit();

    Some(Ok(ParsedSinglePredicateHeader {
        scrutinee,
        pattern: parsed_pattern.pattern,
        then_context: parsed_pattern.arm_scope,
        body_delimiter: classification.body_delimiter,
    }))
}

/// Detects unsupported optional single-predicate forms from classification facts.
///
/// WHAT: rejects `if maybe is none then ...` and literal predicates on optionals
/// because inline optional recovery must use present capture (`|value|`).
/// WHY: these diagnostics stay receiver-only and must not rescan the header.
pub(super) fn unsupported_optional_single_predicate_reason(
    token_stream: &FileTokens,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    classification: IfHeaderClassification,
) -> Option<InvalidControlFlowStatementReason> {
    let is_index = classification.is_index?;
    let pattern_index = classification.token_after_is?;

    let TokenKind::Symbol(scrutinee_name) = token_stream.current_token_kind() else {
        return None;
    };
    if token_stream.index + 1 != is_index {
        return None;
    }

    let scrutinee_type_id = context.get_reference(scrutinee_name)?.value.type_id;
    type_environment.option_inner_type(scrutinee_type_id)?;

    let pattern_token = &token_stream.tokens[pattern_index].kind;

    if matches!(pattern_token, TokenKind::NoneLiteral) {
        return Some(InvalidControlFlowStatementReason::ValueIfOptionNonePredicate);
    }

    if token_is_literal_pattern(pattern_token)
        && classification.inline_then_is_on_same_line_as(token_stream, pattern_index)
    {
        return Some(InvalidControlFlowStatementReason::ValueIfOptionLiteralPredicate);
    }

    None
}

fn scrutinee_is_single_predicate_eligible(
    token_stream: &FileTokens,
    type_interner: &AstTypeInterner<'_>,
    scrutinee: &Expression,
    classification: IfHeaderClassification,
) -> bool {
    let type_environment = type_interner.environment();
    let is_option_present_capture = type_environment
        .option_inner_type(scrutinee.type_id)
        .is_some()
        && classification.option_present_capture_candidate(token_stream);
    let is_choice_predicate = type_environment.variants_for(scrutinee.type_id).is_some();

    is_option_present_capture || is_choice_predicate
}

fn token_is_literal_pattern(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::StringSliceLiteral(_)
            | TokenKind::RawStringLiteral(_)
            | TokenKind::NumericLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::BoolLiteral(_)
    )
}
