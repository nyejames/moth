//! AST cursor bounded-read regression tests.
//!
//! WHAT: asserts that ordinary parser reads over a bounded canonical range respect the
//! active view, while the canonical owner still addresses neighbouring tokens source-wide.
//! WHY: bounded parsing must not inspect a neighbouring declaration merely because its
//! token is addressable in the same source.

use std::sync::Arc;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, Token, TokenIndex, TokenKind, TokenRange, TokenTag,
};

/// Assert a bounded cursor exposes a bool literal with the expected payload at `index`.
fn assert_bounded_bool(cursor: &AstCursor, index: usize, expected: bool) {
    let token = cursor
        .token_ref_at(index)
        .unwrap_or_else(|| panic!("bounded view should expose index {index}"));
    assert_eq!(
        token.tag(),
        TokenTag::BOOL_LITERAL,
        "tag mismatch at index {index}"
    );
    assert_eq!(
        token.bool_value(),
        Some(expected),
        "payload mismatch at index {index}"
    );
}

/// Assert a declaration cursor exposes a bool literal with the expected payload at `index`.
fn assert_declaration_bool(cursor: &DeclarationCursor<'_>, index: usize, expected: bool) {
    assert_eq!(
        cursor.token_tag_at(index),
        Some(TokenTag::BOOL_LITERAL),
        "tag mismatch at index {index}"
    );
    assert_eq!(
        cursor
            .canonical_cursor()
            .parser_token_at(index)
            .and_then(|token| token.bool_value()),
        Some(expected),
        "payload mismatch at index {index}"
    );
}

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
        )
        .expect("cursor fixture should satisfy canonical ownership checks"),
    );
    let range = TokenRange::from_raw(source, 1, 3).expect("fixture range should be ordered");
    let mut cursor =
        AstCursor::from_source_tokens(&owner, None, range).expect("bounded cursor should build");

    assert_eq!(cursor.position(), 1);
    assert_eq!(cursor.length(), 3);
    assert!(cursor.token_ref_at(0).is_none());
    assert_bounded_bool(&cursor, 1, true);
    assert_bounded_bool(&cursor, 2, false);
    assert!(cursor.token_ref_at(3).is_none());
    assert!(cursor.token_ref_at(4).is_none());
    let neighbour = owner
        .token(
            TokenIndex::try_from_index(0)
                .expect("fixture index should fit the checked index domain"),
        )
        .expect("the canonical owner still addresses neighbours source-wide");
    assert_eq!(
        neighbour.tag(),
        TokenTag::MODULE_START,
        "neighbouring tokens stay addressable in the canonical owner, not in the bounded view"
    );
    assert_eq!(
        cursor.token_ref_at_offset(0).and_then(|token| token.bool_value()),
        Some(true)
    );
    assert_eq!(
        cursor.token_ref_at_offset(1).and_then(|token| token.bool_value()),
        Some(false)
    );
    assert!(cursor.token_ref_at_offset(2).is_none());
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
fn canonical_subcursor_window_bounds_contiguous_reads() {
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
        )
        .expect("cursor fixture should satisfy canonical ownership checks"),
    );
    let mut parent =
        AstCursor::from_source_tokens(&owner, None, TokenRange::from_raw(source, 0, 4).unwrap())
            .expect("bounded cursor should build");
    let mut child = parent
        .subcursor_window(1, 3)
        .expect("interior window should validate against canonical backing");

    assert_eq!(child.position(), 1);
    assert_eq!(child.length(), 3);
    assert!(child.token_ref_at(0).is_none());
    assert_bounded_bool(&child, 1, true);
    assert_bounded_bool(&child, 2, false);
    assert!(child.token_ref_at(3).is_none());
    assert!(
        child
            .nested_cursor(TokenRange::from_raw(source, 0, 2).unwrap())
            .is_err(),
        "nested views must retain the window's lower bound",
    );
    let nested = child
        .nested_cursor(TokenRange::from_raw(source, 1, 3).unwrap())
        .expect("a nested view wholly inside the window should be accepted");
    assert_eq!(
        nested.token_ref_at(1).map(|token| token.tag()),
        Some(TokenTag::BOOL_LITERAL),
        "nested views must retain the window's visible tokens",
    );
    assert_eq!(
        nested.token_ref_at(1).and_then(|token| token.bool_value()),
        Some(true),
        "nested views must retain the window's typed payloads",
    );
    assert!(child.peek_next_tag().is_some());
    child.advance();
    child.advance();
    assert!(child.current().is_none());
    assert!(child.span_at(3).is_none());

    assert!(parent.subcursor_window(3, 2).is_err());
    parent
        .set_limit(3)
        .expect("parent limit should fit the canonical range");
    assert!(parent.subcursor_window(1, 4).is_err());
}

