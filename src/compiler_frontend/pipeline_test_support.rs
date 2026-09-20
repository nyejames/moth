//! Test-only per-path preparation observation.
//!
//! WHAT: counts each source path entering frontend preparation and exposes reset/read helpers to
//!       the cache-invariant tests.
//! WHY: the production source provider is not injectable and the existing aggregate preparation
//!      counters do not retain path granularity. The `synthetic_preparation_*` and repeated
//!      template tests therefore need one test-owned observer while the shipping path uses a
//!      zero-cost no-op counterpart.

use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use std::collections::HashMap;
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

pub(crate) fn reset_file_frontend_prepare_count_for_test(tracked_prefix: &Path) {
    *FILE_FRONTEND_PREPARE_TRACK_PREFIX
        .lock()
        .expect("file preparation test hook lock poisoned") = Some(tracked_prefix.to_path_buf());
    FILE_FRONTEND_PREPARE_COUNTS
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .clear();
}

pub(crate) fn file_frontend_prepare_count_for_path_for_test(path: &Path) -> usize {
    FILE_FRONTEND_PREPARE_COUNTS
        .lock()
        .expect("file preparation count test hook lock poisoned")
        .get(path)
        .copied()
        .unwrap_or(0)
}
