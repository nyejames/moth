//! Filesystem regression coverage for conservative validation housekeeping; spelling and
//! Gregorian range ownership markers live in the shape-table test below.

use super::*;
use crate::mode::{BenchmarkMode, ModeParseResult};
use std::fs::{File, FileTimes};
use tempfile::tempdir;

// Names the installed stable toolchain actually emits: a UTC clock text with packed
// milliseconds and a lowercase hex run identity, and the plain report alias.
const NEWEST_REPORT_NAME: &str = "cargo-timing-20260930T000933271Z-01dec10e257a0746.html";
const PLAIN_REPORT_ALIAS: &str = "cargo-timing.html";

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
    let old = now - MINIMUM_REPORT_AGE;

    // Equal old mtimes exercise the filename tie-break as well as the newest-ten rule.
    for second in 1..=14 {
        write_aged_report(
            &timings,
            &format!("cargo-timing-20260101T0000{second:02}350Z-00112233445566{second:02}.html"),
            old,
        );
    }
    write_aged_report(&timings, NEWEST_REPORT_NAME, now);

    // Old files under recognisable-looking spellings that no Cargo report emission writes.
    let unknown_reports = [
        "cargo-timing-T.html",
        "cargo-timing-bogus.html",
        "cargo-timing-20260101T0000033Z.html",
        "cargo-timing-12.html",
        "cargo-timing-12345678901234.html",
        "cargo-timing-20261301T000000000Z-0000000000000000.html",
        "cargo-timing-20260229T000000000Z-0000000000000000.html",
        "cargo-timing-20260930T240000000Z-0000000000000000.html",
    ];
    for name in unknown_reports {
        write_aged_report(&timings, name, old);
    }
    fs::write(timings.join(PLAIN_REPORT_ALIAS), b"latest").expect("latest report");
    fs::write(timings.join("notes.txt"), b"notes").expect("unowned file");
    fs::create_dir(timings.join("cargo-timing-directory.html")).expect("unowned directory");
    let artifacts = workspace.path().join("debug/deps");
    fs::create_dir_all(&artifacts).expect("artifact directory");
    fs::write(artifacts.join("keep.rlib"), b"compiled").expect("artifact fixture");

    // The five oldest archived reports fall below the newest-ten line and go.
    assert_eq!(
        prune_timing_reports(&timings, now).expect("prune reports"),
        5
    );
    for second in 1..=5 {
        assert!(
            !timings
                .join(format!(
                    "cargo-timing-20260101T0000{second:02}350Z-00112233445566{second:02}.html"
                ))
                .exists(),
            "old report {second} survived"
        );
    }
    for second in 6..=14 {
        assert!(
            timings
                .join(format!(
                    "cargo-timing-20260101T0000{second:02}350Z-00112233445566{second:02}.html"
                ))
                .is_file(),
            "retained report {second} was lost"
        );
    }
    assert!(timings.join(NEWEST_REPORT_NAME).is_file());

    // Probes and unowned files survive deletion pressure.
    for name in unknown_reports {
        assert!(
            timings.join(name).is_file(),
            "unknown report {name} was lost"
        );
    }
    assert_eq!(
        fs::read(timings.join(PLAIN_REPORT_ALIAS)).unwrap(),
        b"latest"
    );
    assert_eq!(fs::read(timings.join("notes.txt")).unwrap(), b"notes");
    assert!(timings.join("cargo-timing-directory.html").is_dir());
    assert_eq!(fs::read(artifacts.join("keep.rlib")).unwrap(), b"compiled");

    // Repeated runs delete nothing further once the retained set is stable.
    assert_eq!(
        prune_timing_reports(&timings, now).expect("repeat cleanup"),
        0
    );
}

