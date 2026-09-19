//! Frontend keyword and identifier policy.
//!
//! WHAT: owns the exact keyword-to-token mapping used by lexing and the identifier
//! validation helpers shared with path and dependency parsing. The same direct match also
//! supplies the neutral presentation classes consumed by the HTML code highlighter.
//! WHY: keyword policy is user-visible and must not drift between the tokenizer,
//! dependency alias validation, reserved-name diagnostics and code highlighting.

#[cfg(test)]
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClassifiedSourceWord {
    pub(crate) token_tag: TokenTag,
    pub(crate) bool_value: Option<bool>,
    pub(crate) class: SourceWordClass,
}

impl ClassifiedSourceWord {
    fn keyword(token_tag: TokenTag) -> Self {
        Self {
            token_tag,
            bool_value: None,
            class: SourceWordClass::Keyword,
        }
    }

    fn word_operator(token_tag: TokenTag) -> Self {
        Self {
            token_tag,
            bool_value: None,
            class: SourceWordClass::WordOperator,
        }
    }

    fn literal(token_tag: TokenTag, bool_value: Option<bool>) -> Self {
        Self {
            token_tag,
            bool_value,
            class: SourceWordClass::Literal,
        }
    }

    fn builtin_type(token_tag: TokenTag) -> Self {
        Self {
            token_tag,
            bool_value: None,
            class: SourceWordClass::BuiltinType,
        }
    }
}

/// Returns the stable taxonomy tag and any typed literal payload for an exact source word.
pub(crate) fn classify_source_word(text: &str) -> Option<ClassifiedSourceWord> {
    match text {
        "export" => Some(ClassifiedSourceWord::keyword(TokenTag::EXPORT)),
        "type" => Some(ClassifiedSourceWord::keyword(TokenTag::TYPE)),
        "of" => Some(ClassifiedSourceWord::keyword(TokenTag::OF)),
        "as" => Some(ClassifiedSourceWord::keyword(TokenTag::AS)),
        "copy" => Some(ClassifiedSourceWord::keyword(TokenTag::COPY)),
        "if" => Some(ClassifiedSourceWord::keyword(TokenTag::IF)),
        "return" => Some(ClassifiedSourceWord::keyword(TokenTag::RETURN)),
        "catch" => Some(ClassifiedSourceWord::keyword(TokenTag::CATCH)),
        "then" => Some(ClassifiedSourceWord::keyword(TokenTag::THEN)),
        "else" => Some(ClassifiedSourceWord::keyword(TokenTag::ELSE)),
        "checked" => Some(ClassifiedSourceWord::keyword(TokenTag::CHECKED)),
        "cast" => Some(ClassifiedSourceWord::keyword(TokenTag::CAST)),
        "break" => Some(ClassifiedSourceWord::keyword(TokenTag::BREAK)),
        "continue" => Some(ClassifiedSourceWord::keyword(TokenTag::CONTINUE)),
        "must" => Some(ClassifiedSourceWord::keyword(TokenTag::MUST)),
        "this" => Some(ClassifiedSourceWord::keyword(TokenTag::THIS)),
        "This" => Some(ClassifiedSourceWord::keyword(TokenTag::TRAIT_THIS)),
        "assert" => Some(ClassifiedSourceWord::keyword(TokenTag::ASSERT)),
        "async" => Some(ClassifiedSourceWord::keyword(TokenTag::ASYNC)),
        "yield" => Some(ClassifiedSourceWord::keyword(TokenTag::YIELD)),
        "loop" => Some(ClassifiedSourceWord::keyword(TokenTag::LOOP)),
        "to" => Some(ClassifiedSourceWord::keyword(TokenTag::EXCLUSIVE_RANGE)),
        "by" => Some(ClassifiedSourceWord::keyword(TokenTag::BY)),
        "is" => Some(ClassifiedSourceWord::word_operator(TokenTag::IS)),
        "not" => Some(ClassifiedSourceWord::word_operator(TokenTag::NOT)),
        "and" => Some(ClassifiedSourceWord::word_operator(TokenTag::AND)),
        "or" => Some(ClassifiedSourceWord::word_operator(TokenTag::OR)),
        "true" => Some(ClassifiedSourceWord::literal(TokenTag::BOOL_LITERAL, Some(true))),
        "false" => Some(ClassifiedSourceWord::literal(TokenTag::BOOL_LITERAL, Some(false))),
        "none" => Some(ClassifiedSourceWord::literal(TokenTag::NONE_LITERAL, None)),
        "Int" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_INT)),
        "Float" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_FLOAT)),
        "Bool" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_BOOL)),
        "String" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_STRING)),
        "Char" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_CHAR)),
        "None" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_NONE)),
        "True" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_TRUE)),
        "False" => Some(ClassifiedSourceWord::builtin_type(TokenTag::DATATYPE_FALSE)),
        _ => None,
    }
}

