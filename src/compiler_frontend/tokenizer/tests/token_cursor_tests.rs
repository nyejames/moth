use super::*;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn static_tokens() -> SourceTokens {
    let source = SourceId::COMPILATION_ROOT;
    SourceTokens::try_from_tokens(
        source,
        vec![
            Token::new(TokenKind::ModuleStart, LocalSpan::source_start()),
            Token::new(TokenKind::BoolLiteral(true), LocalSpan::source_start()),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
        NumericLiteralStore::with_source(source),
        PathSyntaxTable::with_source(source),
        TokenStats::default(),
    )
    .expect("static token fixture should satisfy the source-token invariants")
}

#[test]
fn cursor_observes_half_open_boundaries_and_stable_eof() {
    let tokens = static_tokens();
    let start = TokenIndex::try_from_raw(0).unwrap();
    let end = TokenIndex::try_from_raw(3).unwrap();
    let mut cursor = TokenCursor::from_bounds(&tokens, start, end).expect("range should fit");

    assert_eq!(cursor.position(), start);
    assert!(!cursor.is_at_end());
    assert_eq!(cursor.peek().unwrap().index().raw(), 0);
    assert_eq!(cursor.peek_next().unwrap().index().raw(), 1);
    assert_eq!(cursor.advance().unwrap().index().raw(), 0);
    assert!(!cursor.is_eof());
    assert_eq!(cursor.advance().unwrap().index().raw(), 1);
    assert!(cursor.is_eof());
    assert_eq!(cursor.advance().unwrap().index().raw(), 2);
    assert_eq!(cursor.position().raw(), 2, "EOF is a stable cursor boundary");
    assert_eq!(cursor.advance().unwrap().index().raw(), 2);
    assert_eq!(cursor.position().raw(), 2);
}

fn mutable_file_tokens() -> FileTokens {
    let source = SourceId::COMPILATION_ROOT;
    FileTokens::new(
        PathId::ROOT,
        source,
        vec![
            Token::new(TokenKind::ModuleStart, LocalSpan::source_start()),
            Token::new(TokenKind::BoolLiteral(true), LocalSpan::source_start()),
            Token::new(TokenKind::BoolLiteral(false), LocalSpan::source_start()),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ],
    )
}

fn token_range(source: SourceId, start: u32, end: u32) -> TokenRange {
    TokenRange::from_raw(source, start, end).expect("fixture token range should be ordered")
}

#[test]
fn segmented_cursor_matches_contiguous_and_respects_segment_boundaries() {
    let source = SourceId::COMPILATION_ROOT;
    let mut file_tokens = mutable_file_tokens();
    let segments = [token_range(source, 0, 2), token_range(source, 2, 4)];
    let sequence = file_tokens
        .try_register_token_sequence(&segments)
        .expect("ordered adjacent ranges should register");
    let owner = file_tokens
        .source_tokens()
        .expect("file fixture should own canonical source tokens");
    let view = owner
        .token_sequence(sequence)
        .expect("registered sequence should resolve");

    assert_eq!(
        view.ranges().collect::<Vec<_>>(),
        segments,
        "the view must preserve every source-local segment"
    );
    assert_eq!(view.len(), 4);

    let mut segmented = view.cursor().expect("segmented cursor should construct");
    assert_eq!(segmented.range(), segments[0]);
    assert_eq!(segmented.peek_next().unwrap().index().raw(), 1);
    assert_eq!(segmented.advance().unwrap().index().raw(), 0);
    assert!(
        segmented.peek_next().is_none(),
        "peek_next must not cross a segment boundary"
    );
    assert_eq!(segmented.advance().unwrap().index().raw(), 1);
    assert_eq!(segmented.range(), segments[1]);
    assert_eq!(segmented.peek().unwrap().index().raw(), 2);

    let segmented_indexes = {
        let mut segmented_all = view.cursor().expect("segmented cursor should construct");
        let mut indexes = Vec::new();
        while let Some(token) = segmented_all.advance() {
            indexes.push(token.index().raw());
            if token.is_eof() {
                break;
            }
        }
        indexes
    };
    let mut contiguous = owner
        .cursor(owner.full_range().expect("full range should fit"))
        .expect("contiguous cursor should construct");
    let mut contiguous_indexes = Vec::new();
    while let Some(token) = contiguous.advance() {
        contiguous_indexes.push(token.index().raw());
        if token.is_eof() {
            break;
        }
    }
    assert_eq!(
        segmented_indexes, contiguous_indexes,
        "segmented traversal must match the equivalent contiguous view"
    );

    assert_eq!(segmented.advance().unwrap().index().raw(), 2);
    let eof_position = segmented.position();
    let eof_index = segmented
        .advance()
        .expect("EOF should remain reachable")
        .index();
    assert_eq!(eof_index.raw(), 3);
    assert!(segmented.is_eof());
    assert_eq!(segmented.advance().unwrap().index(), eof_index);
    assert_eq!(
        segmented.position(),
        eof_position,
        "repeated EOF advance must not move the cursor"
    );
}

#[test]
fn segmented_cursor_skips_empty_ranges_and_reaches_eof() {
    let source = SourceId::COMPILATION_ROOT;
    let mut file_tokens = mutable_file_tokens();
    let segments = [
        token_range(source, 0, 1),
        token_range(source, 2, 2),
        token_range(source, 3, 4),
    ];
    let sequence = file_tokens
        .try_register_token_sequence(&segments)
        .expect("ordered ranges with an empty middle segment should register");
    let owner = file_tokens
        .source_tokens()
        .expect("file fixture should own canonical source tokens");
    let view = owner
        .token_sequence(sequence)
        .expect("registered sequence should resolve");
    let mut cursor = view.cursor().expect("segmented cursor should construct");

    assert_eq!(cursor.range(), segments[0]);
    assert_eq!(cursor.advance().unwrap().index().raw(), 0);
    assert_eq!(
        cursor.range(),
        segments[2],
        "the empty middle segment must be skipped"
    );
    assert_eq!(cursor.position(), segments[2].start());
    assert!(!cursor.is_at_end());

    let mut indexes = vec![0];
    while let Some(token) = cursor.advance() {
        indexes.push(token.index().raw());
        if token.is_eof() {
            break;
        }
    }
    assert_eq!(indexes, vec![0, 3]);
    assert!(cursor.is_eof());

    let eof_position = cursor.position();
    let eof_index = cursor
        .advance()
        .expect("EOF should remain reachable")
        .index();
    assert_eq!(eof_index.raw(), 3);
    assert_eq!(
        cursor.position(),
        eof_position,
        "repeated EOF advance must not move the segmented cursor"
    );
    assert!(!cursor.is_at_end(), "stable EOF remains the current token");
}

#[test]
fn segmented_cursor_exhaustion_and_nested_ranges_are_bounded() {
    let source = SourceId::COMPILATION_ROOT;
    let mut file_tokens = mutable_file_tokens();
    let segments = [token_range(source, 0, 2), token_range(source, 2, 3)];
    let sequence = file_tokens
        .try_register_token_sequence(&segments)
        .expect("ordered ranges should register");
    let owner = file_tokens.source_tokens().unwrap();
    let view = owner.token_sequence(sequence).unwrap();
    let mut cursor = view.cursor().unwrap();

    assert!(cursor.nested(token_range(source, 0, 1)).is_ok());
    assert!(
        cursor.nested(token_range(source, 1, 3)).is_err(),
        "a nested range crossing a segment boundary must be rejected"
    );
    while cursor.advance().is_some() {}
    assert!(cursor.is_at_end());
    assert!(cursor.peek().is_none());
    assert_eq!(
        cursor.range(),
        token_range(source, 3, 3),
        "an exhausted segmented cursor must expose a zero-width current range"
    );
    assert!(
        cursor.nested(token_range(source, 0, 1)).is_err(),
        "nested ranges must not fall back to the first segment after exhaustion"
    );
}

#[test]
fn sequence_store_rejects_invalid_ranges_and_handles_without_panicking() {
    let source = SourceId::COMPILATION_ROOT;
    let foreign = SourceId::from_index(11);
    let mut store = TokenSequenceStore::new(source, 4);
    assert_eq!(std::mem::size_of::<TokenSequenceRange>(), 8);

    let reversed = TokenRange {
        source,
        start: TokenIndex::try_from_raw(2).unwrap(),
        end: TokenIndex::try_from_raw(1).unwrap(),
    };
    assert!(matches!(
        store.try_push(&[reversed]),
        Err(TokenSequenceError::Reversed { .. })
    ));
    assert!(matches!(
        store.try_push(&[token_range(source, 0, 2), token_range(source, 1, 3)]),
        Err(TokenSequenceError::Overlapping { .. })
    ));
    assert!(matches!(
        store.try_push(&[token_range(source, 2, 3), token_range(source, 0, 1)]),
        Err(TokenSequenceError::Unordered { .. })
    ));
    assert!(matches!(
        store.try_push(&[token_range(foreign, 0, 1)]),
        Err(TokenSequenceError::ForeignSource { .. })
    ));
    assert!(matches!(
        store.try_push(&[token_range(source, 0, 5)]),
        Err(TokenSequenceError::OutOfBounds { .. })
    ));

    let id = store
        .try_push(&[token_range(source, 0, 1)])
        .expect("valid sequence should register");
    assert_eq!(store.ranges(id).unwrap().len(), 1);
    assert!(matches!(
        store.ranges(TokenSequenceId::NONE),
        Err(TokenSequenceError::Absent)
    ));
    let malformed = TokenSequenceId::try_from_raw(u32::MAX).unwrap();
    assert!(matches!(
        store.ranges(malformed),
        Err(TokenSequenceError::OutOfRange { .. })
    ));
    let tokens = static_tokens();
    assert!(matches!(
        tokens.token_sequence(TokenSequenceId::NONE),
        Err(TokenSequenceError::Absent)
    ));
    assert!(matches!(
        tokens.token_sequence(malformed),
        Err(TokenSequenceError::OutOfRange { .. })
    ));
    store.freeze();
    let mut adapter = FileTokens::new_deferred_with_identity(
        PathId::ROOT,
        source,
        None,
        vec![Token::new(TokenKind::Eof, LocalSpan::source_start())],
    );
    assert!(matches!(
        adapter.try_register_token_sequence(&[]),
        Err(TokenSequenceError::NoCanonicalOwner)
    ));

    assert!(matches!(
        store.try_push(&[token_range(source, 1, 2)]),
        Err(TokenSequenceError::Frozen)
    ));
}

#[test]
fn sequence_store_rebinds_one_source_owner_without_rewriting_ranges() {
    let source = SourceId::COMPILATION_ROOT;
    let rebound = SourceId::from_index(12);
    let mut store = TokenSequenceStore::new(source, 4);
    let sequence = store
        .try_push(&[token_range(source, 1, 3)])
        .expect("initial source range should register");

    store.rebind_source_identity(rebound);
    assert_eq!(store.source(), rebound);
    assert!(
        store.ranges(sequence).is_ok(),
        "source-less range entries remain valid after owner rebinding"
    );
    assert!(matches!(
        store.try_push(&[token_range(source, 0, 1)]),
        Err(TokenSequenceError::ForeignSource { .. })
    ));
    assert!(
        store.try_push(&[token_range(rebound, 3, 4)]).is_ok(),
        "new ranges must use the rebound owner identity"
    );
    store.freeze();
    assert!(matches!(
        store.try_push(&[token_range(rebound, 0, 1)]),
        Err(TokenSequenceError::Frozen)
    ));
}

#[test]
fn nested_ranges_and_malformed_handles_are_checked() {
    let tokens = static_tokens();
    let full = tokens.full_range().expect("fixture length fits u32");
    let cursor = tokens.cursor(full).expect("full range should fit");
    let nested = TokenRange::new(
        SourceId::COMPILATION_ROOT,
        TokenIndex::try_from_raw(1).unwrap(),
        TokenIndex::try_from_raw(3).unwrap(),
    )
    .unwrap();
    let nested_cursor = cursor.nested(nested).expect("nested range should fit");
    assert_eq!(nested_cursor.range(), nested);

    let other_source = SourceId::from_index(1);
    let wrong_source = TokenRange::new(
        other_source,
        TokenIndex::try_from_raw(0).unwrap(),
        TokenIndex::try_from_raw(1).unwrap(),
    )
    .unwrap();
    assert!(matches!(cursor.nested(wrong_source), Err(TokenRangeError::ForeignSource { .. })));
    assert!(TokenRange::new(
        SourceId::COMPILATION_ROOT,
        TokenIndex::try_from_raw(2).unwrap(),
        TokenIndex::try_from_raw(1).unwrap(),
    )
    .is_none());
    assert!(tokens
        .token(TokenIndex::try_from_raw(u32::MAX).unwrap())
        .is_err());
    assert!(TokenCursor::from_bounds(
        &tokens,
        TokenIndex::try_from_raw(0).unwrap(),
        TokenIndex::try_from_raw(4).unwrap(),
    )
    .is_err());
}

#[test]
fn token_ref_exposes_shape_span_and_borrowed_cold_rows() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let numeric = NumericLiteralToken::test_new("1.5", &mut strings);
    let numeric_source_text = numeric.source_text;
    let mut numeric_store = NumericLiteralStore::with_source(source);
    numeric_store.push(numeric.clone());

    let path_id = PathId::ROOT;
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_row = path_syntax
        .try_push_for_source(path_id, source, LocalSpan::source_start())
        .expect("path row should fit");
    let tokens = SourceTokens::try_from_tokens(
        source,
        vec![
            Token::new(TokenKind::NumericLiteral(numeric), LocalSpan::source_start()),
            Token::new(TokenKind::Path(path_row), LocalSpan::source_start()),
        ],
        numeric_store,
        path_syntax,
        TokenStats::default(),
    )
    .expect("cold-store fixture should satisfy ownership checks");

    let numeric_ref = tokens.token(TokenIndex::try_from_raw(0).unwrap()).unwrap();
    assert_eq!(numeric_ref.shape().numeric_literal_id().unwrap().raw(), 1);
    assert_eq!(numeric_ref.span(), LocalSpan::source_start());
    assert_eq!(numeric_ref.numeric_literal().unwrap().unwrap().source_text, numeric_source_text);
    let path_ref = tokens.token(TokenIndex::try_from_raw(1).unwrap()).unwrap();
    assert_eq!(path_ref.path_syntax().unwrap().unwrap().root, path_id);
}

