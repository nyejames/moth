//! Structured summary model tests.
//!
//! WHAT: pins the basic-report policy: architecture order, thresholds,
//!      zero suppression, bounded `Other`, hidden unknown metrics and
//!      deterministic construction from shuffled events.
//! WHY:  the summary is pure structured data; these tests run before any
//!       terminal rendering so policy bugs cannot hide behind styling.

use crate::timing::enabled::render::{
    boundary_row_text, boundary_section_title, render_row_text, report_title_text,
    slowest_module_text,
};
use crate::timing::enabled::session::TimingSessionId;
use crate::timing::enabled::summary::{
    TimingBoundarySummary, TimingEmphasis, TimingMeasurementKind, TimingReportItem,
    TimingSlowestModuleSummary, TimingSummaryReport, TimingSummaryRow, TimingSummarySection,
    build_timing_summary,
};
use crate::timing::{
    BenchmarkObservationSnapshot, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord,
    TimingCommandKind, TimingContext, TimingModuleKey, TimingModuleRecord, TimingObservation,
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
                context: None,
            })
            .collect(),
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    }
}

fn boundary_id(index: u32) -> TimingBoundaryId {
    TimingBoundaryId::from_session(TimingSessionId::from_raw(0), index)
}

fn section_items(report: &TimingSummaryReport) -> Vec<&TimingSummarySection> {
    report
        .items
        .iter()
        .filter_map(|item| match item {
            TimingReportItem::Section(section) => Some(section),
            _ => None,
        })
        .collect()
}

fn boundary_items(report: &TimingSummaryReport) -> Option<&[TimingBoundarySummary]> {
    report.items.iter().find_map(|item| match item {
        TimingReportItem::CompilationBoundaries(boundaries) => Some(boundaries.as_slice()),
        _ => None,
    })
}

fn slowest_module_item(report: &TimingSummaryReport) -> Option<&TimingSlowestModuleSummary> {
    report.items.iter().find_map(|item| match item {
        TimingReportItem::SlowestModule(module) => Some(module),
        _ => None,
    })
}

fn build_snapshot() -> BenchmarkObservationSnapshot {
    snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.bootstrap.total", 10.0),
        ("stage0.directory.inventory", 20.0),
        ("stage0.directory.compile", 30.0),
        ("build.frontend.total", 50.0),
        ("build.backend.total", 15.0),
        ("build.output.total", 5.0),
        ("frontend.prepare", 8.0),
        ("frontend.bind_headers", 4.0),
        ("frontend.order_declarations", 2.0),
        ("frontend.ast.total", 50.0),
        ("frontend.hir", 3.0),
        ("frontend.borrow.initial", 1.0),
        ("backend.js.lower_entry", 2.0),
        ("backend.html.render", 1.0),
    ])
}

#[test]
fn report_items_follow_architecture_order() {
    let mut snapshot = build_snapshot();
    snapshot.boundaries.push(boundary_record(0, "@html", 1));
    snapshot
        .modules
        .push(module_record(boundary_id(0), 0, "@html", 1, 512));
    snapshot.timings.push(attributed_observation(
        "frontend.module.semantic_total",
        5.0,
        Some(TimingContext::for_module(TimingModuleKey::new(
            boundary_id(0),
            0,
        ))),
    ));
    snapshot.timings.push(attributed_observation(
        "boundary.compile",
        3.0,
        Some(TimingContext::for_boundary(boundary_id(0))),
    ));

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    let item_kinds: Vec<&str> = report
        .items
        .iter()
        .map(|item| match item {
            TimingReportItem::Section(section) => section.title.as_str(),
            TimingReportItem::CompilationBoundaries(_) => "Compilation boundaries",
            TimingReportItem::SlowestModule(_) => "Slowest module",
        })
        .collect();
    assert_eq!(
        item_kinds,
        vec![
            "Build pipeline",
            "Compilation boundaries",
            "Frontend work · 1 modules · accumulated",
            "Backend",
            "Slowest module",
        ]
    );
}

