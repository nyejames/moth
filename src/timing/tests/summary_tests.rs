//! Structured summary model tests.
//!
//! WHAT: pins the basic-report policy: architecture order, thresholds,
//!      zero suppression, bounded `Other`, hidden unknown metrics and
//!      deterministic construction from shuffled events.
//! WHY:  the summary is pure structured data; these tests run before any
//!       terminal rendering so policy bugs cannot hide behind styling.

use crate::timing::enabled::summary::{
    TimingCommandKind, TimingEmphasis, TimingMeasurementKind, build_timing_summary,
};
use crate::timing::{BenchmarkObservationSnapshot, TimingObservation};
use std::time::Duration;

fn snapshot_with(entries: &[(&'static str, f64)]) -> BenchmarkObservationSnapshot {
    BenchmarkObservationSnapshot {
        timings: entries
            .iter()
            .map(|(name, millis)| TimingObservation {
                name,
                duration: Duration::from_secs_f64(millis / 1000.0),
                label: None,
            })
            .collect(),
        counters: Vec::new(),
    }
}

fn build_snapshot() -> BenchmarkObservationSnapshot {
    snapshot_with(&[
        ("command.build.total", 100.0),
        ("build_project.bootstrap", 10.0),
        ("stage0.directory.module_inventory", 20.0),
        ("stage0.directory.module_compile_batch", 30.0),
        ("build_project.backend", 15.0),
        ("output.write_total", 5.0),
        ("frontend.file_prepare", 8.0),
        ("frontend.header_bind", 4.0),
        ("frontend.dependency_sort", 2.0),
        ("frontend.ast", 50.0),
        ("frontend.hir", 3.0),
        ("frontend.borrow", 1.0),
        ("backend.js.lower_hir", 2.0),
        ("backend.js.render_html_document", 1.0),
    ])
}

#[test]
fn sections_follow_architecture_order() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);

    let titles: Vec<&str> = report
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec![
            "Build pipeline",
            "Frontend work · 1 module · accumulated",
            "Backend"
        ]
    );
}

#[test]
fn command_total_uses_total_emphasis() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let pipeline = &report.sections[0];
    let total_row = &pipeline.rows[0];

    assert_eq!(total_row.label, "Command total");
    assert_eq!(total_row.emphasis, TimingEmphasis::Total);
    assert_eq!(total_row.kind, TimingMeasurementKind::WallSpan);
}

#[test]
fn other_is_bounded_and_never_negative() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let pipeline = &report.sections[0];
    let other = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Other")
        .expect("Other should be present");

    // 100 - (10 + 20 + 30 + 15 + 5) = 20ms.
    assert_eq!(other.total.as_millis(), 20);
}

#[test]
fn other_is_omitted_when_insignificant() {
    let snapshot = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build_project.bootstrap", 10.0),
        ("stage0.directory.module_inventory", 20.0),
        ("stage0.directory.module_compile_batch", 30.0),
        ("build_project.backend", 15.0),
        ("output.write_total", 24.9),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let pipeline = &report.sections[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Other"),
        "0.1ms Other must be omitted"
    );
}

#[test]
fn zero_rows_are_suppressed_before_rounding() {
    let snapshot = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build_project.bootstrap", 0.004),
        ("stage0.directory.module_inventory", 20.0),
        ("stage0.directory.module_compile_batch", 30.0),
        ("build_project.backend", 15.0),
        ("output.write_total", 5.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let pipeline = &report.sections[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Bootstrap"),
        "a 0.004ms row must be suppressed before rounding"
    );
}

#[test]
fn unknown_metrics_stay_hidden_from_basic_output() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "backend.html.tracked_assets_emit",
        duration: Duration::from_millis(50),
        label: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let all_labels: Vec<&str> = report
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| row.label)
        .collect();

    assert!(!all_labels.contains(&"tracked_assets_emit"));
    assert!(
        snapshot
            .timings
            .iter()
            .any(|observation| observation.name == "backend.html.tracked_assets_emit"),
        "raw snapshot must retain unknown metrics"
    );
}

#[test]
fn child_thresholds_apply_at_exact_boundaries() {
    // Child at exactly 1ms and 5% of a 20ms parent: shown.
    let shown = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build_project.backend", 20.0),
        ("backend.js.lower_hir", 1.0),
    ]);
    let report = build_timing_summary(&shown, TimingCommandKind::Build, true);
    let backend = report
        .sections
        .iter()
        .find(|section| section.title == "Backend")
        .expect("backend section should exist");
    assert_eq!(backend.rows.len(), 1);

    // Child at 0.99ms: hidden.
    let hidden = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build_project.backend", 20.0),
        ("backend.js.lower_hir", 0.99),
    ]);
    let report = build_timing_summary(&hidden, TimingCommandKind::Build, true);
    assert!(
        report
            .sections
            .iter()
            .all(|section| section.title != "Backend"),
        "sub-threshold child must hide the whole backend section"
    );
}

#[test]
fn shuffled_event_order_produces_identical_rows() {
    let mut entries: Vec<(&'static str, f64)> = build_snapshot()
        .timings
        .iter()
        .map(|observation| {
            (
                observation.name,
                observation.duration.as_secs_f64() * 1000.0,
            )
        })
        .collect();
    entries.reverse();

    let shuffled = snapshot_with(&entries);
    let ordered = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let shuffled_report = build_timing_summary(&shuffled, TimingCommandKind::Build, true);

    let row_signature = |report: &crate::timing::enabled::summary::TimingSummaryReport| {
        report
            .sections
            .iter()
            .map(|section| {
                section
                    .rows
                    .iter()
                    .map(|row| (row.label, row.total.as_millis()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(row_signature(&ordered), row_signature(&shuffled_report));
}

#[test]
fn failed_command_title_records_duration_without_changing_metrics() {
    let snapshot = build_snapshot();
    let failed = build_timing_summary(&snapshot, TimingCommandKind::Build, false);
    let succeeded = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    assert!(failed.title.starts_with("Build timings · failed after "));
    assert_eq!(failed.command_total, succeeded.command_total);
}