#[test]
fn canonical_subcursor_window_skips_segmented_source_gaps() {
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
    let canonical = owner
        .canonical_source_tokens_arc()
        .expect("the segmented fixture owns its canonical source tokens");
    let mut parent = AstCursor::from_source_sequence(&canonical, None, sequence)
        .expect("segmented AST cursor should build");

    let restore = parent
        .set_limit(2)
        .expect("the parent limit should be in dense sequence coordinates");
    assert!(parent.subcursor_window(1, 3).is_err());
    parent.restore_limit(restore);

    let mut child = parent
        .subcursor_window(1, 3)
        .expect("segmented window should validate against canonical backing");
    let narrow = parent
        .subcursor_window(2, 3)
        .expect("a dense window inside the sequence should validate against canonical backing");
    assert!(
        narrow
            .nested_cursor(TokenRange::from_raw(source, 3, 4).unwrap())
            .is_err(),
        "segmented nested views must retain the dense lower bound",
    );

    assert_eq!(child.position(), 1);
    assert_eq!(child.length(), 3);
    assert!(child.token_ref_at(0).is_none());
    assert_bounded_bool(&child, 1, false);
    assert_bounded_bool(&child, 2, true);
    assert!(child.token_ref_at(3).is_none());
    assert!(child.previous().is_none());
    child.advance();
    assert_eq!(
        child.previous().map(|token| token.span()),
        Some(LocalSpan::source_start())
    );
    child.advance();
    assert!(child.current().is_none());

    assert!(parent.subcursor_window(3, 2).is_err());
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
    let canonical = owner
        .canonical_source_tokens_arc()
        .expect("the segmented fixture owns its canonical source tokens");
    let mut cursor = AstCursor::from_source_sequence(&canonical, None, sequence)
        .expect("segmented AST cursor should build");
    cursor.advance();
    cursor
        .set_limit(2)
        .expect("the active limit should be in dense sequence coordinates");

    let child = cursor
        .nested_cursor(TokenRange::from_raw(source, 3, 4).unwrap())
        .expect("a child ending at the dense limit should be accepted");
    assert_eq!(child.position(), 3);
    assert_eq!(
        child.token_ref_at(3).map(|token| token.tag()),
        Some(TokenTag::BOOL_LITERAL),
        "the child keeps its raw contiguous range after translation",
    );
    assert_eq!(
        child.token_ref_at(3).and_then(|token| token.bool_value()),
        Some(false),
        "the child keeps its typed payload after translation",
    );
    assert!(child.token_ref_at(4).is_none());
    assert!(
        cursor
            .nested_cursor(TokenRange::from_raw(source, 3, 5).unwrap())
            .is_err(),
        "a child crossing the dense limit must be rejected",
    );
}

