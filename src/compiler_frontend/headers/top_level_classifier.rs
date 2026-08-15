//! Shallow top-level token classification for header file parsing.
//!
//! WHAT: classifies the already-read token at a file boundary into the next header-parser action.
//! WHY: declaration parsing, dependency parsing, and runtime-body validation have separate owners; this
//! module only answers which branch the per-file parser should try next.

use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::line_scanning::find_top_level_fat_arrow_on_line_in_tokens;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};

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

pub(super) fn classify_current_item(
    token_stream: &FileTokens,
    current_token: &Token,
) -> HeaderFileItem {
    classify_current_item_with_boundary(
        token_stream,
        current_token,
        current_item_started_at_statement_boundary(token_stream),
    )
}

/// Classify an item whose enclosing parser has already established a top-level boundary.
///
/// WHAT: lets the `export:` block parse its first item even when the author places it directly
/// after the block colon without a newline.
/// WHY: ordinary top-level classification must not treat every token after `:` as a new header,
/// because function and control-flow bodies use the same token boundary.
pub(super) fn classify_export_block_item(
    token_stream: &FileTokens,
    current_token: &Token,
) -> HeaderFileItem {
    classify_current_item_with_boundary(token_stream, current_token, true)
}

fn classify_current_item_with_boundary(
    token_stream: &FileTokens,
    current_token: &Token,
    at_statement_boundary: bool,
) -> HeaderFileItem {
    match current_token.kind {
        TokenKind::Symbol(name_id) if at_statement_boundary => HeaderFileItem::Symbol(name_id),

        TokenKind::Symbol(_) => HeaderFileItem::StartBodyToken,

        TokenKind::DatatypeInt
        | TokenKind::DatatypeFloat
        | TokenKind::DatatypeBool
        | TokenKind::DatatypeString
        | TokenKind::DatatypeChar
            if at_statement_boundary =>
        {
            if let Some(type_name) = builtin_conformance_target_name(&current_token.kind)
                && token_stream.current_token_kind() == &TokenKind::Must
            {
                return HeaderFileItem::BuiltinTypeConformanceTarget(type_name);
            }

            HeaderFileItem::StartBodyToken
        }

        TokenKind::Path(_) if at_statement_boundary => HeaderFileItem::Dependency,

        TokenKind::Export if at_statement_boundary => {
            if token_stream.current_token_kind() == &TokenKind::Colon {
                HeaderFileItem::ExportBlock
            } else {
                HeaderFileItem::Export
            }
        }

        TokenKind::Export => HeaderFileItem::StartBodyToken,

        TokenKind::Hash => HeaderFileItem::Hash {
            at_statement_boundary,
        },

        TokenKind::TemplateHead => HeaderFileItem::RuntimeTemplate,

        TokenKind::Must | TokenKind::TraitThis => HeaderFileItem::ReservedTraitSyntax,

        TokenKind::Eof => HeaderFileItem::Eof,

        _ => HeaderFileItem::StartBodyToken,
    }
}

fn builtin_conformance_target_name(token_kind: &TokenKind) -> Option<&'static str> {
    match token_kind {
        TokenKind::DatatypeInt => Some("Int"),
        TokenKind::DatatypeFloat => Some("Float"),
        TokenKind::DatatypeBool => Some("Bool"),
        TokenKind::DatatypeString => Some("String"),
        TokenKind::DatatypeChar => Some("Char"),
        _ => None,
    }
}

