//! Structural `if` header classification shared by statements, templates and
//! value receivers.
//!
//! WHAT: one nesting-aware scan after `if`, plus statement/template header parsing
//! into Bool, option present-capture, or full-match `is:`.
//! WHY: value receivers reuse these structural facts rather than scanning `is`
//! again. This file does not own choice-predicate value matching.

use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_if_statement_condition, if_condition_is_missing,
};
use crate::compiler_frontend::ast::statements::match_headers::{
    build_option_present_capture_scope_and_pattern, parse_scrutinee_until_is,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, parse_option_pattern,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;

#[allow(clippy::large_enum_variant)]
pub(crate) enum ParsedIfHeader {
    BoolCondition {
        condition: Expression,
    },
    OptionPresentCapture {
        scrutinee: Expression,
        pattern: MatchPattern,
        then_context: ScopeContext,
    },
    MatchStyle {
        scrutinee: Expression,
    },
}

/// Syntax-only shape of tokens after `if`.
///
/// WHAT: distinguishes Bool conditions, full-match `is:`, and potential single-predicate
/// headers without inspecting types.
/// WHY: statement, template, receiver and multi-bind parsers must share one scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IfHeaderShape {
    OrdinaryBool,
    FullMatch,
    PotentialInlineSinglePredicate,
    PotentialBlockSinglePredicate,
}

/// Body delimiter found by the structural `if` header scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IfHeaderDelimiter {
    Colon,
    InlineThen,
    TemplateBody,
    TemplateClose,
    None,
}

/// Structural facts from one nesting-aware `if` header scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IfHeaderClassification {
    pub shape: IfHeaderShape,
    pub is_index: Option<usize>,
    pub token_after_is: Option<usize>,
    pub body_delimiter: IfHeaderDelimiter,
    pub delimiter_index: Option<usize>,
}

impl IfHeaderClassification {
    /// Returns true when the classified `then` sits on the same logical line as
    /// `token_index`.
    ///
    /// WHY: receiver option-literal diagnostics must not fire when `then` is on
    /// a later line; that malformed form stays `InlineValueIfMultiline`.
    pub(crate) fn inline_then_is_on_same_line_as(
        self,
        token_stream: &FileTokens,
        token_index: usize,
    ) -> bool {
        if self.body_delimiter != IfHeaderDelimiter::InlineThen {
            return false;
        }

        let Some(delimiter_index) = self.delimiter_index else {
            return false;
        };
        let Some(token) = token_stream.tokens.get(token_index) else {
            return false;
        };
        let Some(delimiter) = token_stream.tokens.get(delimiter_index) else {
            return false;
        };

        token.location.start_pos.line_number == delimiter.location.start_pos.line_number
    }

    /// Returns true when `|` is the raw next token after `is`.
    ///
    /// WHY: statement, template and value-receiver option capture all commit
    /// only on that adjacency. `token_after_is` skips newlines and must not be
    /// used for this commitment.
    pub(crate) fn option_present_capture_candidate(self, token_stream: &FileTokens) -> bool {
        let Some(is_index) = self.is_index else {
            return false;
        };

        token_stream
            .tokens
            .get(is_index + 1)
            .is_some_and(|token| token.kind == TokenKind::TypeParameterBracket)
    }
}

/// Stage-local result for `if` header parsing and option-present capture helpers.
///
/// WHY: `CompilerDiagnostic` is large enough that returning it directly inside a
/// `Result` triggers `clippy::result_large_err`. Boxing at this boundary keeps the
/// four `if`-header owner functions uniform without changing diagnostic semantics.
type IfHeaderResult<T> = Result<T, ExpressionParseError>;

