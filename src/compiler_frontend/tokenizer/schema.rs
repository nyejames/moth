//! Stable token taxonomy and compact payload shapes.

use crate::compiler_frontend::numeric_text::store::NumericLiteralId;
use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

/// How a diagnostic or source-token shape obtains its user-facing spelling.
///
/// This metadata belongs to the tokenizer taxonomy so source-token stores and diagnostic
/// projections cannot grow independent token-name authorities.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TokenDescriptorPayload {
    Static = 0,
    Symbol = 1,
    StyleDirective = 2,
    StringLiteral = 3,
    NumericLiteral = 4,
    CharLiteral = 5,
    RawStringLiteral = 6,
    BoolLiteral = 7,
    Path = 8,
}

/// Static metadata for one stable token tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenDescriptor {
    text: &'static str,
    payload: TokenDescriptorPayload,
}

impl TokenDescriptor {
    const fn new(text: &'static str, payload: TokenDescriptorPayload) -> Self {
        Self { text, payload }
    }

    pub(crate) const fn text(self) -> &'static str {
        self.text
    }

    pub(crate) const fn payload(self) -> TokenDescriptorPayload {
        self.payload
    }
}

/// Stable compact taxonomy shared by tokenizer shapes and diagnostic projections.
///
/// Every value is explicit in the schema invocation below. It is independent of declaration
/// order so adding or reordering source variants cannot change retained diagnostic data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenTag(u16);

/// Fixed source-token shape: one stable tag, reserved/semantic flags and one compact payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenShape {
    pub(crate) tag: TokenTag,
    pub(crate) flags: u16,
    pub(crate) data: u32,
}
const _: () = assert!(std::mem::size_of::<TokenShape>() == 8);

const TOKEN_CLASS_ASSIGNMENT: u16 = 1 << 0;
const TOKEN_CLASS_CONTINUES_EXPRESSION: u16 = 1 << 1;
const TOKEN_CLASS_CAN_END_EXPRESSION: u16 = 1 << 2;
const TOKEN_CLASS_OPERAND_START: u16 = 1 << 3;
const TOKEN_CLASS_KEYWORD: u16 = 1 << 4;
const TOKEN_CLASS_WORD_OPERATOR: u16 = 1 << 5;
const TOKEN_CLASS_LITERAL: u16 = 1 << 6;
const TOKEN_CLASS_BUILTIN_TYPE: u16 = 1 << 7;
const TOKEN_CLASS_DELIMITER: u16 = 1 << 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenSchema {
    tag: TokenTag,
    descriptor: TokenDescriptor,
    allowed_flags: u16,
    classes: u16,
    precedence: Option<u8>,
}

impl TokenSchema {
    #[cfg(test)]
    pub(crate) const fn tag(self) -> TokenTag {
        self.tag
    }

    #[cfg(test)]
    pub(crate) const fn descriptor(self) -> TokenDescriptor {
        self.descriptor
    }

    #[cfg(test)]
    pub(crate) const fn allowed_flags(self) -> u16 {
        self.allowed_flags
    }

    #[cfg(test)]
    pub(crate) const fn precedence(self) -> Option<u8> {
        self.precedence
    }

    fn has_class(self, class: u16) -> bool {
        self.classes & class != 0
    }
}

