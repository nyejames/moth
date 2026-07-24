//! Unit tests for `TokenStats` classification.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::path::Path;

fn tokenize_source(source: &str) -> (TokenStats, StringTable) {
    let mut string_table = StringTable::new();
    let path =
        InternedPath::try_from_filesystem_path(Path::new("src/main.moth"), &mut string_table)
            .expect("test path should be UTF-8");
    let directives = StyleDirectiveRegistry::built_ins();

    let file_tokens = tokenize(
        source,
        &path,
        TokenizerEntryMode::SourceFile,
        &directives,
        &mut string_table,
        None,
    )
    .expect("source should tokenize");

    (file_tokens.token_stats, string_table)
}

#[test]
fn empty_source_has_only_module_start_and_eof() {
    let (stats, _string_table) = tokenize_source("");
    assert_eq!(stats.total_tokens, 2); // ModuleStart + Eof
    assert_eq!(stats.symbols, 0);
    assert_eq!(stats.literals, 0);
}

#[test]
fn simple_expression_classification() {
    let (stats, _string_table) = tokenize_source("x = 1 + 2");
    assert!(
        stats.total_tokens >= 6,
        "expected at least 6 tokens, got {}",
        stats.total_tokens
    );
    assert_eq!(stats.symbols, 1); // x
    assert_eq!(stats.literals, 2); // 1, 2
    assert_eq!(stats.operators, 1); // +
    assert_eq!(stats.map_or_collection_delimiters, 0);
}

#[test]
fn template_tokens_are_classified() {
    let (stats, _string_table) = tokenize_source("t = [: hello ]");
    assert!(
        stats.template_markers >= 3,
        "expected template markers, got {}",
        stats.template_markers
    );
}

#[test]
fn collection_delimiters_count_curly_braces_and_commas() {
    let (stats, _string_table) = tokenize_source("values = {1, 2, 3}");

    assert_eq!(stats.map_or_collection_delimiters, 4);
}
