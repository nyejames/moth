//! Filesystem regression coverage for conservative validation housekeeping.

use super::*;
use crate::mode::{BenchmarkMode, ModeParseResult};
use std::fs::{File, FileTimes};
use tempfile::tempdir;

#[test]
fn maintenance_mode_rejects_extra_arguments() {
    assert!(matches!(
        BenchmarkMode::parse_args(&["cache-maintain".into()]),
        ModeParseResult::Mode(BenchmarkMode::CacheMaintain)
    ));
    assert!(matches!(
        BenchmarkMode::parse_args(&["cache-maintain".into(), "--delete-all".into()]),
        ModeParseResult::Error(_)
    ));
}

#[test]
fn budget_is_positive_and_cannot_overflow() {
    assert_eq!(budget_bytes("32").expect("valid budget"), 32 * GIB);
    for invalid in ["0", "-1", "", "garbage", "18446744073709551615"] {
        assert!(budget_bytes(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn pruning_preserves_recent_reports_and_all_build_artifacts() {
    let workspace = tempdir().expect("fixture directory");
    let timings = workspace.path().join("cargo-timings");
    fs::create_dir(&timings).expect("timings directory");
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
    let old = now - MINIMUM_REPORT_AGE - Duration::from_secs(1);

    for number in 0..14 {
        let path = timings.join(format!("cargo-timing-{number:02}.html"));
        fs::write(&path, b"report").expect("report fixture");
        let modified = if number == 0 || number == 13 { now } else { old };
        File::options()
            .write(true)
            .open(path)
            .expect("open report")
            .set_times(FileTimes::new().set_modified(modified))
            .expect("set fixture timestamp");
    }
    fs::write(timings.join("cargo-timing.html"), b"latest").expect("latest report");
    fs::write(timings.join("notes.txt"), b"notes").expect("unowned file");
    fs::create_dir(timings.join("cargo-timing-directory.html")).expect("unowned directory");
    let artifacts = workspace.path().join("debug/deps");
    fs::create_dir_all(&artifacts).expect("artifact directory");
    fs::write(artifacts.join("keep.rlib"), b"compiled").expect("artifact fixture");

    assert_eq!(prune_timing_reports(&timings, now).expect("prune reports"), 4);
    assert!(timings.join("cargo-timing-00.html").is_file());
    assert!(timings.join("cargo-timing-13.html").is_file());
    assert_eq!(fs::read(timings.join("cargo-timing.html")).unwrap(), b"latest");
    assert_eq!(fs::read(timings.join("notes.txt")).unwrap(), b"notes");
    assert!(timings.join("cargo-timing-directory.html").is_dir());
    assert_eq!(fs::read(artifacts.join("keep.rlib")).unwrap(), b"compiled");
    assert_eq!(prune_timing_reports(&timings, now).expect("repeat cleanup"), 0);
}

#[test]
fn recent_and_future_reports_are_not_removed() {
    let directory = tempdir().expect("fixture directory");
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
    for number in 0..14 {
        let path = directory.path().join(format!("cargo-timing-{number:02}.html"));
        fs::write(&path, b"recent").expect("report fixture");
        File::options()
            .write(true)
            .open(path)
            .expect("open report")
            .set_times(FileTimes::new().set_modified(now + Duration::from_secs(number)))
            .expect("set fixture timestamp");
    }
    assert_eq!(prune_timing_reports(directory.path(), now).unwrap(), 0);
}

#[test]
fn missing_timings_are_a_no_op_and_size_counts_regular_files() {
    let directory = tempdir().expect("fixture directory");
    assert_eq!(
        prune_timing_reports(&directory.path().join("missing"), SystemTime::now()).unwrap(),
        0
    );
    fs::create_dir(directory.path().join("nested")).expect("nested fixture");
    fs::write(directory.path().join("a"), b"123").expect("first file");
    fs::write(directory.path().join("nested/b"), b"4567").expect("second file");
    assert_eq!(logical_target_bytes(directory.path()).unwrap(), 7);
}

#[cfg(unix)]
#[test]
fn links_never_expand_the_cleanup_or_size_scope() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("fixture directory");
    let outside = tempdir().expect("outside fixture directory");
    let sentinel = outside.path().join("cargo-timing-00.html");
    fs::write(&sentinel, b"outside").expect("sentinel");
    symlink(outside.path(), directory.path().join("cargo-timings")).expect("directory link");
    symlink(&sentinel, directory.path().join("cargo-timing-01.html")).expect("file link");
    assert!(
        prune_timing_reports(&directory.path().join("cargo-timings"), SystemTime::now()).is_err()
    );
    assert_eq!(
        prune_timing_reports(directory.path(), SystemTime::now()).unwrap(),
        0
    );
    assert_eq!(logical_target_bytes(directory.path()).unwrap(), 0);
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside");
}
