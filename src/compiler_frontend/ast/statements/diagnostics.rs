//! Statement-position diagnostic helpers.
//!
//! WHAT: builds diagnostic drafts for unexpected tokens and scope closes encountered during
//!       function-body statement dispatch.
//! WHY: keeps `body_dispatch.rs` focused on parsing logic rather than diagnostic construction.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, token_lexical,
};
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidStatementPositionReason, SourceScopeConstruct,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::line_scanning::{
    ScopeOpening, closer_follows_incomplete_expression, closer_layout_diagnostic,
    closer_trailing_diagnostic, nested_scope_opening_diagnostic,
};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenRef, TokenTag};

/// Attach the exact authored range of the current token when this stream has a source identity.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(Some(token_stream.current_span()));
    }

    diagnostic
}

/// Produce a diagnostic for an unexpected token in statement position.
///
/// WHAT: maps each unexpected token tag to the most appropriate typed diagnostic.
/// WHY: centralizes the constructor decision so the statement dispatch loop stays readable.
pub(crate) fn unexpected_statement_token(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Result<DiagnosticDraft, InfrastructureFailure> {
    let current_tag = token_stream.current_tag();
    let span = Some(token_stream.current_span());
    let diagnostic = match current_tag {
        TokenTag::COMMA => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedComma,
            span,
        ),

        TokenTag::CLOSE_PARENTHESIS => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseParenthesis,
            span,
        ),

        TokenTag::CLOSE_CURLY => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseCurly,
            span,
        ),

        // The `|` token is only valid in type-parameter position, not as a statement.
        TokenTag::TYPE_PARAMETER_BRACKET => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedPipe,
            span,
        ),

        TokenTag::ARROW => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedArrow,
            span,
        ),

        TokenTag::WILDCARD => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedWildcard,
            span,
        ),

        // `type` in statement position looks like an attempt to declare a generic parameter.
        TokenTag::TYPE => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            span,
        ),

        TokenTag::OF => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            span,
        ),

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
                reserved_trait_keyword_error(keyword, span)
            } else {
                // Invariant: Must and TraitThis are always reserved trait keywords.
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        InfrastructureFailure::malformed_token_projection(
                            "statement-position unexpected-token diagnostic",
                            error,
                        )
                    })?
                    .unwrap_or_else(|| {
                        DiagnosticToken::from_static_tag(token_stream.current_tag())
                    });
                token_lexical::unexpected_token_from_tag(found, span)
            }
        }

        _ => {
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    InfrastructureFailure::malformed_token_projection(
                        "statement-position unexpected-token diagnostic",
                        error,
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            token_lexical::unexpected_token_from_tag(found, span)
        }
    };

    Ok(with_current_token_span(token_stream, diagnostic))
}

/// Context for an unexpected scope-close (`;`) diagnostic.
pub(crate) enum UnexpectedScopeCloseContext {
    /// The scope close appeared inside an expression, where `;` is not valid.
    Expression,

    /// The scope close appeared inside a template literal, where `;` is not valid.
    Template,

    /// The scope close appeared in top-level code, where no block is open.
    TopLevel,
}

/// Produce a diagnostic for an unexpected scope-close (`;`) in expression, template or
/// top-level context.
///
/// WHAT: expressions and templates are not terminated with `;`, and top-level code has no
///       open block, so encountering `End` there needs a targeted explanation.
pub(crate) fn unexpected_scope_close(
    context: UnexpectedScopeCloseContext,
    token_stream: &AstCursor,
) -> DiagnosticDraft {
    let reason = match context {
        UnexpectedScopeCloseContext::Expression => {
            InvalidStatementPositionReason::UnexpectedScopeCloseInExpression
        }
        UnexpectedScopeCloseContext::Template => {
            InvalidStatementPositionReason::UnexpectedScopeCloseInTemplate
        }
        UnexpectedScopeCloseContext::TopLevel => {
            InvalidStatementPositionReason::UnexpectedScopeCloseAtTopLevel
        }
    };

    with_current_token_span(
        token_stream,
        choices_control_flow::invalid_statement_position(reason, Some(token_stream.current_span())),
    )
}

