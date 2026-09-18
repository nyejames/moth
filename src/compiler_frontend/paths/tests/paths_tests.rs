//! Path-only syntax tests.

use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, PathKind};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{TokenizeFailure, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};

fn tokenize_source(
    source: &str,
    span_builder: &mut ExtendedSpanBuilder,
) -> (
    crate::compiler_frontend::tokenizer::tokens::FileTokens,
    StringTable,
    PathInternerFork,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let tokens = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        span_builder,
    )
    .expect("source should tokenize");
    (tokens, string_table, path_fork)
}

fn tokenize_source_with_id(
    source: &str,
    span_builder: &mut ExtendedSpanBuilder,
    file_id: SourceId,
) -> (
    crate::compiler_frontend::tokenizer::tokens::FileTokens,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let tokens = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        file_id,
        span_builder,
    )
    .expect("source should tokenize");
    (tokens, string_table)
}

#[test]
fn path_token_terminates_at_unquoted_whitespace() {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (tokens, string_table, path_fork) = tokenize_source("@core/math sin\n", &mut span_builder);
    let path_id = tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("path token");
    let path_root = tokens
        .path_syntax
        .try_path(path_id)
        .expect("valid path handle")
        .root;
    assert_eq!(
        path_fork.render_portable(path_root, &string_table, &mut Vec::new()),
        "core/math"
    );
    assert!(tokens.tokens.iter().any(
        |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "sin")
    ));
}

#[test]
fn quoted_path_component_retains_whitespace() {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (tokens, string_table, path_fork) =
        tokenize_source("@docs/\"my file.md\"\n", &mut span_builder);
    let path_id = tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("path token");
    let path_root = tokens
        .path_syntax
        .try_path(path_id)
        .expect("valid path handle")
        .root;
    assert_eq!(
        path_fork.render_portable(path_root, &string_table, &mut Vec::new()),
        "docs/my file.md"
    );
}

#[test]
fn path_table_freeze_rejects_checked_push_and_rejects_foreign_source() {
    use crate::compiler_frontend::paths::path_syntax::{PathSyntaxError, PathSyntaxTable};

    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let root = path_fork
        .try_intern_components(&[strings.intern("core"), strings.intern("math")])
        .expect("test path fits");
    let mut table = PathSyntaxTable::with_source(SourceId::COMPILATION_ROOT);
    assert_eq!(table.owner_source(), Some(SourceId::COMPILATION_ROOT));
    assert!(!table.is_frozen());
    let id = table
        .try_push_for_source(
            root,
            SourceId::COMPILATION_ROOT,
            crate::compiler_frontend::source::LocalSpan::source_start(),
        )
        .expect("owner push should succeed");
    assert_eq!(id.index(), Some(0));
    assert!(!id.is_none());
    let foreign = SourceId::from_index(7);
    assert!(matches!(
        table.try_push_for_source(
            root,
            foreign,
            crate::compiler_frontend::source::LocalSpan::source_start()
        ),
        Err(PathSyntaxError::ForeignSource { .. })
    ));
    table.freeze();
    assert!(table.is_frozen());
    assert!(
        matches!(
            table.try_push_local(
                root,
                crate::compiler_frontend::source::LocalSpan::source_start()
            ),
            Err(crate::compiler_frontend::paths::path_syntax::PathSyntaxError::Frozen)
        ),
        "a frozen source-owned table must reject another authored row on its frozen lane"
    );
}

