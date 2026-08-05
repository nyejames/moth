//! Structured summary model tests.
//!
//! WHAT: pins the basic-report policy: architecture order, thresholds,
//!      zero suppression, bounded `Other`, hidden unknown metrics and
//!      deterministic construction from shuffled events.
//! WHY:  the summary is pure structured data; these tests run before any
//!       terminal rendering so policy bugs cannot hide behind styling.

use crate::timing::enabled::render::{
    boundary_row_text, boundary_section_title, render_row_text, slowest_module_text,
};
use crate::timing::enabled::summary::{
    TimingBoundarySummary, TimingCommandKind, TimingEmphasis, TimingMeasurementKind,
    TimingSlowestModuleSummary, TimingSummaryRow, build_timing_summary,
};
use crate::timing::{
    BenchmarkObservationSnapshot, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord,
    TimingModuleContext, TimingModuleKey, TimingModuleRecord, TimingObservation,
};
use std::borrow::Cow;
use std::time::Duration;

fn snapshot_with(entries: &[(&'static str, f64)]) -> BenchmarkObservationSnapshot {
    BenchmarkObservationSnapshot {
        timings: entries
            .iter()
            .map(|(name, millis)| TimingObservation {
                name,
                duration: Duration::from_secs_f64(millis / 1000.0),
                label: None,
                boundary: None,
                module: None,
            })
            .collect(),
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
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
        vec!["Build pipeline", "Frontend work · accumulated", "Backend"]
    );
}

#[test]
fn headings_include_command_specific_total() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    assert_eq!(report.title, "Build timings 100.00ms");

    let check_snapshot = snapshot_with(&[
        ("command.check.total", 60.0),
        ("command.check.bootstrap", 5.0),
        ("stage0.directory.module_inventory", 10.0),
        ("stage0.directory.module_compile_batch", 20.0),
    ]);
    let check_report = build_timing_summary(&check_snapshot, TimingCommandKind::Check, true);
    assert_eq!(check_report.title, "Check timings 60.00ms");
}

#[test]
fn pipeline_omits_command_total_row() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let pipeline = &report.sections[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Command total"),
        "the command total belongs in the heading, not in the pipeline rows"
    );
    assert_eq!(report.command_total, Duration::from_millis(100));
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
        boundary: None,
        module: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let all_labels: Vec<&str> = report
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .map(|row| row.label.as_ref())
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
                    .map(|row| (row.label.clone(), row.total.as_millis()))
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

#[test]
fn repeated_rows_show_aggregate_duration_without_sample_noise() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "frontend.file_prepare",
        duration: Duration::from_millis(1),
        label: Some("/absolute/path/module.moth".to_owned()),
        boundary: None,
        module: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let prepare_row = report
        .sections
        .iter()
        .flat_map(|section| section.rows.iter())
        .find(|row| row.label == "Prepare source files")
        .expect("prepare source files row should exist");

    assert_eq!(prepare_row.total, Duration::from_millis(9));
    assert_eq!(prepare_row.suffix, None);

    let text = render_row_text(prepare_row, prepare_row.label.len(), 0);
    assert!(!text.contains("across"));
    assert!(!text.contains("samples"));
    assert!(!text.contains("["));
    assert!(!text.contains("/absolute/path"));
}

#[test]
fn model_supports_dynamic_boundary_and_slowest_module_labels() {
    let mut report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    report.compilation_boundaries.push(TimingBoundarySummary {
        label: Cow::Owned("@html".to_owned()),
        module_count: 1,
        total: Duration::from_millis(5),
    });
    report.slowest_module = Some(TimingSlowestModuleSummary {
        identity: Cow::Owned("@docs/progress".to_owned()),
        source_file_count: 1,
        source_byte_count: 45_000,
        total: Duration::from_millis(16),
    });

    let boundary = &report.compilation_boundaries[0];
    assert_eq!(
        boundary_row_text(boundary, boundary.label.len()),
        "@html  1 module  5.00ms"
    );

    let slowest_module = report
        .slowest_module
        .as_ref()
        .expect("slowest module should exist");
    assert_eq!(
        slowest_module_text(slowest_module),
        "@docs/progress  16.00ms · 1 file · 43.9KB"
    );
}

#[test]
fn explicit_row_suffix_renders_only_when_set() {
    let row = TimingSummaryRow {
        label: Cow::Borrowed("Boundary"),
        kind: TimingMeasurementKind::Accumulated,
        emphasis: TimingEmphasis::Total,
        total: Duration::from_millis(5),
        suffix: Some(Cow::Borrowed("1 module")),
        children: Vec::new(),
    };

    assert_eq!(render_row_text(&row, 8, 0), "Boundary  5.00ms 1 module");

    let plain = TimingSummaryRow {
        suffix: None,
        ..row.clone()
    };
    assert_eq!(render_row_text(&plain, 8, 0), "Boundary  5.00ms");
}