/// The canonical index of the token the cursor is on.
///
/// WHAT: translates the parser coordinate into the canonical source index that layout checks,
///       spans and the token store share.
/// WHY: segmented views report dense parser positions, so only the token reference itself knows
///      where the authored token lives.
pub(crate) fn current_token_index(
    token_stream: &AstCursor,
) -> Result<TokenIndex, InfrastructureFailure> {
    token_stream.current().map(TokenRef::index).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a statement-layout check ran past the end of its token view",
        )
    })
}

/// Read the opener colon at a parser coordinate as one scope opening site.
///
/// WHAT: pairs the colon's canonical index with its authored span and the construct it began.
/// WHY: every opener owner has already validated the colon tag, so this only converts the token
///      into the layout vocabulary rather than repeating the lookup.
pub(crate) fn scope_opening_at(
    token_stream: &AstCursor,
    index: usize,
    construct: SourceScopeConstruct,
) -> Result<ScopeOpening, InfrastructureFailure> {
    let token = token_stream.token_ref_at(index).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a scope opener was read outside its token view",
        )
    })?;
    Ok(ScopeOpening {
        site: token.index(),
        opener: token.index(),
        label_span: token.source_span(),
        construct,
    })
}

/// The span of the last `END` token between an opener colon and the current `EOF`.
///
/// WHAT: walks the canonical token store backward from the cursor's `EOF` token exclusive to
///       `opener` inclusive and returns the first `END` it finds.
/// WHY: at a 0042 EOF site the reported scope is the innermost open scope. Any deeper closed
///      scope's parent must also have closed later, or the reported scope would not be
///      innermost. Grouped `;` is rejected before EOF, so this last `END` closed the direct
///      child that explains why the outer scope remains open.
pub(crate) fn last_child_closer_span(
    token_stream: &AstCursor,
    opener: TokenIndex,
) -> Option<SourceSpan> {
    let eof = token_stream.current()?.index().index();
    let opener = opener.index();
    let mut index = eof.checked_sub(1)?;
    while index >= opener {
        let token = token_stream.canonical_token_ref_at(TokenIndex::from_raw(index as u32))?;
        if token.tag() == TokenTag::END {
            return Some(token.source_span());
        }
        index = index.checked_sub(1)?;
    }
    None
}

/// Report a nested opener that shares its physical line with the enclosing scope's opener.
///
/// WHAT: compares `opener` with the enclosing opener carried by `context`.
/// WHY: one call site per opener keeps the rule identical for statement, value, loop and handler
///      bodies, while the entry start (no enclosing opener) opts out by carrying `None`.
pub(crate) fn nested_scope_opener_error(
    token_stream: &AstCursor,
    context: &ScopeContext,
    opener: ScopeOpening,
) -> Option<DiagnosticDraft> {
    let enclosing = context.enclosing_opener?;
    nested_scope_opening_diagnostic(token_stream.source_tokens(), enclosing, opener)
}

/// Report a misplaced or duplicated scope closer at the `;` the cursor is on.
///
/// WHAT: classifies the closer against `opening` and the token that follows it.
/// WHY: every AST body that consumes a `;` as a closer must issue the same MOTH-SYNTAX-0041
///      reason exactly once, from the one classification the lexer's retained line breaks support.
pub(crate) fn scope_closer_layout_error(
    token_stream: &AstCursor,
    opening: ScopeOpening,
) -> Option<DiagnosticDraft> {
    let closer = token_stream.current()?;
    let tokens = token_stream.source_tokens();
    // An incomplete expression before the closer is earlier in source and belongs to the
    // expression parser, so the placement reason must stay silent. Trailing tokens still change
    // how the rest of the line parses, so their owner keeps reporting them.
    if closer_follows_incomplete_expression(tokens, closer.index()) {
        return closer_trailing_diagnostic(tokens, closer.index(), closer.source_span());
    }
    closer_layout_diagnostic(
        tokens,
        opening.site,
        opening.label_span,
        closer.index(),
        closer.source_span(),
    )
}

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
