//! Template-head directive operand parsing.
//!
//! WHAT:
//! - Recognises the one optional compile-time operand that may follow a directive name.
//! - Rejects operands after directives that take none.
//! - Rejects the removed call shape `$name(...)`.
//!
//! WHY:
//! - Every directive shares one operand grammar: `$name` or `$name value`. A directive never owns
//!   a parenthesised argument list, so the shared call-argument parser stays out of this path.
//!
//! ## Ownership boundary
//!
//! - **This module** owns directive-level lookahead for the operand boundary and the diagnostic
//!   for a removed call shape.
//! - **The head expression owner** parses the operand expression itself.
//! - **Slot modules** own slot schema and composition; they do not parse tokens.

use super::head_parser::parse_head_expression;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

/// Typed result shared by directive-operand parsing helpers.
type DirectiveArgsResult<T> = Result<T, TemplateError>;

/// Identifies the boundaries that end a bare directive.
///
/// An operand begins on the directive's line, so a newline also ends the bare form. Balanced
/// operand values may still continue over later lines once the operand has started. Slot and
/// decorator targets keep their own trailing-token rules and never reach this lookahead.
fn directive_operand_ends_at(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::TEMPLATE_CLOSE
            | TokenTag::START_TEMPLATE_BODY
            | TokenTag::COLON
            | TokenTag::COMMA
            | TokenTag::CLOSE_CURLY
            | TokenTag::ELSE
            | TokenTag::END
            | TokenTag::NEWLINE
            | TokenTag::EOF
    )
}

/// Rejects any operand written after a directive whose contract takes none.
pub(crate) fn reject_directive_operand(
    directive_name: StringId,
    token_stream: &AstCursor,
) -> DirectiveArgsResult<()> {
    let Some(next_tag) = token_stream.peek_next_tag() else {
        return Ok(());
    };
    if directive_operand_ends_at(next_tag) {
        return Ok(());
    }

    Err(with_operand_span(
        token_stream,
        templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::UnexpectedArguments,
            None,
        ),
    )
    .into())
}

/// Parses the optional single compile-time operand that follows a directive name.
///
/// The caller stays on the directive token. `Ok(None)` leaves the cursor unchanged. A successful
/// operand parse leaves the cursor after the expression so the head parser continues at the
/// separator or body boundary.
#[allow(
    clippy::too_many_arguments,
    reason = "operand parsing keeps the directive name and the head expression owners as separate borrows"
)]
pub(crate) fn parse_optional_directive_operand(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<Option<ExprId>> {
    let Some(next_tag) = token_stream.peek_next_tag() else {
        return Ok(None);
    };
    if directive_operand_ends_at(next_tag) {
        return Ok(None);
    }

    // The removed call spelling must not fall through to a generic expression error: an attached
    // `(` is a directive-shaped mistake. A whitespace-separated parenthesised value stays an
    // ordinary operand so a directive whose contract permits a record can accept one.
    if next_tag == TokenTag::OPEN_PARENTHESIS && parenthesis_attaches_to_directive(token_stream) {
        return Err(with_operand_span(
            token_stream,
            templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::UnexpectedArguments,
                None,
            ),
        )
        .into());
    }

    token_stream.advance();
    let expression = parse_head_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
        path_fork,
    )?;

    // A directive owns at most one operand: a second value or a directive-owned argument list is
    // a malformed operand, not another head item. A successful operand parse leaves the cursor on
    // the boundary token, so the check reads the current tag.
    let following_tag = token_stream.current_tag();
    if !directive_operand_ends_at(following_tag) {
        return Err(with_current_token_span(
            token_stream,
            templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::UnexpectedArguments,
                None,
            ),
        )
        .into());
    }

    Ok(Some(expression.id()))
}

/// Reports whether the `(` after the directive name is written as a call.
fn parenthesis_attaches_to_directive(token_stream: &AstCursor) -> bool {
    token_stream
        .token_ref_at_offset(1)
        .is_some_and(|token| token.shape().open_parenthesis_is_attached())
}

/// Attach the authored span of the operand position to a directive syntax diagnostic.
fn with_operand_span(token_stream: &AstCursor, mut diagnostic: DiagnosticDraft) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        let operand_span = token_stream
            .token_ref_at_offset(1)
            .map(|token| token.source_span());
        diagnostic.set_primary_span(operand_span);
    }
    diagnostic
}

/// Attach the authored span of the token the cursor currently owns.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(Some(token_stream.current_span()));
    }
    diagnostic
}
