//! Compiler-owned token mapping for neutral source-word identities.
//!
//! WHAT: maps moth-lexical word identities to `TokenTag` and specializes attached-bang forms into
//! compiler token identities.
//! WHY: lexical spelling and presentation categories stay below the compiler, while token storage
//! and its attached-bang tags remain compiler-owned.

use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::words::{SourceWord, classify_source_word};

/// Returns the compiler token identity for a shared exact source word.
pub(crate) fn token_tag_for_source_word(word: SourceWord) -> TokenTag {
    match word {
        SourceWord::Export => TokenTag::EXPORT,
        SourceWord::Type => TokenTag::TYPE,
        SourceWord::Of => TokenTag::OF,
        SourceWord::As => TokenTag::AS,
        SourceWord::Copy => TokenTag::COPY,
        SourceWord::If => TokenTag::IF,
        SourceWord::Return => TokenTag::RETURN,
        SourceWord::Catch => TokenTag::CATCH,
        SourceWord::Then => TokenTag::THEN,
        SourceWord::Else => TokenTag::ELSE,
        SourceWord::Checked => TokenTag::CHECKED,
        SourceWord::Cast => TokenTag::CAST,
        SourceWord::Break => TokenTag::BREAK,
        SourceWord::Continue => TokenTag::CONTINUE,
        SourceWord::Must => TokenTag::MUST,
        SourceWord::This => TokenTag::THIS,
        SourceWord::ThisType => TokenTag::TRAIT_THIS,
        SourceWord::Assert => TokenTag::ASSERT,
        SourceWord::Async => TokenTag::ASYNC,
        SourceWord::Yield => TokenTag::YIELD,
        SourceWord::Loop => TokenTag::LOOP,
        SourceWord::To => TokenTag::EXCLUSIVE_RANGE,
        SourceWord::By => TokenTag::BY,
        SourceWord::Is => TokenTag::IS,
        SourceWord::Not => TokenTag::NOT,
        SourceWord::And => TokenTag::AND,
        SourceWord::Or => TokenTag::OR,
        SourceWord::TrueLiteral | SourceWord::FalseLiteral => TokenTag::BOOL_LITERAL,
        SourceWord::NoneLiteral => TokenTag::NONE_LITERAL,
        SourceWord::IntType => TokenTag::DATATYPE_INT,
        SourceWord::FloatType => TokenTag::DATATYPE_FLOAT,
        SourceWord::BoolType => TokenTag::DATATYPE_BOOL,
        SourceWord::StringType => TokenTag::DATATYPE_STRING,
        SourceWord::CharType => TokenTag::DATATYPE_CHAR,
        SourceWord::NoneType => TokenTag::DATATYPE_NONE,
        SourceWord::TrueType => TokenTag::DATATYPE_TRUE,
        SourceWord::FalseType => TokenTag::DATATYPE_FALSE,
        SourceWord::I8Type => TokenTag::DATATYPE_I8,
        SourceWord::I16Type => TokenTag::DATATYPE_I16,
        SourceWord::I32Type => TokenTag::DATATYPE_I32,
        SourceWord::I64Type => TokenTag::DATATYPE_I64,
        SourceWord::U8Type => TokenTag::DATATYPE_U8,
        SourceWord::U16Type => TokenTag::DATATYPE_U16,
        SourceWord::U32Type => TokenTag::DATATYPE_U32,
        SourceWord::U64Type => TokenTag::DATATYPE_U64,
        SourceWord::F16Type => TokenTag::DATATYPE_F16,
        SourceWord::F32Type => TokenTag::DATATYPE_F32,
        SourceWord::F64Type => TokenTag::DATATYPE_F64,
        SourceWord::ByteType => TokenTag::DATATYPE_BYTE,
    }
}

/// Returns the compiler token identity for an exact source word with an attached `!`.
pub(crate) fn attached_bang_keyword_token_tag(text: &str) -> Option<TokenTag> {
    match classify_source_word(text)? {
        SourceWord::Return => Some(TokenTag::RETURN_BANG),
        SourceWord::Cast => Some(TokenTag::CAST_BANG),
        _ => None,
    }
}
