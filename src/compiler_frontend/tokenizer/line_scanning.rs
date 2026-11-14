//! Token-line structural scanning helpers.
//!
//! WHAT: exposes the tag-based fat-arrow scan over short-lived indexed token facts, and the
//!       physical-line facts that scope closers use.
//! WHY: dependency-adjacent classifiers only need their line; per-query whole-source
//!      projection pays allocation for untouched tokens.

use crate::compiler_frontend::compiler_messages::SourceScopeConstruct;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenIndex, TokenTag};
use crate::compiler_frontend::utilities::token_scan::{NestingDepth, TokenFactView};

/// Tag-based fat-arrow scan over short-lived indexed facts.
///
/// WHAT: scans one physical line from `start_index` without collecting the source.
/// WHY: dependency-adjacent classifiers only need their line; per-query whole-source
///      projection pays allocation for untouched tokens.
pub(crate) fn find_top_level_fat_arrow_on_line_in_scanned_tokens(
    tokens: TokenFactView<'_>,
    start_index: usize,
) -> Option<usize> {
    find_top_level_fat_arrow_on_line_in_view(tokens, start_index)
}

fn find_top_level_fat_arrow_on_line_in_view(
    tokens: TokenFactView<'_>,
    start_index: usize,
) -> Option<usize> {
    let mut nesting_depth = NestingDepth::default();
    for index in start_index..tokens.len() {
        let token = tokens.get(index)?;
        if token.tag == TokenTag::NEWLINE
            || token.tag == TokenTag::END
            || token.tag == TokenTag::EOF
        {
            break;
        }
        if nesting_depth.is_top_level() && token.tag == TokenTag::FAT_ARROW {
            return Some(index);
        }
        nesting_depth.step_tag(token.tag);
    }
    None
}

/// One authored scope opener as the layout contract addresses it.
///
/// WHAT: the canonical token whose physical line the opener occupies, plus the span that names the
///       construct in a diagnostic label.
/// WHY: a closer and a nested opener are judged against the last token that began the scope's
///      line, which for an `if` branch is the `else` introducing the terminal body while the label
///      must still point at the `:` the author wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScopeOpening {
    /// The canonical token whose physical line the opener owns.
    pub(crate) site: TokenIndex,
    /// The canonical colon that opened the construct, before any branch continuation moves
    /// `site` to `else` or `=>`.
    pub(crate) opener: TokenIndex,
    /// The span that labels the construct that opened.
    pub(crate) label_span: SourceSpan,
    /// The construct this opener began, named by an end-of-file report.
    pub(crate) construct: SourceScopeConstruct,
}

impl ScopeOpening {
    /// The same labelled opener reached through a later token on its opening line.
    ///
    /// WHAT: replaces the effective site with a branch continuation such as `else`.
    /// WHY: `if`/`else` and match terminal arms close on the line of their terminal body, but the
    ///      label must still name the colon that opened the construct.
    pub(crate) fn at_site(self, site: TokenIndex) -> Self {
        Self { site, ..self }
    }
}

/// How a scope closer sits on its physical line.
///
/// WHAT: classifies one authored `;` against its opening site and its previous token.
/// WHY: functions, choices, traits and executable blocks all close under the same layout
///      contract. One classifier keeps every stage from re-deriving the rule, while the typed
///      variant lets each owner emit its own family diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloserPlacement {
    /// The closer shares its physical line with the scope's opening site.
    Inline,
    /// The closer begins its own physical line.
    Standalone,
    /// Another `;` closes a scope directly before this one on the same line.
    AdditionalCloser { previous_closer: SourceSpan },
    /// The closer's line holds authored code after the opening site but before the closer.
    AfterCode,
}

/// What follows a scope closer on its physical line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloserTrailing {
    /// Only whitespace or a comment follows, or the line ends at the closer.
    LineEnd,
    /// Another `;` follows the closer on the same physical line.
    AdditionalCloser { closer: SourceSpan },
    /// Authored code follows the closer on the same physical line.
    Content { first: SourceSpan },
}

/// Whether a physical line break lies strictly between two canonical source tokens.
///
/// WHAT: walks backward from `later` to `earlier`, stopping at the first `NEWLINE` token or at a
///       token whose owned source text crosses a line.
/// WHY: the retained token stream already records every authored line break except those inside
///       quoted strings, raw strings, template-body text and discarded template bodies. The
///       lexer flags the token that owns each of those runs, so this walk is exact without a
///       source scan or a second line table. It is bounded by the later token's own physical
///       line whenever the answer is needed for a misplaced multiline closer, and by the
///       scope's opening site otherwise.
pub(crate) fn has_physical_line_break_between(
    tokens: &SourceTokens,
    earlier: TokenIndex,
    later: TokenIndex,
) -> bool {
    let mut index = later.index();
    while let Some(previous) = index.checked_sub(1) {
        if previous <= earlier.index() {
            return false;
        }
        let Some(previous_index) = TokenIndex::try_from_index(previous) else {
            return false;
        };
        let Ok(token) = tokens.token(previous_index) else {
            return false;
        };
        let shape = token.shape();
        if shape.tag() == TokenTag::NEWLINE || shape.contains_line_break() {
            return true;
        }
        index = previous;
    }
    false
}

