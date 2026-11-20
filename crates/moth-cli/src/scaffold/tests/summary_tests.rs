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
        created: vec![PathBuf::from("@config.moth"), PathBuf::from("@entry.moth")],
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
        created: vec![PathBuf::from("@config.moth"), PathBuf::from("@entry.moth")],
        updated: vec![PathBuf::from(".gitignore")],
        replaced: vec![PathBuf::from("old-config.moth")],
        skipped: vec![PathBuf::from("README.md")],
    };
    let summary = render_summary(&report, None);

    assert!(summary.contains("Created:"));
    assert!(summary.contains("  @config.moth"));
    assert!(summary.contains("  @entry.moth"));
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
    #[cfg(not(windows))]
    assert!(summary.contains("cd '/path/to/site'"));
    #[cfg(windows)]
    assert!(summary.contains("Set-Location -LiteralPath 'FileSystem::"));

    // When the project is the current directory, no cd command appears.
    let temp = tempfile::tempdir().unwrap();
    let current = temp.path();
    let current_report = CreateProjectReport {
        project_path: current.to_path_buf(),
        project_name: String::from("site"),
        created: vec![PathBuf::from("@config.moth")],
        updated: Vec::new(),
        skipped: Vec::new(),
        replaced: Vec::new(),
    };
    let summary = render_summary(&current_report, Some(current));
    assert!(summary.ends_with("Next:\n  moth check .\n  moth dev ."));
}

fn is_directory_command(line: &str) -> bool {
    line.starts_with("  cd ") || line.starts_with("  Set-Location ")
}

#[cfg(unix)]
#[test]
fn summary_directory_command_round_trips_a_literal_path() {
    let temp = tempfile::tempdir().unwrap();
    let mut report = dummy_report();
    report.project_path = temp.path().join("Priya's $site `draft` [1]");
    std::fs::create_dir(&report.project_path).unwrap();

    let summary = render_summary(&report, Some(temp.path()));
    let command = summary
        .lines()
        .find(|line| is_directory_command(line))
        .unwrap();
    let output = std::process::Command::new("sh")
        .args(["-c", &format!("{command} && pwd -P")])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "{:?}", output.stderr);
    let canonical = report.project_path.canonicalize().unwrap();
    assert_eq!(
        output.stdout.strip_suffix(b"\n"),
        Some(canonical.as_os_str().as_encoded_bytes())
    );
}

#[cfg(windows)]
#[test]
fn summary_directory_command_round_trips_in_powershell() {
    let temp = tempfile::tempdir().unwrap();
    let mut report = dummy_report();
    report.project_path = temp.path().join("Priya's $site `draft` [1] ’notes’");
    std::fs::create_dir(&report.project_path).unwrap();
    // A marker proves the shell reached the project directory without comparing path spellings:
    // a CI temp directory can sit under an 8.3 short name such as `RUNNER~1`, which PowerShell
    // need not report in the spelling the path was created with. The probe resolves the marker
    // with .NET from PowerShell's provider path rather than through a relative PowerShell path:
    // Windows PowerShell resolves relative paths against a current location containing `[`, `]`
    // or a backtick through wildcard-aware code, which can miss a file that exists.
    let marker = "moth-test-marker";
    std::fs::write(report.project_path.join(marker), "").unwrap();

    let summary = render_summary(&report, Some(temp.path()));
    let command = summary
        .lines()
        .find(|line| is_directory_command(line))
        .unwrap();
    let script = format!(
        "$ErrorActionPreference = 'Stop'; {command}; \
         $location = (Get-Location).ProviderPath; \
         if (-not [System.IO.File]::Exists([System.IO.Path]::Combine($location, '{marker}'))) {{ \
         [Console]::Error.WriteLine(($location.ToCharArray() | ForEach-Object {{ [int]$_ }}) -join ' '); \
         [System.IO.Directory]::GetFileSystemEntries($location) | ForEach-Object {{ [Console]::Error.WriteLine($_) }}; \
         exit 1 }}"
    );
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let expected_code_points = report
        .project_path
        .to_str()
        .unwrap()
        .encode_utf16()
        .map(|unit| unit.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        output.status.success(),
        "command: {command}\nexpected UTF-16 code units: {expected_code_points}\n\
         stdout: {}\nstderr (location UTF-16 code units, then entries): {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn summary_escapes_controls_and_omits_the_directory_command() {
    let mut report = dummy_report();
    report.project_path = report.project_path.join("bad\u{202e}path");
    report.project_name = String::from("site\u{7}");
    report.created.push(PathBuf::from("file\r.moth"));
    let summary = render_summary(&report, None);

    assert!(summary.contains(r"bad\u{202e}path"));
    assert!(summary.contains(r"site\u{7}"));
    assert!(summary.contains(r"file\r.moth"));
    assert!(!summary.lines().any(is_directory_command));
    assert!(summary.ends_with(
        "Next:\n  Open the project directory in your terminal before running these commands:\n  moth check .\n  moth dev ."
    ));
}

#[cfg(unix)]
#[test]
fn summary_does_not_offer_a_lossy_directory_command() {
    use std::os::unix::ffi::OsStrExt;

    let mut report = dummy_report();
    report
        .project_path
        .push(std::ffi::OsStr::from_bytes(b"site\xff"));
    let summary = render_summary(&report, None);

    assert!(!summary.lines().any(is_directory_command));
    assert!(summary.contains("Open the project directory in your terminal"));
}

#[test]
fn summary_always_shows_next_steps() {
    let report = dummy_report();
    let summary = render_summary(&report, None);
    assert!(summary.contains("Next:"));
    assert!(summary.contains("moth check ."));
    assert!(summary.contains("moth dev ."));
}
