//! Shared exact source-word identities and presentation classes.
//!
//! WHAT: owns the allocation-free spelling inventory, neutral categories, literal facts and the
//! reservation-only words shared by compiler lexing and source-name policy.
//! WHY: the compiler maps these identities to tokens, while the lexical crate remains independent
//! of compiler token models and the highlighter consumes the same categories.

/// Presentation category for an exact source word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceWordClass {
    /// A source keyword.
    Keyword,
    /// A word-form operator.
    WordOperator,
    /// A literal word.
    Literal,
    /// A builtin type spelling.
    BuiltinType,
}

macro_rules! define_source_words {
    (
        $(
            $variant:ident => ($spelling:literal, $class:ident, $bool_value:expr);
        )+
        @reserved_only [$($reserved_only:literal),+ $(,)?]
    ) => {
        /// Neutral identity for one exact Moth source word.
        ///
        /// Exact spelling is case-sensitive. Reserved-only names are intentionally not variants:
        /// they participate in user-name policy but never classify as source words.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum SourceWord {
            $(
                $variant,
            )+
        }

        impl SourceWord {
            /// Every exact source-word identity, in inventory order.
            pub const ALL: &'static [Self] = &[
                $(
                    Self::$variant,
                )+
            ];

            /// Returns this word's exact source spelling.
            pub const fn spelling(self) -> &'static str {
                match self {
                    $(
                        Self::$variant => $spelling,
                    )+
                }
            }

            /// Returns this word's neutral presentation category.
            pub const fn class(self) -> SourceWordClass {
                match self {
                    $(
                        Self::$variant => SourceWordClass::$class,
                    )+
                }
            }

            /// Returns a boolean payload for boolean literal words.
            pub const fn bool_literal_value(self) -> Option<bool> {
                match self {
                    $(
                        Self::$variant => $bool_value,
                    )+
                }
            }
        }

        /// Classifies an exact source-word spelling without allocation or case folding.
        pub fn classify_source_word(spelling: &str) -> Option<SourceWord> {
            match spelling {
                $(
                    $spelling => Some(SourceWord::$variant),
                )+
                _ => None,
            }
        }

        const RESERVED_WORD_SPELLINGS: &[&str] = &[
            $(
                $spelling,
            )+
            $(
                $reserved_only,
            )+
        ];

        pub(crate) fn is_reserved_source_word(spelling: &str) -> bool {
            RESERVED_WORD_SPELLINGS
                .iter()
                .any(|reserved| spelling.eq_ignore_ascii_case(reserved))
        }
    };
}

define_source_words! {
    Export => ("export", Keyword, None);
    Type => ("type", Keyword, None);
    Of => ("of", Keyword, None);
    As => ("as", Keyword, None);
    Copy => ("copy", Keyword, None);
    If => ("if", Keyword, None);
    Return => ("return", Keyword, None);
    Catch => ("catch", Keyword, None);
    Then => ("then", Keyword, None);
    Else => ("else", Keyword, None);
    Checked => ("checked", Keyword, None);
    Cast => ("cast", Keyword, None);
    Break => ("break", Keyword, None);
    Continue => ("continue", Keyword, None);
    Must => ("must", Keyword, None);
    This => ("this", Keyword, None);
    ThisType => ("This", Keyword, None);
    Assert => ("assert", Keyword, None);
    Async => ("async", Keyword, None);
    Yield => ("yield", Keyword, None);
    Loop => ("loop", Keyword, None);
    To => ("to", Keyword, None);
    By => ("by", Keyword, None);
    Is => ("is", WordOperator, None);
    Not => ("not", WordOperator, None);
    And => ("and", WordOperator, None);
    Or => ("or", WordOperator, None);
    TrueLiteral => ("true", Literal, Some(true));
    FalseLiteral => ("false", Literal, Some(false));
    NoneLiteral => ("none", Literal, None);
    IntType => ("Int", BuiltinType, None);
    FloatType => ("Float", BuiltinType, None);
    BoolType => ("Bool", BuiltinType, None);
    StringType => ("String", BuiltinType, None);
    CharType => ("Char", BuiltinType, None);
    NoneType => ("None", BuiltinType, None);
    TrueType => ("True", BuiltinType, None);
    FalseType => ("False", BuiltinType, None);
    I8Type => ("I8", BuiltinType, None);
    I16Type => ("I16", BuiltinType, None);
    I32Type => ("I32", BuiltinType, None);
    I64Type => ("I64", BuiltinType, None);
    U8Type => ("U8", BuiltinType, None);
    U16Type => ("U16", BuiltinType, None);
    U32Type => ("U32", BuiltinType, None);
    U64Type => ("U64", BuiltinType, None);
    F16Type => ("F16", BuiltinType, None);
    F32Type => ("F32", BuiltinType, None);
    F64Type => ("F64", BuiltinType, None);
    ByteType => ("Byte", BuiltinType, None);
    @reserved_only ["fn", "config"]
}

#[cfg(test)]
#[path = "tests/words_tests.rs"]
mod tests;
