//! Structured summary model tests.
//!
//! WHAT: pins the basic-report policy: architecture order, thresholds,
//!      zero suppression, bounded `Other`, hidden unknown metrics and
//!      deterministic construction from shuffled events.
//! WHY:  the summary is pure structured data; these tests run before any
//!       terminal rendering so policy bugs cannot hide behind styling.

use crate::timing::enabled::render::{
    boundary_row_text, boundary_row_text_with_width, boundary_section_title, render_row_text,
    report_title_text, section_label_width, slowest_module_text,
};
use crate::timing::enabled::session::TimingSessionId;
use crate::timing::enabled::summary::{
    TimingAccountingIssue, TimingBoundarySummary, TimingReportItem, TimingSlowestModuleSummary,
    TimingSummaryReport, TimingSummaryRow, TimingSummarySection, build_timing_summary,
};
use crate::timing::{
    BenchmarkObservationSnapshot, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord,
    TimingCommandKind, TimingMetric, TimingMetricAggregate, TimingModuleKey, TimingModuleRecord,
};
use std::borrow::Cow;
use std::time::Duration;

fn snapshot_with(entries: &[(TimingMetric, f64)]) -> BenchmarkObservationSnapshot {
    BenchmarkObservationSnapshot {
        timings: entries
            .iter()
            .map(|(metric, millis)| TimingMetricAggregate {
                metric: *metric,
                total: Duration::from_secs_f64(millis / 1000.0),
                samples: 1,
            })
            .collect(),
        schema_version: 2,
        command: None,
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: Vec::new(),
        modules: Vec::new(),
    }
}

fn aggregate(metric: TimingMetric, millis: f64) -> TimingMetricAggregate {
    TimingMetricAggregate {
        metric,
        total: Duration::from_secs_f64(millis / 1000.0),
        samples: 1,
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
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::Stage0DirectoryInventory, 20.0),
        (TimingMetric::Stage0DirectoryCompile, 30.0),
        (TimingMetric::BuildFrontendTotal, 50.0),
        (TimingMetric::BuildBackendTotal, 15.0),
        (TimingMetric::BuildOutputTotal, 5.0),
        (TimingMetric::FrontendPrepare, 8.0),
        (TimingMetric::FrontendBindHeaders, 4.0),
        (TimingMetric::FrontendOrderDeclarations, 2.0),
        (TimingMetric::FrontendAstTotal, 50.0),
        (TimingMetric::FrontendHir, 3.0),
        (TimingMetric::FrontendBorrowInitial, 1.0),
        (TimingMetric::BackendJsLowerEntry, 2.0),
        (TimingMetric::BackendHtmlRender, 1.0),
    ])
}

#[test]
fn report_items_follow_architecture_order() {
    let mut snapshot = build_snapshot();
    let mut boundary = boundary_record(0, "@html", 1);
    boundary
        .timings
        .push(aggregate(TimingMetric::BoundaryCompile, 3.0));
    let mut module = module_record(boundary_id(0), 0, "@html", 1, 512);
    module
        .timings
        .push(aggregate(TimingMetric::FrontendModuleSemanticTotal, 5.0));
    snapshot.boundaries.push(boundary);
    snapshot.modules.push(module);

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    let item_kinds: Vec<&str> = report
        .items
        .iter()
        .map(|item| match item {
            TimingReportItem::Section(section) => section.title.as_str(),
            TimingReportItem::AccountingNote => "Accounting note",
            TimingReportItem::CompilationBoundaries(_) => "Compilation boundaries",
            TimingReportItem::SlowestModule(_) => "Slowest module",
        })
        .collect();
    assert_eq!(
        item_kinds,
        vec![
            "Build pipeline",
            "Accounting note",
            "Compilation boundaries",
            "Frontend work · 1 module · accumulated",
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
        (TimingMetric::CommandCheckTotal, 60.0),
        (TimingMetric::BuildBootstrapTotal, 5.0),
        (TimingMetric::Stage0DirectoryInventory, 10.0),
        (TimingMetric::Stage0DirectoryCompile, 20.0),
    ]);
    let check_report = build_timing_summary(&check_snapshot, TimingCommandKind::Check, true);
    assert_eq!(check_report.title, "Check timings");
    assert_eq!(check_report.command_total, Duration::from_millis(60));
    assert_eq!(report_title_text(&check_report), "Check timings  60.00ms");
}

