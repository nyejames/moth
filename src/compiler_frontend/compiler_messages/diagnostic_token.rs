//! Compact source-token facts retained by durable compiler diagnostics.
//!
//! A diagnostic only needs a token's stable class plus one immediate payload for
//! rendering. This projection intentionally does not retain the live tokenizer
//! enum or any source-owned side-store handles that would make a diagnostic
//! depend on the source token buffer's lifetime.

use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;

const TAG_MASK: u32 = u16::MAX as u32;
const FLAGS_SHIFT: u32 = u16::BITS;
const NUMERIC_KIND_MASK: u16 = 0b11;

/// How a [`TokenDescriptor`] obtains the user-facing token spelling.
///
/// This is descriptor metadata only; it is not a second token representation.
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
}

/// Static metadata for one compact [`TokenTag`].
///
/// The descriptor is render-boundary metadata and is never stored in
/// [`DiagnosticToken`]. Dynamic descriptors carry the stable label used with
/// the token's one `u32` payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenDescriptor {
    text: &'static str,
    payload: TokenDescriptorPayload,
}

impl TokenDescriptor {
    const fn static_text(text: &'static str) -> Self {
        Self {
            text,
            payload: TokenDescriptorPayload::Static,
        }
    }

    const fn dynamic(text: &'static str, payload: TokenDescriptorPayload) -> Self {
        Self { text, payload }
    }

    pub(crate) const fn text(self) -> &'static str {
        self.text
    }

    pub(crate) const fn payload(self) -> TokenDescriptorPayload {
        self.payload
    }
}

/// Stable compact taxonomy shared by source-token and diagnostic projections.
///
/// Values are explicit so adding or reordering source-token variants cannot
/// change the meaning of retained diagnostic data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenTag(u16);