/// Returns the stable taxonomy tag for an exact source keyword spelling.
#[cfg(test)]
pub(crate) fn keyword_token_tag(text: &str) -> Option<TokenTag> {
    classify_source_word(text).map(|classified| classified.token_tag)
}

/// Legacy token-kind projection retained only for keyword fixtures during migration.
#[cfg(test)]
pub(crate) fn keyword_token_kind(text: &str) -> Option<TokenKind> {
    match text {
        "export" => Some(TokenKind::Export),
        "type" => Some(TokenKind::Type),
        "of" => Some(TokenKind::Of),
        "as" => Some(TokenKind::As),
        "copy" => Some(TokenKind::Copy),
        "if" => Some(TokenKind::If),
        "return" => Some(TokenKind::Return),
        "catch" => Some(TokenKind::Catch),
        "then" => Some(TokenKind::Then),
        "else" => Some(TokenKind::Else),
        "checked" => Some(TokenKind::Checked),
        "cast" => Some(TokenKind::Cast),
        "break" => Some(TokenKind::Break),
        "continue" => Some(TokenKind::Continue),
        "must" => Some(TokenKind::Must),
        "this" => Some(TokenKind::This),
        "This" => Some(TokenKind::TraitThis),
        "assert" => Some(TokenKind::Assert),
        "async" => Some(TokenKind::Async),
        "yield" => Some(TokenKind::Yield),
        "loop" => Some(TokenKind::Loop),
        "to" => Some(TokenKind::ExclusiveRange),
        "by" => Some(TokenKind::By),
        "is" => Some(TokenKind::Is),
        "not" => Some(TokenKind::Not),
        "and" => Some(TokenKind::And),
        "or" => Some(TokenKind::Or),
        "true" => Some(TokenKind::BoolLiteral(true)),
        "false" => Some(TokenKind::BoolLiteral(false)),
        "none" => Some(TokenKind::NoneLiteral),
        "Int" => Some(TokenKind::DatatypeInt),
        "Float" => Some(TokenKind::DatatypeFloat),
        "Bool" => Some(TokenKind::DatatypeBool),
        "String" => Some(TokenKind::DatatypeString),
        "Char" => Some(TokenKind::DatatypeChar),
        "None" => Some(TokenKind::DatatypeNone),
        "True" => Some(TokenKind::DatatypeTrue),
        "False" => Some(TokenKind::DatatypeFalse),
        _ => None,
    }
}

/// Returns the stable taxonomy tag for a keyword form requiring an attached `!`.
pub(crate) fn attached_bang_keyword_token_tag(text: &str) -> Option<TokenTag> {
    match text {
        "return" => Some(TokenTag::RETURN_BANG),
        "cast" => Some(TokenTag::CAST_BANG),
        _ => None,
    }
}

/// Legacy compound-token projection retained only for keyword fixtures.
#[cfg(test)]
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
