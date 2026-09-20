//! Shallow top-level token classification for header file parsing.
//!
//! WHAT: classifies the already-read token at a file boundary into the next header-parser action.
//! WHY: declaration parsing, dependency parsing, and runtime-body validation have separate owners; this
//! module only answers which branch the per-file parser should try next.

use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::line_scanning::find_top_level_fat_arrow_on_line_in_scanned_tokens;
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenIndex, TokenRef, TokenTag};
use crate::compiler_frontend::utilities::token_scan::TokenFactView;

pub(super) enum HeaderFileItem {
    Symbol(StringId),
    BuiltinTypeConformanceTarget(&'static str),
    Dependency,
    Export,
    ExportBlock,
    Hash { at_statement_boundary: bool },
    RuntimeTemplate,
    ReservedTraitSyntax,
    Eof,
    StartBodyToken,
}

/// Shared tag-based header-item core for source-view callers.
///
/// WHAT: maps one already-read top-level token to its header-parser action without
///       cloning cold payloads.
/// WHY: the outer file walk classifies through canonical shapes/spans so downstream
///      declaration parsers share one tag-based entry point.
pub(super) fn classify_tagged_item(
    current_tag: TokenTag,
    current_symbol: Option<StringId>,
    follower_tag: Option<TokenTag>,
    at_statement_boundary: bool,
) -> HeaderFileItem {
    if current_tag == TokenTag::SYMBOL {
        if at_statement_boundary && let Some(name_id) = current_symbol {
            return HeaderFileItem::Symbol(name_id);
        }
        return HeaderFileItem::StartBodyToken;
    }

    if (current_tag == TokenTag::DATATYPE_INT
        || current_tag == TokenTag::DATATYPE_FLOAT
        || current_tag == TokenTag::DATATYPE_BOOL
        || current_tag == TokenTag::DATATYPE_STRING
        || current_tag == TokenTag::DATATYPE_CHAR)
        && at_statement_boundary
    {
        if let Some(type_name) = builtin_conformance_target_name_for_tag(current_tag)
            && follower_tag == Some(TokenTag::MUST)
        {
            return HeaderFileItem::BuiltinTypeConformanceTarget(type_name);
        }

        return HeaderFileItem::StartBodyToken;
    }

    if current_tag == TokenTag::PATH && at_statement_boundary {
        return HeaderFileItem::Dependency;
    }

    if current_tag == TokenTag::EXPORT {
        if at_statement_boundary {
            if follower_tag == Some(TokenTag::COLON) {
                return HeaderFileItem::ExportBlock;
            }
            return HeaderFileItem::Export;
        }
        return HeaderFileItem::StartBodyToken;
    }

    if current_tag == TokenTag::HASH {
        return HeaderFileItem::Hash {
            at_statement_boundary,
        };
    }

    if current_tag == TokenTag::TEMPLATE_HEAD {
        if at_statement_boundary {
            return HeaderFileItem::RuntimeTemplate;
        }
        return HeaderFileItem::StartBodyToken;
    }

    if current_tag == TokenTag::MUST || current_tag == TokenTag::TRAIT_THIS {
        return HeaderFileItem::ReservedTraitSyntax;
    }

    if current_tag == TokenTag::EOF {
        return HeaderFileItem::Eof;
    }

    HeaderFileItem::StartBodyToken
}

/// Classify one already-read token through short-lived source views.
///
/// WHAT: tag-based entry point for the outer file walk; borrows the current
///       source token without cloning cold payloads and retains no durable reference.
/// WHY: branch classification observes canonical shapes/spans through `SourceTokens`.
pub(super) fn classify_current_item_ref(
    current: TokenRef<'_>,
    follower: Option<TokenRef<'_>>,
    at_boundary: bool,
) -> HeaderFileItem {
    classify_tagged_item(
        current.tag(),
        current.string_id(),
        follower.map(|token| token.tag()),
        at_boundary,
    )
}

/// Export-block classification with its established top-level boundary.
///
/// WHAT: lets the `export:` block parse its first item even when placed directly
///       after the block colon without a newline.
/// WHY: shares `classify_tagged_item` with ordinary source-view classification without a second
///       classification table.
pub(super) fn classify_export_block_item_ref(
    current: TokenRef<'_>,
    follower: Option<TokenRef<'_>>,
) -> HeaderFileItem {
    classify_tagged_item(
        current.tag(),
        current.string_id(),
        follower.map(|token| token.tag()),
        true,
    )
}

/// Statement-boundary test for one source-owned token index.
///
/// WHAT: reports whether the token before `current_index` is a file boundary
///       (`ModuleStart`/`Newline`/`End`) or absent at the start of the source.
/// WHY: the outer walk keeps its source-token index while reading boundary
///      facts from canonical shapes/spans.
pub(super) fn statement_boundary_at_source(canonical: &SourceTokens, current_index: usize) -> bool {
    let Some(previous) = current_index.checked_sub(1) else {
        return true;
    };
    let Some(previous_index) = TokenIndex::try_from_index(previous) else {
        return true;
    };
    let Ok(previous_token) = canonical.token(previous_index) else {
        return true;
    };
    let tag = previous_token.tag();
    tag == TokenTag::MODULE_START || tag == TokenTag::NEWLINE || tag == TokenTag::END
}

fn builtin_conformance_target_name_for_tag(tag: TokenTag) -> Option<&'static str> {
    match tag {
        _ if tag == TokenTag::DATATYPE_INT => Some("Int"),
        _ if tag == TokenTag::DATATYPE_FLOAT => Some("Float"),
        _ if tag == TokenTag::DATATYPE_BOOL => Some("Bool"),
        _ if tag == TokenTag::DATATYPE_STRING => Some("String"),
        _ if tag == TokenTag::DATATYPE_CHAR => Some("Char"),
        _ => None,
    }
}