/// Whether the meaningful token before `closer` cannot end an expression.
///
/// WHAT: walks back over `NEWLINE` trivia to the last authored token and reports when its grammar
///       still requires expression input after it.
/// WHY: a `;` written directly after such a token is not the first structural error. The
///      incomplete expression is earlier in source and the expression parser owns its report, so
///      a closer-layout diagnostic must stay silent and let that owner run. The tokenizer schema
///      owns which tokens require a following operand; delimiters and headers do not.
pub(crate) fn closer_follows_incomplete_expression(
    tokens: &SourceTokens,
    closer: TokenIndex,
) -> bool {
    let mut index = closer.index();
    while let Some(previous) = index.checked_sub(1) {
        let Some(previous_index) = TokenIndex::try_from_index(previous) else {
            return false;
        };
        let Ok(token) = tokens.token(previous_index) else {
            return false;
        };
        let tag = token.shape().tag();
        if tag == TokenTag::NEWLINE {
            index = previous;
            continue;
        }
        return tag.requires_following_operand();
    }
    false
}

/// Build the layout diagnostic for a second scope opener that shares an enclosing opener's line.
///
/// WHAT: reports `NestedScopeOpenerOnLine` at `opener` and labels `enclosing` when no physical line
///       break separates them.
/// WHY: the layout contract allows at most one semicolon-terminated scope per physical line, and
///       every opener owner makes that comparison against the same retained newline evidence.
pub(crate) fn nested_scope_opening_diagnostic(
    tokens: &SourceTokens,
    enclosing: ScopeOpening,
    opener: ScopeOpening,
) -> Option<DiagnosticDraft> {
    use crate::compiler_frontend::compiler_messages::InvalidScopeLayoutReason;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
    use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;

    if has_physical_line_break_between(tokens, enclosing.site, opener.site) {
        return None;
    }

    Some(
        token_lexical::invalid_scope_layout(
            InvalidScopeLayoutReason::NestedScopeOpenerOnLine,
            Some(opener.label_span),
        )
        .with_label(
            Some(enclosing.label_span),
            Some(LabelDraftMessage::EarlierScopeOpenerOnLine),
        ),
    )
}

/// Classify the closer at `closer` against the opening site at `opening`.
///
/// WHAT: a closer that directly follows another `;` is an additional closer; a closer directly
///       after a `NEWLINE` stands alone; otherwise the closer is inline only when its opening
///       site shares its physical line.
/// WHY: "only whitespace before a standalone closer" is exactly "the previous token is
///       `NEWLINE`", and a second closer on the same line is always invalid. Keeping both rules
///       here means the Stage 2 declaration owners and the AST statement owners classify the
///       same token the same way.
///
/// `None` means the closer had no canonical predecessor, which cannot happen for authored source.
pub(crate) fn closer_placement(
    tokens: &SourceTokens,
    opening: TokenIndex,
    closer: TokenIndex,
) -> Option<CloserPlacement> {
    let previous_index = TokenIndex::try_from_index(closer.index().checked_sub(1)?)?;
    let previous = tokens.token(previous_index).ok()?;
    if previous.tag() == TokenTag::END {
        return Some(CloserPlacement::AdditionalCloser {
            previous_closer: previous.source_span(),
        });
    }
    if previous.tag() == TokenTag::NEWLINE {
        return Some(CloserPlacement::Standalone);
    }
    if has_physical_line_break_between(tokens, opening, closer) {
        Some(CloserPlacement::AfterCode)
    } else {
        Some(CloserPlacement::Inline)
    }
}

/// Classify the token that follows the closer at `closer`.
///
/// WHAT: only `NEWLINE`, `EOF` or the end of the token store are legal line endings; another `;`
///       is an additional closer and any other token is authored content.
/// WHY: the trailing half of the scope layout contract is the same for every closer owner, and a
///       comment emits no token, so `NEWLINE` already means "whitespace or a comment follows".
pub(crate) fn closer_trailing(tokens: &SourceTokens, closer: TokenIndex) -> CloserTrailing {
    let Some(next_index) = closer
        .index()
        .checked_add(1)
        .and_then(TokenIndex::try_from_index)
    else {
        return CloserTrailing::LineEnd;
    };
    let Ok(next) = tokens.token(next_index) else {
        return CloserTrailing::LineEnd;
    };
    match next.tag() {
        TokenTag::NEWLINE | TokenTag::EOF => CloserTrailing::LineEnd,
        TokenTag::END => CloserTrailing::AdditionalCloser {
            closer: next.source_span(),
        },
        _ => CloserTrailing::Content {
            first: next.source_span(),
        },
    }
}

