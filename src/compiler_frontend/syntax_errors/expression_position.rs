//! Common language-mismatch mistakes in expression position.
//!
//! WHAT: Detects patterns like `==`, `!=`, `&&`, `||`, `!expr`, `&expr` that
//! users from C-family languages write when they first encounter Moth.
//!
//! WHY: Once neighbouring operator tokens are proven adjacent in the source these are
//! unambiguous syntax errors. Catching them early with specific guidance prevents confusion before
//! generic "invalid token" messages waste the user's time.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::CommonSyntaxMistakeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::tokenizer::tokens::{TokenRef, TokenTag};

/// Detect a C-family operator beginning at the parser-coordinate token `start`.
///
/// WHAT: recognises `==`, `===`, `!=`, `&&` and `||`, and spans the whole authored lexeme.
/// WHY: several named-field and named-argument parsers consume the first `=` before expression
/// parsing sees the second one. Keeping the recogniser here gives every caller the same answer and
/// the same highlighted operator.
///
/// Tags alone cannot prove one lexeme, because `= =` and `! =` carry the same tag sequence. The
/// repair is reported only when every token of the run is adjacent in the source, so the span
/// covers the authored operator and never the bytes between two separate tokens.
pub(crate) fn c_family_operator_mistake_at(
    token_stream: &AstCursor,
    start: usize,
) -> Option<DiagnosticDraft> {
    let mut tags = [None; 4];
    let mut tokens = [None; 4];
    for offset in 0..tags.len() {
        let Some(token) = token_stream.token_ref_at(start.saturating_add(offset)) else {
            break;
        };
        tags[offset] = Some(token.tag());
        tokens[offset] = Some(token);
    }

    let (reason, authored_length) = classify_c_family_operator(&tags)?;
    let span = contiguous_operator_span(tokens.get(..authored_length)?)?;
    Some(token_lexical::common_syntax_mistake(reason, Some(span)))
}

fn classify_c_family_operator(
    tags: &[Option<TokenTag>; 4],
) -> Option<(CommonSyntaxMistakeReason, usize)> {
    match tags[0]? {
        TokenTag::ASSIGN => {
            let run_length = tags
                .iter()
                .flatten()
                .take_while(|tag| **tag == TokenTag::ASSIGN)
                .count();
            (run_length >= 2).then_some((CommonSyntaxMistakeReason::EqualityOperator, run_length))
        }
        TokenTag::BANG if tags[1] == Some(TokenTag::ASSIGN) => {
            Some((CommonSyntaxMistakeReason::InequalityOperator, 2))
        }
        TokenTag::AMPERSAND if tags[1] == Some(TokenTag::AMPERSAND) => {
            Some((CommonSyntaxMistakeReason::LogicalAndOperator, 2))
        }
        TokenTag::TYPE_PARAMETER_BRACKET if tags[1] == Some(TokenTag::TYPE_PARAMETER_BRACKET) => {
            Some((CommonSyntaxMistakeReason::LogicalOrOperator, 2))
        }
        _ => None,
    }
}

/// The authored span of a run of operator tokens written with no bytes between them.
///
/// WHAT: resolves each token's own byte range and accepts the run only when every token belongs to
/// that source and each range starts exactly where the previous one ends.
/// WHY: tags cannot prove that the user wrote `==` rather than `= =`, and a literal byte count
/// could underline the whitespace between two separate tokens. A token whose span needs the
/// source's extended table also fails the check, because an unresolvable range cannot prove
/// adjacency and the caller must keep the ordinary syntax diagnostic instead.
fn contiguous_operator_span(tokens: &[Option<TokenRef<'_>>]) -> Option<SourceSpan> {
    let mut tokens = tokens.iter();
    let first = (*tokens.next()?)?;
    let first_range = first.span().inline_range()?;
    let start = first_range.start();
    let mut end = first_range.end();

    for token in tokens {
        let token = (*token)?;
        if token.source() != first.source() {
            return None;
        }

        let range = token.span().inline_range()?;
        if range.start() != end {
            return None;
        }
        end = range.end();
    }

    let length = end.checked_sub(start)?;
    LocalSpan::exact_inline(start, length).map(|local| SourceSpan::new(first.source(), local))
}

/// Report an equality whose first `=` a separator parser is about to consume.
///
/// Declaration defaults, typed declarations and assignment targets own the first `=` as
/// punctuation. Checking before they advance keeps the whole authored operator visible instead of
/// leaving expression parsing to report a lone `=` at the second character.
///
/// The tokens must be adjacent in the source, so `name = = value` keeps the ordinary diagnostic
/// rather than claiming the user wrote `==`.
pub(crate) fn separator_equality_mistake(
    first: Option<TokenRef<'_>>,
    second: Option<TokenRef<'_>>,
    third: Option<TokenRef<'_>>,
) -> Option<DiagnosticDraft> {
    let first = first?;
    let second = second?;
    if first.tag() != TokenTag::ASSIGN || second.tag() != TokenTag::ASSIGN {
        return None;
    }

    let third = third.filter(|token| token.tag() == TokenTag::ASSIGN);
    let authored_length = if third.is_some() { 3 } else { 2 };
    let span = contiguous_operator_span(&[Some(first), Some(second), third][..authored_length])?;
    Some(token_lexical::common_syntax_mistake(
        CommonSyntaxMistakeReason::EqualityOperator,
        Some(span),
    ))
}

/// Find a C-family operator in the small window a statement or field parser is inspecting.
///
/// WHAT: tries the current parser-coordinate position and the two positions after it.
/// WHY: segmented cursors and already-consumed separators disagree with the canonical token index
/// by one in different contexts. Scanning the local window keeps every caller on one recogniser.
pub(crate) fn c_family_operator_mistake_near(token_stream: &AstCursor) -> Option<DiagnosticDraft> {
    let position = token_stream.position();
    (0..=2).find_map(|offset| {
        c_family_operator_mistake_at(token_stream, position.saturating_add(offset))
    })
}

/// Check for common expression-position mistakes before falling back to a generic error.
///
/// WHAT: inspects the current token (and sometimes the next token) for patterns
/// that are valid in other languages but not in Moth.
///
/// Returns `Some(diagnostic)` when a known mistake is detected, `None` otherwise.
pub(crate) fn check_expression_common_mistake(
    token_stream: &AstCursor,
    expression_is_empty: bool,
) -> Option<DiagnosticDraft> {
    let current = token_stream.current_tag();
    let location = token_stream.current_span();

    if let Some(diagnostic) = c_family_operator_mistake_at(token_stream, token_stream.position()) {
        return Some(diagnostic);
    }

    match current {
        // `!` used as boolean negation (not fallible handling)
        // Fallible handling `!` is parsed as a postfix suffix after the primary expression,
        // so encountering `Bang` at the start of an operand or after an operator means
        // the user is trying to use it as unary negation.
        TokenTag::BANG if expression_is_empty => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::BooleanBangNegation,
            Some(location),
        )),

        // Single `=` in expression position where it is not valid.
        // `=` is only valid in declarations and assignments (statement position).
        TokenTag::ASSIGN => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::ExpressionAssignment,
            Some(location),
        )),

        // Single `&` in expression position (likely Rust borrow attempt)
        TokenTag::AMPERSAND if expression_is_empty => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::RustBorrowPrefix,
            Some(location),
        )),

        // `as` outside its three supported domains
        TokenTag::AS => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::InvalidAsOperator,
            Some(location),
        )),

        _ => None,
    }
}
