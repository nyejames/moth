//! Frozen report counts, canonical run order and producer-invariant validation.
//!
//! WHAT: compacts ordinary drafts into report stores and checks the immutable read paths.
//! WHY: boundary renderers must retain legacy cross-domain order without copying stores or views.

use super::exact_span;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::unreachable_match_arm;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unused_import;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    duplicate_declaration, unknown_value_name,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::SeverityOverride;
use crate::compiler_frontend::compiler_messages::diagnostics::report::{
    DiagnosticReport, DiagnosticReportSet, DiagnosticRun, SeverityCounts,
};
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::source::{FrozenIdentityContext, SourceDatabase};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::sync::Arc;

fn frozen_context() -> Arc<FrozenIdentityContext> {
    Arc::new(FrozenIdentityContext::from_parts(
        StringTable::new(),
        SourceDatabase::empty(),
    ))
}

fn compact_store(drafts: impl IntoIterator<Item = DiagnosticDraft>) -> DiagnosticStore {
    let mut store = DiagnosticStore::new();
    for draft in drafts {
        assert!(matches!(store.compact(draft), CompactOutcome::Stored(_)));
    }
    store
}

fn report_with_context(
    context: Arc<FrozenIdentityContext>,
    drafts: impl IntoIterator<Item = DiagnosticDraft>,
) -> DiagnosticReport {
    DiagnosticReport::new(
        context,
        compact_store(drafts),
        TypeDisplaySnapshot::default(),
    )
}

fn report(drafts: impl IntoIterator<Item = DiagnosticDraft>) -> DiagnosticReport {
    report_with_context(frozen_context(), drafts)
}

fn interleaved_report_set(
    project_context: Arc<FrozenIdentityContext>,
    package_context: Arc<FrozenIdentityContext>,
) -> DiagnosticReportSet {
    let project = report_with_context(
        project_context,
        [
            unreachable_match_arm(Some(exact_span(0, 1))),
            unused_import(Some(exact_span(1, 1))),
            unknown_value_name(StringId::from_index(1), Some(exact_span(2, 1))),
            duplicate_declaration(StringId::from_index(2), Some(exact_span(3, 1))),
        ],
    );
    let package = report_with_context(
        package_context,
        [
            unused_import(Some(exact_span(4, 1))),
            unreachable_match_arm(Some(exact_span(5, 1))),
            duplicate_declaration(StringId::from_index(3), Some(exact_span(6, 1))),
            unknown_value_name(StringId::from_index(4), Some(exact_span(7, 1))),
        ],
    );

    DiagnosticReportSet::new(
        vec![project, package],
        vec![
            DiagnosticRun {
                report: 0,
                first: 0,
                len: 2,
            },
            DiagnosticRun {
                report: 1,
                first: 0,
                len: 2,
            },
            DiagnosticRun {
                report: 0,
                first: 2,
                len: 2,
            },
            DiagnosticRun {
                report: 1,
                first: 2,
                len: 2,
            },
        ],
    )
}

#[test]
fn report_counts_resolve_default_and_overridden_severities_once() {
    let mut warning_as_error = unreachable_match_arm(Some(exact_span(2, 1)));
    warning_as_error.core.severity = SeverityOverride::Error;
    let mut error_as_warning = unknown_value_name(StringId::from_index(2), Some(exact_span(3, 1)));
    error_as_warning.core.severity = SeverityOverride::Warning;
    let mut warning_as_note = unreachable_match_arm(Some(exact_span(4, 1)));
    warning_as_note.core.severity = SeverityOverride::Note;

    let report = report([
        unknown_value_name(StringId::from_index(1), Some(exact_span(0, 1))),
        unreachable_match_arm(Some(exact_span(1, 1))),
        warning_as_error,
        error_as_warning,
        warning_as_note,
    ]);

    assert_eq!(report.len(), 5);
    assert_eq!(
        report.counts(),
        SeverityCounts {
            errors: 2,
            warnings: 2,
            notes: 1,
        }
    );
    assert_eq!(
        report
            .iter()
            .map(|view| view.family().descriptor().code)
            .collect::<Vec<_>>(),
        [
            "MOTH-RULE-0034",
            "MOTH-RULE-0022",
            "MOTH-RULE-0022",
            "MOTH-RULE-0034",
            "MOTH-RULE-0022",
        ]
    );
    assert_eq!(
        report
            .iter_severity(DiagnosticSeverity::Warning)
            .map(|view| view.family().descriptor().code)
            .collect::<Vec<_>>(),
        ["MOTH-RULE-0022", "MOTH-RULE-0034"]
    );
}

