//! Centralized template-head directive argument parsing.
//!
//! WHAT:
//! - Shared helpers for parsing parenthesized arguments after `$directive` tokens.
//! - Directive-level validation for empty arguments and single-argument arity.
//!
//! WHY:
//! - The shared call-argument owner handles parentheses, separators, newlines,
//!   named-entry recognition, access markers, and expression boundaries.
//! - Directive modules retain semantic validation: type checking, compile-time
//!   restrictions, and normalization into template/style state.
//!
//! ## Ownership boundary
//!
//! - **This module** owns directive-level token lookahead and the one-argument
//!   arity/category adapter.
//! - **The shared call-argument owner** owns the parenthesized list syntax.
//! - **Slot modules** own slot schema and composition; they do not parse tokens.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::CallAccessMode;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentDiagnosticContext, CallArgumentNamingPolicy, CallArgumentReceivingContext,
    CallArgumentSyntax, CallArgumentSyntaxContext, CallArgumentValuePolicy,
    parse_call_arguments_with_receiving_context,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, DiagnosticToken, InvalidCallShapeReason,
    InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::numeric_text::parse::materialize_i32;
use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Typed result shared by directive-argument parsing helpers.
type DirectiveArgsResult<T> = Result<T, TemplateError>;

/// Returns true if the next token after the current directive is `(`.
///
/// WHAT: pure read-only `(` lookahead on the canonical cursor view.
/// WHY: directive dispatch must not advance before committing to the paren
/// path; the cached cursor lookahead keeps that fact read-only.
pub(crate) fn directive_has_arguments(token_stream: &AstCursor) -> bool {
    token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS)
}

/// Advances the token stream from the directive token past `(` into the
/// first argument position.
///
/// Precondition: `directive_has_arguments` returned `true`.
pub(crate) fn advance_into_directive_arguments(token_stream: &mut AstCursor) {
    token_stream.advance(); // past directive token
    token_stream.advance(); // past '('
}

/// Rejects parenthesized arguments for directives that do not accept them.
pub(crate) fn reject_unexpected_directive_arguments(
    directive_name: StringId,
    token_stream: &AstCursor,
) -> DirectiveArgsResult<()> {
    if directive_has_arguments(token_stream) {
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::UnexpectedArguments,
                None,
            ),
        )
        .into());
    }
    Ok(())
}

/// Returns an error if the current token is `)`, signalling empty directive
/// parentheses.
pub(crate) fn reject_empty_directive_parens(
    directive_name: StringId,
    token_stream: &AstCursor,
) -> DirectiveArgsResult<()> {
    if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::EmptyArguments,
                None,
            ),
        )
        .into());
    }
    Ok(())
}

/// Identifies template boundaries that end a directive argument before its expression begins.
/// This keeps empty arguments with the directive parser while header balancing retains true EOF.
fn ends_directive_argument_without_expression(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::TEMPLATE_CLOSE
            | TokenTag::START_TEMPLATE_BODY
            | TokenTag::COLON
            | TokenTag::CLOSE_CURLY
            | TokenTag::ELSE
            | TokenTag::END
    )
}

/// Expects the current token to be `)`. Returns a syntax error with a
/// suggestion if it is not.
pub(crate) fn expect_directive_close_paren(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> DirectiveArgsResult<()> {
    if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
        return Ok(());
    }

    let found = token_stream
        .current_diagnostic_token(string_table)
        .map_err(|error| {
            CompilerDiagnostic::token_view_invariant_error(
                error,
                "template directive close-paren diagnostic",
            )
        })
        .map_err(TemplateError::from)?;
    let found =
        found.or_else(|| Some(DiagnosticToken::from_static_tag(token_stream.current_tag())));
    Err(with_current_token_span(
        token_stream,
        CompilerDiagnostic::expected_token_from_tags(TokenTag::CLOSE_PARENTHESIS, found, None),
    )
    .into())
}

