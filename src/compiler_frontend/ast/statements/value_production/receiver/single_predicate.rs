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
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, StatementExpression,
};
use crate::compiler_frontend::ast::statements::if_headers::{
    IfHeaderClassification, IfHeaderDelimiter,
};
use crate::compiler_frontend::ast::statements::match_headers::{
    parse_scrutinee_until_is, parse_single_predicate_match_pattern, retained_match_subject,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchSubject;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Shared facts after a committed single-predicate header.
///
/// WHAT: carries the parsed subject, pattern, capture scope and the delimiter
/// that starts the authored body.
/// WHY: inline and block match parsers own only their body grammar after this
/// point, and a subject or pattern that still has to resolve keeps its authored
/// fragment instead of fabricating a typed value here.
pub(in crate::compiler_frontend::ast::statements::value_production) struct ParsedSinglePredicateHeader
{
    pub scrutinee: StatementExpression,
    pub pattern: DeferredArmPattern,
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
/// Returns `None` when the header is not a single predicate, so the Bool owner parses it. Any
/// outcome other than a committed header tombstones the generic requests the probe appended and
/// abandons the forward producers it registered: its scrutinee is dropped, and the Bool owner
/// reparses the same tokens into fresh tickets and fresh producers.
pub(in crate::compiler_frontend::ast::statements::value_production) fn try_parse_single_predicate_header(
    input: SinglePredicateHeaderInput<'_, '_, '_>,
) -> Option<Result<ParsedSinglePredicateHeader, ExpressionParseError>> {
    let context = input.context;
    let producer_checkpoint = context.numeric_state().forward_producer_checkpoint();
    let request_checkpoint = context.generic_request_checkpoint();
    let outcome = parse_single_predicate_probe(input);
    if !matches!(outcome, Some(Ok(_))) {
        context.discard_generic_requests_since(request_checkpoint);
        context
            .numeric_state_mut()
            .abandon_forward_producers_since(producer_checkpoint);
    }
    outcome
}

/// Parses one single-predicate probe. The caller owns request and producer rollback on non-commit.
fn parse_single_predicate_probe(
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
    let start_span = token_stream.current_span();
    let scrutinee_context =
        context.new_child_control_flow(ContextKind::Condition, string_table, path_fork);
    let scrutinee = match parse_scrutinee_until_is(
        token_stream,
        &scrutinee_context,
        type_interner,
        string_table,
        path_fork,
    ) {
        Ok(scrutinee) => scrutinee,
        Err(ExpressionParseError::Diagnostic(_)) => {
            if let Err(error) = token_stream.set_position(start_index) {
                return Some(Err(ExpressionParseError::Infrastructure(Box::new(error))));
            }
            return None;
        }
        Err(error) => return Some(Err(error)),
    };

    if token_stream.current_tag() != TokenTag::IS {
        if let Err(error) = token_stream.set_position(start_index) {
            return Some(Err(ExpressionParseError::Infrastructure(Box::new(error))));
        }
        return None;
    }

    let subject = match &scrutinee {
        StatementExpression::Built(scrutinee) => MatchSubject::Known(scrutinee.type_id),
        // A retained subject classifies through the shared retained operand owner: a concrete type
        // names its subject type, a retained generic choice names its variants from its declared
        // owner, and any other pending numeric subject stays `Pending` until the body root selects it.
        StatementExpression::Retained(retained) => {
            retained_match_subject(context, type_interner.environment(), &retained.items)
        }
    };

    // An optional subject is already an option, whether its exact type is known or still retained,
    // so `none` and literal predicates are rejected with the same diagnostic the ready symbol path
    // reports, before the header can fall back to Bool. The authored pattern is the classified
    // token after `is`, which may sit on the following line.
    let option_rejection = match classification.token_after_is {
        Some(pattern_index) if subject_is_optional(type_interner.environment(), &subject) => {
            option_predicate_rejection(token_stream, classification, pattern_index)
        }
        _ => None,
    };
    if let Some(reason) = option_rejection {
        return Some(Err(CompilerDiagnostic::invalid_control_flow_statement(
            reason,
            Some(start_span),
        )
        .into()));
    }

    if !subject_is_single_predicate_eligible(
        token_stream,
        type_interner.environment(),
        &subject,
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
        &subject,
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

    let Some(scrutinee_reference) = context.get_reference(&scrutinee_name) else {
        return Ok(None);
    };
    let Some(scrutinee_type_id) = scrutinee_reference
        .ready_declaration()
        .map(|declaration| declaration.value.type_id)
    else {
        return Ok(None);
    };
    if type_environment
        .option_inner_type(scrutinee_type_id)
        .is_none()
    {
        return Ok(None);
    }

    Ok(option_predicate_rejection(
        token_stream,
        classification,
        pattern_index,
    ))
}

/// Decides whether a committed `is` header is a receiver-level option or choice predicate.
///
/// WHAT: a known subject follows the ready option or choice rule; a pending choice is eligible
/// because its variants are declared; a pending option accepts only present capture; a pending
/// scalar is not a predicate subject.
/// WHY: a pending scalar `is` is an ordinary Bool comparison, so it must reach the Bool owner,
/// which retains the whole condition for body-root completion. A retained subject never widens the
/// predicate surface beyond what its resolved shape allows.
fn subject_is_single_predicate_eligible(
    token_stream: &AstCursor,
    type_environment: &TypeEnvironment,
    subject: &MatchSubject,
    classification: IfHeaderClassification,
) -> bool {
    match subject {
        MatchSubject::Known(type_id) => {
            let is_option_present_capture = type_environment.option_inner_type(*type_id).is_some()
                && classification.option_present_capture_candidate(token_stream);
            let is_choice_predicate = type_environment.variants_for(*type_id).is_some();
            is_option_present_capture || is_choice_predicate
        }
        MatchSubject::PendingChoice(_) => true,
        MatchSubject::PendingOption { .. } => {
            classification.option_present_capture_candidate(token_stream)
        }
        MatchSubject::Pending => false,
    }
}

/// Whether a subject is an optional, whether its exact type is known or still retained.
///
/// WHY: an optional is already an option however its type was reached, so a grouped or
/// newline-separated known subject rejects `none` and literal predicates like a pending one does.
fn subject_is_optional(type_environment: &TypeEnvironment, subject: &MatchSubject) -> bool {
    match subject {
        MatchSubject::Known(type_id) => type_environment.option_inner_type(*type_id).is_some(),
        MatchSubject::PendingOption { .. } => true,
        MatchSubject::Pending | MatchSubject::PendingChoice(_) => false,
    }
}

/// Reason for a `none` or literal predicate on an option, which must use present capture instead.
///
/// WHAT: `none` is always rejected; a literal is rejected when its body starts on the same line.
/// WHY: ready, known optional and pending option subjects share this rule, so one authored
/// predicate reports the same diagnostic whether its subject is already known or still retained.
fn option_predicate_rejection(
    token_stream: &AstCursor,
    classification: IfHeaderClassification,
    pattern_index: usize,
) -> Option<InvalidControlFlowStatementReason> {
    let pattern_tag = token_stream.token_ref_at(pattern_index)?.tag();
    if pattern_tag == TokenTag::NONE_LITERAL {
        return Some(InvalidControlFlowStatementReason::ValueIfOptionNonePredicate);
    }
    if token_is_literal_pattern(pattern_tag)
        && classification.inline_then_is_on_same_line_as(token_stream, pattern_index)
    {
        return Some(InvalidControlFlowStatementReason::ValueIfOptionLiteralPredicate);
    }
    None
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
