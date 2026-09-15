//! Compact source-token facts retained by durable compiler diagnostics.
//!
//! A diagnostic only needs a token's stable class plus one immediate payload for
//! rendering. This projection intentionally does not retain the live tokenizer
//! enum or any source-owned side-store handles that would make a diagnostic
//! depend on the source token buffer's lifetime.

use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::{TokenDescriptorPayload, TokenKind, TokenTag};

const TAG_MASK: u32 = u16::MAX as u32;
const FLAGS_SHIFT: u32 = u16::BITS;

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
        TokenTag::from_raw_unchecked((self.tag_and_flags & TAG_MASK) as u16)
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
        // The remap set is derived from the shared descriptor authority so it cannot drift
        // from the projection below: every payload that carries a `StringId` is remapped.
        if matches!(
            self.tag().descriptor().payload(),
            TokenDescriptorPayload::Symbol
                | TokenDescriptorPayload::StyleDirective
                | TokenDescriptorPayload::StringLiteral
                | TokenDescriptorPayload::NumericLiteral
                | TokenDescriptorPayload::RawStringLiteral
        ) {
            self.data = remap.get(self.string_id()).index();
        }
    }

    pub(crate) fn is_whole_number(self) -> bool {
        self.tag() == TokenTag::NUMERIC_LITERAL
            && self.flags() & TokenTag::NUMERIC_LITERAL.allowed_flags() == 0
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
        let tag = token_kind.token_tag();

        // The tag and payload kind come from the shared tokenizer schema. This match is retained
        // only at the projection boundary because dynamic values need typed extraction before
        match tag.descriptor().payload() {
            TokenDescriptorPayload::Static | TokenDescriptorPayload::Path => {
                Self::static_token(tag)
            }
            TokenDescriptorPayload::Symbol => match token_kind {
                TokenKind::Symbol(value) => Self::string_token(tag, value),
                _ => unreachable!("token schema symbol payload does not match TokenKind"),
            },
            TokenDescriptorPayload::StyleDirective => match token_kind {
                TokenKind::StyleDirective(value) => Self::string_token(tag, value),
                _ => unreachable!("token schema style payload does not match TokenKind"),
            },
            TokenDescriptorPayload::StringLiteral => match token_kind {
                TokenKind::StringSliceLiteral(value) => Self::string_token(tag, value),
                _ => unreachable!("token schema string payload does not match TokenKind"),
            },
            TokenDescriptorPayload::NumericLiteral => match token_kind {
                TokenKind::NumericLiteral(value) => Self::new(
                    tag,
                    numeric_kind_flags(value.kind),
                    value.source_text.index(),
                ),
                _ => unreachable!("token schema numeric payload does not match TokenKind"),
            },
            TokenDescriptorPayload::CharLiteral => match token_kind {
                TokenKind::CharLiteral(value) => Self::new(tag, 0, value as u32),
                _ => unreachable!("token schema character payload does not match TokenKind"),
            },
            TokenDescriptorPayload::RawStringLiteral => match token_kind {
                TokenKind::RawStringLiteral(value) => Self::string_token(tag, value),
                _ => unreachable!("token schema raw-string payload does not match TokenKind"),
            },
            TokenDescriptorPayload::BoolLiteral => match token_kind {
                TokenKind::BoolLiteral(value) => Self::new(tag, 0, value as u32),
                _ => unreachable!("token schema bool payload does not match TokenKind"),
            },
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
