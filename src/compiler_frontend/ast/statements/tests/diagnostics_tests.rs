//! Statement-position diagnostic span tests.
//!
//! WHAT: verifies statement dispatch diagnostics retain the authored token span alongside the
//!       existing source location.
//! WHY: the legacy line/column interval remains a migration bridge, while exact UTF-8 byte spans
//!      must follow the token that owns each statement-position error.

use super::{UnexpectedScopeCloseContext, unexpected_scope_close, unexpected_statement_token};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidStatementPositionReason,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};

fn tokenize_source(source: &str) -> (FileTokens, StringTable) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("statement diagnostic fixture should tokenize");

    (file_tokens, string_table)
}

fn move_to_token(tokens: &mut FileTokens, kind: TokenKind) {
    tokens.index = tokens
        .tokens
        .iter()
        .position(|token| token.kind == kind)
        .expect("statement diagnostic fixture should contain the requested token");
}

#[test]
fn unexpected_statement_token_retains_exact_multibyte_span() {
    let (mut tokens, mut string_table) = tokenize_source("value = \"é\"\n,\n");
    move_to_token(&mut tokens, TokenKind::Comma);
    let token_span = tokens.current_token().span;

    let diagnostic = unexpected_statement_token(&tokens, &mut string_table);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedComma
        }
    ));
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, token_span))
    );
}

#[test]
fn unexpected_scope_close_retains_exact_multibyte_span() {
    let (mut tokens, _string_table) = tokenize_source("value = \"é\";\n");
    move_to_token(&mut tokens, TokenKind::End);
    let token_span = tokens.current_token().span;

    let diagnostic = unexpected_scope_close(UnexpectedScopeCloseContext::Expression, &tokens);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedScopeCloseInExpression
        }
    ));
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, token_span))
    );
}
