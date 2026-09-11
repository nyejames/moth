//! Compiler-owned fixed symbol preseed infrastructure.
//!
//! WHAT: defines the set of compiler-owned symbols that are interned deterministically into every
//!      StringTable before per-file frontend preparation begins.
//! WHY: parallel tokenization and header parsing need stable IDs for fixed language/compiler names
//!      without sharing a mutable global table. Preseeding gives each local table the same symbol
//!      prefix with identical IDs.

use crate::compiler_frontend::builtins::error_type::{
    ERROR_FIELD_CODE, ERROR_FIELD_MESSAGE, ERROR_TYPE_NAME,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

/// Owner for fixed compiler-owned symbols and deterministic table preseeding.
///
/// WHAT: interns the fixed compiler symbol set into a StringTable in a stable order.
/// WHY: every per-file local table starts from the same prefix so common compiler symbols share
///      the same IDs without cross-file coordination.
pub struct CompilerSymbolSet;

impl CompilerSymbolSet {
    /// Deterministically intern all fixed compiler symbols into `string_table`.
    ///
    /// Symbols are interned in declaration order so the prefix is stable across independently
    /// created tables that start empty.
    ///
    /// # Stability invariant
    /// Stable numeric IDs for compiler-owned symbols require the table to be preseeded before any
    /// source or project strings are interned. If user strings are already present in the table,
    /// the compiler symbols will not occupy the canonical prefix.
    pub fn preseed(string_table: &mut StringTable) {
        string_table.intern(IMPLICIT_START_FUNC_NAME);
        string_table.intern("this");
        string_table.intern(ERROR_TYPE_NAME);
        string_table.intern(ERROR_FIELD_MESSAGE);
        string_table.intern(ERROR_FIELD_CODE);
        string_table.intern(";");
        string_table.intern("]");
        string_table.intern("<unknown>");
    }

    /// Create a new `StringTable` with the given capacity and preseed it with compiler-owned
    /// symbols.
    ///
    /// This is the production entry point for frontend table construction. It guarantees that the
    /// table starts with the stable compiler symbol prefix before any source or project strings are
    /// interned.
    pub fn preseeded_table(capacity: usize) -> StringTable {
        let mut string_table = StringTable::with_capacity(capacity);
        Self::preseed(&mut string_table);
        string_table
    }
}