#[test]
fn headings_keep_total_in_a_separate_field() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    assert_eq!(report.title, "Build timings");
    assert_eq!(report.command_total, Duration::from_millis(100));
    assert_eq!(report_title_text(&report), "Build timings  100.00ms");

    let check_snapshot = snapshot_with(&[
        ("command.check.total", 60.0),
        ("build.bootstrap.total", 5.0),
        ("stage0.directory.inventory", 10.0),
        ("stage0.directory.compile", 20.0),
    ]);
    let check_report = build_timing_summary(&check_snapshot, TimingCommandKind::Check, true);
    assert_eq!(check_report.title, "Check timings");
    assert_eq!(check_report.command_total, Duration::from_millis(60));
    assert_eq!(report_title_text(&check_report), "Check timings  60.00ms");
}

#[test]
fn check_summary_excludes_build_only_backend_and_output_metrics() {
    let snapshot = snapshot_with(&[
        ("command.check.total", 100.0),
        ("build.bootstrap.total", 10.0),
        ("build.frontend.total", 20.0),
        ("build.backend.total", 40.0),
        ("build.output.total", 30.0),
        ("backend.js.lower_entry", 20.0),
    ]);

    let report = build_timing_summary(&snapshot, TimingCommandKind::Check, true);
    let pipeline = section_items(&report)[0];

    assert!(
        pipeline
            .rows
            .iter()
            .all(|row| row.label != "Backend" && row.label != "Output")
    );
    let other = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Other")
        .expect("check pipeline should retain unmeasured work in Other");
    assert_eq!(other.total, Duration::from_millis(70));
    assert!(report.items.iter().all(|item| {
        !matches!(
            item,
            TimingReportItem::Section(section) if section.title == "Backend"
        )
    }));
}

#[test]
fn pipeline_omits_command_total_row() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let pipeline = section_items(&report)[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Command total"),
        "the command total belongs in the heading, not in the pipeline rows"
    );
    assert_eq!(report.command_total, Duration::from_millis(100));
}

#[test]
fn other_is_bounded_and_never_negative() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let pipeline = section_items(&report)[0];
    let other = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Other")
        .expect("Other should be present");

    // 100 - (10 + 50 + 15 + 5) = 20ms.
    assert_eq!(other.total.as_millis(), 20);
}

#[test]
fn other_is_omitted_when_insignificant() {
    let snapshot = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.bootstrap.total", 10.0),
        ("stage0.directory.inventory", 20.0),
        ("stage0.directory.compile", 30.0),
        ("build.frontend.total", 50.0),
        ("build.backend.total", 15.0),
        ("build.output.total", 24.9),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let pipeline = section_items(&report)[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Other"),
        "0.1ms Other must be omitted"
    );
}

