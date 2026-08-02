#[cfg(windows)]
#[test]
fn hard_link_inspection_fails_closed_when_a_file_cannot_be_opened() {
    let root =
        std::env::temp_dir().join(format!("moth-hard-link-inspection-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("should create test directory");
    let metadata = std::fs::metadata(&root).expect("test directory metadata should exist");
    let result = super::inspect_hard_link_count(&root.join("missing-output"), &metadata);

    assert!(
        result.is_err(),
        "unopenable files must fail hard-link inspection"
    );
    std::fs::remove_dir_all(&root).expect("should remove test directory");
}