#[test]
fn report_names_require_supported_spelling_and_valid_timestamps() {
    // Gregorian boundary cases distinguish ordinary, century and 400-year leap rules.
    for name in [
        "cargo-timing-20260930T000933271Z-01dec10e257a0746.html",
        "cargo-timing-20240229T120000000Z-0123456789abcdef.html",
        "cargo-timing-20000229T235959999Z-deadb00fbabe0f00.html",
        "cargo-timing-19000228T000059000Z-0fedcba987654321.html",
    ] {
        assert!(report_timestamp_shape(name), "unowned: {name}");
    }

    // Misspelled stamps, timestamps outside Gregorian clock bounds, and arbitrary user
    // files stay out of ownership even when they carry the report stem.
    for name in [
        "cargo-timing.html",
        "cargo-timing-.html",
        "cargo-timing-T.html",
        "cargo-timing-bogus.html",
        "cargo-timing-old-report.html",
        "cargo-timing-12.html",
        "cargo-timing-1.html",
        "cargo-timing-2026.html",
        "cargo-timing-20260101T.html",
        "cargo-timing-202601X1T000001Z.html",
        "cargo-timing-20260101t000001z.html",
        "cargo-timing-2026010T000001Z.html",
        "cargo-timing-202601017T000001Z.html",
        "cargo-timing-20260101X000001Z.html",
        "cargo-timing-20260101T00001Z.html",
        "cargo-timing-20260101T0000017Z.html",
        "cargo-timing-20260101T00000129Z.html",
        "cargo-timing-20260930T000933.html",
        "cargo-timing-20260101T000001.001Z.html",
        "cargo-timing-20260101T000001.001Z-0123456789abcdef.html",
        "cargo-timing-20260101T000000000Z-1.html",
        "cargo-timing-20260101T000000000Z-01dec10e257a074.html",
        "cargo-timing-20260101T000000000Z-01dec10e257a07466.html",
        "cargo-timing-20260101T000000000Z-01dec10e257a074g.html",
        "cargo-timing-20260101T000000000Z-A1DEC10E257A0746.html",
        "cargo-timing-20260101T000000000Z-0011223344AABBCCDD.html",
        "cargo-timing-20260101T00000129Z.html",
        "cargo-timing-20260101T000000000Z.html",
        "cargo-timing-20260930T000933Z-01dec10e257a0746.html",
        "cargo-timing-20260930T000933Z.html",
        "cargo-timing-69268a0ab3.html",
        "cargo-timing-12345678901234.html",
        // Gregorian calendar bounds, including leap-day handling.
        "cargo-timing-20260132T000000000Z-0000000000000000.html",
        "cargo-timing-20260229T000000000Z-0000000000000000.html",
        "cargo-timing-20240230T000000000Z-0000000000000000.html",
        "cargo-timing-20260931T000000000Z-0000000000000000.html",
        // Clock bounds.
        "cargo-timing-20260930T240000000Z-0000000000000000.html",
        "cargo-timing-20260930T006000000Z-0000000000000000.html",
        "cargo-timing-20260930T000060000Z-0000000000000000.html",
    ] {
        assert!(!report_timestamp_shape(name), "owned unexpectedly: {name}");
    }
}

#[test]
fn recent_and_future_reports_are_not_removed() {
    let directory = tempdir().expect("fixture directory");
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000);
    for number in 0..14 {
        let (name, modified) = if number % 2 == 0 {
            (
                format!("cargo-timing-20260101T0000{number:02}350Z-0011223344556677.html"),
                now - MINIMUM_REPORT_AGE + Duration::from_secs(1),
            )
        } else {
            (
                format!("cargo-timing-20260101T0000{number:02}999Z-0011223344556678.html"),
                now + Duration::from_secs(1 + number),
            )
        };
        write_aged_report(directory.path(), &name, modified);
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
    let sentinel = outside.path().join(NEWEST_REPORT_NAME);
    fs::write(&sentinel, b"outside").expect("sentinel");
    symlink(outside.path(), directory.path().join("cargo-timings")).expect("directory link");
    symlink(&sentinel, directory.path().join(NEWEST_REPORT_NAME)).expect("file link");
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

fn write_aged_report(directory: &Path, name: &str, modified: SystemTime) {
    let path = directory.join(name);
    fs::write(&path, b"report content").expect("write report fixture");
    File::options()
        .write(true)
        .open(path)
        .expect("open report")
        .set_times(FileTimes::new().set_modified(modified))
        .expect("set fixture timestamp");
}
