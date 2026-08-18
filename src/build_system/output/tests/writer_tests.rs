#[cfg(windows)]
#[test]
fn hard_link_inspection_fails_closed_when_a_file_cannot_be_opened() {
    let temp = tempfile::tempdir().expect("should create test directory");
    let root = temp.path().to_path_buf();
    let metadata = std::fs::metadata(&root).expect("test directory metadata should exist");
    let result = super::inspect_hard_link_count(&root.join("missing-output"), &metadata);

    assert!(
        result.is_err(),
        "unopenable files must fail hard-link inspection"
    );
}