macro_rules! token_schema {
    (
        $(
            (
                $pattern:pat,
                $tag_name:ident,
                $raw:literal,
                $text:literal,
                $payload:ident,
                $allowed_flags:expr,
                $classes:expr,
                $precedence:expr
            )
        ),+ $(,)?
    ) => {
        impl TokenTag {
            $(pub(crate) const $tag_name: Self = Self($raw);)+

            pub(crate) const fn raw(self) -> u16 {
                self.0
            }

            pub(crate) const fn from_raw(raw: u16) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$tag_name),)+
                    _ => None,
                }
            }
            /// Preserve a raw value from a packed boundary even when it is unknown to this
            /// version of the taxonomy. Unknown values render through the descriptor fallback.
            pub(crate) const fn from_raw_unchecked(raw: u16) -> Self {
                Self(raw)
            }


            #[cfg(test)]
            pub(crate) const fn all() -> &'static [Self] {
                &[$(Self::$tag_name,)+]
            }

            pub(crate) const fn descriptor(self) -> TokenDescriptor {
                match self {
                    $(Self::$tag_name => TokenDescriptor::new(
                        $text,
                        TokenDescriptorPayload::$payload,
                    ),)+
                    _ => TokenDescriptor::new("token", TokenDescriptorPayload::Static),
                }
            }

            pub(crate) const fn schema(self) -> Option<TokenSchema> {
                match self {
                    $(Self::$tag_name => Some(TokenSchema {
                        tag: Self::$tag_name,
                        descriptor: TokenDescriptor::new(
                            $text,
                            TokenDescriptorPayload::$payload,
                        ),
                        allowed_flags: $allowed_flags,
                        classes: $classes,
                        precedence: $precedence,
                    }),)+
                    _ => None,
                }
            }

            pub(crate) const fn allowed_flags(self) -> u16 {
                match self {
                    $(Self::$tag_name => $allowed_flags,)+
                    _ => 0,
                }
            }

            pub(crate) const fn flags_are_valid(self, flags: u16) -> bool {
                flags & !self.allowed_flags() == 0
            }

            pub(crate) fn is_assignment_operator(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_ASSIGNMENT)
                })
            }

            pub(crate) fn continues_expression(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_CONTINUES_EXPRESSION)
                })
            }

            pub(crate) fn can_end_expression(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_CAN_END_EXPRESSION)
                })
            }

            pub(crate) fn is_operand_start(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_OPERAND_START)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_keyword(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_KEYWORD)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_word_operator(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_WORD_OPERATOR)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_literal(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_LITERAL)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_builtin_type(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_BUILTIN_TYPE)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_delimiter(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_DELIMITER)
                })
            }

            /// Whether this tag counts toward the stats symbol bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::symbols`.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained symbol table.
            pub(crate) fn is_stats_symbol(self) -> bool {
                self == Self::SYMBOL
            }

            /// Whether this tag counts toward the stats literal bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::literals`. It covers the string,
            ///      raw-string, numeric, char, bool, and `none` value spellings that the legacy
            ///      classifier counted.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained literal table.
            pub(crate) fn is_stats_literal(self) -> bool {
                matches!(
                    self,
                    Self::STRING_SLICE_LITERAL
                        | Self::RAW_STRING_LITERAL
                        | Self::NUMERIC_LITERAL
                        | Self::CHAR_LITERAL
                        | Self::BOOL_LITERAL
                        | Self::NONE_LITERAL
                )
            }

            /// Whether this tag counts toward the stats operator bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::operators`. It preserves the exact
            ///      legacy operator set, including expression-continuing punctuation (`->`),
            ///      word operators, postfix markers (`!`, `?`), `copy`, channels, `&`, and `=>`.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained operator table.
            pub(crate) fn is_stats_operator(self) -> bool {
                matches!(
                    self,
                    Self::ARROW
                        | Self::ADD
                        | Self::SUBTRACT
                        | Self::MULTIPLY
                        | Self::DIVIDE
                        | Self::MODULUS
                        | Self::INT_DIVIDE
                        | Self::EXPONENT
                        | Self::NEGATIVE
                        | Self::ADD_ASSIGN
                        | Self::SUBTRACT_ASSIGN
                        | Self::MULTIPLY_ASSIGN
                        | Self::DIVIDE_ASSIGN
                        | Self::MODULUS_ASSIGN
                        | Self::EXPONENT_ASSIGN
                        | Self::INT_DIVIDE_ASSIGN
                        | Self::LESS_THAN
                        | Self::LESS_THAN_OR_EQUAL
                        | Self::GREATER_THAN
                        | Self::GREATER_THAN_OR_EQUAL
                        | Self::IS
                        | Self::AND
                        | Self::OR
                        | Self::NOT
                        | Self::BANG
                        | Self::QUESTION_MARK
                        | Self::COPY
                        | Self::CHANNEL_SEND
                        | Self::CHANNEL_RECEIVE
                        | Self::AMPERSAND
                        | Self::FAT_ARROW
                )
            }

            #[cfg(test)]
            pub(crate) fn precedence(self) -> Option<u8> {
                self.schema().and_then(TokenSchema::precedence)
            }
        }

    };
}

