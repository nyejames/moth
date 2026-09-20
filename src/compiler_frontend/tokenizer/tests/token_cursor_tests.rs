use super::*;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

fn static_tokens() -> Arc<SourceTokens> {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_static(TokenTag::MODULE_START, LocalSpan::source_start())
        .expect("static token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF token fixture should build");
    builder
        .finish()
        .expect("static token fixture should satisfy source-token invariants")
}

fn mutable_source_tokens() -> Arc<SourceTokens> {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_static(TokenTag::MODULE_START, LocalSpan::source_start())
        .expect("module-start token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, false, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF token fixture should build");
    builder
        .finish_unfrozen()
        .expect("mutable source-token fixture should build")
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
    assert_eq!(
        cursor.position().raw(),
        2,
        "EOF is a stable cursor boundary"
    );
    assert_eq!(cursor.advance().unwrap().index().raw(), 2);
    assert_eq!(cursor.position().raw(), 2);
}

#[test]
fn contiguous_parser_reads_respect_a_nonzero_active_range() {
    let tokens = static_tokens();
    let source = SourceId::COMPILATION_ROOT;
    let range = token_range(source, 1, 3);
    let cursor = tokens
        .cursor(range)
        .expect("bounded range should construct");

    assert_eq!(cursor.parser_position(), 1);
    assert_eq!(cursor.parser_length(), 3);
    assert!(
        cursor.parser_token_at(0).is_none(),
        "parser reads must not escape below the active range"
    );
    assert_eq!(cursor.parser_token_at(1).unwrap().index().raw(), 1);
    assert_eq!(cursor.parser_token_at(2).unwrap().index().raw(), 2);
    assert!(
        cursor.parser_token_at(3).is_none(),
        "the half-open active range excludes its end"
    );
    assert!(
        cursor.parser_token_at(4).is_none(),
        "parser reads must not escape beyond the active range"
    );
    assert_eq!(cursor.parser_peek_next().unwrap().index().raw(), 2);
    assert!(cursor.parser_previous().is_none());

    let mut at_end = cursor;
    at_end
        .set_position(TokenIndex::try_from_raw(2).unwrap())
        .expect("position inside range should be accepted");
    assert!(at_end.parser_peek_next().is_none());
    assert_eq!(at_end.parser_previous().unwrap().index().raw(), 1);
}

fn token_range(source: SourceId, start: u32, end: u32) -> TokenRange {
    TokenRange::from_raw(source, start, end).expect("fixture token range should be ordered")
}

