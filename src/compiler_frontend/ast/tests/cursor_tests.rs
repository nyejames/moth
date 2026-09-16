//! AST cursor bounded-read regression tests.
//!
//! WHAT: asserts that ordinary parser reads over a bounded canonical range respect the
//! active view, while explicit raw source-index inspection stays source-wide.
//! WHY: bounded parsing must not inspect a neighbouring declaration merely because its
//! token is addressable in the same source.

use std::sync::Arc;

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, Token, TokenKind, TokenRange,
};

#[test]
fn bounded_canonical_reads_respect_range_and_offset_limits() {
    let source = SourceId::COMPILATION_ROOT;
    let owner = Arc::new(
        SourceTokens::try_from_tokens(
            source,
            vec![
                Token::new(TokenKind::ModuleStart, LocalSpan::source_start()),
                Token::new(TokenKind::BoolLiteral(true), LocalSpan::source_start()),
                Token::new(TokenKind::BoolLiteral(false), LocalSpan::source_start()),
                Token::new(TokenKind::Eof, LocalSpan::source_start()),
            ],
            NumericLiteralStore::with_source(source),
            PathSyntaxTable::with_source(source),
            TokenStats::default(),
        )
        .expect("cursor fixture should satisfy canonical ownership checks"),
    );
    let range = TokenRange::from_raw(source, 1, 3).expect("fixture range should be ordered");
    let mut cursor =
        AstCursor::from_source_tokens(&owner, None, range).expect("bounded cursor should build");

    assert_eq!(cursor.position(), 1);
    assert_eq!(cursor.length(), 3);
    assert!(cursor.token_at(0).is_none());
    assert_eq!(cursor.token_kind_at(1), Some(TokenKind::BoolLiteral(true)));
    assert_eq!(cursor.token_kind_at(2), Some(TokenKind::BoolLiteral(false)));
    assert!(cursor.token_at(3).is_none());
    assert!(cursor.token_at(4).is_none());
    assert_eq!(
        cursor.raw_token_kind_at(0),
        Some(TokenKind::ModuleStart),
        "raw source-index inspection remains explicitly source-wide"
    );
    assert_eq!(
        cursor.token_kind_at_offset(0),
        Some(TokenKind::BoolLiteral(true))
    );
    assert_eq!(
        cursor.token_kind_at_offset(1),
        Some(TokenKind::BoolLiteral(false))
    );
    assert!(cursor.token_kind_at_offset(2).is_none());
    let active_limit = cursor
        .set_limit(2)
        .expect("a limit inside the bounded range should be accepted");
    assert!(
        cursor
            .nested_cursor(TokenRange::from_raw(source, 1, 3).unwrap())
            .is_err(),
        "nested parser views must not escape an active parent limit"
    );
    cursor.restore_limit(active_limit);
}
#[test]
fn segmented_nested_cursor_translates_active_limit() {
    let source = SourceId::COMPILATION_ROOT;
    let mut owner = FileTokens::new(
        crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
        source,
        (0..5)
            .map(|index| {
                Token::new(
                    TokenKind::BoolLiteral(index % 2 == 0),
                    LocalSpan::source_start(),
                )
            })
            .collect(),
    );
    let segments = [
        TokenRange::from_raw(source, 0, 1).unwrap(),
        TokenRange::from_raw(source, 3, 5).unwrap(),
    ];
    let sequence = owner
        .try_register_token_sequence(&segments)
        .expect("segmented fixture should register");
    owner.freeze_path_syntax_for_test();
    let mut adapter = FileTokens::new_bounded_sequence_substream(
        &owner,
        sequence,
        crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
    )
    .expect("segmented adapter should retain canonical provenance");
    let mut cursor =
        AstCursor::from_file_tokens(&mut adapter).expect("segmented AST cursor should build");
    cursor.advance();
    cursor
        .set_limit(2)
        .expect("the active limit should be in dense sequence coordinates");

    let child = cursor
        .nested_cursor(TokenRange::from_raw(source, 3, 4).unwrap())
        .expect("a child ending at the dense limit should be accepted");
    assert_eq!(child.position(), 3);
    assert_eq!(
        child.token_kind_at(3),
        Some(TokenKind::BoolLiteral(false)),
        "the child keeps its raw contiguous range after translation",
    );
    assert!(child.token_at(4).is_none());
    assert!(
        cursor
            .nested_cursor(TokenRange::from_raw(source, 3, 5).unwrap())
            .is_err(),
        "a child crossing the dense limit must be rejected",
    );
}