fn current_item_started_at_statement_boundary(token_stream: &FileTokens) -> bool {
    token_stream
        .tokens
        .get(token_stream.index.saturating_sub(2))
        .map(|previous_token| {
            matches!(
                previous_token.kind,
                TokenKind::ModuleStart | TokenKind::Newline | TokenKind::End
            )
        })
        .unwrap_or(true)
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

/// Classify the token sequence immediately after an already-read symbol name.
///
/// WHAT: peeks at the token following the symbol to determine whether it starts a
///       declaration, and if so, which kind.
/// WHY: one token-only classifier serves both duplicate-header detection and dependency-
///      clause continuation diagnostics.
pub(super) fn classify_symbol_statement_start(token_stream: &FileTokens) -> SymbolStatementStart {
    classify_symbol_statement_start_at(&token_stream.tokens, token_stream.index)
}

/// Classify a follower token in a raw token slice.
///
/// WHAT: the shared core used by `FileTokens` header classification and dependency-clause
///       continuation diagnostics.
/// WHY: the dependency parser cannot construct a `FileTokens` cursor just to ask whether a
///      name starts a statement.
pub(crate) fn classify_symbol_statement_start_at(
    tokens: &[Token],
    follower_index: usize,
) -> SymbolStatementStart {
    let Some(follower) = tokens.get(follower_index) else {
        return SymbolStatementStart::Other;
    };

    // Qualified match arms such as `Status::Ready => ...` are executable start-body
    // syntax, not a second top-level `Status :: ...` declaration.
    if follower.kind == TokenKind::DoubleColon
        && find_top_level_fat_arrow_on_line_in_tokens(tokens, follower_index).is_some()
    {
        return SymbolStatementStart::Other;
    }

    match &follower.kind {
        TokenKind::TypeParameterBracket | TokenKind::Type => SymbolStatementStart::Function,
        TokenKind::Assign => {
            if matches!(
                tokens.get(follower_index + 1).map(|token| &token.kind),
                Some(TokenKind::TypeParameterBracket)
            ) {
                SymbolStatementStart::Struct
            } else {
                SymbolStatementStart::ValueBinding
            }
        }
        TokenKind::DoubleColon => SymbolStatementStart::Choice,
        TokenKind::As => SymbolStatementStart::TypeAlias,
        TokenKind::Hash => SymbolStatementStart::CompileTimeBinding,
        TokenKind::Must => SymbolStatementStart::Trait,
        TokenKind::Of => {
            if starts_specialized_generic_conformance_at(tokens, follower_index) {
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

/// Detect whether a repeated top-level symbol is starting another header declaration.
/// Already in the context of parsing a variable name that exists in this scope.
///
/// WHAT: peeks at the token sequence immediately after an already-seen symbol name.
/// WHY: duplicate header declarations must fail during header parsing instead of being
///      misclassified as references inside the implicit start function.
pub(super) fn starts_duplicate_top_level_header_declaration(token_stream: &FileTokens) -> bool {
    classify_symbol_statement_start(token_stream).starts_header_declaration()
}

/// Detect whether the current `must` token starts a trait declaration rather than conformance.
///
/// WHY: repeated `Type must TRAIT` conformance declarations reuse the target type name and do not
/// shadow it, but repeated `TRAIT must:` declarations are ordinary duplicate headers.
pub(super) fn starts_trait_declaration_after_must(token_stream: &FileTokens) -> bool {
    token_stream.current_token_kind() == &TokenKind::Must
        && matches!(token_stream.peek_next_token(), Some(TokenKind::Colon))
}

/// Detect whether the current `must` token starts a trait incompatibility declaration.
///
/// WHY: repeated `TRAIT must not TRAIT` incompatibility declarations reuse the subject trait name
/// and must not shadow the original trait declaration.
pub(super) fn starts_specialized_generic_conformance_declaration(
    token_stream: &FileTokens,
) -> bool {
    starts_specialized_generic_conformance_at(&token_stream.tokens, token_stream.index)
}

#[cfg(test)]
#[path = "tests/top_level_classifier_tests.rs"]
mod top_level_classifier_tests;

fn starts_specialized_generic_conformance_at(tokens: &[Token], start_index: usize) -> bool {
    if !matches!(
        tokens.get(start_index).map(|token| &token.kind),
        Some(TokenKind::Of)
    ) {
        return false;
    }

    let mut index = start_index;
    while let Some(token) = tokens.get(index) {
        match token.kind {
            TokenKind::Must => return true,
            TokenKind::Newline | TokenKind::End | TokenKind::Eof => return false,
            _ => index += 1,
        }
    }

    false
}