/// Classification of the token following a top-level symbol name.
///
/// WHAT: a shallow token-only classifier that identifies whether a declaration-start or
///       ordinary continuation follows an already-read symbol token.
/// WHY: duplicate-header detection and dependency-clause continuation diagnostics both need
///      to know whether a symbol starts a declaration. Keeping one classifier prevents a
///      second hard-coded token list in the dependency parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SymbolStatementStart {
    /// `name = ...` or `name = |...|` (struct).
    ValueBinding,
    /// `name #= ...` or `name #Type = ...`.
    CompileTimeBinding,
    /// `name |...|` or `name type T |...|`.
    Function,
    /// `name = |...|`.
    Struct,
    /// `name :: ...`.
    Choice,
    /// `name as ...`.
    TypeAlias,
    /// `name must:` or `name must TRAIT`.
    Trait,
    /// `Name of T must TRAIT` (specialized generic conformance).
    SpecializedGenericConformance,
    /// Not a declaration start.
    Other,
}

/// Classify a follower token in canonical source storage.
///
/// WHAT: reads shapes/spans on demand for canonical source callers.
/// WHY: keeps the same offsets without a second implementation or retained references.
pub(crate) fn classify_symbol_statement_start_at_source(
    tokens: &crate::compiler_frontend::tokenizer::tokens::SourceTokens,
    follower_index: usize,
) -> SymbolStatementStart {
    classify_symbol_statement_start_at_scanned(TokenFactView::from_source(tokens), follower_index)
}

