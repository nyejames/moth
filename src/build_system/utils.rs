//! Shared build-system helpers used across output writing, cleanup, and HTML project assembly.
//!
//! WHAT: small one-liner wrappers that appear in multiple build-system modules.
//! WHY: avoids duplicating the same helper in every file that touches filesystem paths.

use crate::compiler_frontend::compiler_messages::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey,
};

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

/// Shorthand to build a file-error infrastructure failure.
///
/// WHAT: wraps `CompilerError::file_error` with the same argument shape everywhere.
/// WHY: keeps call sites short and consistent across build, cleanup, and HTML builder modules.
pub(crate) fn file_error(path: &Path, msg: impl Into<String>) -> CompilerError {
    CompilerError::file_error(path, msg)
}

/// Like `file_error`, but also sets a typed rejection reason as metadata.
///
/// WHAT: creates a file-error `CompilerError` with the `OutputRejectionReason`
/// metadata key set, so tests can assert the exact rejection contract.
/// WHY: all writer rejections share `ErrorType::File`; the typed reason seam
/// lets tests distinguish between distinct safety contracts.
pub(crate) fn file_error_with_rejection_reason(
    path: &Path,
    msg: impl Into<String>,
    reason: crate::build_system::output::OutputRejectionReason,
) -> CompilerError {
    let mut error = CompilerError::file_error(path, msg);
    error.new_metadata_entry(
        CompilerErrorMetadataKey::OutputRejectionReason,
        reason.as_metadata_value().to_owned(),
    );
    error
}
