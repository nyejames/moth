//! Test-only per-path preparation observation.
//!
//! WHAT: counts each source path entering frontend preparation and exposes reset/read helpers to
//!       the cache-invariant tests.
//! WHY: the production source provider is not injectable and the existing aggregate preparation
//!      counters do not retain path granularity. The `synthetic_preparation_*` and repeated
//!      template tests therefore need one test-owned observer while the shipping path uses a
//!      zero-cost no-op counterpart.
//!
//! Paths are keyed in canonical form because the source identity supplies a canonical OS path.
//! Reset and lookup canonicalize their caller's path for the same reason: temp-directory paths
//! such as macOS `/var/...` resolve to `/private/var/...`, and comparing a raw path against a
//! canonical key would silently observe zero preparations.

use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static FILE_FRONTEND_PREPARE_COUNTS: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FILE_FRONTEND_PREPARE_TRACK_PREFIX: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(super) fn record_prepare(source_files: &SourceDatabase, source_id: SourceId) {
    let source_path = source_files
        .get(source_id)
        .and_then(|identity| identity.canonical_os_path.as_deref())
        .map(Path::to_path_buf)
        .expect("frontend preparation source should have a canonical OS path");
    let prefix = FILE_FRONTEND_PREPARE_TRACK_PREFIX
        .lock()
        .expect("file preparation test hook lock poisoned");
    if prefix
        .as_ref()
        .is_none_or(|tracked_prefix| source_path.starts_with(tracked_prefix))
    {
        *FILE_FRONTEND_PREPARE_COUNTS
            .lock()
            .expect("file preparation count test hook lock poisoned")
            .entry(source_path.clone())
            .or_insert(0) += 1;
    }
}

/// Canonicalize a caller-supplied test path, falling back to the supplied spelling when the path
/// does not exist. The fallback keeps missing-fixture lookups from panicking; recorded entries
/// always come from the source identity's canonical OS path.
fn canonical_or_supplied(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn reset_file_frontend_prepare_count_for_test(tracked_prefix: &Path) {
    *FILE_FRONTEND_PREPARE_TRACK_PREFIX
        .lock()
        .expect("file preparation test hook lock poisoned") =
        Some(canonical_or_supplied(tracked_prefix));
    FILE_FRONTEND_PREPARE_COUNTS
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .clear();
}

pub(crate) fn file_frontend_prepare_count_for_path_for_test(path: &Path) -> usize {
    let canonical_path = canonical_or_supplied(path);
    FILE_FRONTEND_PREPARE_COUNTS
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .get(&canonical_path)
        .copied()
        .unwrap_or(0)
}