impl TokenTag {
    pub(crate) const MODULE_START: Self = Self(1);
    pub(crate) const EOF: Self = Self(2);
    pub(crate) const EXPORT: Self = Self(3);
    pub(crate) const HASH: Self = Self(4);
    pub(crate) const REACTIVE: Self = Self(5);
    pub(crate) const ARROW: Self = Self(6);
    pub(crate) const SYMBOL: Self = Self(7);
    pub(crate) const STYLE_DIRECTIVE: Self = Self(8);
    pub(crate) const STRING_SLICE_LITERAL: Self = Self(9);
    pub(crate) const PATH: Self = Self(10);
    pub(crate) const NUMERIC_LITERAL: Self = Self(11);
    pub(crate) const CHAR_LITERAL: Self = Self(12);
    pub(crate) const RAW_STRING_LITERAL: Self = Self(13);
    pub(crate) const BOOL_LITERAL: Self = Self(14);
    pub(crate) const OPEN_CURLY: Self = Self(15);
    pub(crate) const CLOSE_CURLY: Self = Self(16);
    pub(crate) const TYPE_PARAMETER_BRACKET: Self = Self(17);
    pub(crate) const NEWLINE: Self = Self(18);
    pub(crate) const END: Self = Self(19);
    pub(crate) const START_TEMPLATE_BODY: Self = Self(20);
    pub(crate) const COMMA: Self = Self(21);
    pub(crate) const DOT: Self = Self(22);
    pub(crate) const COLON: Self = Self(23);
    pub(crate) const DOUBLE_COLON: Self = Self(24);
    pub(crate) const ASSIGN: Self = Self(25);
    pub(crate) const THIS: Self = Self(26);
    pub(crate) const MUST: Self = Self(27);
    pub(crate) const TRAIT_THIS: Self = Self(28);
    pub(crate) const OPEN_PARENTHESIS: Self = Self(29);
    pub(crate) const CLOSE_PARENTHESIS: Self = Self(30);
    pub(crate) const AS: Self = Self(31);
    pub(crate) const TYPE: Self = Self(32);
    pub(crate) const OF: Self = Self(33);
    pub(crate) const VARIADIC: Self = Self(34);
    pub(crate) const MUTABLE: Self = Self(35);
    pub(crate) const DATATYPE_NONE: Self = Self(36);
    pub(crate) const NONE_LITERAL: Self = Self(37);
    pub(crate) const DATATYPE_INT: Self = Self(38);
    pub(crate) const DATATYPE_FLOAT: Self = Self(39);
    pub(crate) const DATATYPE_BOOL: Self = Self(40);
    pub(crate) const DATATYPE_TRUE: Self = Self(41);
    pub(crate) const DATATYPE_FALSE: Self = Self(42);
    pub(crate) const DATATYPE_STRING: Self = Self(43);
    pub(crate) const DATATYPE_CHAR: Self = Self(44);
    pub(crate) const BANG: Self = Self(45);
    pub(crate) const QUESTION_MARK: Self = Self(46);
    pub(crate) const NEGATIVE: Self = Self(47);
    pub(crate) const EXPONENT: Self = Self(48);
    pub(crate) const MULTIPLY: Self = Self(49);
    pub(crate) const DIVIDE: Self = Self(50);
    pub(crate) const MODULUS: Self = Self(51);
    pub(crate) const INT_DIVIDE: Self = Self(52);
    pub(crate) const EXPONENT_ASSIGN: Self = Self(53);
    pub(crate) const MULTIPLY_ASSIGN: Self = Self(54);
    pub(crate) const DIVIDE_ASSIGN: Self = Self(55);
    pub(crate) const MODULUS_ASSIGN: Self = Self(56);
    pub(crate) const INT_DIVIDE_ASSIGN: Self = Self(57);
    pub(crate) const ADD: Self = Self(58);
    pub(crate) const SUBTRACT: Self = Self(59);
    pub(crate) const ADD_ASSIGN: Self = Self(60);
    pub(crate) const SUBTRACT_ASSIGN: Self = Self(61);
    pub(crate) const NOT: Self = Self(62);
    pub(crate) const IS: Self = Self(63);
    pub(crate) const LESS_THAN: Self = Self(64);
    pub(crate) const LESS_THAN_OR_EQUAL: Self = Self(65);
    pub(crate) const GREATER_THAN: Self = Self(66);
    pub(crate) const GREATER_THAN_OR_EQUAL: Self = Self(67);
    pub(crate) const AND: Self = Self(68);
    pub(crate) const OR: Self = Self(69);
    pub(crate) const IF: Self = Self(70);
    pub(crate) const ELSE: Self = Self(71);
    pub(crate) const RETURN: Self = Self(72);
    pub(crate) const RETURN_BANG: Self = Self(73);
    pub(crate) const CATCH: Self = Self(74);
    pub(crate) const THEN: Self = Self(75);
    pub(crate) const CHECKED: Self = Self(76);
    pub(crate) const ASYNC: Self = Self(77);
    pub(crate) const CAST: Self = Self(78);
    pub(crate) const CAST_BANG: Self = Self(79);
    pub(crate) const ASSERT: Self = Self(80);
    pub(crate) const LOOP: Self = Self(81);
    pub(crate) const BY: Self = Self(82);
    pub(crate) const BREAK: Self = Self(83);
    pub(crate) const CONTINUE: Self = Self(84);
    pub(crate) const EXCLUSIVE_RANGE: Self = Self(85);
    pub(crate) const AMPERSAND: Self = Self(86);
    pub(crate) const FAT_ARROW: Self = Self(87);
    pub(crate) const WILDCARD: Self = Self(88);
    pub(crate) const COPY: Self = Self(89);
    pub(crate) const TEMPLATE_CLOSE: Self = Self(90);
    pub(crate) const TEMPLATE_HEAD: Self = Self(91);
    pub(crate) const CHANNEL_SEND: Self = Self(92);
    pub(crate) const CHANNEL_RECEIVE: Self = Self(93);
    pub(crate) const YIELD: Self = Self(94);

