//! Shared build-system helpers used across output writing, cleanup, resource hashing and HTML
//! project assembly.
//!
//! WHAT: small one-liner wrappers that appear in multiple build-system modules.
//! WHY: avoids duplicating the same helper in every file that touches filesystem paths or bytes.

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
//  Content Hashing
// -------------------------

/// Stable 64-bit FNV-1a hash of `bytes`.
///
/// WHY: build facts keyed by content must match across processes and machines, which rules out
/// `DefaultHasher`'s random seed, and FNV-1a needs no extra dependency.
pub(crate) fn stable_content_hash(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;

    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
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
