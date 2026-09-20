//! Type-aware single-predicate header parsing for value receivers.
//!
//! WHAT: parses the scrutinee once, confirms option or choice eligibility, consumes
//! `is`, and returns the shared pattern plus capture scope.
//! WHY: inline and block match bodies must not rescan the header. Syntax
//! classification stays in `if_headers.rs`; this file owns receiver-only pattern
//! eligibility and option `none`/literal diagnostics.

use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
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
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Shared facts after a committed single-predicate header.
///
/// WHAT: carries the parsed scrutinee, pattern, capture scope and the delimiter
/// that starts the authored body.
/// WHY: inline and block match parsers own only their body grammar after this
/// point.
pub(in crate::compiler_frontend::ast::statements::value_production) struct ParsedSinglePredicateHeader
{
    pub scrutinee: Expression,
    pub pattern: MatchPattern,
    pub then_context: ScopeContext,
    pub body_delimiter: IfHeaderDelimiter,
}

pub(in crate::compiler_frontend::ast::statements::value_production) struct SinglePredicateHeaderInput<
    'a,
    'b,
    'tokens,
> {
    pub token_stream: &'a mut AstCursor<'tokens>,
    pub context: &'a ScopeContext,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub string_table: &'a mut StringTable,
    pub classification: IfHeaderClassification,
    pub path_fork: &'a mut PathInternerFork,
}

/// Attempts to parse a type-eligible single-predicate header after `if`.
///
/// Returns `None` only when an authored diagnostic still allows Bool fallback
pub(in crate::compiler_frontend::ast::statements::value_production) fn try_parse_single_predicate_header(
    input: SinglePredicateHeaderInput<'_, '_, '_>,
) -> Option<Result<ParsedSinglePredicateHeader, ExpressionParseError>> {
    let SinglePredicateHeaderInput {
        token_stream,
        context,
        type_interner,
        string_table,
        classification,
        path_fork,
    } = input;

    let start_index = token_stream.position();
    let scrutinee_context =
        context.new_child_control_flow(ContextKind::Condition, string_table, path_fork);
    let scrutinee = match parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
        path_fork,
    ) {
        Ok(expression) => expression,
        Err(ExpressionParseError::Diagnostic(_)) => {
            if let Err(error) = token_stream.set_position(start_index) {
                return Some(Err(ExpressionParseError::Infrastructure(Box::new(error))));
            }
            return None;
        }
        Err(error @ ExpressionParseError::Infrastructure(_)) => return Some(Err(error)),
    };

    if token_stream.current_tag() != TokenTag::IS {
        if let Err(error) = token_stream.set_position(start_index) {
            return Some(Err(ExpressionParseError::Infrastructure(Box::new(error))));
        }
        return None;
    }

    if !scrutinee_is_single_predicate_eligible(
        token_stream,
        type_interner,
        &scrutinee,
        classification,
    ) {
        if let Err(error) = token_stream.set_position(start_index) {
            return Some(Err(ExpressionParseError::Infrastructure(Box::new(error))));
        }
        return None;
    }

    token_stream.advance(); // consume `is`

    // Capture bindings such as `|name|` belong on the matched arm, not the
    // receiving declaration or the else branch.
    let match_context =
        context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
    let parsed_pattern = match parse_single_predicate_match_pattern(
        &scrutinee,
        token_stream,
        &match_context,
        type_interner,
        string_table,
        path_fork,
    ) {
        Ok(pattern) => pattern,
        Err(error) => return Some(Err(error)),
    };

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
pub(in crate::compiler_frontend::ast::statements::value_production) fn unsupported_optional_single_predicate_reason(
    token_stream: &AstCursor,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    classification: IfHeaderClassification,
) -> Result<Option<InvalidControlFlowStatementReason>, CompilerError> {
    let Some(is_index) = classification.is_index else {
        return Ok(None);
    };
    let Some(pattern_index) = classification.token_after_is else {
        return Ok(None);
    };

    if token_stream.current_tag() != TokenTag::SYMBOL {
        return Ok(None);
    }
    let Some(scrutinee_name) = token_stream.current_string_id_in(string_table)? else {
        return Ok(None);
    };
    if token_stream.position() + 1 != is_index {
        return Ok(None);
    }

    let Some(scrutinee_type_id) = context
        .get_reference(&scrutinee_name)
        .map(|reference| reference.value.type_id)
    else {
        return Ok(None);
    };
    if type_environment
        .option_inner_type(scrutinee_type_id)
        .is_none()
    {
        return Ok(None);
    }

    let Some(pattern_tag) = token_stream
        .token_ref_at(pattern_index)
        .map(|token| token.tag())
    else {
        return Ok(None);
    };
    if pattern_tag == TokenTag::NONE_LITERAL {
        return Ok(Some(
            InvalidControlFlowStatementReason::ValueIfOptionNonePredicate,
        ));
    }

    if token_is_literal_pattern(pattern_tag)
        && classification.inline_then_is_on_same_line_as(token_stream, pattern_index)
    {
        return Ok(Some(
            InvalidControlFlowStatementReason::ValueIfOptionLiteralPredicate,
        ));
    }

    Ok(None)
}

fn scrutinee_is_single_predicate_eligible(
    token_stream: &AstCursor,
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

fn token_is_literal_pattern(token: TokenTag) -> bool {
    matches!(
        token,
        TokenTag::STRING_SLICE_LITERAL
            | TokenTag::RAW_STRING_LITERAL
            | TokenTag::NUMERIC_LITERAL
            | TokenTag::CHAR_LITERAL
            | TokenTag::BOOL_LITERAL
    )
}