const TOKEN_NUMERIC_FLAGS: u16 = 0b11;

token_schema! {
    (ModuleStart, MODULE_START, 1, "module start", Static, 0, 0, None),
    (Eof, EOF, 2, "end of file", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (Export, EXPORT, 3, "`export`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Hash, HASH, 4, "`#`", Static, 0, 0, None),
    (Reactive, REACTIVE, 5, "`$`", Static, 0, 0, None),
    (Arrow, ARROW, 6, "`->`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, None),
    (
        Symbol(_),
        SYMBOL,
        7,
        "name",
        Symbol,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        StyleDirective(_),
        STYLE_DIRECTIVE,
        8,
        "style directive",
        StyleDirective,
        0,
        0,
        None
    ),
    (
        StringSliceLiteral(_),
        STRING_SLICE_LITERAL,
        9,
        "string literal",
        StringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (Path(_), PATH, 10, "path", Path, 0, TOKEN_CLASS_OPERAND_START, None),
    (
        NumericLiteral(_),
        NUMERIC_LITERAL,
        11,
        "numeric literal",
        NumericLiteral,
        TOKEN_NUMERIC_FLAGS,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CharLiteral(_),
        CHAR_LITERAL,
        12,
        "character literal",
        CharLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        RawStringLiteral(_),
        RAW_STRING_LITERAL,
        13,
        "raw string literal",
        RawStringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        BoolLiteral(_),
        BOOL_LITERAL,
        14,
        "boolean literal",
        BoolLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        OpenCurly,
        OPEN_CURLY,
        15,
        "`{`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CloseCurly,
        CLOSE_CURLY,
        16,
        "`}`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TypeParameterBracket,
        TYPE_PARAMETER_BRACKET,
        17,
        "`|`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Newline,
        NEWLINE,
        18,
        "newline",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        End,
        END,
        19,
        "`;`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        StartTemplateBody,
        START_TEMPLATE_BODY,
        20,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Comma,
        COMMA,
        21,
        "`,`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (Dot, DOT, 22, "`.`", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (
        Colon,
        COLON,
        23,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        DoubleColon,
        DOUBLE_COLON,
        24,
        "`::`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Assign,
        ASSIGN,
        25,
        "`=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        This,
        THIS,
        26,
        "`this`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (Must, MUST, 27, "`must`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TraitThis,
        TRAIT_THIS,
        28,
        "`This`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        OpenParenthesis,
        OPEN_PARENTHESIS,
        29,
        "`(`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CloseParenthesis,
        CLOSE_PARENTHESIS,
        30,
        "`)`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (As, AS, 31, "`as`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Type, TYPE, 32, "`type`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Of, OF, 33, "`of`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        Variadic,
        VARIADIC,
        34,
        "`..`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Mutable,
        MUTABLE,
        35,
        "`~`",
        Static,
        0,
        TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        DatatypeNone,
        DATATYPE_NONE,
        36,
        "`None` type",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        NoneLiteral,
        NONE_LITERAL,
        37,
        "`none`",
        Static,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        DatatypeInt,
        DATATYPE_INT,
        38,
        "`Int`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeFloat,
        DATATYPE_FLOAT,
        39,
        "`Float`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeBool,
        DATATYPE_BOOL,
        40,
        "`Bool`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeTrue,
        DATATYPE_TRUE,
        41,
        "`True`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeFalse,
        DATATYPE_FALSE,
        42,
        "`False`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeString,
        DATATYPE_STRING,
        43,
        "`String`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeChar,
        DATATYPE_CHAR,
        44,
        "`Char`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        Bang,
        BANG,
        45,
        "`!`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        QuestionMark,
        QUESTION_MARK,
        46,
        "`?`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (Negative, NEGATIVE, 47, "unary `-`", Static, 0, 0, Some(6)),
    (Exponent, EXPONENT, 48, "`^`", Static, 0, 0, Some(5)),
    (Multiply, MULTIPLY, 49, "`*`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (Divide, DIVIDE, 50, "`/`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (Modulus, MODULUS, 51, "`%`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (IntDivide, INT_DIVIDE, 52, "`//`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (
        ExponentAssign,
        EXPONENT_ASSIGN,
        53,
        "`^=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        MultiplyAssign,
        MULTIPLY_ASSIGN,
        54,
        "`*=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        DivideAssign,
        DIVIDE_ASSIGN,
        55,
        "`/=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        ModulusAssign,
        MODULUS_ASSIGN,
        56,
        "`%=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        IntDivideAssign,
        INT_DIVIDE_ASSIGN,
        57,
        "`//=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Add,
        ADD,
        58,
        "`+`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        Subtract,
        SUBTRACT,
        59,
        "`-`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        AddAssign,
        ADD_ASSIGN,
        60,
        "`+=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        SubtractAssign,
        SUBTRACT_ASSIGN,
        61,
        "`-=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Not,
        NOT,
        62,
        "`not`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(6)
    ),
    (
        Is,
        IS,
        63,
        "`is`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR | TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        LessThan,
        LESS_THAN,
        64,
        "`<`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        LessThanOrEqual,
        LESS_THAN_OR_EQUAL,
        65,
        "`<=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        GreaterThan,
        GREATER_THAN,
        66,
        "`>`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        GreaterThanOrEqual,
        GREATER_THAN_OR_EQUAL,
        67,
        "`>=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        And,
        AND,
        68,
        "`and`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(1)
    ),
    (Or, OR, 69, "`or`", Static, 0, TOKEN_CLASS_WORD_OPERATOR, Some(0)),
    (If, IF, 70, "`if`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Else, ELSE, 71, "`else`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Return, RETURN, 72, "`return`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        ReturnBang,
        RETURN_BANG,
        73,
        "`return!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (Catch, CATCH, 74, "`catch`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Then, THEN, 75, "`then`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Checked, CHECKED, 76, "`checked`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Async, ASYNC, 77, "`async`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Cast, CAST, 78, "`cast`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        CastBang,
        CAST_BANG,
        79,
        "`cast!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (Assert, ASSERT, 80, "`assert`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Loop, LOOP, 81, "`loop`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (By, BY, 82, "`by`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Break, BREAK, 83, "`break`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        Continue,
        CONTINUE,
        84,
        "`continue`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        ExclusiveRange,
        EXCLUSIVE_RANGE,
        85,
        "`to`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        Some(6)
    ),
    (Ampersand, AMPERSAND, 86, "`&`", Static, 0, 0, None),
    (
        FatArrow,
        FAT_ARROW,
        87,
        "`=>`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (Wildcard, WILDCARD, 88, "`_`", Static, 0, 0, None),
    (
        Copy,
        COPY,
        89,
        "`copy`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TemplateClose,
        TEMPLATE_CLOSE,
        90,
        "`]`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TemplateHead,
        TEMPLATE_HEAD,
        91,
        "`[`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        ChannelSend,
        CHANNEL_SEND,
        92,
        "`>>`",
        Static,
        0,
        0,
        None
    ),
    (
        ChannelReceive,
        CHANNEL_RECEIVE,
        93,
        "`<<`",
        Static,
        0,
        0,
        None
    ),
    (Yield, YIELD, 94, "`yield`", Static, 0, TOKEN_CLASS_KEYWORD, None),
}

impl TokenShape {
    /// Construct a shape only when its flags are valid for the selected token tag.
    ///
    /// Payload validation is performed by [`Self::from_raw_parts`] and the typed accessors. This
    /// constructor remains a low-level packing primitive for taxonomy tests and source adapters.
    #[cfg(test)]
    pub(crate) const fn new(tag: TokenTag, flags: u16, data: u32) -> Option<Self> {
        if tag.flags_are_valid(flags) {
            Some(Self { tag, flags, data })
        } else {
            None
        }
    }

    /// Decode a raw shape while rejecting unknown tags, reserved flags and malformed payloads.
    pub(crate) fn from_raw_parts(raw_tag: u16, flags: u16, data: u32) -> Option<Self> {
        let tag = TokenTag::from_raw(raw_tag)?;
        if !tag.flags_are_valid(flags) || !Self::payload_is_valid(tag, flags, data) {
            return None;
        }
        Some(Self { tag, flags, data })
    }

    fn payload_is_valid(tag: TokenTag, flags: u16, data: u32) -> bool {
        match tag.descriptor().payload() {
            TokenDescriptorPayload::Static => flags == 0 && data == 0,
            TokenDescriptorPayload::Path => {
                flags == 0 && PathSyntaxId::try_from_raw(data).is_some()
            }
            TokenDescriptorPayload::NumericLiteral => {
                flags <= 2 && NumericLiteralId::try_from_raw(data).is_some()
            }
            TokenDescriptorPayload::BoolLiteral => flags == 0 && data <= 1,
            TokenDescriptorPayload::CharLiteral => flags == 0 && char::from_u32(data).is_some(),
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => flags == 0,
        }
    }

    pub(crate) fn numeric_kind(self) -> Option<NumericLiteralKind> {
        if self.tag != TokenTag::NUMERIC_LITERAL || self.flags > 2 {
            return None;
        }
        Some(match self.flags {
            0 => NumericLiteralKind::WholeNumber,
            1 => NumericLiteralKind::DecimalPoint,
            2 => NumericLiteralKind::Exponent,
            _ => unreachable!("numeric flags were checked above"),
        })
    }

    pub(crate) fn numeric_literal_id(self) -> Option<NumericLiteralId> {
        if self.numeric_kind().is_some() {
            NumericLiteralId::try_from_raw(self.data)
        } else {
            None
        }
    }

    pub(crate) fn path_syntax_id(self) -> Option<PathSyntaxId> {
        if self.tag == TokenTag::PATH && self.flags == 0 {
            PathSyntaxId::try_from_raw(self.data)
        } else {
            None
        }
    }

    pub(crate) fn string_id(self) -> Option<StringId> {
        let payload = self.tag.descriptor().payload();
        if matches!(
            payload,
            TokenDescriptorPayload::Symbol
                | TokenDescriptorPayload::StyleDirective
                | TokenDescriptorPayload::StringLiteral
                | TokenDescriptorPayload::RawStringLiteral
        ) && self.flags == 0
        {
            Some(StringId::from_index(self.data))
        } else {
            None
        }
    }

    pub(crate) fn bool_value_checked(self) -> Option<bool> {
        if self.tag == TokenTag::BOOL_LITERAL && self.flags == 0 {
            match self.data {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            }
        } else {
            None
        }
    }

    pub(crate) fn char_value_checked(self) -> Option<char> {
        if self.tag == TokenTag::CHAR_LITERAL && self.flags == 0 {
            char::from_u32(self.data)
        } else {
            None
        }
    }

    pub(crate) const fn tag(self) -> TokenTag {
        self.tag
    }

    pub(crate) const fn flags(self) -> u16 {
        self.flags
    }

    pub(crate) const fn data(self) -> u32 {
        self.data
    }
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.string_id().is_some() {
            self.data = remap.get(StringId::from_index(self.data)).index();
        }
    }
}
pub(crate) const fn numeric_kind_flags(kind: NumericLiteralKind) -> u16 {
    match kind {
        NumericLiteralKind::WholeNumber => 0,
        NumericLiteralKind::DecimalPoint => 1,
        NumericLiteralKind::Exponent => 2,
    }
}
