//! Canonical synthetic initializer owner tests.
//!
//! WHAT: proves Markdown and Moth-template wrappers build a transient `SourceTokens` owner
//! with the parser's template tokens and the body's own payloads.
//! WHY: fold must parse synthetic content from a canonical owner rather than a `FileTokens`
//! vector.

use crate::compiler_frontend::headers::synthetic_content_header::materialize_synthetic_content_initializer;
use crate::compiler_frontend::headers::types::SyntheticContentPayload;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, Token, TokenIndex, TokenKind, TokenRange, TokenTag,
};

#[test]
fn rendered_html_owner_is_one_interned_string_token() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let html = strings.intern("<p>hi</p>");
    let owner = materialize_synthetic_content_initializer(
        SyntheticContentPayload::RenderedHtml(html),
        None,
        TokenRange::from_raw(source, 0, 0).expect("empty markdown range is valid"),
    )
    .expect("rendered HTML owner should build");

    assert_eq!(owner.source(), source);
    assert_eq!(owner.len(), 1);
    let token = owner
        .token(TokenIndex::try_from_index(0).expect("first token fits"))
        .expect("rendered HTML owner has one token");
    assert_eq!(token.tag(), TokenTag::STRING_SLICE_LITERAL);
    assert_eq!(token.string_id(), Some(html));
    assert_eq!(token.span(), LocalSpan::source_start());
    assert!(owner.numeric_literal_store().is_empty());
}

#[test]
fn moth_template_owner_wraps_the_body_window_in_markdown_template_tokens() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let markdown = strings.intern("md");
    let mut spans = ExtendedSpanBuilder::new();
    let body_span = LocalSpan::exact(4, 4, &mut spans).expect("authored body span fits");
    let body_owner = SourceTokens::try_from_tokens(
        source,
        vec![
            Token::new(
                TokenKind::StringSliceLiteral(strings.intern("hello")),
                body_span,
            ),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
        NumericLiteralStore::with_source(source),
        PathSyntaxTable::with_source(source),
    )
    .expect("template body fixture should satisfy canonical ownership checks");
    let body_range = TokenRange::from_raw(source, 0, 1).expect("body range excludes EOF");

    let owner = materialize_synthetic_content_initializer(
        SyntheticContentPayload::MothTemplate {
            markdown_directive: markdown,
        },
        Some(&body_owner),
        body_range,
    )
    .expect("Moth template wrapper owner should build");

    assert_eq!(owner.source(), source);
    assert_eq!(owner.len(), 5);
    let tags: Vec<TokenTag> = (0..owner.len())
        .map(|index| {
            owner
                .token(TokenIndex::try_from_index(index).expect("wrapper index fits"))
                .expect("wrapper token exists")
                .tag()
        })
        .collect();
    assert_eq!(
        tags,
        vec![
            TokenTag::TEMPLATE_HEAD,
            TokenTag::STYLE_DIRECTIVE,
            TokenTag::START_TEMPLATE_BODY,
            TokenTag::STRING_SLICE_LITERAL,
            TokenTag::TEMPLATE_CLOSE,
        ]
    );
    let directive = owner
        .token(TokenIndex::try_from_index(1).expect("directive index fits"))
        .expect("style directive exists");
    assert_eq!(directive.string_id(), Some(markdown));
    assert_eq!(directive.span(), LocalSpan::source_start());
    let body = owner
        .token(TokenIndex::try_from_index(3).expect("body index fits"))
        .expect("copied body token exists");
    assert_eq!(body.span(), body_span);
    assert_eq!(body.string_id(), Some(strings.intern("hello")));
}

#[test]
fn moth_template_owner_densifies_numeric_rows_from_the_body_window() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let markdown = strings.intern("md");
    let first = NumericLiteralToken::test_new("1", &mut strings);
    let second = NumericLiteralToken::test_new("2", &mut strings);
    let mut numeric_store = NumericLiteralStore::with_source(source);
    numeric_store.push(first.clone());
    numeric_store.push(second.clone());
    let mut spans = ExtendedSpanBuilder::new();
    let body_span = LocalSpan::exact(2, 1, &mut spans).expect("authored numeric span fits");
    let body_owner = SourceTokens::try_from_tokens(
        source,
        vec![
            Token::new(TokenKind::NumericLiteral(first), LocalSpan::source_start()),
            Token::new(TokenKind::NumericLiteral(second.clone()), body_span),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
        numeric_store,
        PathSyntaxTable::with_source(source),
    )
    .expect("numeric body fixture should satisfy canonical ownership checks");
    let body_range = TokenRange::from_raw(source, 1, 2).expect("window is the second numeric");

    let owner = materialize_synthetic_content_initializer(
        SyntheticContentPayload::MothTemplate {
            markdown_directive: markdown,
        },
        Some(&body_owner),
        body_range,
    )
    .expect("numeric wrapper owner should build");

    assert_eq!(owner.numeric_literal_store().len(), 1);
    let copied = owner
        .token(TokenIndex::try_from_index(3).expect("copied numeric index fits"))
        .expect("copied numeric token exists");
    assert_eq!(copied.tag(), TokenTag::NUMERIC_LITERAL);
    assert_eq!(copied.span(), body_span);
    let numeric_id = copied
        .shape()
        .numeric_literal_id()
        .expect("copied numeric keeps a dense handle");
    assert_eq!(numeric_id.raw(), 1);
    assert_eq!(
        copied
            .numeric_literal()
            .expect("copied numeric row is readable")
            .expect("copied numeric row is present")
            .source_text,
        second.source_text
    );
}

#[test]
fn moth_template_owner_shares_the_donor_path_table_for_copied_path_tokens() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let markdown = strings.intern("md");
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_row = path_syntax
        .try_push_for_source(PathId::ROOT, source, LocalSpan::source_start())
        .expect("path row should fit");
    let body_owner = SourceTokens::try_from_tokens(
        source,
        vec![
            Token::new(TokenKind::Path(path_row), LocalSpan::source_start()),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
        NumericLiteralStore::with_source(source),
        path_syntax,
    )
    .expect("path body fixture should satisfy canonical ownership checks");
    let donor_table = body_owner
        .path_syntax_arc()
        .expect("donor body must publish its path table");
    let body_range = TokenRange::from_raw(source, 0, 1).expect("body range excludes EOF");

    let owner = materialize_synthetic_content_initializer(
        SyntheticContentPayload::MothTemplate {
            markdown_directive: markdown,
        },
        Some(&body_owner),
        body_range,
    )
    .expect("path wrapper owner should build");

    let copied = owner
        .token(TokenIndex::try_from_index(3).expect("copied path index fits"))
        .expect("copied path token exists");
    assert_eq!(copied.tag(), TokenTag::PATH);
    assert_eq!(copied.path_syntax_id(), Some(path_row));
    let copied_path = copied
        .path_syntax()
        .expect("copied path row is readable")
        .expect("copied path row is present");
    assert_eq!(copied_path.root, PathId::ROOT);
    let wrapper_table = owner
        .path_syntax_arc()
        .expect("wrapper must attach a path table");
    assert!(std::sync::Arc::ptr_eq(&donor_table, &wrapper_table));
}