#[test]
fn try_path_for_token_rejects_wrong_table_and_span_mismatch() {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (tokens, _, _) = tokenize_source("@core/math\n", &mut span_builder);
    let path_token = tokens
        .tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("path token");
    let TokenKind::Path(path_id) = path_token.kind else {
        panic!("expected a path token");
    };
    let token_span = SourceSpan::new(tokens.file_id, path_token.span);

    tokens
        .path_syntax
        .try_path_for_token(path_id, token_span)
        .expect("the owning table must accept its own token");

    let none_error = tokens
        .path_syntax
        .try_path_for_token(
            crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
            token_span,
        )
        .expect_err("NONE must stay an infrastructure failure");
    assert!(none_error.msg.contains("absent PathSyntaxId marker"));

    let empty_error = crate::compiler_frontend::paths::path_syntax::PathSyntaxTable::new()
        .try_path_for_token(path_id, token_span)
        .expect_err("an empty wrong table must stay an infrastructure failure");
    assert!(empty_error.msg.contains("outside a table"));

    let mut other_span_builder = ExtendedSpanBuilder::new();
    let (other, _) = tokenize_source_with_id(
        "@other/path\n",
        &mut other_span_builder,
        SourceId::from_index(7),
    );
    let other_error = other
        .path_syntax
        .try_path_for_token(path_id, token_span)
        .expect_err("a same-index row from another source must stay an infrastructure failure");
    assert!(
        other_error
            .msg
            .contains("does not belong to the consumed path token")
    );

    let mismatched_span = SourceSpan::new(
        tokens.file_id,
        crate::compiler_frontend::source::LocalSpan::source_start(),
    );
    let span_error = tokens
        .path_syntax
        .try_path_for_token(path_id, mismatched_span)
        .expect_err("a span mismatch must stay an infrastructure failure");
    assert!(
        span_error
            .msg
            .contains("does not belong to the consumed path token")
    );
}

#[test]
fn path_rejects_whitespace_after_introducer_or_separator() {
    for source in ["@ docs\n", "@docs/ my\n"] {
        let mut strings = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let source_path = path_fork
            .try_intern_portable_path("test.moth", &mut strings)
            .expect("test path fits");
        let mut span_builder = ExtendedSpanBuilder::new();
        let error = match tokenize(
            source,
            source_path,
            TokenizerEntryMode::SourceFile,
            &StyleDirectiveRegistry::built_ins(),
            &mut strings,
            &mut path_fork,
            SourceId::COMPILATION_ROOT,
            &mut span_builder,
        ) {
            Ok(_) => panic!(
                "whitespace cannot separate a path introducer or separator from its component"
            ),
            Err(TokenizeFailure::Diagnosed(diagnostic)) => diagnostic,
            Err(TokenizeFailure::Infrastructure(error)) => {
                panic!("path fixture tokenization encountered infrastructure failure: {error:?}")
            }
        };
        assert!(matches!(
            error.payload,
            DiagnosticPayload::InvalidPath { .. }
        ));
    }
}

#[test]
fn path_errors_remain_structured() {
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut strings)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let error = match tokenize(
        "@/child",
        source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    ) {
        Ok(_) => panic!("public root suffix should fail"),
        Err(TokenizeFailure::Diagnosed(diagnostic)) => diagnostic,
        Err(TokenizeFailure::Infrastructure(error)) => {
            panic!("path fixture tokenization encountered infrastructure failure: {error:?}")
        }
    };
    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidPath {
            path_kind: PathKind::OnlyRootSlashSupported
        }
    ));
}

/// Long path rows retain the token's single source-owned encoding.
#[test]
fn long_multibyte_path_retains_one_original_span() {
    let path_text = format!("@docs/\"{}.md\"", "é".repeat(700));
    let source = format!("-- 🦋\n{path_text}\n");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("long-path.moth", &mut strings)
        .expect("test path fits");
    let canonical_path = path_fork.render_native(scope, &strings, &mut Vec::new());
    let sources = SourceDatabase::build([&canonical_path], &canonical_path, None, &mut strings)
        .expect("source should register");
    let source_id = sources
        .get_by_canonical_path(&canonical_path)
        .expect("registered source")
        .id;
    let mut spans = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        &source,
        scope,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        &mut path_fork,
        source_id,
        &mut spans,
    )
    .expect("quoted multibyte path should tokenize");
    let token = tokens
        .tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("path token");
    let TokenKind::Path(path_id) = token.kind else {
        unreachable!()
    };
    let row = tokens
        .path_syntax
        .try_path_for_token(path_id, SourceSpan::new(tokens.file_id, token.span))
        .expect("owned path");
    assert_eq!(row.span, token.span);

    let retained_span = SourceSpan::new(tokens.file_id, row.span);
    let mut database = SourceDatabaseBuilder::new(sources);
    database
        .sources_mut()
        .retain_text(source_id, source.clone())
        .expect("retain source snapshot");
    database.retain_span_builder(source_id, spans);
    let database = database.finish().expect("install original span table");
    let range = retained_span.byte_range(&database);
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        path_text
    );
}