#[test]
fn single_file_build_shows_frontend_parent_and_evidence() {
    let snapshot = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.bootstrap.total", 10.0),
        ("build.frontend.total", 40.0),
        ("stage0.single_file.total", 40.0),
        ("build.backend.total", 15.0),
        ("build.output.total", 5.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let pipeline = section_items(&report)[0];

    let frontend = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Frontend")
        .expect("single-file builds should show the frontend parent row");
    assert_eq!(frontend.total, Duration::from_millis(40));
    assert_eq!(frontend.children[0].label, "Single-file frontend");
    assert_eq!(frontend.children[0].total, Duration::from_millis(40));
    assert!(
        pipeline
            .rows
            .iter()
            .all(|row| row.label != "Directory inventory" && row.label != "Directory compile"),
        "directory evidence must not appear in single-file mode"
    );

    // 100 - (10 + 40 + 15 + 5) = 30ms.
    let other = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Other")
        .expect("Other should account the unmeasured single-file work");
    assert_eq!(other.total, Duration::from_millis(30));
}

#[test]
fn single_file_check_shows_frontend_parent_and_evidence() {
    let snapshot = snapshot_with(&[
        ("command.check.total", 60.0),
        ("build.bootstrap.total", 5.0),
        ("build.frontend.total", 30.0),
        ("stage0.single_file.total", 30.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Check, true);
    let pipeline = section_items(&report)[0];

    let frontend = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Frontend")
        .expect("single-file checks should show the frontend parent row");
    assert_eq!(frontend.total, Duration::from_millis(30));
    assert_eq!(frontend.children[0].label, "Single-file frontend");
    assert_eq!(frontend.children[0].total, Duration::from_millis(30));

    // 60 - (5 + 30) = 25ms.
    let other = pipeline
        .rows
        .iter()
        .find(|row| row.label == "Other")
        .expect("Other should account the unmeasured single-file work");
    assert_eq!(other.total, Duration::from_millis(25));
}

#[test]
fn frontend_heading_shows_registered_module_count() {
    let mut snapshot = build_snapshot();
    snapshot
        .modules
        .push(module_record(boundary_id(0), 0, "moth_docs/site", 1, 512));
    snapshot
        .modules
        .push(module_record(boundary_id(0), 1, "moth_docs/api", 1, 1024));

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");

    assert_eq!(frontend.title, "Frontend work · 2 modules · accumulated");
}

#[test]
fn frontend_heading_omits_count_when_no_modules_registered() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");

    assert_eq!(frontend.title, "Frontend work · accumulated");
}

#[test]
fn public_interface_repeated_samples_sum_into_one_row() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "frontend.public_interface.project",
        duration: Duration::from_millis(8),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.public_interface.finalise",
        duration: Duration::from_millis(4),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");
    let public_interface = frontend
        .rows
        .iter()
        .find(|row| row.label == "Public interface")
        .expect("public interface row should exist");

    assert_eq!(public_interface.total, Duration::from_millis(12));
}

#[test]
fn generated_borrow_work_is_classified_once() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "frontend.generated.materialise",
        duration: Duration::from_millis(6),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.generated.borrow_recheck",
        duration: Duration::from_millis(2),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.borrow.converge",
        duration: Duration::from_millis(3),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");

    let generated = frontend
        .rows
        .iter()
        .find(|row| row.label == "Generated functions")
        .expect("generated functions row should exist");
    assert_eq!(
        generated.total,
        Duration::from_millis(8),
        "generated materialisation plus sidecar borrow rechecks belong to one row"
    );

    let borrow = frontend
        .rows
        .iter()
        .find(|row| row.label == "Borrow validation")
        .expect("borrow validation row should exist");
    assert_eq!(
        borrow.total,
        Duration::from_millis(4),
        "borrow validation sums direct borrow-check calls only"
    );
}

#[test]
fn ast_children_show_tir_and_constant_finalization_labels() {
    let mut snapshot = build_snapshot();
    let module = TimingModuleKey::new(boundary_id(0), 0);
    snapshot.timings.push(TimingObservation {
        name: "frontend.ast.environment",
        duration: Duration::from_millis(30),
        context: Some(TimingContext::for_module(module)),
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.ast.emit",
        duration: Duration::from_millis(40),
        context: Some(TimingContext::for_module(module)),
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.ast.finalise",
        duration: Duration::from_millis(20),
        context: Some(TimingContext::for_module(module)),
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");
    let ast = frontend
        .rows
        .iter()
        .find(|row| row.label == "Semantic frontend / AST")
        .expect("AST row should exist");

    let child_labels = ast
        .children
        .iter()
        .map(|child| child.label.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        child_labels,
        vec![
            "Environment, types and constants",
            "Bodies and TIR construction",
            "Template and constant finalisation",
        ]
    );
    assert_eq!(ast.children[1].total, Duration::from_millis(40));
    assert_eq!(ast.children[2].total, Duration::from_millis(20));
}

#[test]
fn ast_children_hidden_below_significance_threshold() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "frontend.ast.finalise",
        duration: Duration::from_secs_f64(0.0005),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");
    let ast = frontend
        .rows
        .iter()
        .find(|row| row.label == "Semantic frontend / AST")
        .expect("AST row should exist");

    assert!(
        ast.children
            .iter()
            .all(|child| child.label != "Template and constant finalisation"),
        "sub-threshold AST children must stay hidden"
    );
}

#[test]
fn js_lowering_aggregates_entry_and_linked_observations() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "backend.js.lower_linked",
        duration: Duration::from_millis(3),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let backend = sections
        .iter()
        .find(|section| section.title == "Backend")
        .expect("backend section should exist");
    let js_lowering = backend
        .rows
        .iter()
        .find(|row| row.label == "JS lowering")
        .expect("JS lowering row should exist");

    assert_eq!(
        js_lowering.total,
        Duration::from_millis(5),
        "entry-module and linked-module lowering must share one human row"
    );
}