#[test]
fn check_summary_excludes_build_only_backend_and_output_metrics() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandCheckTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::BuildFrontendTotal, 20.0),
        (TimingMetric::BuildBackendTotal, 40.0),
        (TimingMetric::BuildOutputTotal, 30.0),
        (TimingMetric::BackendJsLowerEntry, 20.0),
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
fn duplicate_pipeline_span_is_reported_as_a_policy_issue() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::BuildFrontendTotal, 50.0),
    ]);

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    assert_eq!(
        report.accounting_issue,
        Some(TimingAccountingIssue::DuplicateSpan {
            metric: TimingMetric::BuildBootstrapTotal,
            samples: 2,
        })
    );
    assert!(
        section_items(&report)[0]
            .rows
            .iter()
            .all(|row| row.label != "Other"),
        "duplicate accounting spans must not produce a fabricated Other row"
    );
}

#[test]
fn over_accounted_pipeline_is_reported_without_saturating_other() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 60.0),
        (TimingMetric::BuildFrontendTotal, 60.0),
    ]);

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);

    assert_eq!(
        report.accounting_issue,
        Some(TimingAccountingIssue::OverAccounted {
            accounted: Duration::from_millis(120),
            command_total: Duration::from_millis(100),
        })
    );
    assert!(
        section_items(&report)[0]
            .rows
            .iter()
            .all(|row| row.label != "Other"),
        "over-accounting must not be hidden by saturating subtraction"
    );
}

#[test]
fn other_is_omitted_when_insignificant() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::Stage0DirectoryInventory, 20.0),
        (TimingMetric::Stage0DirectoryCompile, 30.0),
        (TimingMetric::BuildFrontendTotal, 50.0),
        (TimingMetric::BuildBackendTotal, 15.0),
        (TimingMetric::BuildOutputTotal, 24.9),
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
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::BuildFrontendTotal, 40.0),
        (TimingMetric::Stage0SingleFileTotal, 40.0),
        (TimingMetric::BuildBackendTotal, 15.0),
        (TimingMetric::BuildOutputTotal, 5.0),
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
        (TimingMetric::CommandCheckTotal, 60.0),
        (TimingMetric::BuildBootstrapTotal, 5.0),
        (TimingMetric::BuildFrontendTotal, 30.0),
        (TimingMetric::Stage0SingleFileTotal, 30.0),
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
    snapshot
        .timings
        .push(aggregate(TimingMetric::FrontendPublicInterfaceProject, 8.0));
    snapshot.timings.push(aggregate(
        TimingMetric::FrontendPublicInterfaceFinalise,
        4.0,
    ));

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
    assert_eq!(
        public_interface
            .children
            .iter()
            .map(|row| row.label.as_ref())
            .collect::<Vec<_>>(),
        ["Projection", "Finalisation"]
    );
    assert_eq!(public_interface.children[0].total, Duration::from_millis(8));
    assert_eq!(public_interface.children[1].total, Duration::from_millis(4));

    let public_index = frontend
        .rows
        .iter()
        .position(|row| row.label == "Public interface")
        .expect("public interface row should have an index");
    let hir_index = frontend
        .rows
        .iter()
        .position(|row| row.label == "HIR")
        .expect("HIR row should have an index");
    assert!(public_index < hir_index);
}

#[test]
fn public_interface_children_obey_significance_threshold() {
    let mut snapshot = build_snapshot();
    snapshot.timings.push(aggregate(
        TimingMetric::FrontendPublicInterfaceProject,
        100.0,
    ));
    snapshot.timings.push(aggregate(
        TimingMetric::FrontendPublicInterfaceFinalise,
        4.0,
    ));

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let frontend = section_items(&report)
        .into_iter()
        .find(|section| section.title.starts_with("Frontend work"))
        .expect("frontend section should exist");
    let public_interface = frontend
        .rows
        .iter()
        .find(|row| row.label == "Public interface")
        .expect("public interface row should exist");

    assert_eq!(public_interface.total, Duration::from_millis(104));
    assert_eq!(
        public_interface
            .children
            .iter()
            .map(|row| row.label.as_ref())
            .collect::<Vec<_>>(),
        ["Projection"]
    );
    assert_eq!(
        public_interface.children[0].total,
        Duration::from_millis(100)
    );
}