#[test]
fn report_set_iterates_interleaved_project_and_package_runs() {
    let project_context = frozen_context();
    let package_context = frozen_context();
    let reports =
        interleaved_report_set(Arc::clone(&project_context), Arc::clone(&package_context));

    assert_eq!(
        reports.counts(),
        SeverityCounts {
            errors: 4,
            warnings: 4,
            notes: 0,
        }
    );
    assert!(!reports.is_empty());

    let actual = reports
        .iter()
        .map(|(report, view)| {
            (
                std::ptr::eq(report.context(), project_context.as_ref()),
                view.family().descriptor().code,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            (true, "MOTH-RULE-0022"),
            (true, "MOTH-IMPORT-0002"),
            (false, "MOTH-IMPORT-0002"),
            (false, "MOTH-RULE-0022"),
            (true, "MOTH-RULE-0034"),
            (true, "MOTH-RULE-0002"),
            (false, "MOTH-RULE-0002"),
            (false, "MOTH-RULE-0034"),
        ]
    );
}

#[test]
fn report_set_severity_filter_keeps_production_order_within_each_run() {
    let project_context = frozen_context();
    let package_context = frozen_context();
    let reports =
        interleaved_report_set(Arc::clone(&project_context), Arc::clone(&package_context));
    assert_eq!(
        reports
            .iter_severity(DiagnosticSeverity::Warning)
            .map(|(report, view)| {
                (
                    std::ptr::eq(report.context(), project_context.as_ref()),
                    view.family().descriptor().code,
                )
            })
            .collect::<Vec<_>>(),
        [
            (true, "MOTH-RULE-0022"),
            (true, "MOTH-IMPORT-0002"),
            (false, "MOTH-IMPORT-0002"),
            (false, "MOTH-RULE-0022"),
        ]
    );

    assert_eq!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .map(|(_, view)| view.family().descriptor().code)
            .collect::<Vec<_>>(),
        [
            "MOTH-RULE-0034",
            "MOTH-RULE-0002",
            "MOTH-RULE-0002",
            "MOTH-RULE-0034",
        ]
    );
}

#[test]
fn empty_report_set_has_no_reports_or_diagnostics() {
    let reports = DiagnosticReportSet::empty();

    assert!(reports.is_empty());
    assert_eq!(reports.counts(), SeverityCounts::default());
    assert_eq!(reports.iter().count(), 0);
    assert_eq!(reports.iter_severity(DiagnosticSeverity::Error).count(), 0);
}

#[test]
#[should_panic(expected = "diagnostic run refers to a report that does not exist")]
fn report_set_rejects_a_run_for_a_missing_report() {
    let _reports = DiagnosticReportSet::new(
        vec![report([])],
        vec![DiagnosticRun {
            report: 1,
            first: 0,
            len: 0,
        }],
    );
}

#[test]
#[should_panic(expected = "diagnostic run extends beyond its report's records")]
fn report_set_rejects_a_run_outside_its_report() {
    let _reports = DiagnosticReportSet::new(
        vec![report([unreachable_match_arm(Some(exact_span(0, 1)))])],
        vec![DiagnosticRun {
            report: 0,
            first: 0,
            len: 2,
        }],
    );
}

#[test]
#[should_panic(expected = "diagnostic run end index fits in u32")]
fn report_set_rejects_a_run_whose_end_overflows() {
    let _reports = DiagnosticReportSet::new(
        vec![report([])],
        vec![DiagnosticRun {
            report: 0,
            first: u32::MAX,
            len: 1,
        }],
    );
}

#[test]
#[should_panic(expected = "ascending order")]
fn report_set_rejects_runs_that_start_out_of_order() {
    let _reports = DiagnosticReportSet::new(
        vec![report([
            unreachable_match_arm(Some(exact_span(0, 1))),
            unreachable_match_arm(Some(exact_span(1, 1))),
        ])],
        vec![
            DiagnosticRun {
                report: 0,
                first: 1,
                len: 1,
            },
            DiagnosticRun {
                report: 0,
                first: 0,
                len: 1,
            },
        ],
    );
}

#[test]
#[should_panic(expected = "ascending order")]
fn report_set_rejects_overlapping_runs() {
    let _reports = DiagnosticReportSet::new(
        vec![report([
            unreachable_match_arm(Some(exact_span(0, 1))),
            unreachable_match_arm(Some(exact_span(1, 1))),
        ])],
        vec![
            DiagnosticRun {
                report: 0,
                first: 0,
                len: 1,
            },
            DiagnosticRun {
                report: 0,
                first: 0,
                len: 1,
            },
        ],
    );
}

#[test]
#[should_panic(expected = "cover every report record exactly once")]
fn report_set_rejects_uncovered_records() {
    let _reports = DiagnosticReportSet::new(
        vec![report([
            unreachable_match_arm(Some(exact_span(0, 1))),
            unreachable_match_arm(Some(exact_span(1, 1))),
        ])],
        vec![DiagnosticRun {
            report: 0,
            first: 0,
            len: 1,
        }],
    );
}
