//! Statement-position diagnostics retain exact source-owned byte spans.
//!
//! WHY: the plain diagnostic lane must preserve the token's authored span,
//! including multibyte source, rather than reconstructing line/column data.

use super::{UnexpectedScopeCloseContext, unexpected_scope_close, unexpected_statement_token};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidStatementPositionReason,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};

fn tokenize_source(source: &str) -> (LexedSource, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("statement diagnostic fixture should tokenize");

    (lexed, string_table)
}

fn token_position(tokens: &LexedSource, tag: TokenTag) -> usize {
    (0..tokens.tokens.len())
        .find_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            let token = tokens.tokens.token(index).ok()?;
            (token.tag() == tag).then_some(index.index())
        })
        .expect("statement diagnostic fixture should contain the requested token")
}

#[test]
fn unexpected_statement_token_retains_exact_multibyte_span() {
    let (tokens, mut string_table) = tokenize_source("value = \"é\"\n,\n");
    let token_position = token_position(&tokens, TokenTag::COMMA);
    let owner = &tokens.tokens;
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut cursor = AstCursor::from_source_tokens(owner, range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(token_position)
        .expect("the canonical cursor must seek to the requested token");
    assert_eq!(cursor.current_tag(), TokenTag::COMMA);
    let token_span = cursor.current_span();
    let diagnostic = unexpected_statement_token(&cursor, &mut string_table);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedComma
        }
    ));
    assert_eq!(diagnostic.primary_span, Some(token_span));
}
#[test]
fn unexpected_scope_close_retains_exact_multibyte_span() {
    let (tokens, _string_table) = tokenize_source("value = \"é\";\n");
    let token_position = token_position(&tokens, TokenTag::END);
    let owner = &tokens.tokens;
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut cursor = AstCursor::from_source_tokens(owner, range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(token_position)
        .expect("the canonical cursor must seek to the requested token");
    assert_eq!(cursor.current_tag(), TokenTag::END);
    let token_span = cursor.current_span();
    let diagnostic = unexpected_scope_close(UnexpectedScopeCloseContext::Expression, &cursor);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedScopeCloseInExpression
        }
    ));
    assert_eq!(diagnostic.primary_span, Some(token_span));
}
