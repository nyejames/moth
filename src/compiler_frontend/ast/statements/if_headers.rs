//! Narrow `if` header parsing shared by statements and template control-flow syntax.
//!
//! WHAT: disambiguates the header after an `if` keyword into a boolean condition,
//! single-predicate option-present capture, or full match-style `if value is:`.
//! WHY: template control-flow parsing uses the first two forms and match-style
//! detection without exposing full match-arm parsing outside `branching.rs`.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression, create_expression_until,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_if_statement_condition, if_condition_is_missing,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, parse_option_pattern,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidMatchPatternReason,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::{NestingDepth, find_expression_end_index};
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

/// Stage-local result for `if` header parsing and option-present capture helpers.
///
/// WHY: `CompilerDiagnostic` is large enough that returning it directly inside a
/// `Result` triggers `clippy::result_large_err`. Boxing at this boundary keeps the
/// four `if`-header owner functions uniform without changing diagnostic semantics.
type IfHeaderResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Parse the header after `if`, leaving the stream at the colon or body marker.
pub(crate) fn parse_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    // Single-predicate option match: `if option is |name|:`.
    // Detected before normal expression parsing because `|name|` is not a valid expression.
    if is_single_predicate_option_capture(token_stream) {
        return parse_option_present_capture_if_header(
            token_stream,
            context,
            type_interner,
            string_table,
        );
    }

    if is_full_match_style_header(token_stream) {
        return parse_match_style_if_header(token_stream, context, type_interner, string_table);
    }

    if if_condition_is_missing(token_stream) {
        return Err(Box::new(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedConditionAfterIf,
                token_stream.current_location(),
            ),
        ));
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
    )
    .map_err(|error| Box::new(CompilerDiagnostic::from(error)))?;

    if token_stream.current_token_kind() == &TokenKind::Is {
        token_stream.advance();
        return Ok(ParsedIfHeader::MatchStyle {
            scrutinee: condition,
        });
    }

    ensure_if_statement_condition(&condition, type_interner.environment())?;

    Ok(ParsedIfHeader::BoolCondition { condition })
}

fn is_full_match_style_header(token_stream: &FileTokens) -> bool {
    let mut nesting_depth = NestingDepth::default();
    let mut index = token_stream.index;

    while index < token_stream.length {
        let token = &token_stream.tokens[index];

        if nesting_depth.is_top_level() {
            match token.kind {
                TokenKind::Is => {
                    return next_meaningful_token_is_header_boundary(token_stream, index + 1);
                }
                TokenKind::Colon
                | TokenKind::StartTemplateBody
                | TokenKind::TemplateClose
                | TokenKind::Eof => return false,
                _ => {}
            }
        }

        nesting_depth.step(&token.kind);
        index += 1;
    }

    false
}

fn next_meaningful_token_is_header_boundary(token_stream: &FileTokens, start_index: usize) -> bool {
    let mut index = start_index;

    while index < token_stream.length {
        match token_stream.tokens[index].kind {
            TokenKind::Newline => index += 1,
            TokenKind::Colon | TokenKind::StartTemplateBody | TokenKind::TemplateClose => {
                return true;
            }
            _ => return false,
        }
    }

    false
}

fn parse_match_style_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    let condition_context = if_condition_parse_context(context, string_table);
    let mut condition_type = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &condition_context,
        type_interner,
        expected_type: &mut condition_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let scrutinee = create_expression_until(input, &[TokenKind::Is])
        .map_err(|error| Box::new(CompilerDiagnostic::from(error)))?;
    token_stream.advance(); // consume `is`

    Ok(ParsedIfHeader::MatchStyle { scrutinee })
}

/// Check whether the upcoming tokens form `if <expr> is |...|:`.
///
/// WHAT: scans for a top-level `is` before a top-level colon. If `is` exists and
/// the next token is `|`, this is a single-predicate option-present capture.
/// WHY: `|name|` is not a valid expression, so the normal expression parser would
/// fail. Detecting the shape early lets callers parse the scrutinee and pattern separately.
fn is_single_predicate_option_capture(token_stream: &FileTokens) -> bool {
    let is_index = find_expression_end_index(
        &token_stream.tokens,
        token_stream.index,
        &[
            TokenKind::Is,
            TokenKind::Colon,
            TokenKind::StartTemplateBody,
            TokenKind::TemplateClose,
        ],
    );
    if is_index >= token_stream.length {
        return false;
    }
    if token_stream.tokens[is_index].kind != TokenKind::Is {
        return false;
    }
    token_stream
        .tokens
        .get(is_index + 1)
        .is_some_and(|t| t.kind == TokenKind::TypeParameterBracket)
}

fn parse_option_present_capture_if_header(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<ParsedIfHeader> {
    let condition_context = if_condition_parse_context(context, string_table);
    let mut condition_type = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream,
        scope_context: &condition_context,
        type_interner,
        expected_type: &mut condition_type,
        cast_target_context: &mut cast_target_context,
        value_mode: &ValueMode::ImmutableOwned,
        string_table,
    });
    let scrutinee = create_expression_until(input, &[TokenKind::Is])
        .map_err(|error| Box::new(CompilerDiagnostic::from(error)))?;
    token_stream.advance(); // consume `is`

    let type_environment = type_interner.environment();
    let Some(inner_type_id) = type_environment.option_inner_type(scrutinee.type_id) else {
        return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::OptionPresentCaptureOnNonOptional,
            None,
            None,
            scrutinee.location.clone(),
        )));
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
        return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::ExpectedBindingInOptionPresentCapture,
            None,
            None,
            pattern.location().clone(),
        )));
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

/// Build an option-present capture arm scope and pattern.
///
/// WHAT: clones the parent branch/match context and adds a `Declaration` entry
/// for the inner payload binding so the branch body can reference the unwrapped value.
/// WHY: option present captures share one binding path construction rule across
/// statement `if`, value `if`, full matches, and future template `if` suffixes.
///
/// Validates:
/// - No capture name shadows an existing visible local (Moth no-shadowing rule).
pub(crate) fn build_option_present_capture_scope_and_pattern(
    match_context: &ScopeContext,
    capture_name: StringId,
    binding_location: &SourceLocation,
    inner_type_id: TypeId,
    pattern_location: &SourceLocation,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> IfHeaderResult<(ScopeContext, MatchPattern)> {
    let mut arm_scope = match_context.clone();

    if let Some(_existing) = arm_scope.get_reference(&capture_name) {
        return Err(Box::new(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::CaptureBindingShadowsVariable,
            None,
            None,
            binding_location.clone(),
        )));
    }

    let binding_name_str = string_table.resolve(capture_name).to_owned();
    let binding_path = arm_scope.scope.join_str(&binding_name_str, string_table);

    let capture_data_type = diagnostic_type_spelling(inner_type_id, type_interner.environment());
    let declaration = Declaration {
        id: binding_path.clone(),
        value: Expression::new(
            ExpressionKind::NoValue,
            binding_location.clone(),
            inner_type_id,
            capture_data_type,
            ValueMode::ImmutableOwned,
        ),
    };

    let binding_location = declaration.value.location.clone();
    arm_scope.add_var(declaration, binding_location.clone());

    let pattern = MatchPattern::OptionPresentCapture {
        name: capture_name,
        binding_path,
        inner_type_id,
        location: pattern_location.clone(),
        binding_location: binding_location.clone(),
    };

    Ok((arm_scope, pattern))
}