    pub(crate) const fn raw(self) -> u16 {
        self.0
    }

    /// Return the immutable render metadata for this stable token tag.
    pub(crate) const fn descriptor(self) -> TokenDescriptor {
        match self {
            Self::MODULE_START => TokenDescriptor::static_text("module start"),
            Self::EOF => TokenDescriptor::static_text("end of file"),
            Self::EXPORT => TokenDescriptor::static_text("`export`"),
            Self::HASH => TokenDescriptor::static_text("`#`"),
            Self::REACTIVE => TokenDescriptor::static_text("`$`"),
            Self::ARROW => TokenDescriptor::static_text("`->`"),
            Self::SYMBOL => TokenDescriptor::dynamic("name", TokenDescriptorPayload::Symbol),
            Self::STYLE_DIRECTIVE => {
                TokenDescriptor::dynamic("style directive", TokenDescriptorPayload::StyleDirective)
            }
            Self::STRING_SLICE_LITERAL => {
                TokenDescriptor::dynamic("string literal", TokenDescriptorPayload::StringLiteral)
            }
            Self::PATH => TokenDescriptor::static_text("path"),
            Self::NUMERIC_LITERAL => {
                TokenDescriptor::dynamic("numeric literal", TokenDescriptorPayload::NumericLiteral)
            }
            Self::CHAR_LITERAL => {
                TokenDescriptor::dynamic("character literal", TokenDescriptorPayload::CharLiteral)
            }
            Self::RAW_STRING_LITERAL => TokenDescriptor::dynamic(
                "raw string literal",
                TokenDescriptorPayload::RawStringLiteral,
            ),
            Self::BOOL_LITERAL => {
                TokenDescriptor::dynamic("boolean literal", TokenDescriptorPayload::BoolLiteral)
            }
            Self::OPEN_CURLY => TokenDescriptor::static_text("`{`"),
            Self::CLOSE_CURLY => TokenDescriptor::static_text("`}`"),
            Self::TYPE_PARAMETER_BRACKET => TokenDescriptor::static_text("`|`"),
            Self::NEWLINE => TokenDescriptor::static_text("newline"),
            Self::END => TokenDescriptor::static_text("`;`"),
            Self::START_TEMPLATE_BODY => TokenDescriptor::static_text("`:`"),
            Self::COMMA => TokenDescriptor::static_text("`,`"),
            Self::DOT => TokenDescriptor::static_text("`.`"),
            Self::COLON => TokenDescriptor::static_text("`:`"),
            Self::DOUBLE_COLON => TokenDescriptor::static_text("`::`"),
            Self::ASSIGN => TokenDescriptor::static_text("`=`"),
            Self::THIS => TokenDescriptor::static_text("`this`"),
            Self::MUST => TokenDescriptor::static_text("`must`"),
            Self::TRAIT_THIS => TokenDescriptor::static_text("`This`"),
            Self::OPEN_PARENTHESIS => TokenDescriptor::static_text("`(`"),
            Self::CLOSE_PARENTHESIS => TokenDescriptor::static_text("`)`"),
            Self::AS => TokenDescriptor::static_text("`as`"),
            Self::TYPE => TokenDescriptor::static_text("`type`"),
            Self::OF => TokenDescriptor::static_text("`of`"),
            Self::VARIADIC => TokenDescriptor::static_text("`..`"),
            Self::MUTABLE => TokenDescriptor::static_text("`~`"),
            Self::DATATYPE_NONE => TokenDescriptor::static_text("`None` type"),
            Self::NONE_LITERAL => TokenDescriptor::static_text("`none`"),
            Self::DATATYPE_INT => TokenDescriptor::static_text("`Int`"),
            Self::DATATYPE_FLOAT => TokenDescriptor::static_text("`Float`"),
            Self::DATATYPE_BOOL => TokenDescriptor::static_text("`Bool`"),
            Self::DATATYPE_TRUE => TokenDescriptor::static_text("`True`"),
            Self::DATATYPE_FALSE => TokenDescriptor::static_text("`False`"),
            Self::DATATYPE_STRING => TokenDescriptor::static_text("`String`"),
            Self::DATATYPE_CHAR => TokenDescriptor::static_text("`Char`"),
            Self::BANG => TokenDescriptor::static_text("`!`"),
            Self::QUESTION_MARK => TokenDescriptor::static_text("`?`"),
            Self::NEGATIVE => TokenDescriptor::static_text("unary `-`"),
            Self::EXPONENT => TokenDescriptor::static_text("`^`"),
            Self::MULTIPLY => TokenDescriptor::static_text("`*`"),
            Self::DIVIDE => TokenDescriptor::static_text("`/`"),
            Self::MODULUS => TokenDescriptor::static_text("`%`"),
            Self::INT_DIVIDE => TokenDescriptor::static_text("`//`"),
            Self::EXPONENT_ASSIGN => TokenDescriptor::static_text("`^=`"),
            Self::MULTIPLY_ASSIGN => TokenDescriptor::static_text("`*=`"),
            Self::DIVIDE_ASSIGN => TokenDescriptor::static_text("`/=`"),
            Self::MODULUS_ASSIGN => TokenDescriptor::static_text("`%=`"),
            Self::INT_DIVIDE_ASSIGN => TokenDescriptor::static_text("`//=`"),
            Self::ADD => TokenDescriptor::static_text("`+`"),
            Self::SUBTRACT => TokenDescriptor::static_text("`-`"),
            Self::ADD_ASSIGN => TokenDescriptor::static_text("`+=`"),
            Self::SUBTRACT_ASSIGN => TokenDescriptor::static_text("`-=`"),
            Self::NOT => TokenDescriptor::static_text("`not`"),
            Self::IS => TokenDescriptor::static_text("`is`"),
            Self::LESS_THAN => TokenDescriptor::static_text("`<`"),
            Self::LESS_THAN_OR_EQUAL => TokenDescriptor::static_text("`<=`"),
            Self::GREATER_THAN => TokenDescriptor::static_text("`>`"),
            Self::GREATER_THAN_OR_EQUAL => TokenDescriptor::static_text("`>=`"),
            Self::AND => TokenDescriptor::static_text("`and`"),
            Self::OR => TokenDescriptor::static_text("`or`"),
            Self::IF => TokenDescriptor::static_text("`if`"),
            Self::ELSE => TokenDescriptor::static_text("`else`"),
            Self::RETURN => TokenDescriptor::static_text("`return`"),
            Self::RETURN_BANG => TokenDescriptor::static_text("`return!`"),
            Self::CATCH => TokenDescriptor::static_text("`catch`"),
            Self::THEN => TokenDescriptor::static_text("`then`"),
            Self::CHECKED => TokenDescriptor::static_text("`checked`"),
            Self::ASYNC => TokenDescriptor::static_text("`async`"),
            Self::LOOP => TokenDescriptor::static_text("`loop`"),
            Self::BY => TokenDescriptor::static_text("`by`"),
            Self::BREAK => TokenDescriptor::static_text("`break`"),
            Self::CONTINUE => TokenDescriptor::static_text("`continue`"),
            Self::EXCLUSIVE_RANGE => TokenDescriptor::static_text("`to`"),
            Self::AMPERSAND => TokenDescriptor::static_text("`&`"),
            Self::FAT_ARROW => TokenDescriptor::static_text("`=>`"),
            Self::WILDCARD => TokenDescriptor::static_text("`_`"),
            Self::COPY => TokenDescriptor::static_text("`copy`"),
            Self::TEMPLATE_CLOSE => TokenDescriptor::static_text("`]`"),
            Self::TEMPLATE_HEAD => TokenDescriptor::static_text("`[`"),
            Self::CHANNEL_SEND => TokenDescriptor::static_text("`>>`"),
            Self::CHANNEL_RECEIVE => TokenDescriptor::static_text("`<<`"),
            Self::YIELD => TokenDescriptor::static_text("`yield`"),
            Self::CAST => TokenDescriptor::static_text("`cast`"),
            Self::CAST_BANG => TokenDescriptor::static_text("`cast!`"),
            Self::ASSERT => TokenDescriptor::static_text("`assert`"),
            _ => TokenDescriptor::static_text("token"),
        }
    }
}