/// Tag-based follower classification over short-lived indexed facts.
///
/// WHAT: classifies the token after an already-read symbol over short-lived indexed
///       facts, preserving match-arm/choice disambiguation and declaration kinds.
/// WHY: indexed views read only the follower and its bounded lookahead instead of
///      retaining a second indexed copy per query.
pub(crate) fn classify_symbol_statement_start_at_scanned(
    tokens: TokenFactView<'_>,
    follower_index: usize,
) -> SymbolStatementStart {
    let Some(follower) = tokens.get(follower_index) else {
        return SymbolStatementStart::Other;
    };

    if follower.tag == TokenTag::DOUBLE_COLON
        && find_top_level_fat_arrow_on_line_in_scanned_tokens(tokens, follower_index).is_some()
    {
        return SymbolStatementStart::Other;
    }

    match follower.tag {
        TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::TYPE => SymbolStatementStart::Function,
        TokenTag::ASSIGN => {
            if follower_index
                .checked_add(1)
                .and_then(|index| tokens.get(index))
                .is_some_and(|token| token.tag == TokenTag::TYPE_PARAMETER_BRACKET)
            {
                SymbolStatementStart::Struct
            } else {
                SymbolStatementStart::ValueBinding
            }
        }
        TokenTag::DOUBLE_COLON => SymbolStatementStart::Choice,
        TokenTag::AS => SymbolStatementStart::TypeAlias,
        TokenTag::HASH => SymbolStatementStart::CompileTimeBinding,
        TokenTag::MUST => SymbolStatementStart::Trait,
        TokenTag::OF => {
            if starts_specialized_generic_conformance_at_scanned(tokens, follower_index) {
                SymbolStatementStart::SpecializedGenericConformance
            } else {
                SymbolStatementStart::Other
            }
        }
        _ => SymbolStatementStart::Other,
    }
}

impl SymbolStatementStart {
    /// A comma-continued dependency name that starts any following statement.
    pub(crate) fn starts_statement_after_dependency_selection(self) -> bool {
        !matches!(self, Self::Other)
    }

    /// A follower that starts an actual header declaration, not a runtime binding.
    pub(super) fn starts_header_declaration(self) -> bool {
        matches!(
            self,
            Self::CompileTimeBinding
                | Self::Function
                | Self::Struct
                | Self::Choice
                | Self::TypeAlias
                | Self::Trait
                | Self::SpecializedGenericConformance
        )
    }
}

/// Detect whether a repeated top-level symbol is starting another header declaration through the
/// canonical source owner.
///
/// WHAT: reuses the indexed tag classifier over a checked source-local follower index.
/// WHY: duplicate-header lookahead reads bounded `SourceTokens` facts without cloning.
pub(super) fn starts_duplicate_top_level_header_declaration_at_source(
    tokens: &SourceTokens,
    follower_index: TokenIndex,
) -> bool {
    classify_symbol_statement_start_at_source(tokens, follower_index.index())
        .starts_header_declaration()
}

/// Detect whether the current canonical `must` token starts a trait declaration rather than
/// conformance.
///
/// The shared scanned body keeps source and bounded fact-view callers on the same lookahead.
pub(super) fn starts_trait_declaration_after_must_at_source(
    tokens: &SourceTokens,
    current_index: TokenIndex,
) -> bool {
    starts_trait_declaration_after_must_scanned(
        TokenFactView::from_source(tokens),
        current_index.index(),
    )
}

fn starts_trait_declaration_after_must_scanned(
    tokens: TokenFactView<'_>,
    current_index: usize,
) -> bool {
    let Some(current) = tokens.get(current_index) else {
        return false;
    };
    current.tag == TokenTag::MUST
        && current_index
            .checked_add(1)
            .and_then(|index| tokens.get(index))
            .is_some_and(|token| token.tag == TokenTag::COLON)
}

/// Detect whether the canonical follower starts specialized generic conformance.
///
/// The implementation shares the scanned lookahead core with all remaining source and bounded
/// fact-view callers.
pub(super) fn starts_specialized_generic_conformance_declaration_at_source(
    tokens: &SourceTokens,
    start_index: TokenIndex,
) -> bool {
    starts_specialized_generic_conformance_at_scanned(
        TokenFactView::from_source(tokens),
        start_index.index(),
    )
}

#[cfg(test)]
#[path = "tests/top_level_classifier_tests.rs"]
mod top_level_classifier_tests;

fn starts_specialized_generic_conformance_at_scanned(
    tokens: TokenFactView<'_>,
    start_index: usize,
) -> bool {
    if tokens
        .get(start_index)
        .is_none_or(|token| token.tag != TokenTag::OF)
    {
        return false;
    }

    let mut index = start_index;
    while let Some(token) = tokens.get(index) {
        match token.tag {
            TokenTag::MUST => return true,
            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => return false,
            _ => {
                let Some(next) = index.checked_add(1) else {
                    return false;
                };
                index = next;
            }
        }
    }

    false
}