/// Build the layout diagnostic for one scope closer, if its line placement is invalid.
///
/// WHAT: maps the closer's placement and trailing tokens to one typed MOTH-SYNTAX-0041 reason
///       with the role labels that explain it.
/// WHY: every stage that consumes a scope closer must issue the same family for the same authored
///       shape exactly once. Keeping the mapping beside the line evidence stops each owner from
///       rebuilding the rule and its labels.
///
/// `opening_span` labels the active scope opener for a misplaced multiline closer.
pub(crate) fn closer_layout_diagnostic(
    tokens: &SourceTokens,
    opening: TokenIndex,
    opening_span: SourceSpan,
    closer: TokenIndex,
    closer_span: SourceSpan,
) -> Option<DiagnosticDraft> {
    let placement = closer_placement(tokens, opening, closer);
    closer_layout_diagnostic_for_placement(tokens, placement, opening_span, closer, closer_span)
}

/// Build the trailing half of a scope closer's layout diagnostic.
///
/// WHAT: reports an additional closer or authored content after `closer`, ignoring placement.
/// WHY: an incomplete expression before the closer suppresses the placement reason because the
///      expression parser owns that earlier report, but trailing tokens change how the rest of
///      the line parses. The closer's owner must still report them.
pub(crate) fn closer_trailing_diagnostic(
    tokens: &SourceTokens,
    closer: TokenIndex,
    closer_span: SourceSpan,
) -> Option<DiagnosticDraft> {
    use crate::compiler_frontend::compiler_messages::InvalidScopeLayoutReason;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
    use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;

    match closer_trailing(tokens, closer) {
        CloserTrailing::LineEnd => None,
        CloserTrailing::AdditionalCloser {
            closer: extra_closer,
        } => Some(
            token_lexical::invalid_scope_layout(
                InvalidScopeLayoutReason::AdditionalCloserOnLine,
                Some(extra_closer),
            )
            .with_label(
                Some(closer_span),
                Some(LabelDraftMessage::ScopeCloserBeforeThis),
            ),
        ),
        CloserTrailing::Content { first } => Some(
            token_lexical::invalid_scope_layout(
                InvalidScopeLayoutReason::ContentAfterCloser,
                Some(first),
            )
            .with_label(
                Some(closer_span),
                Some(LabelDraftMessage::ScopeCloserBeforeThis),
            ),
        ),
    }
}

/// Build the layout diagnostic for an already classified closer.
///
/// WHAT: maps one placement and the closer's trailing token to the same typed draft as
///       [`closer_layout_diagnostic`].
/// WHY: capture classifies its function closer once to compare it with the last inner closer,
///      then reuses that fact instead of walking the same physical line twice.
pub(crate) fn closer_layout_diagnostic_for_placement(
    tokens: &SourceTokens,
    placement: Option<CloserPlacement>,
    opening_span: SourceSpan,
    closer: TokenIndex,
    closer_span: SourceSpan,
) -> Option<DiagnosticDraft> {
    use crate::compiler_frontend::compiler_messages::InvalidScopeLayoutReason;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
    use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;

    match placement {
        Some(CloserPlacement::AdditionalCloser { previous_closer }) => {
            return Some(
                token_lexical::invalid_scope_layout(
                    InvalidScopeLayoutReason::AdditionalCloserOnLine,
                    Some(closer_span),
                )
                .with_label(
                    Some(previous_closer),
                    Some(LabelDraftMessage::ScopeCloserBeforeThis),
                ),
            );
        }
        Some(CloserPlacement::AfterCode) => {
            return Some(
                token_lexical::invalid_scope_layout(
                    InvalidScopeLayoutReason::MultilineCloserAfterCode,
                    Some(closer_span),
                )
                .with_label(
                    Some(opening_span),
                    Some(LabelDraftMessage::ScopeOpenerStartsHere),
                ),
            );
        }
        Some(CloserPlacement::Inline | CloserPlacement::Standalone) | None => {}
    }

    closer_trailing_diagnostic(tokens, closer, closer_span)
}

#[cfg(test)]
#[path = "tests/line_scanning_tests.rs"]
mod line_scanning_tests;
