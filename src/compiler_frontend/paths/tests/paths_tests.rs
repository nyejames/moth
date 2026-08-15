//! Path-only syntax tests.

use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, PathKind};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};

fn tokenize_source(
    source: &str,
) -> (
    crate::compiler_frontend::tokenizer::tokens::FileTokens,
    StringTable,
) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("source should tokenize");
    (tokens, string_table)
}

#[test]
fn path_token_terminates_at_unquoted_whitespace() {
    let (tokens, string_table) = tokenize_source("@core/math sin\n");
    let path_id = tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("path token");
    assert_eq!(
        tokens
            .path_syntax
            .try_path(path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "core/math"
    );
    assert!(tokens.tokens.iter().any(
        |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "sin")
    ));
}

#[test]
fn quoted_path_component_retains_whitespace() {
    let (tokens, string_table) = tokenize_source("@docs/\"my file.md\"\n");
    let path_id = tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        })
        .expect("path token");
    assert_eq!(
        tokens
            .path_syntax
            .try_path(path_id)
            .expect("valid path handle")
            .root
            .to_portable_string(&string_table),
        "docs/my file.md"
    );
}

#[test]
fn try_path_for_token_rejects_wrong_table_and_location_mismatch() {
    let (tokens, _) = tokenize_source("@core/math\n");
    let path_token = tokens
        .tokens
        .iter()
        .find(|token| matches!(token.kind, TokenKind::Path(_)))
        .expect("path token");
    let TokenKind::Path(path_id) = path_token.kind else {
        panic!("expected a path token");
    };

    tokens
        .path_syntax
        .try_path_for_token(path_id, &path_token.location)
        .expect("the owning table must accept its own token");

    let none_error = tokens
        .path_syntax
        .try_path_for_token(
            crate::compiler_frontend::paths::path_syntax::PathSyntaxId::NONE,
            &path_token.location,
        )
        .expect_err("NONE must stay an infrastructure failure");
    assert!(none_error.msg.contains("absent PathSyntaxId marker"));

    let empty_error = crate::compiler_frontend::paths::path_syntax::PathSyntaxTable::new()
        .try_path_for_token(path_id, &path_token.location)
        .expect_err("an empty wrong table must stay an infrastructure failure");
    assert!(empty_error.msg.contains("outside a table"));

    let (other, _) = tokenize_source("@other/path\n");
    let other_error = other
        .path_syntax
        .try_path_for_token(path_id, &path_token.location)
        .expect_err("a same-index row from another table must stay an infrastructure failure");
    assert!(
        other_error
            .msg
            .contains("does not belong to the consumed path token")
    );

    let mut mismatched_location = path_token.location.clone();
    mismatched_location.start_pos.line_number += 4;
    let location_error = tokens
        .path_syntax
        .try_path_for_token(path_id, &mismatched_location)
        .expect_err("a location mismatch must stay an infrastructure failure");
    assert!(
        location_error
            .msg
            .contains("does not belong to the consumed path token")
    );
}

#[test]
fn path_rejects_whitespace_after_introducer_or_separator() {
    for source in ["@ docs\n", "@docs/ my\n"] {
        let mut strings = StringTable::new();
        let source_path = InternedPath::from_single_str("test.moth", &mut strings);
        let error = tokenize(
            source,
            &source_path,
            TokenizerEntryMode::SourceFile,
            &StyleDirectiveRegistry::built_ins(),
            &mut strings,
            None,
        )
        .expect_err("whitespace cannot separate a path introducer or separator from its component");
        assert!(matches!(
            error.payload,
            DiagnosticPayload::InvalidPath { .. }
        ));
    }
}

#[test]
fn path_errors_remain_structured() {
    let mut strings = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut strings);
    let error = tokenize(
        "@/child",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut strings,
        None,
    )
    .expect_err("public root suffix should fail");
    assert!(matches!(
        error.payload,
        DiagnosticPayload::InvalidPath {
            path_kind: PathKind::OnlyRootSlashSupported
        }
    ));
}