/// A bounded contiguous view must stay bounded once it becomes a declaration cursor.
///
/// WHAT: the declaration handoff inherits the parent's active window.
/// WHY: an initializer parse addresses the same source as its neighbours, so a dropped window
/// lets a declaration read the following declaration's tokens.
#[test]
fn declaration_cursor_inherits_contiguous_parser_window() {
    let source = SourceId::COMPILATION_ROOT;
    let owner = Arc::new(
        SourceTokens::try_from_tokens(
            source,
            vec![
                Token::new(TokenKind::ModuleStart, LocalSpan::source_start()),
                Token::new(TokenKind::BoolLiteral(true), LocalSpan::source_start()),
                Token::new(TokenKind::BoolLiteral(false), LocalSpan::source_start()),
                Token::new(TokenKind::Newline, LocalSpan::source_start()),
                Token::new(TokenKind::Eof, LocalSpan::source_start()),
            ],
            NumericLiteralStore::with_source(source),
            PathSyntaxTable::with_source(source),
        )
        .expect("cursor fixture should satisfy canonical ownership checks"),
    );
    let parent =
        AstCursor::from_source_tokens(&owner, None, TokenRange::from_raw(source, 0, 5).unwrap())
            .expect("bounded cursor should build");
    let child = parent
        .subcursor_window(1, 3)
        .expect("interior window should validate against canonical backing");

    let declaration = child
        .declaration_cursor()
        .expect("a canonical window should produce a declaration cursor");

    assert_eq!(declaration.position(), 1);
    assert_eq!(
        declaration.length, 3,
        "the declaration cursor ends at the window, not at the source end"
    );
    assert!(
        declaration.token_tag_at(0).is_none(),
        "a declaration cursor must not read before its window"
    );
    assert_declaration_bool(&declaration, 1, true);
    assert_declaration_bool(&declaration, 2, false);
    assert!(
        declaration.token_tag_at(3).is_none(),
        "a declaration cursor must not read the token after its window"
    );
    assert!(
        declaration.token_tag_at(4).is_none(),
        "a declaration cursor must not reach the trailing source EOF"
    );

    let mut walking = child
        .declaration_cursor()
        .expect("a canonical window should produce a declaration cursor");
    assert_eq!(walking.current_tag(), TokenTag::BOOL_LITERAL);
    assert_eq!(walking.peek_next_tag(), Some(TokenTag::BOOL_LITERAL));
    walking.advance();
    assert_eq!(walking.current_tag(), TokenTag::BOOL_LITERAL);
    assert_eq!(
        walking.peek_next_tag(),
        None,
        "a declaration cursor must not peek past its window"
    );
    walking.advance();
    assert_eq!(
        walking.current_tag(),
        TokenTag::EOF,
        "walking to the window end reports EOF rather than the following token"
    );
    assert_eq!(walking.position(), 3, "the walk stops at the window end");
    walking.advance();
    assert_eq!(
        walking.position(),
        3,
        "a declaration cursor at its window end cannot advance further"
    );
}

/// The same guarantee in dense sequence coordinates, where the window skips source gaps.
#[test]
fn declaration_cursor_inherits_segmented_parser_window() {
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
    let canonical = owner
        .canonical_source_tokens_arc()
        .expect("the segmented fixture owns its canonical source tokens");
    let parent = AstCursor::from_source_sequence(&canonical, None, sequence)
        .expect("segmented AST cursor should build");
    let child = parent
        .subcursor_window(1, 2)
        .expect("dense window should validate against canonical backing");

    let declaration = child
        .declaration_cursor()
        .expect("a canonical window should produce a declaration cursor");

    assert_eq!(declaration.position(), 1);
    assert_eq!(declaration.length, 2);
    assert!(
        declaration.token_tag_at(0).is_none(),
        "a segmented declaration cursor must not read before its dense window"
    );
    assert_declaration_bool(&declaration, 1, false);
    assert!(
        declaration.token_tag_at(2).is_none(),
        "a segmented declaration cursor must not read past its dense window"
    );
}

/// A limit narrower than the cursor's range must also reach the declaration handoff.
#[test]
fn declaration_cursor_respects_a_stricter_parent_limit() {
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
        )
        .expect("cursor fixture should satisfy canonical ownership checks"),
    );
    let mut cursor =
        AstCursor::from_source_tokens(&owner, None, TokenRange::from_raw(source, 1, 4).unwrap())
            .expect("bounded cursor should build");
    let restore = cursor
        .set_limit(2)
        .expect("a limit inside the bounded range should be accepted");

    let declaration = cursor
        .declaration_cursor()
        .expect("a limited canonical cursor should produce a declaration cursor");
    assert_eq!(declaration.length, 2);
    assert_declaration_bool(&declaration, 1, true);
    assert!(
        declaration.token_tag_at(2).is_none(),
        "a declaration cursor must not read at or beyond an active parent limit"
    );

    cursor.restore_limit(restore);
    let restored = cursor
        .declaration_cursor()
        .expect("restoring the limit should keep the cursor usable");
    assert_eq!(
        restored.length, 4,
        "restoring a limit returns the natural range end"
    );
}