#[test]
fn segmented_cursor_matches_contiguous_and_respects_segment_boundaries() {
    let source = SourceId::COMPILATION_ROOT;
    let mut owner = mutable_source_tokens();
    let segments = [token_range(source, 0, 2), token_range(source, 2, 4)];
    let sequence = Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .try_register_token_sequence(&segments)
        .expect("ordered adjacent ranges should register");
    Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .freeze_numeric_literals();
    let owner = owner.as_ref();
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
fn segmented_cursor_marks_only_the_first_token_after_a_source_gap() {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_static(TokenTag::MODULE_START, LocalSpan::source_start())
        .expect("module-start token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, false, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("boolean token fixture should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF token fixture should build");
    let mut owner = builder
        .finish_unfrozen()
        .expect("gapped fixture should build");

    let segments = [token_range(source, 0, 2), token_range(source, 3, 5)];
    let sequence = Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .try_register_token_sequence(&segments)
        .expect("gapped ranges should register");
    Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .freeze_numeric_literals();
    let owner = owner.as_ref();
    let view = owner
        .token_sequence(sequence)
        .expect("registered sequence should resolve");
    let mut cursor = view.cursor().expect("segmented cursor should construct");
    assert!(
        !cursor.is_at_segment_start(),
        "the first range has no preceding range"
    );
    assert_eq!(cursor.advance().unwrap().index().raw(), 0);
    assert!(!cursor.is_at_segment_start());
    assert_eq!(cursor.advance().unwrap().index().raw(), 1);
    assert_eq!(cursor.position().raw(), 3);
    assert!(
        cursor.is_at_segment_start(),
        "a gapped range should mark its first token"
    );
    assert_eq!(cursor.advance().unwrap().index().raw(), 3);
    assert!(
        !cursor.is_at_segment_start(),
        "the marker must clear after advancing within a gapped range"
    );
    assert_eq!(cursor.advance().unwrap().index().raw(), 4);
    assert!(
        !cursor.is_at_segment_start(),
        "later tokens in a gapped range are ordinary adjacent tokens"
    );
}
#[test]
fn segmented_cursor_many_gaps_matches_flattened_reference() {
    let source = SourceId::COMPILATION_ROOT;
    let token_count = 129usize;
    let mut builder = TestSourceTokensBuilder::new(source);
    for index in 0..token_count {
        if index + 1 == token_count {
            builder
                .push_static(TokenTag::EOF, LocalSpan::source_start())
                .expect("EOF token fixture should build");
        } else {
            builder
                .push_bool(
                    TokenTag::BOOL_LITERAL,
                    index % 2 == 0,
                    LocalSpan::source_start(),
                )
                .expect("boolean token fixture should build");
        }
    }
    let mut owner = builder
        .finish_unfrozen()
        .expect("many-gap fixture should build");
    let mut segments = (0..64)
        .map(|segment| {
            let start = (segment * 2) as u32;
            token_range(source, start, start + 1)
        })
        .collect::<Vec<_>>();
    segments.push(token_range(source, 128, 129));
    let sequence = Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .try_register_token_sequence(&segments)
        .expect("many ordered gapped ranges should register");
    Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .freeze_numeric_literals();
    let owner = owner.as_ref();
    let view = owner
        .token_sequence(sequence)
        .expect("registered sequence should resolve");
    let expected = view
        .ranges()
        .flat_map(|range| range.start().index()..range.end().index())
        .collect::<Vec<_>>();
    assert_eq!(view.len(), expected.len());

    let mut cursor = view.cursor().expect("segmented cursor should construct");
    for (position, expected_index) in expected.iter().copied().enumerate() {
        assert_eq!(cursor.parser_position(), position);
        assert_eq!(cursor.parser_length(), expected.len());
        assert_eq!(cursor.current().unwrap().index().index(), expected_index);
        assert_eq!(
            cursor.parser_peek_next().map(|token| token.index().index()),
            expected.get(position + 1).copied()
        );
        assert_eq!(
            cursor.parser_previous().map(|token| token.index().index()),
            position
                .checked_sub(1)
                .and_then(|previous| expected.get(previous).copied())
        );
        let current = cursor.advance().expect("flattened reference has a token");
        assert_eq!(current.index().index(), expected_index);
        if current.is_eof() {
            break;
        }
    }

    let mut seek = view.cursor().expect("segmented cursor should construct");
    for position in [0, 1, expected.len() / 2, expected.len()] {
        seek.set_parser_position(position)
            .expect("dense positions should be seekable");
        assert_eq!(seek.parser_position(), position);
        assert_eq!(
            seek.current().map(|token| token.index().index()),
            expected.get(position).copied()
        );
    }
    for position in [expected.len(), expected.len() / 2, 1, 0] {
        seek.set_parser_position(position)
            .expect("dense positions should be seekable backwards");
        assert_eq!(seek.parser_position(), position);
        assert_eq!(
            seek.current().map(|token| token.index().index()),
            expected.get(position).copied()
        );
    }
}

#[test]
fn segmented_cursor_skips_empty_ranges_and_reaches_eof() {
    let source = SourceId::COMPILATION_ROOT;
    let mut owner = mutable_source_tokens();
    let segments = [
        token_range(source, 0, 1),
        token_range(source, 2, 2),
        token_range(source, 3, 4),
    ];
    let sequence = Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .try_register_token_sequence(&segments)
        .expect("ordered ranges with an empty middle segment should register");
    Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .freeze_numeric_literals();
    let owner = owner.as_ref();
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
    let mut owner = mutable_source_tokens();
    let segments = [token_range(source, 0, 2), token_range(source, 2, 3)];
    let sequence = Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .try_register_token_sequence(&segments)
        .expect("ordered ranges should register");
    Arc::get_mut(&mut owner)
        .expect("segmented fixture should remain uniquely owned")
        .freeze_numeric_literals();
    let owner = owner.as_ref();
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
    assert!(matches!(
        cursor.nested(wrong_source),
        Err(TokenRangeError::ForeignSource { .. })
    ));
    assert!(
        TokenRange::new(
            SourceId::COMPILATION_ROOT,
            TokenIndex::try_from_raw(2).unwrap(),
            TokenIndex::try_from_raw(1).unwrap(),
        )
        .is_none()
    );
    assert!(
        tokens
            .token(TokenIndex::try_from_raw(u32::MAX).unwrap())
            .is_err()
    );
    assert!(
        TokenCursor::from_bounds(
            &tokens,
            TokenIndex::try_from_raw(0).unwrap(),
            TokenIndex::try_from_raw(4).unwrap(),
        )
        .is_err()
    );
}

#[test]
fn token_ref_exposes_shape_span_and_borrowed_cold_rows() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let numeric = NumericLiteralToken::test_new("1.5", &mut strings);
    let numeric_source_text = numeric.source_text;

    let path_id = PathId::ROOT;
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_row = path_syntax
        .try_push_for_source(path_id, source, LocalSpan::source_start())
        .expect("path row should fit");
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder
        .push_numeric(numeric, LocalSpan::source_start())
        .expect("numeric token fixture should build");
    builder
        .push_path(TokenTag::PATH, path_row, LocalSpan::source_start())
        .expect("path token fixture should build");
    let tokens = builder
        .finish()
        .expect("cold-store fixture should satisfy source-token invariants");

    let numeric_ref = tokens.token(TokenIndex::try_from_raw(0).unwrap()).unwrap();
    assert_eq!(numeric_ref.shape().numeric_literal_id().unwrap().raw(), 1);
    assert_eq!(numeric_ref.tag(), TokenTag::NUMERIC_LITERAL);
    assert_eq!(numeric_ref.span(), LocalSpan::source_start());
    assert_eq!(
        numeric_ref.numeric_literal().unwrap().unwrap().source_text,
        numeric_source_text
    );
    let path_ref = tokens.token(TokenIndex::try_from_raw(1).unwrap()).unwrap();
    assert_eq!(path_ref.tag(), TokenTag::PATH);
    assert_eq!(path_ref.path_syntax().unwrap().unwrap().root, path_id);
}

#[test]
fn canonical_publication_attaches_the_shared_path_table_to_source_tokens() {
    let source = SourceId::COMPILATION_ROOT;
    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path_row = path_syntax
        .try_push_for_source(PathId::ROOT, source, LocalSpan::source_start())
        .expect("path row should fit");
    let mut builder = SourceTokensBuilder::with_capacity(source, 1);
    builder
        .push_payload(TokenTag::PATH, 0, path_row.raw(), LocalSpan::source_start())
        .expect("path token shape should build");
    let mut tokens = Arc::new(
        builder
            .finish(NumericLiteralStore::with_source(source))
            .expect("canonical path owner should build"),
    );

    let path_index = TokenIndex::try_from_raw(0).unwrap();
    assert!(matches!(
        tokens.token(path_index).unwrap().path_syntax(),
        Err(TokenViewError::MissingPathTable)
    ));

    path_syntax
        .validate_file_owned_locations(source)
        .expect("path table should remain source-owned");
    path_syntax.freeze();
    Arc::get_mut(&mut tokens)
        .expect("canonical owner should remain uniquely owned")
        .attach_shared_path_syntax(Arc::new(path_syntax));
    let resolved = tokens
        .token(path_index)
        .unwrap()
        .path_syntax()
        .expect("published canonical path table should resolve")
        .expect("path token should carry a row");
    assert_eq!(resolved.root, PathId::ROOT);
}

#[test]
fn canonical_owner_rebinds_without_duplicate_source_storage() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_numeric(
            NumericLiteralToken::test_new("7", &mut strings),
            LocalSpan::source_start(),
        )
        .expect("numeric token fixture should build");
    let mut owner = builder
        .finish()
        .expect("canonical numeric fixture should publish");

    let second_handle = owner.clone();
    assert!(
        Arc::get_mut(&mut owner).is_none(),
        "a shared canonical owner must not expose a second mutable owner"
    );
    drop(second_handle);

    let rebound = SourceId::from_index(11);
    Arc::get_mut(&mut owner)
        .expect("canonical owner should be uniquely mutable before rebinding")
        .rebind_source_identity(rebound);
    assert_eq!(owner.source(), rebound);
    assert_eq!(owner.numeric_literal_store().owner_source(), Some(rebound));
}

