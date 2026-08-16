//! Path and source-position display helpers for diagnostic rendering.
//!
//! WHAT: converts canonical/interned paths and raw source positions into user-facing display text.
//! WHY: diagnostics should centralize filesystem-adjacent rendering at the render boundary.

use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::basic::{normalize_path, portable_path_text};
use std::path::{Path, PathBuf};

pub(crate) fn relative_display_path_from_root(scope: &Path, root: &Path) -> String {
    let normalized_scope = normalize_path(scope);
    let normalized_root = normalize_path(root);

    let display_path = normalized_scope
        .strip_prefix(&normalized_root)
        .unwrap_or(&normalized_scope);

    portable_path_text(display_path)
}

pub(crate) fn resolved_display_path(scope: &InternedPath, string_table: &StringTable) -> String {
    let source_file = resolve_source_file_path(scope, string_table);

    match std::env::current_dir() {
        Ok(dir) => relative_display_path_from_root(&source_file, &dir),
        Err(err) => {
            eprintln!(
                "Compiler failed to determine the current directory for diagnostic display. {err}"
            );
            portable_path_text(&source_file)
        }
    }
}

pub(crate) fn resolve_source_file_path(
    scope: &InternedPath,
    string_table: &StringTable,
) -> PathBuf {
    let mut source_file = normalize_path(&scope.to_path_buf(string_table));

    // Header diagnostics use a synthetic "file.moth/header_name.header" scope so the terminal and
    // dev-server error pages both need to strip that suffix back to the original source file.
    if source_file
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name.ends_with(".header"))
    {
        source_file = match source_file.parent() {
            Some(parent) => parent.to_path_buf(),
            None => source_file,
        };
    }

    match std::fs::canonicalize(&source_file) {
        Ok(canonical_path) => normalize_path(&canonical_path),
        Err(_) => source_file,
    }
}