#[test]
fn deferred_publication_attaches_the_shared_path_table_to_both_token_owners() {
    use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let src_path = path_fork
        .try_intern_portable_path("deferred.moth", &mut strings)
        .expect("test path fits");
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_row = path_syntax
        .try_push_for_source(PathId::ROOT, source, LocalSpan::source_start())
        .expect("path row should fit");
    let mut file_tokens = FileTokens::new_with_identity(
        src_path,
        source,
        None,
        vec![Token::new(TokenKind::Path(path_row), LocalSpan::source_start())],
        path_syntax,
    );
    assert!(file_tokens.has_canonical_source_tokens());
    let path_index = TokenIndex::try_from_raw(0).unwrap();
    assert!(matches!(
        file_tokens
            .source_tokens()
            .expect("canonical owner stays readable")
            .token(path_index)
            .unwrap()
            .path_syntax(),
        Err(TokenViewError::MissingPathTable)
    ));

    let table = file_tokens
        .take_preparing_path_syntax()
        .expect("preparing stream should own its path table");
    file_tokens.attach_preflighted_shared_path_syntax(table);
    file_tokens.freeze_numeric_literals();
    let resolved = file_tokens
        .source_tokens()
        .expect("canonical owner stays readable after publication")
        .token(path_index)
        .unwrap()
        .path_syntax()
        .expect("attached table should resolve")
        .expect("path token should carry a row");
    assert_eq!(resolved.root, PathId::ROOT);
}

