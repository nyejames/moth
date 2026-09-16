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
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenCursor, TokenKind};

pub(crate) fn cursor_current_span(cursor: &TokenCursor<'_>) -> Option<SourceSpan> {
    cursor.current().map(|token| token.source_span())
}

pub(crate) mod type_syntax;

/// Short-lived canonical adapter used by declaration/sig/type parsers.
///
/// WHAT: preserves the old parser ergonomics (`current_token_kind`, `advance` and newline
/// skipping) while the backing storage is a checked `TokenCursor` over `SourceTokens`.
/// WHY: declaration syntax still needs the exact `TokenKind` payload at a few diagnostics and
/// semantic handoffs, but no parser shell may retain the materialised values.
pub(crate) struct DeclarationCursor<'a> {
    cursor: TokenCursor<'a>,
    /// Parser position for a cursor without a compatibility lane.
    ///
    /// Contiguous cursors retain their canonical absolute position; segmented cursors expose the
    /// dense sequence position used by parser APIs.
    ///
    /// When a parser adapter supplies `compatibility_tokens`, `index` becomes that adapter's
    /// relative position so bounded and segmented adapters share the legacy parser contract.
    pub(crate) index: usize,
    /// End of the active parser view. This is canonical for a bare contiguous cursor, logical for
    /// a bare segmented cursor, and adapter-relative when a compatibility lane is present.
    pub(crate) length: usize,
    source: crate::compiler_frontend::source::SourceId,
    current_kind: TokenKind,
    current_token: Option<Token>,
    compatibility_tokens: Option<&'a [Token]>,
    compatibility_base: usize,
    compatibility_end: usize,
}

impl<'a> DeclarationCursor<'a> {
    pub(crate) fn new(
        cursor: TokenCursor<'a>,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        // SourceTokens validates every compact payload at construction/publication. Adapters
        // validate their legacy lane when they are built, so validating the entire remaining
        // range here would repeat that work for every declaration in a file.
        let current_token = cursor.current_token_owned()?;
        let source = cursor.range().source();
        let index = cursor.parser_position();
        let length = cursor.parser_length();
        let current_kind = current_token
            .as_ref()
            .map(|token| token.kind.clone())
            .unwrap_or(TokenKind::Eof);
        Ok(Self {
            cursor,
            index,
            length,
            source,
            current_kind,
            current_token,
            compatibility_tokens: None,
            compatibility_base: 0,
            compatibility_end: 0,
        })
    }

    /// Build a declaration cursor from a parser stream's canonical provenance and compatibility
    /// lane. The lane is borrowed only for this parser handoff, never retained by a syntax shell.
    pub(crate) fn from_file_tokens(
        token_stream: &'a crate::compiler_frontend::tokenizer::tokens::FileTokens,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        let cursor = token_stream.canonical_cursor_from_current()?;
        Self::with_compatibility_bounds(
            cursor,
            &token_stream.tokens,
            token_stream.index,
            token_stream.tokens.len(),
        )
    }