fn boundary_record(id: u32, display_name: &str, module_count: u64) -> TimingBoundaryRecord {
    TimingBoundaryRecord {
        id: TimingBoundaryId::from_index(id),
        kind: TimingBoundaryKind::SourcePackage,
        display_name: display_name.to_owned(),
        module_count,
    }
}

fn module_record(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_identity: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleRecord {
    TimingModuleRecord {
        key: TimingModuleKey {
            boundary,
            module_index,
        },
        logical_identity: logical_identity.to_owned(),
        source_file_count,
        source_byte_count,
    }
}

fn attributed_observation(
    name: &'static str,
    millis: f64,
    context: TimingModuleContext,
) -> TimingObservation {
    TimingObservation {
        name,
        duration: Duration::from_secs_f64(millis / 1000.0),
        label: None,
        boundary: context.boundary,
        module: context.module,
    }
}

#[test]
fn boundary_rows_separate_packages_and_project_totals() {
    let html = TimingBoundaryId::from_index(0);
    let project = TimingBoundaryId::from_index(1);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "build.boundary.inventory",
                2.0,
                TimingModuleContext::for_boundary(html),
            ),
            attributed_observation(
                "build.boundary.compile",
                3.0,
                TimingModuleContext::for_boundary(html),
            ),
            attributed_observation(
                "build.boundary.inventory",
                40.0,
                TimingModuleContext::for_boundary(project),
            ),
            attributed_observation(
                "build.boundary.compile",
                300.0,
                TimingModuleContext::for_boundary(project),
            ),
            attributed_observation("command.build.total", 500.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![
            boundary_record(0, "@html", 1),
            boundary_record(1, "moth_docs", 69),
        ],
        modules: Vec::new(),
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    assert_eq!(report.compilation_boundaries.len(), 2);
    assert_eq!(report.compilation_boundaries[0].label, "@html");
    assert_eq!(report.compilation_boundaries[0].module_count, 1);
    assert_eq!(
        report.compilation_boundaries[0].total,
        Duration::from_millis(5)
    );
    assert_eq!(report.compilation_boundaries[1].label, "moth_docs");
    assert_eq!(report.compilation_boundaries[1].module_count, 69);
    assert_eq!(
        report.compilation_boundaries[1].total,
        Duration::from_millis(340)
    );
}

#[test]
fn boundary_rows_follow_registration_order() {
    let first = TimingBoundaryId::from_index(0);
    let second = TimingBoundaryId::from_index(1);
    let project = TimingBoundaryId::from_index(2);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "build.boundary.compile",
                1.0,
                TimingModuleContext::for_boundary(project),
            ),
            attributed_observation(
                "build.boundary.compile",
                1.0,
                TimingModuleContext::for_boundary(first),
            ),
            attributed_observation(
                "build.boundary.compile",
                1.0,
                TimingModuleContext::for_boundary(second),
            ),
            attributed_observation("command.build.total", 100.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![
            boundary_record(0, "@zeta", 1),
            boundary_record(1, "@alpha", 1),
            boundary_record(2, "main_project", 1),
        ],
        modules: Vec::new(),
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let labels = report
        .compilation_boundaries
        .iter()
        .map(|boundary| boundary.label.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(
        labels,
        vec!["@zeta", "@alpha", "main_project"],
        "display order must be registration order, not event insertion order"
    );
}

#[test]
fn same_module_index_in_two_boundaries_does_not_collide() {
    let html = TimingBoundaryId::from_index(0);
    let project = TimingBoundaryId::from_index(1);
    let html_module = TimingModuleKey {
        boundary: html,
        module_index: 0,
    };
    let project_module = TimingModuleKey {
        boundary: project,
        module_index: 0,
    };

    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.module.semantic_total",
                4.0,
                TimingModuleContext::for_module(html_module),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                9.0,
                TimingModuleContext::for_module(project_module),
            ),
            attributed_observation("command.build.total", 100.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![
            boundary_record(0, "@html", 1),
            boundary_record(1, "moth_docs", 1),
        ],
        modules: vec![
            module_record(html, 0, "@html", 1, 512),
            module_record(project, 0, "moth_docs", 1, 1024),
        ],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = report
        .slowest_module
        .as_ref()
        .expect("slowest module should exist");

    assert_eq!(slowest_module.identity, "moth_docs");
    assert_eq!(slowest_module.total, Duration::from_millis(9));
    assert_eq!(slowest_module.source_byte_count, 1024);
}

#[test]
fn shuffled_events_do_not_change_boundary_or_slowest_module() {
    let html = TimingBoundaryId::from_index(0);
    let html_module = TimingModuleKey {
        boundary: html,
        module_index: 0,
    };
    let mut entries = [
        (
            "build.boundary.inventory",
            2.0,
            TimingModuleContext::for_boundary(html),
        ),
        (
            "build.boundary.compile",
            3.0,
            TimingModuleContext::for_boundary(html),
        ),
        (
            "frontend.file_prepare",
            1.5,
            TimingModuleContext::for_module(html_module),
        ),
        (
            "frontend.module.semantic_total",
            6.0,
            TimingModuleContext::for_module(html_module),
        ),
        ("command.build.total", 100.0, TimingModuleContext::default()),
    ];

    let ordered = BenchmarkObservationSnapshot {
        timings: entries
            .iter()
            .map(|(name, millis, context)| attributed_observation(name, *millis, *context))
            .collect(),
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "@html", 1)],
        modules: vec![module_record(html, 0, "@html", 1, 512)],
    };
    entries.reverse();
    let shuffled = BenchmarkObservationSnapshot {
        timings: entries
            .iter()
            .map(|(name, millis, context)| attributed_observation(name, *millis, *context))
            .collect(),
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "@html", 1)],
        modules: vec![module_record(html, 0, "@html", 1, 512)],
    };

    let ordered_report = build_timing_summary(&ordered, TimingCommandKind::Build, true);
    let shuffled_report = build_timing_summary(&shuffled, TimingCommandKind::Build, true);

    assert_eq!(
        ordered_report.compilation_boundaries,
        shuffled_report.compilation_boundaries
    );
    assert_eq!(
        ordered_report.slowest_module,
        shuffled_report.slowest_module
    );
}

