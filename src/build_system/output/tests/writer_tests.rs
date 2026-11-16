use crate::build_system::output::WriteMode;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::fs;

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

// Generic filesystem failures are not path rejections: the native path stays authoritative on the
// failure and the message names it through portable display text. On Windows these cases exercise
// the native `\` to portable `/` rewrite without depending on a literal-backslash filename.
#[test]
fn parent_creation_failure_names_the_parent_in_portable_text() {
    let temp = tempfile::tempdir().expect("should create test directory");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).expect("should create project directory");
    let blocked_parent = project.join("out");
    fs::write(&blocked_parent, "not a directory").expect("should block parent creation");
    let destination = blocked_parent.join("page.js");

    let failure = super::write_bytes_output(&destination, b"page", WriteMode::AlwaysWrite)
        .expect_err("a regular file parent must block directory creation");

    assert_eq!(
        failure.host_path.as_deref(),
        Some(blocked_parent.as_path()),
        "the native parent path stays authoritative"
    );
    let portable_parent = portable_path_text(&blocked_parent);
    assert!(
        failure.message.contains(&format!("'{portable_parent}'")),
        "failure text should name the portable parent, got: {}",
        failure.message
    );
}

#[test]
fn file_write_failure_names_the_destination_in_portable_text() {
    let temp = tempfile::tempdir().expect("should create test directory");
    let destination = temp.path().join("project").join("out").join("page.js");
    fs::create_dir_all(&destination).expect("should create directory destination");

    let failure = super::write_bytes_output(&destination, b"page", WriteMode::AlwaysWrite)
        .expect_err("writing over a directory must fail");

    assert_eq!(
        failure.host_path.as_deref(),
        Some(destination.as_path()),
        "the native destination path stays authoritative"
    );
    let portable_destination = portable_path_text(&destination);
    assert!(
        failure
            .message
            .contains(&format!("'{portable_destination}'")),
        "failure text should name the portable destination, got: {}",
        failure.message
    );
}