/// Parse the header after `if`, leaving the stream at the colon or body marker.
pub(crate) fn parse_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    let classification = classify_if_header(token_stream);

    // `|name|` is not a valid expression, so option present-capture must be committed
    // before ordinary condition parsing. Choice-shaped headers stay Bool here.
    if classification.option_present_capture_candidate(token_stream) {
        return parse_option_present_capture_if_header(
            token_stream,
            context,
            type_interner,
            string_table,
        );
    }

    if classification.shape == IfHeaderShape::FullMatch {
        return parse_match_style_if_header(token_stream, context, type_interner, string_table);
    }

    if if_condition_is_missing(token_stream) {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
            token_stream.current_location(),
        )
        .into());
    }

    let condition_context = if_condition_parse_context(context, string_table);
    let mut condition_type = ExpectedType::Infer;
    let condition = create_expression(
        token_stream,
        &condition_context,
        type_interner,
        &mut condition_type,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
    )?;

    if token_stream.current_token_kind() == &TokenKind::Is {
        token_stream.advance();
        return Ok(ParsedIfHeader::MatchStyle {
            scrutinee: condition,
        });
    }

    ensure_if_statement_condition(&condition, type_interner.environment())?;

    Ok(ParsedIfHeader::BoolCondition { condition })
}

/// Classify the token stream after `if` with one nesting-aware scan.
pub(crate) fn classify_if_header(token_stream: &FileTokens) -> IfHeaderClassification {
    let mut nesting_depth = NestingDepth::default();
    let mut index = token_stream.index;

    while index < token_stream.length {
        let token_kind = &token_stream.tokens[index].kind;

        if nesting_depth.is_top_level() {
            match token_kind {
                TokenKind::Is => return classify_from_is(token_stream, index),
                TokenKind::Colon => {
                    return ordinary_bool_header(IfHeaderDelimiter::Colon, Some(index));
                }
                TokenKind::Then => {
                    return ordinary_bool_header(IfHeaderDelimiter::InlineThen, Some(index));
                }
                TokenKind::StartTemplateBody => {
                    return ordinary_bool_header(IfHeaderDelimiter::TemplateBody, Some(index));
                }
                TokenKind::TemplateClose => {
                    return ordinary_bool_header(IfHeaderDelimiter::TemplateClose, Some(index));
                }
                TokenKind::Eof => break,
                _ => {}
            }
        }

        nesting_depth.step(token_kind);
        index += 1;
    }

    ordinary_bool_header(IfHeaderDelimiter::None, None)
}

fn classify_from_is(token_stream: &FileTokens, is_index: usize) -> IfHeaderClassification {
    let token_after_is = next_meaningful_token_index(token_stream, is_index + 1);
    let Some(after_is) = token_after_is else {
        return IfHeaderClassification {
            shape: IfHeaderShape::OrdinaryBool,
            is_index: Some(is_index),
            token_after_is: None,
            body_delimiter: IfHeaderDelimiter::None,
            delimiter_index: None,
        };
    };

    if let Some(delimiter) = full_match_delimiter(&token_stream.tokens[after_is].kind) {
        return IfHeaderClassification {
            shape: IfHeaderShape::FullMatch,
            is_index: Some(is_index),
            token_after_is: Some(after_is),
            body_delimiter: delimiter,
            delimiter_index: Some(after_is),
        };
    }

    classify_single_predicate_after_pattern(token_stream, is_index, after_is)
}

fn classify_single_predicate_after_pattern(
    token_stream: &FileTokens,
    is_index: usize,
    pattern_start: usize,
) -> IfHeaderClassification {
    let mut nesting_depth = NestingDepth::default();
    let mut index = pattern_start;

    while index < token_stream.length {
        let token_kind = &token_stream.tokens[index].kind;

        if nesting_depth.is_top_level()
            && let Some(delimiter) = predicate_body_delimiter(token_kind)
        {
            let shape = match delimiter {
                IfHeaderDelimiter::InlineThen => IfHeaderShape::PotentialInlineSinglePredicate,
                IfHeaderDelimiter::Colon
                | IfHeaderDelimiter::TemplateBody
                | IfHeaderDelimiter::TemplateClose => IfHeaderShape::PotentialBlockSinglePredicate,
                IfHeaderDelimiter::None => IfHeaderShape::OrdinaryBool,
            };

            return IfHeaderClassification {
                shape,
                is_index: Some(is_index),
                token_after_is: Some(pattern_start),
                body_delimiter: delimiter,
                delimiter_index: Some(index),
            };
        }

        nesting_depth.step(token_kind);
        index += 1;
    }

    IfHeaderClassification {
        shape: IfHeaderShape::OrdinaryBool,
        is_index: Some(is_index),
        token_after_is: Some(pattern_start),
        body_delimiter: IfHeaderDelimiter::None,
        delimiter_index: None,
    }
}