#[test]
fn generated_borrow_work_is_classified_once() {
    let mut snapshot = build_snapshot();
    snapshot
        .timings
        .push(aggregate(TimingMetric::FrontendGeneratedMaterialise, 6.0));
    snapshot
        .timings
        .push(aggregate(TimingMetric::FrontendGeneratedBorrowRecheck, 2.0));
    snapshot
        .timings
        .push(aggregate(TimingMetric::FrontendBorrowConverge, 3.0));

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
    let mut module = module_record(boundary_id(0), 0, "@html", 1, 512);
    module
        .timings
        .push(aggregate(TimingMetric::FrontendAstEnvironment, 30.0));
    module
        .timings
        .push(aggregate(TimingMetric::FrontendAstEmit, 40.0));
    module
        .timings
        .push(aggregate(TimingMetric::FrontendAstFinalise, 20.0));
    snapshot.modules.push(module);

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
    let mut module = module_record(boundary_id(0), 0, "@html", 1, 512);
    module
        .timings
        .push(aggregate(TimingMetric::FrontendAstFinalise, 0.0005));
    snapshot.modules.push(module);

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
    snapshot
        .timings
        .push(aggregate(TimingMetric::BackendJsLowerLinked, 3.0));

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
    snapshot
        .timings
        .push(aggregate(TimingMetric::BackendWasmTotal, 2.0));
    snapshot
        .timings
        .push(aggregate(TimingMetric::BackendAssetsEmit, 1.0));

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
        (TimingMetric::CommandDevBuildWrite, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::Stage0DirectoryInventory, 20.0),
        (TimingMetric::Stage0DirectoryCompile, 30.0),
        (TimingMetric::BuildFrontendTotal, 50.0),
        (TimingMetric::BuildBackendTotal, 15.0),
        (TimingMetric::BuildOutputTotal, 5.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Dev, true);

    assert_eq!(report.title, "Dev timings");
    assert_eq!(report.command_total, Duration::from_millis(100));

    let sections = section_items(&report);
    let pipeline = sections
        .iter()
        .find(|section| section.title == "Dev pipeline")
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
fn check_report_uses_a_check_pipeline_title() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandCheckTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 10.0),
        (TimingMetric::BuildFrontendTotal, 20.0),
    ]);
    let report = build_timing_summary(&snapshot, TimingCommandKind::Check, true);
    assert!(
        section_items(&report)
            .iter()
            .any(|section| section.title == "Check pipeline")
    );
}

#[test]
fn zero_rows_are_suppressed_before_rounding() {
    let snapshot = snapshot_with(&[
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBootstrapTotal, 0.004),
        (TimingMetric::Stage0DirectoryInventory, 20.0),
        (TimingMetric::Stage0DirectoryCompile, 30.0),
        (TimingMetric::BuildFrontendTotal, 50.0),
        (TimingMetric::BuildBackendTotal, 15.0),
        (TimingMetric::BuildOutputTotal, 5.0),
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
    snapshot
        .timings
        .push(aggregate(TimingMetric::BackendWasmLower, 50.0));

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
        snapshot.timings.iter().any(|aggregate| {
            aggregate.metric == TimingMetric::BackendWasmLower && aggregate.samples > 0
        }),
        "raw snapshot must retain detailed metrics hidden from basic output"
    );
}

