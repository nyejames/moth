//! Tests for the `moth new html` success summary.
//!
//! Each test owns one summary contract: the project identity header, the four
//! file-action sections, empty-section omission, the cd-conditional next-step
//! block, and the always-present next steps.

use super::{CreateProjectReport, render_summary};
use std::path::{Path, PathBuf};

fn dummy_report() -> CreateProjectReport {
    CreateProjectReport {
        project_path: PathBuf::from("/path/to/site"),
        project_name: String::from("site"),
        created: vec![
            PathBuf::from("config.moth"),
            PathBuf::from("src/#page.moth"),
        ],
        updated: Vec::new(),
        skipped: Vec::new(),
        replaced: Vec::new(),
    }
}

#[test]
fn summary_shows_project_path_and_name() {
    let report = dummy_report();
    let summary = render_summary(&report, None);
    assert!(summary.contains("Created Moth HTML project:"));
    assert!(summary.contains("Project path: /path/to/site"));
    assert!(summary.contains("Project name: site"));
}

#[test]
fn summary_lists_each_file_action_section() {
    let report = CreateProjectReport {
        project_path: PathBuf::from("/path/to/site"),
        project_name: String::from("site"),
        created: vec![
            PathBuf::from("config.moth"),
            PathBuf::from("src/#page.moth"),
        ],
        updated: vec![PathBuf::from(".gitignore")],
        replaced: vec![PathBuf::from("old-config.moth")],
        skipped: vec![PathBuf::from("README.md")],
    };
    let summary = render_summary(&report, None);

    assert!(summary.contains("Created:"));
    assert!(summary.contains("  config.moth"));
    assert!(summary.contains("  src/#page.moth"));
    assert!(summary.contains("Updated:"));
    assert!(summary.contains("  .gitignore"));
    assert!(summary.contains("Replaced:"));
    assert!(summary.contains("  old-config.moth"));
    assert!(summary.contains("Skipped:"));
    assert!(summary.contains("  README.md"));
}

#[test]
fn summary_omits_empty_sections() {
    let report = dummy_report();
    let summary = render_summary(&report, None);
    assert!(!summary.contains("Updated:"));
    assert!(!summary.contains("Replaced:"));
    assert!(!summary.contains("Skipped:"));
}

#[test]
fn summary_includes_cd_only_when_project_is_not_current_dir() {
    // When the project is not the current directory, the cd command appears.
    let report = dummy_report();
    let summary = render_summary(&report, Some(Path::new("/other")));
    assert!(summary.contains("cd /path/to/site"));

    // When the project is the current directory, no cd command appears.
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path();
    let current_report = CreateProjectReport {
        project_path: current.to_path_buf(),
        project_name: String::from("site"),
        created: vec![PathBuf::from("config.moth")],
        updated: Vec::new(),
        skipped: Vec::new(),
        replaced: Vec::new(),
    };
    let summary = render_summary(&current_report, Some(current));
    assert!(!summary.contains("cd "));
}

#[test]
fn summary_always_shows_next_steps() {
    let report = dummy_report();
    let summary = render_summary(&report, None);
    assert!(summary.contains("Next:"));
    assert!(summary.contains("moth check ."));
    assert!(summary.contains("moth dev ."));
}
