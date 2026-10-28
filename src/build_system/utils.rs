//! Shared build-system helpers used across output writing, cleanup, and HTML project assembly.
//!
//! WHAT: small one-liner wrappers that appear in multiple build-system modules.
//! WHY: avoids duplicating the same helper in every file that touches filesystem paths.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    InfrastructureDetail, InfrastructureFailure,
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

/// Shorthand to build a filesystem failure during output handling.
///
/// WHAT: wraps the stage-bearing filesystem constructor with the same argument shape everywhere.
/// WHY: keeps call sites short and consistent across output-writing modules.
pub(crate) fn file_error(path: &Path, msg: impl Into<String>) -> InfrastructureFailure {
    InfrastructureFailure::filesystem(CompilerStage::Output, path, msg.into())
}

/// Like `file_error`, but also retains the typed output-rejection reason.
pub(crate) fn file_error_with_rejection_reason(
    path: &Path,
    msg: impl Into<String>,
    reason: crate::build_system::output::OutputRejectionReason,
) -> InfrastructureFailure {
    InfrastructureFailure::filesystem(CompilerStage::Output, path, msg.into()).with_detail(
        InfrastructureDetail::OutputRejectionReason(reason.as_metadata_value().into()),
    )
}
