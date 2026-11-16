//! Diagnostic path-fact interning at the command-owned string-table boundary.
//!
//! WHAT: asserts the exact portable text and identity reuse of `intern_path`, and that ordinary
//! text interning still keeps its input verbatim.
//! WHY: path facts must share the command's one string identity domain and render through the sole
//! display formatter, never through raw native path text.

use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

#[test]
fn intern_path_renders_native_components_with_forward_slashes() {
    let mut table = StringTable::new();
    let mut builder = DiagnosticStringBuilder::new(&mut table);
    let native_path = Path::new("src").join("docs").join("main.moth");

    let path_id = builder.intern_path(&native_path);

    assert_eq!(builder.strings().resolve(path_id), "src/docs/main.moth");
}

#[test]
fn intern_path_rewrites_synthetic_backslash_input() {
    let mut table = StringTable::new();
    let mut builder = DiagnosticStringBuilder::new(&mut table);

    let path_id = builder.intern_path(Path::new(r"src\docs\main.moth"));

    assert_eq!(builder.strings().resolve(path_id), "src/docs/main.moth");
}

#[cfg(windows)]
#[test]
fn intern_path_strips_verbatim_drive_prefix() {
    let mut table = StringTable::new();
    let mut builder = DiagnosticStringBuilder::new(&mut table);

    let path_id = builder.intern_path(Path::new(r"\\?\C:\work\main.moth"));

    assert_eq!(builder.strings().resolve(path_id), "C:/work/main.moth");
}

#[test]
fn intern_path_reuses_ids_within_the_command_string_domain() {
    let mut table = StringTable::new();
    let mut builder = DiagnosticStringBuilder::new(&mut table);
    let native_path = Path::new("src").join("docs").join("main.moth");

    let first = builder.intern_path(&native_path);
    let repeated = builder.intern_path(&native_path);
    let already_interned = builder.intern("src/docs/main.moth");

    assert_eq!(first, repeated);
    assert_eq!(first, already_interned);
}

#[test]
fn intern_text_preserves_backslashes_verbatim() {
    let mut table = StringTable::new();
    let mut builder = DiagnosticStringBuilder::new(&mut table);

    let text_id = builder.intern(r"src\docs\main.moth");

    assert_eq!(builder.strings().resolve(text_id), r"src\docs\main.moth");
}