#[test]
fn wasm_and_tracked_asset_children_appear_when_significant() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "backend.wasm.total",
        duration: Duration::from_millis(2),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "backend.assets.emit",
        duration: Duration::from_millis(1),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let backend = sections
        .iter()
        .find(|section| section.title == "Backend")
        .expect("backend section should exist");
    let labels = backend
        .rows
        .iter()
        .map(|row| row.label.as_ref())
        .collect::<Vec<_>>();

    assert!(labels.contains(&"Wasm build"));
    assert!(labels.contains(&"Tracked assets"));
    assert!(
        !labels.contains(&"Site config") && !labels.contains(&"Document config"),
        "config microsteps must stay hidden from basic output"
    );
}

#[test]
fn dev_report_uses_build_and_write_total_and_build_pipeline_rows() {
    let snapshot = snapshot_with(&[
        ("command.dev.build_write", 100.0),
        ("build.bootstrap.total", 10.0),
        ("stage0.directory.inventory", 20.0),
        ("stage0.directory.compile", 30.0),
        ("build.frontend.total", 50.0),
        ("build.backend.total", 15.0),
        ("build.output.total", 5.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Dev, true);

    assert_eq!(report.title, "Dev timings");
    assert_eq!(report.command_total, Duration::from_millis(100));

    let sections = section_items(&report);
    let pipeline = sections
        .iter()
        .find(|section| section.title == "Build pipeline")
        .expect("dev report should show the build pipeline");
    let labels = pipeline
        .rows
        .iter()
        .map(|row| row.label.as_ref())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"Bootstrap"));
    assert!(labels.contains(&"Frontend"));
    assert!(labels.contains(&"Backend"));
    assert!(labels.contains(&"Output"));
}

#[test]
fn zero_rows_are_suppressed_before_rounding() {
    let snapshot = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.bootstrap.total", 0.004),
        ("stage0.directory.inventory", 20.0),
        ("stage0.directory.compile", 30.0),
        ("build.frontend.total", 50.0),
        ("build.backend.total", 15.0),
        ("build.output.total", 5.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let pipeline = section_items(&report)[0];

    assert!(
        pipeline.rows.iter().all(|row| row.label != "Bootstrap"),
        "a 0.004ms row must be suppressed before rounding"
    );
}

#[test]
fn unknown_metrics_stay_hidden_from_basic_output() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "backend.wasm.lower",
        duration: Duration::from_millis(50),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let all_labels: Vec<&str> = report
        .items
        .iter()
        .filter_map(|item| match item {
            TimingReportItem::Section(section) => Some(section.rows.iter()),
            _ => None,
        })
        .flatten()
        .map(|row| row.label.as_ref())
        .collect();

    assert!(!all_labels.contains(&"Wasm build"));
    assert!(
        snapshot
            .timings
            .iter()
            .any(|observation| observation.name == "backend.wasm.lower"),
        "raw snapshot must retain detailed metrics hidden from basic output"
    );
}