/// Parses one implemented-directive argument list through the shared call
/// argument owner, then applies the directive's positional single-argument
/// contract.
///
/// The shared owner consumes the opening and closing parentheses. This adapter
/// restores the cursor to the closing `)` so the template-head parser keeps its
/// existing separator-advance contract.
fn parse_single_directive_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<Expression> {
    // The caller is still on the directive token. Keep the opening-paren
    // position so the shared owner receives exactly the cursor shape it owns.
    token_stream.advance();
    let opening_position = token_stream.position();

    // Preserve the directive-owned empty and template-boundary diagnostics
    // before handing the parenthesized list to the shared parser.
    token_stream.advance();
    reject_empty_directive_parens(directive_name, token_stream)?;
    token_stream.skip_newlines();
    reject_empty_directive_parens(directive_name, token_stream)?;
    if ends_directive_argument_without_expression(token_stream.current_tag()) {
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::EmptyArguments,
                None,
            ),
        )
        .into());
    }

    token_stream
        .set_position(opening_position)
        .map_err(TemplateError::from)?;

    let syntax = CallArgumentSyntax::Supported {
        callee_name: Some(directive_name),
    };
    let receiving_context = CallArgumentReceivingContext::with_policies(
        CallArgumentDiagnosticContext::from_syntax(syntax),
        CallArgumentNamingPolicy::PositionalOnly,
        CallArgumentValuePolicy::Ordinary,
    );
    let arguments = parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        string_table,
        receiving_context,
        None,
        CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )
    .map_err(|error| {
        let error = TemplateError::from(error);
        if token_stream.current_tag() == TokenTag::EOF {
            error.map_diagnostic(|diagnostic| {
                if matches!(
                    diagnostic.payload,
                    DiagnosticPayload::UnexpectedToken { .. }
                ) {
                    return with_current_token_span(
                        token_stream,
                        CompilerDiagnostic::expected_token_from_tags(
                            TokenTag::CLOSE_PARENTHESIS,
                            Some(DiagnosticToken::from_static_tag(TokenTag::EOF)),
                            None,
                        ),
                    );
                }
                diagnostic
            })
        } else {
            error
        }
    })?;

    // The shared parser has consumed `)`. Keep the old directive-helper
    // postcondition so the head parser advances it exactly once.
    let close_position = token_stream.position().saturating_sub(1);
    token_stream
        .set_position(close_position)
        .map_err(TemplateError::from)?;

    let Some(argument) = arguments.first() else {
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::EmptyArguments,
                None,
            ),
        )
        .into());
    };

    // Positional-only is a directive contract, not a reason to discard a
    // parsed label. The shared owner retains the label and this adapter emits
    // the supported-call diagnostic lane instead of silently accepting it.
    if argument.target_param.is_some() {
        return Err(CompilerDiagnostic::invalid_call_shape(
            InvalidCallShapeReason::NamedArgumentsNotSupported,
            Some(directive_name),
            argument.target_span.or(argument.span),
        )
        .into());
    }

    if argument.access_mode != CallAccessMode::Shared {
        return Err(CompilerDiagnostic::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::invalid_argument(),
            argument.marker_span.or(argument.span),
        )
        .into());
    }

    if arguments.len() > 1 {
        let diagnostic = CompilerDiagnostic::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(TokenTag::COMMA),
            argument.following_separator_span.or(argument.span),
        );
        return Err(diagnostic.into());
    }

    Ok(arguments
        .into_iter()
        .next()
        .expect("validated directive argument list is non-empty")
        .value)
}

/// Parses an optional parenthesized compile-time expression after a directive.
///
/// Returns `Ok(None)` if no `(` follows the directive.
/// Returns `Ok(Some(expression))` if a single expression was parsed.
pub(crate) fn parse_optional_parenthesized_expression(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<Option<Expression>> {
    if !directive_has_arguments(token_stream) {
        return Ok(None);
    }

    parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        string_table,
        path_fork,
    )
    .map(Some)
}

