//! Self-tests for the shared source-tree walk and its display names.

use super::{relative_display_path, walk_rust_files};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn relative_paths_use_forward_slashes_under_the_workspace_root() {
    let root = Path::new("/work/moth");
    let path = root.join("xtask").join("src").join("source_audit.rs");

    assert_eq!(
        relative_display_path(root, &path).expect("an ASCII path is valid UTF-8"),
        "xtask/src/source_audit.rs"
    );
}

#[test]
fn walking_a_tree_finds_every_rust_file_and_nothing_else() {
    let workspace = tempdir().expect("temp dir");
    let root = workspace.path();
    fs::create_dir_all(root.join("nested").join("deeper")).expect("the tree should be created");
    fs::write(root.join("top.rs"), "").expect("write");
    fs::write(root.join("nested").join("middle.rs"), "").expect("write");
    fs::write(root.join("nested").join("deeper").join("leaf.rs"), "").expect("write");
    fs::write(root.join("notes.md"), "").expect("write");
    fs::write(root.join("nested").join("Cargo.toml"), "").expect("write");

    let found: Vec<String> = walk_rust_files(root)
        .expect("a readable tree should walk")
        .iter()
        .map(|path| relative_display_path(root, path).expect("a temporary path is valid UTF-8"))
        .collect();

    assert_eq!(
        found,
        vec![
            "nested/deeper/leaf.rs".to_string(),
            "nested/middle.rs".to_string(),
            "top.rs".to_string(),
        ]
    );
}

/// A walk that cannot read a directory must fail rather than report the files it did reach.
///
/// A gate reports how many files it audited. If an unreadable directory were skipped, that count
/// would describe less than the gate claims to have checked, and the gate would pass by looking
/// at a smaller tree instead of a clean one.
#[test]
fn walking_a_missing_root_fails_rather_than_reporting_an_empty_tree() {
    let workspace = tempdir().expect("temp dir");
    let missing = workspace.path().join("absent");

    let error = walk_rust_files(&missing).expect_err("a missing root is not an empty tree");

    assert!(
        error.contains("failed to read") && error.contains("absent"),
        "unexpected error: {error}"
    );
}