#[test]
fn canonical_numeric_publication_retains_its_checked_cold_row() {
    let source = SourceId::COMPILATION_ROOT;
    let mut strings = StringTable::new();
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_numeric(
            NumericLiteralToken::test_new("9", &mut strings),
            LocalSpan::source_start(),
        )
        .expect("numeric token fixture should build");
    let owner = builder
        .finish()
        .expect("canonical numeric fixture should publish");
    let token = owner.token(TokenIndex::try_from_raw(0).unwrap()).unwrap();
    let handle = token
        .shape()
        .numeric_literal_id()
        .expect("numeric token must carry a canonical handle");
    owner
        .numeric_literal_store()
        .try_get_for_source(handle, source)
        .expect("canonical numeric row must resolve after publication");
    assert!(token.numeric_literal().unwrap().is_some());
}
#[test]
fn canonical_construction_rejects_unowned_cold_stores() {
    let source = SourceId::COMPILATION_ROOT;
    let span = LocalSpan::source_start();
    let mut strings = StringTable::new();
    let numeric = NumericLiteralToken::test_new("9", &mut strings);
    let mut unbound_numeric = NumericLiteralStore::new();
    unbound_numeric.push(numeric.clone());

    let mut canonical_builder = SourceTokensBuilder::with_capacity(source, 1);
    let numeric_id = canonical_builder
        .preflight_numeric()
        .expect("numeric shape preflight should fit");
    canonical_builder
        .push_numeric(
            TokenTag::NUMERIC_LITERAL,
            numeric_kind_flags(numeric.kind),
            numeric_id,
            span,
        )
        .expect("numeric shape fixture should pack");
    let unbound_result = canonical_builder.finish(unbound_numeric);
    assert!(
        unbound_result.is_err(),
        "non-empty numeric stores must carry their source identity"
    );

    let foreign_path_builder = TestSourceTokensBuilder::with_path_syntax(
        source,
        PathSyntaxTable::with_source(SourceId::from_index(11)),
    );
    let foreign_path_result = foreign_path_builder.finish();
    assert!(
        foreign_path_result.is_err(),
        "canonical path stores must match the source even without path tokens"
    );
}