    pub(crate) fn with_compatibility_bounds(
        cursor: TokenCursor<'a>,
        compatibility_tokens: &'a [Token],
        compatibility_index: usize,
        compatibility_end: usize,
    ) -> Result<Self, crate::compiler_frontend::compiler_errors::CompilerError> {
        if compatibility_index > compatibility_end || compatibility_end > compatibility_tokens.len()
        {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "declaration compatibility cursor bounds are outside its token lane",
                ),
            );
        }
        let mut declaration_cursor = Self::new(cursor)?;
        let local_position = cursor.compatibility_position_from_start()?;
        let compatibility_base =
            compatibility_index
                .checked_sub(local_position)
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "declaration compatibility cursor position precedes its canonical range",
                    )
                })?;
        declaration_cursor.compatibility_tokens = Some(compatibility_tokens);
        declaration_cursor.compatibility_base = compatibility_base;
        declaration_cursor.compatibility_end = compatibility_end;
        declaration_cursor.refresh();
        Ok(declaration_cursor)
    }

    pub(crate) fn canonical_cursor(&self) -> TokenCursor<'a> {
        self.cursor
    }

    pub(crate) fn canonical_cursor_mut(&mut self) -> &mut TokenCursor<'a> {
        &mut self.cursor
    }

    pub(crate) fn compatibility_tokens(&self) -> Option<&'a [Token]> {
        self.compatibility_tokens
    }
    pub(crate) fn compatibility_index(&self) -> Option<usize> {
        self.compatibility_tokens.map(|_| self.index)
    }
    pub(crate) fn token_at(&self, index: usize) -> Option<Token> {
        let lower_bound = self.compatibility_tokens.map_or_else(
            || {
                if self.cursor.is_segmented() {
                    0
                } else {
                    self.cursor.range().start().index()
                }
            },
            |_| self.compatibility_base,
        );
        if index < lower_bound || index >= self.length {
            return None;
        }
        if let Some(tokens) = self.compatibility_tokens {
            return tokens.get(index).cloned();
        }

        let token = if self.cursor.is_segmented() {
            self.cursor.parser_token_at(index)?
        } else {
            let index =
                crate::compiler_frontend::tokenizer::tokens::TokenIndex::try_from_index(index)?;
            self.cursor.source_tokens().token(index).ok()?
        };
        token
            .to_token_kind()
            .ok()
            .map(|kind| Token::new(kind, token.span()))
    }
    pub(crate) fn token_kind_at(&self, index: usize) -> Option<TokenKind> {
        self.token_at(index).map(|token| token.kind)
    }

    pub(crate) fn position(&self) -> usize {
        self.index
    }

    pub(crate) fn refresh(&mut self) {
        if let Some(tokens) = self.compatibility_tokens {
            let local_position = self
                .cursor
                .compatibility_position_from_start()
                .expect("validated declaration cursor must have a valid compatibility position");
            self.index = self
                .compatibility_base
                .checked_add(local_position)
                .expect("declaration compatibility cursor index must fit usize");
            self.length = self.compatibility_end;
            self.current_token = (self.index < self.compatibility_end)
                .then(|| tokens.get(self.index))
                .flatten()
                .cloned();
        } else {
            self.index = self.cursor.parser_position();
            self.length = self.cursor.parser_length();
            self.current_token = self.cursor.current().map(|token| {
                let kind = token
                    .to_token_kind()
                    .expect("validated declaration cursor token");
                Token::new(kind, token.span())
            });
        }
        self.current_kind = self
            .current_token
            .as_ref()
            .map(|token| token.kind.clone())
            .unwrap_or(TokenKind::Eof);
    }

    pub(crate) fn current_token_kind(&self) -> &TokenKind {
        &self.current_kind
    }

    pub(crate) fn current_span(&self) -> Option<SourceSpan> {
        self.current_token
            .as_ref()
            .map(|token| SourceSpan::new(self.source, token.span))
    }

    pub(crate) fn peek_next_token(&self) -> Option<TokenKind> {
        if let Some(tokens) = self.compatibility_tokens {
            let next_index = self.index.checked_add(1)?;
            return (next_index < self.compatibility_end)
                .then(|| tokens.get(next_index))
                .flatten()
                .map(|token| token.kind.clone());
        }
        let token = if self.cursor.is_segmented() {
            self.cursor.parser_peek_next()
        } else {
            self.cursor.peek_next()
        }?;
        Some(
            token
                .to_token_kind()
                .expect("validated declaration cursor token"),
        )
    }

    pub(crate) fn advance(&mut self) {
        let _ = self.cursor.advance();
        self.refresh();
    }

    pub(crate) fn skip_newlines(&mut self) {
        while self.current_kind == TokenKind::Newline {
            self.advance();
        }
    }
}
