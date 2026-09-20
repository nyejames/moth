//! Reusable declaration shell parsers shared between the header stage and AST.
//!
//! WHAT: `declaration_syntax` owns the syntax parsers that build declaration shells (unresolved
//! structural metadata) for both header top-level declarations and AST body-local declarations.
//! Header parsing stores shells; AST resolves shells.
//!
//! WHY: centralising shell parsing prevents header and AST from rediscovering the same syntax
//! rules independently, and makes the shell/resolution boundary explicit.
//!
//! AST stage and headers also parse function signatures and type annotations the same way,
//! so that logic is centralized in this module.

pub(crate) mod binding_mode;
pub(crate) mod build_config_contract;
pub(crate) mod choice;
pub(crate) mod declaration_shell;
pub(crate) mod generic_parameters;
pub(crate) mod record_body;
pub(crate) mod signature_members;
pub(crate) mod r#struct;

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{
    TokenCursor, TokenPayloadOrigin, TokenRef, TokenTag,
};

pub(crate) fn cursor_current_span(cursor: &TokenCursor<'_>) -> Option<SourceSpan> {
    cursor.current().map(|token| token.source_span())
}

pub(crate) mod type_syntax;

/// Short-lived canonical adapter used by declaration/sig/type parsers.
///
/// WHAT: preserves parser ergonomics (`current_tag`, `advance` and newline skipping) while the
/// backing storage is a checked `TokenCursor` over `SourceTokens`.
/// WHY: classification uses `TokenTag`; typed payloads come from the canonical `TokenRef` view.
pub(crate) struct DeclarationCursor<'a> {
    cursor: TokenCursor<'a>,
    /// Canonical parser position exposed for the declaration shell handoff.
    pub(crate) index: usize,
    /// Exclusive end of the active canonical parser view.
    pub(crate) length: usize,
    current_tag: TokenTag,
    /// Frozen donor identity for retained foreign bodies. Payload IDs are translated only by
    /// the explicit `*_in` accessors at the requester read boundary.
    payload_origin: Option<TokenPayloadOrigin<'a>>,
}

impl<'a> DeclarationCursor<'a> {
    pub(crate) fn new(
        cursor: TokenCursor<'a>,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        let index = cursor.parser_position();
        let length = cursor.parser_length();
        let current_tag = cursor.current().map(TokenRef::tag).unwrap_or(TokenTag::EOF);
        Ok(Self {
            cursor,
            index,
            length,
            current_tag,
            payload_origin: None,
        })
    }

    pub(crate) fn with_payload_origin(mut self, origin: Option<TokenPayloadOrigin<'a>>) -> Self {
        self.payload_origin = origin;
        self
    }

    pub(crate) fn payload_origin(&self) -> Option<TokenPayloadOrigin<'a>> {
        self.payload_origin
    }

    pub(crate) fn canonical_cursor(&self) -> TokenCursor<'a> {
        self.cursor
    }

    pub(crate) fn canonical_cursor_mut(&mut self) -> &mut TokenCursor<'a> {
        &mut self.cursor
    }

    pub(crate) fn token_tag_at(&self, index: usize) -> Option<TokenTag> {
        if index < self.cursor.parser_window_start() || index >= self.length {
            return None;
        }
        self.cursor.parser_token_at(index).map(TokenRef::tag)
    }
    /// Read a test-only indexed symbol payload through the requester string-table domain.
    #[cfg(test)]
    pub(crate) fn token_string_id_at_in(
        &self,
        index: usize,
        destination: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Result<Option<StringId>, crate::compiler_frontend::compiler_errors::CompilerError> {
        if index < self.cursor.parser_window_start() || index >= self.length {
            return Ok(None);
        }
        let token = self.cursor.parser_token_at(index).ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "indexed symbol token is missing its canonical source view",
            )
        })?;
        self.string_id_in(token, destination)
    }

    pub(crate) fn position(&self) -> usize {
        self.index
    }

    pub(crate) fn refresh(&mut self) {
        self.index = self.cursor.parser_position();
        self.length = self.cursor.parser_length();
        self.current_tag = self
            .cursor
            .current()
            .map(TokenRef::tag)
            .unwrap_or(TokenTag::EOF);
    }

    pub(crate) fn current_tag(&self) -> TokenTag {
        self.current_tag
    }

    pub(crate) fn current_span(&self) -> Option<SourceSpan> {
        self.cursor.current().map(TokenRef::source_span)
    }

    pub(crate) fn peek_next_tag(&self) -> Option<TokenTag> {
        let token = if self.cursor.is_segmented() {
            self.cursor.parser_peek_next()
        } else {
            self.cursor.peek_next()
        }?;
        Some(token.tag())
    }

    /// Borrow the current canonical token without materialising a wide payload.
    pub(crate) fn current_token_ref(&self) -> Option<TokenRef<'a>> {
        self.cursor.current()
    }

    fn string_id_in(
        &self,
        token: TokenRef<'a>,
        destination: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Result<Option<StringId>, crate::compiler_frontend::compiler_errors::CompilerError> {
        let Some(id) = token.string_id() else {
            return Ok(None);
        };
        let Some(origin) = self.payload_origin else {
            return Ok(Some(id));
        };
        let spelling = origin.strings.try_resolve(id).ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                "donor string handle {id:?} is outside its issuing frozen table"
            ))
        })?;
        Ok(Some(destination.intern(spelling)))
    }

    /// Read the current symbol payload in the requester's string-table domain.
    pub(crate) fn current_string_id_in(
        &self,
        destination: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Result<Option<StringId>, crate::compiler_frontend::compiler_errors::CompilerError> {
        if self.current_tag != TokenTag::SYMBOL {
            return Ok(None);
        }
        let token = self.current_token_ref().ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "symbol token is missing its canonical source view",
            )
        })?;
        if token.string_id().is_none() {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "canonical symbol token is missing its payload",
                ),
            );
        }
        self.string_id_in(token, destination)
    }

    /// Read the next symbol payload in the requester's string-table domain.
    pub(crate) fn peek_next_string_id_in(
        &self,
        destination: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Result<Option<StringId>, crate::compiler_frontend::compiler_errors::CompilerError> {
        if self.peek_next_tag() != Some(TokenTag::SYMBOL) {
            return Ok(None);
        }
        let token = self.peek_next_token_ref().ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "next symbol token is missing its canonical source view",
            )
        })?;
        if token.string_id().is_none() {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "canonical next symbol token is missing its payload",
                ),
            );
        }
        self.string_id_in(token, destination)
    }

    /// Borrow the next canonical token without materialising a wide payload.
    pub(crate) fn peek_next_token_ref(&self) -> Option<TokenRef<'a>> {
        if self.cursor.is_segmented() {
            self.cursor.parser_peek_next()
        } else {
            self.cursor.peek_next()
        }
    }

    pub(crate) fn advance(&mut self) {
        let _ = self.cursor.advance();
        self.refresh();
    }

    pub(crate) fn skip_newlines(&mut self) {
        while self.current_tag == TokenTag::NEWLINE {
            self.advance();
        }
    }
}