fn full_match_delimiter(token_kind: &TokenKind) -> Option<IfHeaderDelimiter> {
    match token_kind {
        TokenKind::Colon => Some(IfHeaderDelimiter::Colon),
        TokenKind::StartTemplateBody => Some(IfHeaderDelimiter::TemplateBody),
        TokenKind::TemplateClose => Some(IfHeaderDelimiter::TemplateClose),
        _ => None,
    }
}

fn predicate_body_delimiter(token_kind: &TokenKind) -> Option<IfHeaderDelimiter> {
    match token_kind {
        TokenKind::Then => Some(IfHeaderDelimiter::InlineThen),
        other => full_match_delimiter(other),
    }
}

fn ordinary_bool_header(
    body_delimiter: IfHeaderDelimiter,
    delimiter_index: Option<usize>,
) -> IfHeaderClassification {
    IfHeaderClassification {
        shape: IfHeaderShape::OrdinaryBool,
        is_index: None,
        token_after_is: None,
        body_delimiter,
        delimiter_index,
    }
}

fn next_meaningful_token_index(token_stream: &FileTokens, start_index: usize) -> Option<usize> {
    let mut index = start_index;

    while index < token_stream.length {
        match token_stream.tokens[index].kind {
            TokenKind::Newline => index += 1,
            TokenKind::Eof => return None,
            _ => return Some(index),
        }
    }

    None
}

fn parse_match_style_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    let condition_context = if_condition_parse_context(context, string_table);
    let scrutinee = parse_scrutinee_until_is(
        token_stream,
        &condition_context,
        type_interner,
        string_table,
    )?;
    token_stream.advance(); // consume `is`

    Ok(ParsedIfHeader::MatchStyle { scrutinee })
}

fn parse_option_present_capture_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    let condition_context = if_condition_parse_context(context, string_table);
    let scrutinee = parse_scrutinee_until_is(
        token_stream,
        &condition_context,
        type_interner,
        string_table,
    )?;
    token_stream.advance(); // consume `is`

    let type_environment = type_interner.environment();
    let Some(inner_type_id) = type_environment.option_inner_type(scrutinee.type_id) else {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::OptionPresentCaptureOnNonOptional,
            None,
            None,
            scrutinee.location.clone(),
        )
        .into());
    };

    let pattern =
        parse_option_pattern(token_stream, inner_type_id, string_table, type_environment)?;
    let MatchPattern::OptionPresentCapture {
        name,
        binding_location,
        inner_type_id: capture_inner_type_id,
        location: pattern_location,
        ..
    } = &pattern
    else {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::ExpectedBindingInOptionPresentCapture,
            None,
            None,
            pattern.location().clone(),
        )
        .into());
    };

    let (then_context, pattern) = build_option_present_capture_scope_and_pattern(
        context,
        *name,
        binding_location,
        *capture_inner_type_id,
        pattern_location,
        type_interner,
        string_table,
    )?;

    Ok(ParsedIfHeader::OptionPresentCapture {
        scrutinee,
        pattern,
        then_context,
    })
}

fn if_condition_parse_context(
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> ScopeContext {
    context.new_child_control_flow(ContextKind::Condition, string_table)
}
