use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

static SOURCE_READ_TRACK_PREFIX_FOR_TEST: Mutex<Option<PathBuf>> = Mutex::new(None);
static SOURCE_READ_COUNTS_BY_PATH_FOR_TEST: LazyLock<Mutex<HashMap<PathBuf, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn should_count_source_read_for_test(file_path: &Path) -> bool {
    let prefix = SOURCE_READ_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("source read test hook lock poisoned");

    prefix
        .as_ref()
        .is_none_or(|tracked_prefix| file_path.starts_with(tracked_prefix))
}

pub(crate) fn reset_source_read_count_for_test(tracked_prefix: &Path) {
    let mut prefix = SOURCE_READ_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("source read test hook lock poisoned");
    *prefix = Some(tracked_prefix.to_path_buf());
    SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
        .lock()
        .expect("source read path-count test hook lock poisoned")
        .clear();
}

pub(crate) fn source_read_count_for_path_for_test(path: &Path) -> usize {
    SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
        .lock()
        .expect("source read path-count test hook lock poisoned")
        .get(path)
        .copied()
        .unwrap_or(0)
}

pub(crate) fn record_source_read_for_test(file_path: &Path) {
    *SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
        .lock()
        .expect("source read path-count test hook lock poisoned")
        .entry(file_path.to_path_buf())
        .or_insert(0) += 1;
}
