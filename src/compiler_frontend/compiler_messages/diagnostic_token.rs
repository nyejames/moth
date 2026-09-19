//! Compact source-token facts retained by durable compiler diagnostics.
//!
//! A diagnostic only needs a token's stable class plus one immediate payload for
//! rendering. This projection intentionally does not retain the live tokenizer
//! enum or any source-owned side-store handles that would make a diagnostic
//! depend on the source token buffer's lifetime.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::{
    TokenDescriptorPayload, TokenRef, TokenTag, TokenViewError, numeric_kind_flags,
};

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
    pub(crate) fn from_string_tag(tag: TokenTag, value: StringId) -> Self {
        debug_assert!(matches!(
            tag.descriptor().payload(),
            TokenDescriptorPayload::Symbol
                | TokenDescriptorPayload::StyleDirective
                | TokenDescriptorPayload::StringLiteral
                | TokenDescriptorPayload::RawStringLiteral
        ));
        Self::string_token(tag, value)
    }

    /// Project a static/path descriptor without touching cold payload rows.
    /// WHAT: builds the retained token for expected delimiters/keywords whose
    ///       descriptor payload is `Static` or `Path`.
    /// WHY: expected-token diagnostics name a stable spelling; path handles and
    ///      string identities are never retained for the expected side.
    pub(crate) fn from_static_tag(tag: TokenTag) -> Self {
        debug_assert!(
            matches!(
                tag.descriptor().payload(),
                TokenDescriptorPayload::Static | TokenDescriptorPayload::Path
            ),
            "diagnostic expected token must be a static/path descriptor"
        );
        Self::static_token(tag)
    }

    /// Checked projection from a short-lived canonical token view.
    ///
    /// WHAT: copies only the stable tag plus the one immediate payload the
    ///       renderer needs, borrowing cold numeric rows instead of cloning them.
    /// WHY: diagnostics must outlive the source token buffer without retaining
    ///      its lifetime, cloning the live token view, or materializing path tables.
    ///      `Path` keeps only its stable tag; the dense path handle is dropped.
    pub(crate) fn try_from_token_ref(token: TokenRef<'_>) -> Result<Self, TokenViewError> {
        let tag = token.tag();
        match tag.descriptor().payload() {
            TokenDescriptorPayload::Static => {
                let shape = token.shape();
                if shape.flags() != 0 || shape.data() != 0 {
                    return Err(TokenViewError::MalformedNumericHandle);
                }
                Ok(Self::static_token(tag))
            }
            TokenDescriptorPayload::Path => {
                token
                    .path_syntax_id()
                    .ok_or(TokenViewError::MalformedPathHandle)?;
                Ok(Self::static_token(tag))
            }
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => {
                let value = token
                    .string_id()
                    .ok_or(TokenViewError::MalformedNumericHandle)?;
                Ok(Self::string_token(tag, value))
            }
            TokenDescriptorPayload::NumericLiteral => {
                let literal = token
                    .numeric_literal()?
                    .ok_or(TokenViewError::MalformedNumericHandle)?;
                Ok(Self::new(
                    tag,
                    numeric_kind_flags(literal.kind),
                    literal.source_text.index(),
                ))
            }
            TokenDescriptorPayload::CharLiteral => {
                let value = token
                    .char_value()
                    .ok_or(TokenViewError::MalformedNumericHandle)?;
                Ok(Self::new(tag, 0, value as u32))
            }
            TokenDescriptorPayload::BoolLiteral => {
                let value = token
                    .bool_value()
                    .ok_or(TokenViewError::MalformedNumericHandle)?;
                Ok(Self::new(tag, 0, u32::from(value)))
            }
        }
    }

    /// Infallible projection over validated source token stores.
    ///
    /// WHAT: copies the compact tag/payload pair from the validated view.
    /// WHY: `SourceTokens` validates every shape and cold-store handle at
    ///      construction/publication, so the checked views above hold. Callers
    ///      with unvalidated views must use `try_from_token_ref` and map the
    ///      `TokenViewError` into the compiler-invariant error lane.
    pub(crate) fn from_token_ref(token: TokenRef<'_>) -> Self {
        Self::try_from_token_ref(token)
            .expect("validated source token view must project to a diagnostic token")
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
        // A projected token always carries a valid scalar value when built through the
        // checked projection. The fallback below keeps a corrupt retained record
        // renderable without panicking.
        char::from_u32(self.data).unwrap_or('\u{FFFD}')
    }

    pub(crate) fn bool_value(self) -> bool {
        self.data != 0
    }
}