#[test]
fn slowest_module_uses_preparation_plus_semantic_total() {
    let project = TimingBoundaryId::from_index(0);
    let first = TimingModuleKey {
        boundary: project,
        module_index: 0,
    };
    let second = TimingModuleKey {
        boundary: project,
        module_index: 1,
    };
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.file_prepare",
                3.0,
                TimingModuleContext::for_module(first),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                5.0,
                TimingModuleContext::for_module(first),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                6.0,
                TimingModuleContext::for_module(second),
            ),
            attributed_observation("command.build.total", 100.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 2)],
        modules: vec![
            module_record(project, 0, "moth_docs/site", 2, 2048),
            module_record(project, 1, "moth_docs/api", 1, 1024),
        ],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = report
        .slowest_module
        .as_ref()
        .expect("slowest module should exist");

    assert_eq!(slowest_module.identity, "moth_docs/site");
    assert_eq!(slowest_module.total, Duration::from_millis(8));
    assert_eq!(slowest_module.source_file_count, 2);
}

#[test]
fn slowest_module_identity_uses_logical_path_not_absolute_path() {
    let project = TimingBoundaryId::from_index(0);
    let module = TimingModuleKey {
        boundary: project,
        module_index: 0,
    };
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.module.semantic_total",
                10.0,
                TimingModuleContext::for_module(module),
            ),
            attributed_observation("command.build.total", 100.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 1)],
        modules: vec![module_record(
            project,
            0,
            "moth_docs/docs/progress",
            1,
            44_928,
        )],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = report
        .slowest_module
        .as_ref()
        .expect("slowest module should exist");
    let text = slowest_module_text(slowest_module);

    assert_eq!(slowest_module.identity, "moth_docs/docs/progress");
    assert!(!text.contains("/Users/"));
    assert!(!text.contains("/private/tmp"));
    assert!(!text.contains("moth/"));
}

#[test]
fn only_registered_boundaries_produce_rows() {
    let html = TimingBoundaryId::from_index(0);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "build.boundary.compile",
                5.0,
                TimingModuleContext::for_boundary(html),
            ),
            attributed_observation("command.build.total", 100.0, TimingModuleContext::default()),
        ],
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "@html", 1)],
        modules: Vec::new(),
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    assert_eq!(report.compilation_boundaries.len(), 1);
    assert!(
        report
            .compilation_boundaries
            .iter()
            .all(|boundary| boundary.label != "@web/canvas"),
        "binding-backed packages are never registered as source boundaries"
    );
}

#[test]
fn boundary_section_heading_marks_accumulated_work() {
    assert_eq!(
        boundary_section_title(),
        "Compilation boundaries · accumulated work"
    );
}