#[test]
fn child_thresholds_apply_at_exact_boundaries() {
    // Child at exactly 1ms and 5% of a 20ms parent: shown.
    let shown = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.backend.total", 20.0),
        ("backend.js.lower_entry", 1.0),
    ]);
    let report = build_timing_summary(&shown, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let backend = sections
        .iter()
        .find(|section| section.title == "Backend")
        .expect("backend section should exist");
    assert_eq!(backend.rows.len(), 1);

    // Child at 0.99ms: hidden.
    let hidden = snapshot_with(&[
        ("command.build.total", 100.0),
        ("build.backend.total", 20.0),
        ("backend.js.lower_entry", 0.99),
    ]);
    let report = build_timing_summary(&hidden, TimingCommandKind::Build, true);
    assert!(
        section_items(&report)
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
            .items
            .iter()
            .filter_map(|item| match item {
                TimingReportItem::Section(section) => Some(
                    section
                        .rows
                        .iter()
                        .map(|row| (row.label.clone(), row.total.as_millis()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
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

    assert_eq!(failed.title, "Build timings · failed");
    assert_eq!(failed.command_total, succeeded.command_total);
}

#[test]
fn repeated_rows_show_aggregate_duration_without_sample_noise() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "frontend.prepare",
        duration: Duration::from_millis(1),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let prepare_row = report
        .items
        .iter()
        .filter_map(|item| match item {
            TimingReportItem::Section(section) => Some(section.rows.iter()),
            _ => None,
        })
        .flatten()
        .find(|row| row.label == "Prepare source files")
        .expect("prepare source files row should exist");

    assert_eq!(prepare_row.total, Duration::from_millis(9));
    assert_eq!(prepare_row.suffix, None);

    let text = render_row_text(prepare_row, prepare_row.label.len(), 0);
    assert!(!text.contains("across"));
    assert!(!text.contains("samples"));
    assert!(!text.contains("["));
}

#[test]
fn model_supports_dynamic_boundary_and_slowest_module_labels() {
    let mut report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    report
        .items
        .push(TimingReportItem::CompilationBoundaries(vec![
            TimingBoundarySummary {
                label: Cow::Owned("@html".to_owned()),
                module_count: 1,
                total: Duration::from_millis(5),
            },
        ]));
    report.items.push(TimingReportItem::SlowestModule(
        TimingSlowestModuleSummary {
            identity: Cow::Owned("@docs/progress".to_owned()),
            source_file_count: 1,
            source_byte_count: 45_000,
            total: Duration::from_millis(16),
        },
    ));

    let boundary = &boundary_items(&report).expect("boundaries should exist")[0];
    assert_eq!(
        boundary_row_text(boundary, boundary.label.len()),
        "@html  1 module  5.00ms"
    );

    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");
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
        id: TimingBoundaryId::from_session(TimingSessionId::from_raw(0), id),
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
        key: TimingModuleKey::new(boundary, module_index),
        logical_identity: logical_identity.to_owned(),
        source_file_count,
        source_byte_count,
    }
}

fn attributed_observation(
    name: &'static str,
    millis: f64,
    context: Option<TimingContext>,
) -> TimingObservation {
    TimingObservation {
        name,
        duration: Duration::from_secs_f64(millis / 1000.0),
        context,
    }
}

#[test]
fn boundary_rows_separate_packages_and_project_totals() {
    let html = boundary_id(0);
    let project = boundary_id(1);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "boundary.inventory",
                2.0,
                Some(TimingContext::for_boundary(html)),
            ),
            attributed_observation(
                "boundary.compile",
                3.0,
                Some(TimingContext::for_boundary(html)),
            ),
            attributed_observation(
                "boundary.inventory",
                40.0,
                Some(TimingContext::for_boundary(project)),
            ),
            attributed_observation(
                "boundary.compile",
                300.0,
                Some(TimingContext::for_boundary(project)),
            ),
            attributed_observation("command.build.total", 500.0, None),
        ],
        counters: Vec::new(),
        boundaries: vec![
            boundary_record(0, "@html", 1),
            boundary_record(1, "moth_docs", 69),
        ],
        modules: Vec::new(),
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let boundaries = boundary_items(&report).expect("boundaries should exist");

    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0].label, "@html");
    assert_eq!(boundaries[0].module_count, 1);
    assert_eq!(boundaries[0].total, Duration::from_millis(5));
    assert_eq!(boundaries[1].label, "moth_docs");
    assert_eq!(boundaries[1].module_count, 69);
    assert_eq!(boundaries[1].total, Duration::from_millis(340));
}

#[test]
fn boundary_rows_follow_registration_order() {
    let first = boundary_id(0);
    let second = boundary_id(1);
    let project = boundary_id(2);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "boundary.compile",
                1.0,
                Some(TimingContext::for_boundary(project)),
            ),
            attributed_observation(
                "boundary.compile",
                1.0,
                Some(TimingContext::for_boundary(first)),
            ),
            attributed_observation(
                "boundary.compile",
                1.0,
                Some(TimingContext::for_boundary(second)),
            ),
            attributed_observation("command.build.total", 100.0, None),
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
        .items
        .iter()
        .filter_map(|item| match item {
            TimingReportItem::CompilationBoundaries(boundaries) => Some(boundaries.iter()),
            _ => None,
        })
        .flatten()
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
    let html = boundary_id(0);
    let project = boundary_id(1);
    let html_module = TimingModuleKey::new(html, 0);
    let project_module = TimingModuleKey::new(project, 0);

    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.module.semantic_total",
                4.0,
                Some(TimingContext::for_module(html_module)),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                9.0,
                Some(TimingContext::for_module(project_module)),
            ),
            attributed_observation("command.build.total", 100.0, None),
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
    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");

    assert_eq!(slowest_module.identity, "moth_docs");
    assert_eq!(slowest_module.total, Duration::from_millis(9));
    assert_eq!(slowest_module.source_byte_count, 1024);
}