#[test]
fn child_thresholds_apply_at_exact_boundaries() {
    // Child at exactly 1ms and 5% of a 20ms parent: shown.
    let shown = snapshot_with(&[
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBackendTotal, 20.0),
        (TimingMetric::BackendJsLowerEntry, 1.0),
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
        (TimingMetric::CommandBuildTotal, 100.0),
        (TimingMetric::BuildBackendTotal, 20.0),
        (TimingMetric::BackendJsLowerEntry, 0.99),
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
    let mut entries: Vec<(TimingMetric, f64)> = build_snapshot()
        .timings
        .iter()
        .filter(|aggregate| aggregate.samples > 0)
        .map(|aggregate| (aggregate.metric, aggregate.total.as_secs_f64() * 1000.0))
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
    snapshot
        .timings
        .push(aggregate(TimingMetric::FrontendPrepare, 1.0));

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

    let text = render_row_text(prepare_row, prepare_row.label.len(), 0);
    assert!(!text.contains("across"));
    assert!(!text.contains("samples"));
    assert!(!text.contains("["));
}

#[test]
fn nested_summary_labels_share_a_recursive_value_column() {
    let rows = vec![TimingSummaryRow {
        label: Cow::Borrowed("AST"),
        total: Duration::from_millis(10),
        children: vec![TimingSummaryRow {
            label: Cow::Borrowed("Long nested AST child"),
            total: Duration::from_millis(2),
            children: Vec::new(),
        }],
    }];
    let label_width = section_label_width(&rows);

    let parent = render_row_text(&rows[0], label_width, 0);
    let child = render_row_text(&rows[0].children[0], label_width, 1);
    let parent_value_start = parent
        .find("  10.00ms")
        .expect("parent value should be present");
    let child_value_start = child
        .find("  2.00ms")
        .expect("child value should be present");

    assert_eq!(label_width, "  Long nested AST child".len());
    assert_eq!(parent_value_start, child_value_start);
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
        "@docs/progress  16.00ms · 1 file · 43.9KiB"
    );
}

#[test]
fn boundary_module_count_columns_align_across_rows() {
    let boundaries = [
        TimingBoundarySummary {
            label: Cow::Borrowed("@html"),
            module_count: 1,
            total: Duration::from_millis(5),
        },
        TimingBoundarySummary {
            label: Cow::Borrowed("main_project"),
            module_count: 12,
            total: Duration::from_millis(15),
        },
    ];
    let label_width = boundaries
        .iter()
        .map(|boundary| boundary.label.len())
        .max()
        .unwrap();
    let module_width = boundaries
        .iter()
        .map(|boundary| {
            if boundary.module_count == 1 {
                "1 module".len()
            } else {
                format!("{} modules", boundary.module_count).len()
            }
        })
        .max()
        .unwrap();

    let first = boundary_row_text_with_width(&boundaries[0], label_width, module_width);
    let second = boundary_row_text_with_width(&boundaries[1], label_width, module_width);
    assert_eq!(
        first.find("  1 module").unwrap(),
        second.find("  12 modules").unwrap()
    );
}

fn boundary_record(id: u32, display_name: &str, module_count: u64) -> TimingBoundaryRecord {
    TimingBoundaryRecord {
        id: TimingBoundaryId::from_session(TimingSessionId::from_raw(0), id),
        kind: TimingBoundaryKind::SourcePackage,
        display_name: display_name.to_owned(),
        module_count,
        timings: Vec::new(),
    }
}

fn boundary_record_with_timings(
    id: u32,
    display_name: &str,
    module_count: u64,
    entries: &[(TimingMetric, f64)],
) -> TimingBoundaryRecord {
    let mut record = boundary_record(id, display_name, module_count);
    record.timings = entries
        .iter()
        .map(|(metric, millis)| aggregate(*metric, *millis))
        .collect();
    record
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
        source_facts_finalized: true,
        timings: Vec::new(),
    }
}

fn module_record_with_timings(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_identity: &str,
    source_file_count: u64,
    source_byte_count: u64,
    entries: &[(TimingMetric, f64)],
) -> TimingModuleRecord {
    let mut record = module_record(
        boundary,
        module_index,
        logical_identity,
        source_file_count,
        source_byte_count,
    );
    record.timings = entries
        .iter()
        .map(|(metric, millis)| aggregate(*metric, *millis))
        .collect();
    record
}

