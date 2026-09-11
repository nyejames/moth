//! Tests for compiler-owned symbol preseed behavior.
//!
//! WHAT: exercises deterministic preseeding, stable prefix IDs across independent tables, and
//!      merge/remap behavior when the shared prefix contains compiler symbols.
//! WHY: parallel per-file frontend preparation depends on every local table starting from the same
//!      fixed symbol universe with an identical prefix.

use crate::compiler_frontend::symbols::compiler_symbols::CompilerSymbolSet;
use crate::compiler_frontend::symbols::string_interning::StringTable;

const COMPILER_SYMBOLS: [&str; 8] = [
    "start",
    "this",
    "Error",
    "message",
    "code",
    ";",
    "]",
    "<unknown>",
];

fn preseeded_table() -> StringTable {
    CompilerSymbolSet::preseeded_table(0)
}

fn assert_compiler_symbols_resolve(table: &mut StringTable) {
    for &symbol in &COMPILER_SYMBOLS {
        let id = table.intern(symbol);
        assert_eq!(table.resolve(id), symbol);
    }
}

#[test]
fn preseeded_table_helper_returns_fixed_strings() {
    let mut table = CompilerSymbolSet::preseeded_table(32);

    assert_compiler_symbols_resolve(&mut table);
    let resolved = table.iter().map(|(_, symbol)| symbol).collect::<Vec<_>>();
    assert_eq!(resolved.as_slice(), COMPILER_SYMBOLS.as_slice());
}

#[test]
fn source_strings_interned_after_preseed_are_not_compiler_symbols() {
    let mut table = preseeded_table();
    let prefix_len = table.len();

    let user_id = table.intern("my_user_function");

    // Every compiler symbol remains distinct from user-interned strings that come after the
    // preseed.
    for &symbol in &COMPILER_SYMBOLS {
        let symbol_id = table.intern(symbol);
        assert_ne!(user_id, symbol_id);
        assert_eq!(table.resolve(symbol_id), symbol);
    }

    // The table should have grown by exactly one entry for the user string.
    assert_eq!(table.len(), prefix_len + 1);
}

#[test]
fn preseeded_fork_tables_merge_with_identity_prefix() {
    let mut build_table = preseeded_table();
    let prefix_len = build_table.len();
    let prefix_ids = COMPILER_SYMBOLS
        .iter()
        .map(|&symbol| build_table.intern(symbol))
        .collect::<Vec<_>>();

    let fork_source = build_table.fork_source();

    let first_fork = fork_source.fork_for_module();
    let (mut first_table, first_base_len) = first_fork.into_parts();
    first_table.intern("first-only");

    let second_fork = fork_source.fork_for_module();
    let (mut second_table, second_base_len) = second_fork.into_parts();
    second_table.intern("second-only");

    build_table.merge_delta_from(&first_table, first_base_len);
    let second_remap = build_table.merge_delta_from(&second_table, second_base_len);

    // Preseeded strings belong to the shared prefix, so fork remaps keep them addressable without
    // re-interning fixed symbols after local source strings.
    for (&symbol, &id) in COMPILER_SYMBOLS.iter().zip(&prefix_ids) {
        let remapped_id = second_remap.get(id);
        assert_eq!(remapped_id, id);
        assert_eq!(build_table.resolve(remapped_id), symbol);
    }

    let resolved_prefix = build_table
        .iter()
        .take(prefix_len)
        .map(|(_, symbol)| symbol)
        .collect::<Vec<_>>();
    assert_eq!(resolved_prefix.as_slice(), COMPILER_SYMBOLS.as_slice());
}