#[test]
fn ordinary_substream_exposes_no_second_canonical_owner() {
    use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let src_path = path_fork
        .try_intern_portable_path("owner.moth", &mut strings)
        .expect("test path fits");
    let mut owner = FileTokens::new_with_identity(
        src_path,
        source,
        None,
        vec![Token::new(
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("7", &mut strings)),
            LocalSpan::source_start(),
        )],
        PathSyntaxTable::with_source(source),
    );
    assert!(owner.has_canonical_source_tokens());
    assert!(owner.source_tokens().is_ok());

    let substream = FileTokens::new_substream(
        &owner,
        src_path,
        source,
        vec![Token::new(
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("7", &mut strings)),
            LocalSpan::source_start(),
        )],
    );
    assert!(!substream.has_canonical_source_tokens());
    assert!(substream.source_tokens().is_err());
    assert_eq!(substream.numeric_literal_store().len(), 1);
    assert_eq!(
        substream.numeric_literal_store().owner_source(),
        Some(source)
    );
    assert!(substream.numeric_literal_id_at(0).is_some());

    let rebound = SourceId::from_index(11);
    owner.rebind_file_identity(src_path, rebound, None);
    assert_eq!(
        owner
            .source_tokens()
            .expect("canonical owner stays readable")
            .source(),
        rebound
    );
    assert_eq!(substream.numeric_literal_store().owner_source(), Some(source));
}