#[test]
fn boundary_rows_separate_packages_and_project_totals() {
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 500.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![
            boundary_record_with_timings(
                0,
                "@html",
                1,
                &[
                    (TimingMetric::BoundaryInventory, 2.0),
                    (TimingMetric::BoundaryCompile, 3.0),
                ],
            ),
            boundary_record_with_timings(
                1,
                "moth_docs",
                69,
                &[
                    (TimingMetric::BoundaryInventory, 40.0),
                    (TimingMetric::BoundaryCompile, 300.0),
                ],
            ),
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
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![
            boundary_record_with_timings(0, "@zeta", 1, &[(TimingMetric::BoundaryCompile, 1.0)]),
            boundary_record_with_timings(1, "@alpha", 1, &[(TimingMetric::BoundaryCompile, 1.0)]),
            boundary_record_with_timings(
                2,
                "main_project",
                1,
                &[(TimingMetric::BoundaryCompile, 1.0)],
            ),
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

    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![
            boundary_record(0, "@html", 1),
            boundary_record(1, "moth_docs", 1),
        ],
        modules: vec![
            module_record_with_timings(
                html,
                0,
                "@html",
                1,
                512,
                &[(TimingMetric::FrontendModuleSemanticTotal, 4.0)],
            ),
            module_record_with_timings(
                project,
                0,
                "moth_docs",
                1,
                1024,
                &[(TimingMetric::FrontendModuleSemanticTotal, 9.0)],
            ),
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
    let ordered = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record_with_timings(
            0,
            "@html",
            1,
            &[
                (TimingMetric::BoundaryInventory, 2.0),
                (TimingMetric::BoundaryCompile, 3.0),
            ],
        )],
        modules: vec![module_record_with_timings(
            html,
            0,
            "@html",
            1,
            512,
            &[
                (TimingMetric::FrontendPrepare, 1.5),
                (TimingMetric::FrontendModuleSemanticTotal, 6.0),
            ],
        )],
    };
    let shuffled = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record_with_timings(
            0,
            "@html",
            1,
            &[
                (TimingMetric::BoundaryCompile, 3.0),
                (TimingMetric::BoundaryInventory, 2.0),
            ],
        )],
        modules: vec![module_record_with_timings(
            html,
            0,
            "@html",
            1,
            512,
            &[
                (TimingMetric::FrontendModuleSemanticTotal, 6.0),
                (TimingMetric::FrontendPrepare, 1.5),
            ],
        )],
    };

    let ordered_report = build_timing_summary(&ordered, TimingCommandKind::Build, true);
    let shuffled_report = build_timing_summary(&shuffled, TimingCommandKind::Build, true);

    assert_eq!(ordered_report.items, shuffled_report.items);
}

#[test]
fn slowest_module_uses_preparation_plus_semantic_total() {
    let project = boundary_id(0);
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 2)],
        modules: vec![
            module_record_with_timings(
                project,
                0,
                "moth_docs/site",
                2,
                2048,
                &[
                    (TimingMetric::FrontendPrepare, 3.0),
                    (TimingMetric::FrontendModuleSemanticTotal, 5.0),
                ],
            ),
            module_record_with_timings(
                project,
                1,
                "moth_docs/api",
                1,
                1024,
                &[(TimingMetric::FrontendModuleSemanticTotal, 6.0)],
            ),
        ],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");

    assert_eq!(slowest_module.identity, "moth_docs/site");
    assert_eq!(slowest_module.total, Duration::from_millis(8));
    assert_eq!(slowest_module.source_file_count, 2);
}

#[test]
fn slowest_module_ignores_unfinished_source_facts() {
    let project = boundary_id(0);
    let mut interrupted = module_record_with_timings(
        project,
        0,
        "moth_docs/interrupted",
        0,
        0,
        &[
            (TimingMetric::FrontendPrepare, 100.0),
            (TimingMetric::FrontendModuleSemanticTotal, 100.0),
        ],
    );
    interrupted.source_facts_finalized = false;

    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 2)],
        modules: vec![
            interrupted,
            module_record_with_timings(
                project,
                1,
                "moth_docs/completed",
                1,
                1024,
                &[(TimingMetric::FrontendModuleSemanticTotal, 2.0)],
            ),
        ],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = slowest_module_item(&report).expect("completed module should exist");

    assert_eq!(slowest_module.identity, "moth_docs/completed");
    assert_eq!(slowest_module.source_file_count, 1);
    assert_eq!(slowest_module.source_byte_count, 1024);
    assert_eq!(slowest_module.total, Duration::from_millis(2));
}

