//! Frontend keyword and identifier policy.
//!
//! WHAT: owns the exact keyword-to-token mapping used by lexing and the identifier
//! validation helpers shared with path and dependency parsing. The same direct match also
//! supplies the neutral presentation classes consumed by the HTML code highlighter.
//! WHY: keyword policy is user-visible and must not drift between the tokenizer,
//! dependency alias validation, reserved-name diagnostics and code highlighting.

use crate::compiler_frontend::tokenizer::tokens::TokenKind;

/// Keywords that may not be shadowed by identifiers after case folding and
/// stripping leading underscores.
pub(crate) const RESERVED_KEYWORD_SHADOWS: [&str; 36] = [
    "export", "if", "return", "yield", "else", "checked", "async", "cast", "as", "copy", "type",
    "of", "must", "this", "catch", "then", "loop", "to", "by", "break", "continue", "is", "not",
    "and", "or", "true", "false", "none", "fn", "float", "int", "string", "bool", "char", "assert",
    "config",
];

/// Neutral presentation class for an exact Moth source word.
///
/// WHAT: shared by the tokenizer and the HTML code highlighter so one direct
/// match owns both the token identity and the general word category.
/// WHY: the highlighter must never maintain a second current Moth word list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceWordClass {
    Keyword,
    WordOperator,
    Literal,
    BuiltinType,
}

/// Exact source-word classification result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClassifiedSourceWord {
    pub(crate) token_kind: TokenKind,
    pub(crate) class: SourceWordClass,
}

impl ClassifiedSourceWord {
    fn keyword(token_kind: TokenKind) -> Self {
        Self {
            token_kind,
            class: SourceWordClass::Keyword,
        }
    }

    fn word_operator(token_kind: TokenKind) -> Self {
        Self {
            token_kind,
            class: SourceWordClass::WordOperator,
        }
    }

    fn literal(token_kind: TokenKind) -> Self {
        Self {
            token_kind,
            class: SourceWordClass::Literal,
        }
    }

    fn builtin_type(token_kind: TokenKind) -> Self {
        Self {
            token_kind,
            class: SourceWordClass::BuiltinType,
        }
    }
}

/// Returns the tokenizer token kind and neutral presentation class for an exact
/// source keyword spelling, or `None` for ordinary identifiers.
pub(crate) fn classify_source_word(text: &str) -> Option<ClassifiedSourceWord> {
    match text {
        // Module and declaration keywords.
        "export" => Some(ClassifiedSourceWord::keyword(TokenKind::Export)),
        "type" => Some(ClassifiedSourceWord::keyword(TokenKind::Type)),
        "of" => Some(ClassifiedSourceWord::keyword(TokenKind::Of)),
        "as" => Some(ClassifiedSourceWord::keyword(TokenKind::As)),
        "copy" => Some(ClassifiedSourceWord::keyword(TokenKind::Copy)),

        // Control flow, blocks and casts.
        "if" => Some(ClassifiedSourceWord::keyword(TokenKind::If)),
        "return" => Some(ClassifiedSourceWord::keyword(TokenKind::Return)),
        "catch" => Some(ClassifiedSourceWord::keyword(TokenKind::Catch)),
        "then" => Some(ClassifiedSourceWord::keyword(TokenKind::Then)),
        "else" => Some(ClassifiedSourceWord::keyword(TokenKind::Else)),
        "checked" => Some(ClassifiedSourceWord::keyword(TokenKind::Checked)),
        "cast" => Some(ClassifiedSourceWord::keyword(TokenKind::Cast)),
        "break" => Some(ClassifiedSourceWord::keyword(TokenKind::Break)),
        "continue" => Some(ClassifiedSourceWord::keyword(TokenKind::Continue)),

        // Reserved receiver, trait and assertion syntax.
        "must" => Some(ClassifiedSourceWord::keyword(TokenKind::Must)),
        "this" => Some(ClassifiedSourceWord::keyword(TokenKind::This)),
        "This" => Some(ClassifiedSourceWord::keyword(TokenKind::TraitThis)),
        "assert" => Some(ClassifiedSourceWord::keyword(TokenKind::Assert)),

        // Deferred async syntax uses the ordinary keyword class.
        "async" => Some(ClassifiedSourceWord::keyword(TokenKind::Async)),
        "yield" => Some(ClassifiedSourceWord::keyword(TokenKind::Yield)),

        // Loops.
        "loop" => Some(ClassifiedSourceWord::keyword(TokenKind::Loop)),
        "to" => Some(ClassifiedSourceWord::keyword(TokenKind::ExclusiveRange)),
        "by" => Some(ClassifiedSourceWord::keyword(TokenKind::By)),

        // Word operators.
        "is" => Some(ClassifiedSourceWord::word_operator(TokenKind::Is)),
        "not" => Some(ClassifiedSourceWord::word_operator(TokenKind::Not)),
        "and" => Some(ClassifiedSourceWord::word_operator(TokenKind::And)),
        "or" => Some(ClassifiedSourceWord::word_operator(TokenKind::Or)),

        // Value literals.
        "true" => Some(ClassifiedSourceWord::literal(TokenKind::BoolLiteral(true))),
        "false" => Some(ClassifiedSourceWord::literal(TokenKind::BoolLiteral(false))),
        "none" => Some(ClassifiedSourceWord::literal(TokenKind::NoneLiteral)),

        // Builtin and singleton type spellings.
        "Int" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeInt)),
        "Float" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeFloat)),
        "Bool" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeBool)),
        "String" => Some(ClassifiedSourceWord::builtin_type(
            TokenKind::DatatypeString,
        )),
        "Char" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeChar)),
        "None" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeNone)),
        "True" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeTrue)),
        "False" => Some(ClassifiedSourceWord::builtin_type(TokenKind::DatatypeFalse)),

        _ => None,
    }
}

/// Returns the tokenizer token kind for an exact source keyword spelling.
pub(crate) fn keyword_token_kind(text: &str) -> Option<TokenKind> {
    classify_source_word(text).map(|classified| classified.token_kind)
}

/// Returns the compound token for keyword forms that require an attached `!`.
///
/// WHAT: `return!` and `cast!` are lexical forms, not a keyword followed by a
///       whitespace-sensitive postfix operator.
/// WHY: keeping attachment in tokenization prevents AST parsing from having to
///      reconstruct source adjacency from locations.
pub(crate) fn attached_bang_keyword_token_kind(text: &str) -> Option<TokenKind> {
    match text {
        "return" => Some(TokenKind::ReturnBang),
        "cast" => Some(TokenKind::CastBang),
        _ => None,
    }
}

/// True when `text` is an exact keyword spelling that lexes to a dedicated token.
#[cfg(test)]
pub(crate) fn is_keyword(text: &str) -> bool {
    keyword_token_kind(text).is_some()
}

/// True when a character can appear after the first character of an identifier.
pub(crate) fn is_identifier_continue(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

/// True when a string is a source-level identifier spelling.
pub(crate) fn is_valid_identifier(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|char| char.is_alphabetic() || char == '_')
        && text.chars().all(is_identifier_continue)
}