/// A source-token projection that can outlive the source tokenizer and its side stores.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiagnosticToken {
    tag_and_flags: u32,
    data: u32,
}

const _: () = assert!(std::mem::size_of::<DiagnosticToken>() == 8);

impl DiagnosticToken {
    fn new(tag: TokenTag, flags: u16, data: u32) -> Self {
        Self {
            tag_and_flags: u32::from(tag.raw()) | (u32::from(flags) << FLAGS_SHIFT),
            data,
        }
    }

    fn static_token(tag: TokenTag) -> Self {
        Self::new(tag, 0, 0)
    }

    fn string_token(tag: TokenTag, value: StringId) -> Self {
        Self::new(tag, 0, value.index())
    }

    pub(crate) fn tag(self) -> TokenTag {
        TokenTag((self.tag_and_flags & TAG_MASK) as u16)
    }

    pub(crate) fn flags(self) -> u16 {
        (self.tag_and_flags >> FLAGS_SHIFT) as u16
    }
    #[cfg(test)]
    pub(crate) fn data(self) -> u32 {
        self.data
    }

    pub(crate) fn string_id(self) -> StringId {
        StringId::from_index(self.data)
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if matches!(
            self.tag(),
            TokenTag::SYMBOL
                | TokenTag::STYLE_DIRECTIVE
                | TokenTag::STRING_SLICE_LITERAL
                | TokenTag::NUMERIC_LITERAL
                | TokenTag::RAW_STRING_LITERAL
        ) {
            self.data = remap.get(self.string_id()).index();
        }
    }