/// Parses a required parenthesized compile-time expression after a directive.
///
/// Returns an error if no `(` follows the directive.
pub(crate) fn parse_required_parenthesized_expression(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<Expression> {
    if !directive_has_arguments(token_stream) {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "template directive opening-paren diagnostic",
                )
            })
            .map_err(TemplateError::from)?
            .or_else(|| Some(DiagnosticToken::from_static_tag(token_stream.current_tag())));
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::expected_token_from_tags(TokenTag::OPEN_PARENTHESIS, found, None),
        )
        .into());
    }

    parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        string_table,
        path_fork,
    )
}

// ----------------------------------------------------------------------------
// Slot-target argument parsers (moved from template_slots.rs)
// ----------------------------------------------------------------------------

/// Parses the optional argument to `$slot`: default (no parens), named string,
/// or positive positional integer.
pub(crate) fn parse_optional_slot_target_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
) -> DirectiveArgsResult<SlotKey> {
    if !directive_has_arguments(token_stream) {
        return Ok(SlotKey::Default);
    }

    advance_into_directive_arguments(token_stream);

    let target = match token_stream.current_tag() {
        TokenTag::STRING_SLICE_LITERAL => {
            let name = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "string literal token had no payload",
                    )
                })?;
            SlotKey::Named(name)
        }
        TokenTag::NUMERIC_LITERAL => {
            let token = token_stream
                .current_numeric_literal_in(string_table)?
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "numeric literal token had no payload",
                    )
                })?;
            if token.kind != NumericLiteralKind::WholeNumber {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_template_directive(
                        Some(directive_name),
                        InvalidTemplateDirectiveReason::InvalidSlotTarget,
                        None,
                    ),
                )
                .into());
            }

            let index = materialize_i32(&token, string_table).map_err(|reason| {
                with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_number_literal(token.source_text, reason, None),
                )
            })?;

            if index <= 0 {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_template_directive(
                        Some(directive_name),
                        InvalidTemplateDirectiveReason::InvalidSlotTarget,
                        None,
                    ),
                )
                .into());
            }

            SlotKey::Positional(index as usize)
        }
        TokenTag::CLOSE_PARENTHESIS => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::EmptyArguments,
                    None,
                ),
            )
            .into());
        }
        _ => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidSlotTarget,
                    None,
                ),
            )
            .into());
        }
    };

    token_stream.advance();
    expect_directive_close_paren(token_stream, string_table)?;
    Ok(target)
}

/// Parses the required named target argument to `$insert("name")`.
pub(crate) fn parse_required_slot_name_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
) -> DirectiveArgsResult<StringId> {
    if !directive_has_arguments(token_stream) {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "template insert opening-paren diagnostic",
                )
            })
            .map_err(TemplateError::from)?
            .or_else(|| Some(DiagnosticToken::from_static_tag(token_stream.current_tag())));
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::expected_token_from_tags(TokenTag::OPEN_PARENTHESIS, found, None),
        )
        .into());
    }

    advance_into_directive_arguments(token_stream);

    let slot_name = match token_stream.current_tag() {
        TokenTag::STRING_SLICE_LITERAL => token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "string literal token had no payload",
                )
            })?,
        TokenTag::NUMERIC_LITERAL => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidInsertTarget,
                    None,
                ),
            )
            .into());
        }
        TokenTag::CLOSE_PARENTHESIS => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::EmptyArguments,
                    None,
                ),
            )
            .into());
        }
        _ => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidInsertTarget,
                    None,
                ),
            )
            .into());
        }
    };

    token_stream.advance();
    expect_directive_close_paren(token_stream, string_table)?;
    Ok(slot_name)
}

/// Attach the authored span of the token that owns a directive syntax diagnostic.
///
/// Directive argument validation keeps the exact global span of the token
/// owning a syntax diagnostic.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(token_stream.current_span());
    }
    diagnostic
}
