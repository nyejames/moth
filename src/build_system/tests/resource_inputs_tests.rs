//! Invariants for the build-owned, byte-free resource input registry.

use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use std::path::Path;

#[test]
fn repeated_physical_sources_share_one_unhashed_record() {
    let mut registry = ResourceInputRegistry::new();
    let first = registry.register_source(Path::new("/project/assets/logo.svg").to_path_buf());
    let second = registry.register_source(Path::new("/project/assets/logo.svg").to_path_buf());

    assert_eq!(first, second);
    assert_eq!(registry.records().len(), 1);
    assert_eq!(registry.records()[0].source_id(), first);
    assert_eq!(
        registry.records()[0].canonical_source_path(),
        Path::new("/project/assets/logo.svg")
    );
    assert_eq!(registry.records()[0].watch_interests().len(), 1);
}

#[test]
fn missing_targets_keep_only_deduplicated_watch_interests() {
    let mut registry = ResourceInputRegistry::new();
    registry.record_missing_target_watch(Path::new("/project/assets/missing.svg").to_path_buf());
    registry.record_missing_target_watch(Path::new("/project/assets/missing.svg").to_path_buf());

    assert!(registry.records().is_empty());
    assert_eq!(registry.missing_watch_interests().len(), 1);
    assert_eq!(
        registry.missing_watch_interests()[0].canonical_path(),
        Path::new("/project/assets/missing.svg")
    );
}
