//! Stable path-derived identifiers for HTML external JavaScript assets.
//!
//! WHAT: centralizes the deterministic canonical-path hashing used for glue module names.
//! WHY: generated glue module output names need a stable path-derived name without relying
//! on process-random hashers; generated package identities and JS asset output names are
//! portable logical spellings and must never hash the canonical path.

use std::path::Path;

/// Computes a stable 64-bit FNV-1a hash for a canonical path and returns lowercase hex.
pub(crate) fn stable_path_hash_hex(path: &Path) -> String {
    stable_hash_hex(&path.display().to_string())
}

fn stable_hash_hex(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")
}