#[test]
fn deferred_adapter_retains_numeric_lifecycle_without_canonical_shapes() {
    use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let src_path = path_fork
        .try_intern_portable_path("deferred-numeric.moth", &mut strings)
        .expect("test path fits");
    let deferred = FileTokens::new_deferred_with_identity(
        src_path,
        source,
        None,
        vec![Token::new(
            TokenKind::NumericLiteral(NumericLiteralToken::test_new("9", &mut strings)),
            LocalSpan::source_start(),
        )],
    );
    assert!(!deferred.has_canonical_source_tokens());
    assert!(deferred.source_tokens().is_err());
    let handle = deferred
        .numeric_literal_id_at(0)
        .expect("adapter numeric token must carry a handle");
    deferred
        .numeric_literal_store()
        .try_get_for_source(handle, source)
        .expect("adapter numeric row must resolve before freeze");
}
#[test]
fn canonical_construction_rejects_unowned_cold_stores() {
    let source = SourceId::COMPILATION_ROOT;
    let span = LocalSpan::source_start();
    let mut strings = StringTable::new();
    let numeric = NumericLiteralToken::test_new("9", &mut strings);
    let mut unbound_numeric = NumericLiteralStore::new();
    unbound_numeric.push(numeric.clone());

    let unbound_result = SourceTokens::try_from_tokens(
        source,
        vec![Token::new(TokenKind::NumericLiteral(numeric), span)],
        unbound_numeric,
        PathSyntaxTable::with_source(source),
        TokenStats::default(),
    );
    assert!(
        unbound_result.is_err(),
        "non-empty numeric stores must carry their source identity"
    );

    let foreign_path_result = SourceTokens::try_from_tokens(
        source,
        vec![Token::new(TokenKind::Eof, span)],
        NumericLiteralStore::with_source(source),
        PathSyntaxTable::with_source(SourceId::from_index(11)),
        TokenStats::default(),
    );
    assert!(
        foreign_path_result.is_err(),
        "canonical path stores must match the source even without path tokens"
    );
}