    pub(crate) fn is_whole_number(self) -> bool {
        self.tag() == TokenTag::NUMERIC_LITERAL && self.flags() & NUMERIC_KIND_MASK == 0
    }

    pub(crate) fn char_value(self) -> char {
        // A DiagnosticToken created from TokenKind always carries a valid scalar value. The
        // replacement fallback keeps a corrupt retained record renderable without panicking.
        char::from_u32(self.data).unwrap_or('\u{FFFD}')
    }

    pub(crate) fn bool_value(self) -> bool {
        self.data != 0
    }
}
impl From<TokenKind> for DiagnosticToken {
    fn from(token_kind: TokenKind) -> Self {
        match token_kind {
            TokenKind::ModuleStart => Self::static_token(TokenTag::MODULE_START),
            TokenKind::Eof => Self::static_token(TokenTag::EOF),
            TokenKind::Export => Self::static_token(TokenTag::EXPORT),
            TokenKind::Hash => Self::static_token(TokenTag::HASH),
            TokenKind::Reactive => Self::static_token(TokenTag::REACTIVE),
            TokenKind::Arrow => Self::static_token(TokenTag::ARROW),
            TokenKind::Symbol(value) => Self::string_token(TokenTag::SYMBOL, value),
            TokenKind::StyleDirective(value) => {
                Self::string_token(TokenTag::STYLE_DIRECTIVE, value)
            }
            TokenKind::StringSliceLiteral(value) => {
                Self::string_token(TokenTag::STRING_SLICE_LITERAL, value)
            }
            TokenKind::Path(_) => Self::static_token(TokenTag::PATH),
            TokenKind::NumericLiteral(value) => Self::new(
                TokenTag::NUMERIC_LITERAL,
                numeric_kind_flags(value.kind),
                value.source_text.index(),
            ),
            TokenKind::CharLiteral(value) => Self::new(TokenTag::CHAR_LITERAL, 0, value as u32),
            TokenKind::RawStringLiteral(value) => {
                Self::string_token(TokenTag::RAW_STRING_LITERAL, value)
            }
            TokenKind::BoolLiteral(value) => Self::new(TokenTag::BOOL_LITERAL, 0, value as u32),
            TokenKind::OpenCurly => Self::static_token(TokenTag::OPEN_CURLY),
            TokenKind::CloseCurly => Self::static_token(TokenTag::CLOSE_CURLY),
            TokenKind::TypeParameterBracket => Self::static_token(TokenTag::TYPE_PARAMETER_BRACKET),
            TokenKind::Newline => Self::static_token(TokenTag::NEWLINE),
            TokenKind::End => Self::static_token(TokenTag::END),
            TokenKind::StartTemplateBody => Self::static_token(TokenTag::START_TEMPLATE_BODY),
            TokenKind::Comma => Self::static_token(TokenTag::COMMA),
            TokenKind::Dot => Self::static_token(TokenTag::DOT),
            TokenKind::Colon => Self::static_token(TokenTag::COLON),
            TokenKind::DoubleColon => Self::static_token(TokenTag::DOUBLE_COLON),
            TokenKind::Assign => Self::static_token(TokenTag::ASSIGN),
            TokenKind::This => Self::static_token(TokenTag::THIS),
            TokenKind::Must => Self::static_token(TokenTag::MUST),
            TokenKind::TraitThis => Self::static_token(TokenTag::TRAIT_THIS),
            TokenKind::OpenParenthesis => Self::static_token(TokenTag::OPEN_PARENTHESIS),
            TokenKind::CloseParenthesis => Self::static_token(TokenTag::CLOSE_PARENTHESIS),
            TokenKind::As => Self::static_token(TokenTag::AS),
            TokenKind::Type => Self::static_token(TokenTag::TYPE),
            TokenKind::Of => Self::static_token(TokenTag::OF),
            TokenKind::Variadic => Self::static_token(TokenTag::VARIADIC),
            TokenKind::Mutable => Self::static_token(TokenTag::MUTABLE),
            TokenKind::DatatypeNone => Self::static_token(TokenTag::DATATYPE_NONE),
            TokenKind::NoneLiteral => Self::static_token(TokenTag::NONE_LITERAL),
            TokenKind::DatatypeInt => Self::static_token(TokenTag::DATATYPE_INT),
            TokenKind::DatatypeFloat => Self::static_token(TokenTag::DATATYPE_FLOAT),
            TokenKind::DatatypeBool => Self::static_token(TokenTag::DATATYPE_BOOL),
            TokenKind::DatatypeTrue => Self::static_token(TokenTag::DATATYPE_TRUE),
            TokenKind::DatatypeFalse => Self::static_token(TokenTag::DATATYPE_FALSE),
            TokenKind::DatatypeString => Self::static_token(TokenTag::DATATYPE_STRING),
            TokenKind::DatatypeChar => Self::static_token(TokenTag::DATATYPE_CHAR),
            TokenKind::Bang => Self::static_token(TokenTag::BANG),
            TokenKind::QuestionMark => Self::static_token(TokenTag::QUESTION_MARK),
            TokenKind::Negative => Self::static_token(TokenTag::NEGATIVE),
            TokenKind::Exponent => Self::static_token(TokenTag::EXPONENT),
            TokenKind::Multiply => Self::static_token(TokenTag::MULTIPLY),
            TokenKind::Divide => Self::static_token(TokenTag::DIVIDE),
            TokenKind::Modulus => Self::static_token(TokenTag::MODULUS),
            TokenKind::IntDivide => Self::static_token(TokenTag::INT_DIVIDE),
            TokenKind::ExponentAssign => Self::static_token(TokenTag::EXPONENT_ASSIGN),
            TokenKind::MultiplyAssign => Self::static_token(TokenTag::MULTIPLY_ASSIGN),
            TokenKind::DivideAssign => Self::static_token(TokenTag::DIVIDE_ASSIGN),
            TokenKind::ModulusAssign => Self::static_token(TokenTag::MODULUS_ASSIGN),
            TokenKind::IntDivideAssign => Self::static_token(TokenTag::INT_DIVIDE_ASSIGN),
            TokenKind::Add => Self::static_token(TokenTag::ADD),
            TokenKind::Subtract => Self::static_token(TokenTag::SUBTRACT),
            TokenKind::AddAssign => Self::static_token(TokenTag::ADD_ASSIGN),
            TokenKind::SubtractAssign => Self::static_token(TokenTag::SUBTRACT_ASSIGN),
            TokenKind::Not => Self::static_token(TokenTag::NOT),
            TokenKind::Is => Self::static_token(TokenTag::IS),
            TokenKind::LessThan => Self::static_token(TokenTag::LESS_THAN),
            TokenKind::LessThanOrEqual => Self::static_token(TokenTag::LESS_THAN_OR_EQUAL),
            TokenKind::GreaterThan => Self::static_token(TokenTag::GREATER_THAN),
            TokenKind::GreaterThanOrEqual => Self::static_token(TokenTag::GREATER_THAN_OR_EQUAL),
            TokenKind::And => Self::static_token(TokenTag::AND),
            TokenKind::Or => Self::static_token(TokenTag::OR),
            TokenKind::If => Self::static_token(TokenTag::IF),
            TokenKind::Else => Self::static_token(TokenTag::ELSE),
            TokenKind::Return => Self::static_token(TokenTag::RETURN),
            TokenKind::ReturnBang => Self::static_token(TokenTag::RETURN_BANG),
            TokenKind::Catch => Self::static_token(TokenTag::CATCH),
            TokenKind::Then => Self::static_token(TokenTag::THEN),
            TokenKind::Checked => Self::static_token(TokenTag::CHECKED),
            TokenKind::Async => Self::static_token(TokenTag::ASYNC),
            TokenKind::Cast => Self::static_token(TokenTag::CAST),
            TokenKind::CastBang => Self::static_token(TokenTag::CAST_BANG),
            TokenKind::Assert => Self::static_token(TokenTag::ASSERT),
            TokenKind::Loop => Self::static_token(TokenTag::LOOP),
            TokenKind::By => Self::static_token(TokenTag::BY),
            TokenKind::Break => Self::static_token(TokenTag::BREAK),
            TokenKind::Continue => Self::static_token(TokenTag::CONTINUE),
            TokenKind::ExclusiveRange => Self::static_token(TokenTag::EXCLUSIVE_RANGE),
            TokenKind::Ampersand => Self::static_token(TokenTag::AMPERSAND),
            TokenKind::FatArrow => Self::static_token(TokenTag::FAT_ARROW),
            TokenKind::Wildcard => Self::static_token(TokenTag::WILDCARD),
            TokenKind::Copy => Self::static_token(TokenTag::COPY),
            TokenKind::TemplateClose => Self::static_token(TokenTag::TEMPLATE_CLOSE),
            TokenKind::TemplateHead => Self::static_token(TokenTag::TEMPLATE_HEAD),
            TokenKind::ChannelSend => Self::static_token(TokenTag::CHANNEL_SEND),
            TokenKind::ChannelReceive => Self::static_token(TokenTag::CHANNEL_RECEIVE),
            TokenKind::Yield => Self::static_token(TokenTag::YIELD),
        }
    }
}

impl From<&TokenKind> for DiagnosticToken {
    fn from(token_kind: &TokenKind) -> Self {
        Self::from(token_kind.clone())
    }
}

const fn numeric_kind_flags(kind: NumericLiteralKind) -> u16 {
    match kind {
        NumericLiteralKind::WholeNumber => 0,
        NumericLiteralKind::DecimalPoint => 1,
        NumericLiteralKind::Exponent => 2,
    }
}
