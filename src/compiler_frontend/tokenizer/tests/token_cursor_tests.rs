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

#[test]
fn nested_ranges_and_malformed_handles_are_checked() {
    let tokens = static_tokens();
    let full = tokens.full_range().expect("fixture length fits u32");
    let mut cursor = tokens.cursor(full).expect("full range should fit");
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
    let mut deferred = FileTokens::new_deferred_with_identity(
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