#[test]
fn slowest_module_identity_uses_logical_path_not_absolute_path() {
    let project = boundary_id(0);
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 1)],
        modules: vec![module_record_with_timings(
            project,
            0,
            "moth_docs/docs/progress",
            1,
            44_928,
            &[(TimingMetric::FrontendModuleSemanticTotal, 10.0)],
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
fn slowest_module_identity_is_bounded_to_its_unique_tail() {
    let project = boundary_id(0);
    let long_identity = "moth_docs/a/very/deeply/nested/module/path/with/a/unique_tail";
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record(0, "moth_docs", 1)],
        modules: vec![module_record_with_timings(
            project,
            0,
            long_identity,
            1,
            1024,
            &[(TimingMetric::FrontendModuleSemanticTotal, 10.0)],
        )],
    };

    let report = build_timing_summary(&snapshot, TimingCommandKind::Build, true);
    let slowest_module = slowest_module_item(&report).expect("slowest module should exist");
    let text = slowest_module_text(slowest_module);

    assert!(text.starts_with("…"));
    assert!(text.contains("unique_tail"));
    assert!(!text.contains(long_identity));
}

#[test]
fn only_registered_boundaries_produce_rows() {
    let snapshot = BenchmarkObservationSnapshot {
        schema_version: 2,
        command: Some(TimingCommandKind::Build),
        timings: vec![aggregate(TimingMetric::CommandBuildTotal, 100.0)],
        #[cfg(feature = "benchmark_counters")]
        counters: Vec::new(),
        boundaries: vec![boundary_record_with_timings(
            0,
            "@html",
            1,
            &[(TimingMetric::BoundaryCompile, 5.0)],
        )],
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
    for metric in [
        TimingMetric::ConfigAstEnvironment,
        TimingMetric::ConfigAstEmit,
        TimingMetric::ConfigAstFinalise,
        TimingMetric::FrontendGeneratedAstEnvironment,
        TimingMetric::FrontendGeneratedAstEmit,
        TimingMetric::FrontendGeneratedAstFinalise,
    ] {
        snapshot.timings.push(aggregate(metric, 90.0));
    }

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

/// The accounting note appears exactly once after the pipeline section.
#[test]
fn accounting_note_appears_after_pipeline_for_build() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Build, true);
    let note_count = report
        .items
        .iter()
        .filter(|item| matches!(item, TimingReportItem::AccountingNote))
        .count();
    assert_eq!(note_count, 1, "exactly one accounting note is required");

    let mut items = report.items.iter();
    let pipeline = items.next().expect("pipeline section should be first");
    assert!(
        matches!(pipeline, TimingReportItem::Section(section) if section.title == "Build pipeline"),
        "pipeline section should be first"
    );
    let note = items
        .next()
        .expect("accounting note should follow pipeline");
    assert!(
        matches!(note, TimingReportItem::AccountingNote),
        "accounting note should follow the pipeline section"
    );
}

/// The accounting note appears for the check command.
#[test]
fn accounting_note_appears_after_pipeline_for_check() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Check, true);
    let note_count = report
        .items
        .iter()
        .filter(|item| matches!(item, TimingReportItem::AccountingNote))
        .count();
    assert_eq!(note_count, 1, "exactly one accounting note is required");
}

/// The accounting note appears for the dev command.
#[test]
fn accounting_note_appears_after_pipeline_for_dev() {
    let report = build_timing_summary(&build_snapshot(), TimingCommandKind::Dev, true);
    let note_count = report
        .items
        .iter()
        .filter(|item| matches!(item, TimingReportItem::AccountingNote))
        .count();
    assert_eq!(note_count, 1, "exactly one accounting note is required");
}

/// The accounting note text states the pipeline-only rule.
#[test]
fn accounting_note_text_states_pipeline_only_rule() {
    assert!(
        crate::timing::enabled::render::ACCOUNTING_NOTE_TEXT
            .contains("Only pipeline rows account for the command total"),
        "accounting note must state the pipeline-only rule"
    );
    assert!(
        crate::timing::enabled::render::ACCOUNTING_NOTE_TEXT.contains("overlapping attribution"),
        "accounting note must mention overlapping attribution"
    );
}
