//! Shared build-system helpers used across output writing, cleanup, and HTML project assembly.
//!
//! WHAT: small one-liner wrappers that appear in multiple build-system modules.
//! WHY: avoids duplicating the same helper in every file that touches filesystem paths.

use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::fs;
use std::path::Path;

// -------------------------
//  Filesystem Helpers
// -------------------------

/// Skip writing when the destination already has identical bytes.
///
/// WHAT: reads the existing file and compares against the proposed content.
/// WHY: avoids touching the filesystem (and triggering downstream watchers) when nothing changed.
pub(crate) fn should_skip_unchanged_write(
    path: &Path,
    next_bytes: &[u8],
    write_mode: crate::build_system::output::WriteMode,
) -> bool {
    if write_mode != crate::build_system::output::WriteMode::SkipUnchanged {
        return false;
    }

    match fs::read(path) {
        Ok(existing_bytes) => existing_bytes == next_bytes,
        Err(_) => false,
    }
}

// -------------------------
//  Diagnostic Helpers
// -------------------------

/// Shorthand to build a file-error diagnostic message.
///
/// WHAT: wraps `CompilerMessages::file_error` with the same argument shape everywhere.
/// WHY: keeps call sites short and consistent across build, cleanup, and HTML builder modules.
pub(crate) fn file_error_messages(
    path: &Path,
    msg: impl Into<String>,
    string_table: &StringTable,
) -> CompilerMessages {
    CompilerMessages::file_error(path, msg, string_table)
}