#[test]
fn shuffled_events_do_not_change_boundary_or_slowest_module() {
    let html = boundary_id(0);
    let html_module = TimingModuleKey::new(html, 0);
    let mut entries = [
        (
            "boundary.inventory",
            2.0,
            Some(TimingContext::for_boundary(html)),
        ),
        (
            "boundary.compile",
            3.0,
            Some(TimingContext::for_boundary(html)),
        ),
        (
            "frontend.prepare",
            1.5,
            Some(TimingContext::for_module(html_module)),
        ),
        (
            "frontend.module.semantic_total",
            6.0,
            Some(TimingContext::for_module(html_module)),
        ),
        ("command.build.total", 100.0, None),
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

    assert_eq!(ordered_report.items, shuffled_report.items);
}

#[test]
fn slowest_module_uses_preparation_plus_semantic_total() {
    let project = boundary_id(0);
    let first = TimingModuleKey::new(project, 0);
    let second = TimingModuleKey::new(project, 1);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.prepare",
                3.0,
                Some(TimingContext::for_module(first)),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                5.0,
                Some(TimingContext::for_module(first)),
            ),
            attributed_observation(
                "frontend.module.semantic_total",
                6.0,
                Some(TimingContext::for_module(second)),
            ),
            attributed_observation("command.build.total", 100.0, None),
        ],
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 2)],
        modules: vec![
            module_record(project, 0, "moth_docs/site", 2, 2048),
            module_record(project, 1, "moth_docs/api", 1, 1024),
        ],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");

    assert_eq!(slowest_module.identity, "moth_docs/site");
    assert_eq!(slowest_module.total, Duration::from_millis(8));
    assert_eq!(slowest_module.source_file_count, 2);
}

#[test]
fn slowest_module_identity_uses_logical_path_not_absolute_path() {
    let project = boundary_id(0);
    let module = TimingModuleKey::new(project, 0);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "frontend.module.semantic_total",
                10.0,
                Some(TimingContext::for_module(module)),
            ),
            attributed_observation("command.build.total", 100.0, None),
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
    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");
    let text = slowest_module_text(slowest_module);

    assert_eq!(slowest_module.identity, "moth_docs/docs/progress");
    assert!(!text.contains("/Users/"));
    assert!(!text.contains("/private/tmp"));
    assert!(!text.contains("moth/"));
}

#[test]
fn only_registered_boundaries_produce_rows() {
    let html = boundary_id(0);
    let snapshot = BenchmarkObservationSnapshot {
        timings: vec![
            attributed_observation(
                "boundary.compile",
                5.0,
                Some(TimingContext::for_boundary(html)),
            ),
            attributed_observation("command.build.total", 100.0, None),
        ],
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "@html", 1)],
        modules: Vec::new(),
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let boundaries = boundary_items(&report).expect("boundaries should exist");

    assert_eq!(boundaries.len(), 1);
    assert!(
        boundaries
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

#[test]
fn config_and_generated_ast_observations_never_enter_module_ast_children() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(TimingObservation {
        name: "config.ast.environment",
        duration: Duration::from_millis(90),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "config.ast.emit",
        duration: Duration::from_millis(90),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "config.ast.finalise",
        duration: Duration::from_millis(90),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.generated.ast.environment",
        duration: Duration::from_millis(90),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.generated.ast.emit",
        duration: Duration::from_millis(90),
        context: None,
    });
    snapshot.timings.push(TimingObservation {
        name: "frontend.generated.ast.finalise",
        duration: Duration::from_millis(90),
        context: None,
    });

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let sections = section_items(&report);
    let frontend = sections
        .iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");
    let ast = frontend
        .rows
        .iter()
        .find(|row| row.label == "Semantic frontend / AST")
        .expect("AST row should exist");

    assert!(
        ast.children.is_empty(),
        "config and generated AST work must not appear as module AST children: {:?}",
        ast.children
    );
}
