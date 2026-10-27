//! Frozen report counts, canonical run order and producer-invariant validation.
//!
//! WHAT: compacts ordinary drafts into report stores and checks the immutable read paths.
//! WHY: boundary renderers must keep the canonical cross-domain run order without copying stores or views.

use super::{exact_span, frozen_identity_context, identity_for};
use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::compaction::CompactOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::unreachable_match_arm;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unused_import;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, UnknownValueNameView, duplicate_declaration, unknown_value_name,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CompilerCapacityExceededView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::{
    ForeignSiteDraft, ForeignSlot, SiteRef,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    GenericSubstitutionDiagnostic, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::record::SeverityOverride;
use crate::compiler_frontend::compiler_messages::diagnostics::report::{
    DiagnosticReport, DiagnosticReportSet, DiagnosticRun, SeverityCounts,
};
use crate::compiler_frontend::compiler_messages::diagnostics::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::diagnostics::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    compact_bag_for_test, finish_prepared_batch,
};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticBag, DiagnosticCapacityResource, DiagnosticSeverity,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{FrozenIdentityContext, SourceDatabase, SourceSpan};
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
        assert!(matches!(
            store.compact(draft, &mut Vec::new()),
            CompactOutcome::Stored(_)
        ));
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
        Vec::new(),
        TypeDisplaySnapshot::default(),
    )
}

fn report(drafts: impl IntoIterator<Item = DiagnosticDraft>) -> DiagnosticReport {
    report_with_context(frozen_context(), drafts)
}

fn capacity_report_with_cold_warning(
    context: Arc<FrozenIdentityContext>,
    foreign_context: Arc<FrozenIdentityContext>,
    warning_span: SourceSpan,
    label_span: SourceSpan,
    capacity_span: SourceSpan,
) -> DiagnosticReport {
    let mut types = TypeDisplaySnapshotBuilder::new(None);
    let mut warning = unreachable_match_arm(Some(warning_span))
        .with_label(
            Some(label_span),
            Some(LabelDraftMessage::GenericInstantiationSubstitutions {
                substitutions: vec![GenericSubstitutionDiagnostic {
                    parameter_name: StringId::from_index(1),
                    concrete_type_id: TypeId(7),
                }],
            }),
        )
        .with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Label(0),
            context: identity_for(&foreign_context),
        });
    warning
        .capture_types(&mut |type_id| types.display_top_level(type_id))
        .expect("test type display fits");
    let types = types.finish();

    let mut limits = DiagnosticLimits::production();
    limits.records = 2;
    let mut diagnostics = DiagnosticStore::with_limits(limits);
    let mut foreign_contexts = Vec::new();
    assert!(matches!(
        diagnostics.compact(warning, &mut foreign_contexts),
        CompactOutcome::Stored(_)
    ));
    assert_eq!(
        diagnostics.compact(
            unreachable_match_arm(Some(capacity_span)),
            &mut foreign_contexts,
        ),
        CompactOutcome::Rejected(DiagnosticCapacityResource::Records)
    );

    DiagnosticReport::new(context, diagnostics, foreign_contexts, types)
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
            duplicate_declaration(StringId::from_index(2), Some(exact_span(3, 1))).with_label(
                Some(exact_span(4, 1)),
                Some(LabelDraftMessage::PreviousDeclaration),
            ),
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
    let reports = DiagnosticReportSet::new(
        vec![report],
        vec![DiagnosticRun {
            report: 0,
            first: 0,
            len: 5,
        }],
    );
    assert_eq!(
        reports
            .iter()
            .map(|(_, view)| view.family().descriptor().code)
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
        reports
            .iter_severity(DiagnosticSeverity::Warning)
            .map(|(_, view)| view.family().descriptor().code)
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
    assert!(reports.iter().next().is_some());

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

    let diagnosed = reports.into_errors();
    assert_eq!(
        diagnosed.counts(),
        SeverityCounts {
            errors: 4,
            warnings: 0,
            notes: 0,
        }
    );
    assert_eq!(
        diagnosed
            .iter()
            .map(|(report, view)| {
                (
                    std::ptr::eq(report.context(), project_context.as_ref()),
                    view.descriptor().code,
                )
            })
            .collect::<Vec<_>>(),
        [
            (true, "MOTH-RULE-0034"),
            (true, "MOTH-RULE-0002"),
            (false, "MOTH-RULE-0002"),
            (false, "MOTH-RULE-0034"),
        ]
    );
    let duplicate = diagnosed
        .iter()
        .find(|(report, view)| {
            std::ptr::eq(report.context(), project_context.as_ref())
                && view.descriptor().code == "MOTH-RULE-0002"
        })
        .expect("the labelled ordinary error remains")
        .1;
    assert_eq!(duplicate.durable_labels().count(), 1);
}

#[test]
fn warning_only_report_set_is_unchanged_by_into_errors() {
    let span = exact_span(1, 1);
    let warnings = DiagnosticReportSet::new(
        vec![report([unreachable_match_arm(Some(span))])],
        vec![DiagnosticRun {
            report: 0,
            first: 0,
            len: 1,
        }],
    );

    let reports = warnings.into_errors();

    assert_eq!(
        reports.counts(),
        SeverityCounts {
            errors: 0,
            warnings: 1,
            notes: 0,
        }
    );
    let (report, warning) = reports.iter().next().expect("the warning remains");
    assert_eq!(warning.descriptor().code, "MOTH-RULE-0022");
    assert_eq!(warning.severity(), DiagnosticSeverity::Warning);
    assert!(matches!(warning.primary_site(), SiteRef::Local(actual) if actual == span));
    assert_eq!(report.len(), 1);
}

#[test]
fn empty_report_set_has_no_reports_or_diagnostics() {
    let reports = DiagnosticReportSet::new(Vec::new(), Vec::new());

    assert!(reports.iter().next().is_none());
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

#[test]
fn explicit_severity_can_override_descriptor_default() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("unused_name");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(
            names_declarations::unknown_value_name(name, None)
                .with_severity(DiagnosticSeverity::Warning),
        ),
        None,
    );
    let view = compact.first().expect("unknown-name draft compacts");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("unknown-name facts remain typed");

    assert_eq!(view.severity(), DiagnosticSeverity::Warning);
    assert_eq!(string_table.resolve(facts.name), "unused_name");
}

#[test]
fn diagnostic_bag_tracks_errors_warnings_and_order() {
    let mut string_table = StringTable::new();
    let first = names_declarations::unknown_value_name(string_table.intern("missing"), None);
    let second = names_declarations::unknown_value_name(string_table.intern("warning_name"), None)
        .with_severity(DiagnosticSeverity::Warning);
    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(vec![first, second]), None);
    let ordered = compact
        .views()
        .map(|view| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("unknown-value family");
            (
                view.descriptor().code,
                view.severity(),
                string_table.resolve(facts.name).to_owned(),
                matches!(view.primary_site(), SiteRef::Absent),
                view.durable_labels().count(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ordered
            .iter()
            .filter(|entry| entry.1 == DiagnosticSeverity::Error)
            .count(),
        1
    );
    assert_eq!(
        ordered
            .iter()
            .filter(|entry| entry.1 == DiagnosticSeverity::Warning)
            .count(),
        1
    );
    assert_eq!(
        ordered,
        [
            (
                "MOTH-RULE-0034",
                DiagnosticSeverity::Error,
                String::from("missing"),
                true,
                0
            ),
            (
                "MOTH-RULE-0034",
                DiagnosticSeverity::Warning,
                String::from("warning_name"),
                true,
                0
            ),
        ]
    );
    assert_eq!(compact.len(), 2);
}

#[test]
fn diagnostic_bag_counts_and_orders_structured_diagnostics() {
    let mut string_table = StringTable::new();
    let error = names_declarations::unknown_value_name(string_table.intern("missing"), None);
    let warning = names_declarations::unknown_value_name(string_table.intern("maybe_unused"), None)
        .with_severity(DiagnosticSeverity::Warning);
    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(vec![warning, error]), None);
    let ordered = compact
        .views()
        .map(|view| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("unknown-value family");
            (view.severity(), string_table.resolve(facts.name).to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ordered,
        [
            (DiagnosticSeverity::Warning, String::from("maybe_unused")),
            (DiagnosticSeverity::Error, String::from("missing")),
        ]
    );

    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![
            names_declarations::unknown_value_name(string_table.intern("maybe_unused"), None)
                .with_severity(DiagnosticSeverity::Warning),
            names_declarations::unknown_value_name(string_table.intern("missing"), None),
        ]),
        string_table,
        None,
        None,
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("the prepared error should produce a diagnosed result");
    };
    assert_eq!(reports.counts().errors, 1);
    assert_eq!(reports.counts().warnings, 0);
    let (report, view) = reports
        .iter()
        .next()
        .expect("the prepared error is retained");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("unknown-value family");
    assert_eq!(view.severity(), DiagnosticSeverity::Error);
    assert_eq!(report.context().strings().resolve(facts.name), "missing");
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Warning)
            .next()
            .is_none()
    );
}

#[test]
fn warning_capacity_failure_discards_orphaned_cold_report_data() {
    let project_context = frozen_identity_context();
    let package_context = frozen_identity_context();
    let project_foreign_context = frozen_identity_context();
    let package_foreign_context = frozen_identity_context();
    let project_capacity_span = exact_span(20, 1);
    let package_capacity_span = exact_span(40, 1);
    let reports = DiagnosticReportSet::new(
        vec![
            capacity_report_with_cold_warning(
                Arc::clone(&project_context),
                project_foreign_context,
                exact_span(0, 1),
                exact_span(1, 1),
                project_capacity_span,
            ),
            capacity_report_with_cold_warning(
                Arc::clone(&package_context),
                package_foreign_context,
                exact_span(10, 1),
                exact_span(11, 1),
                package_capacity_span,
            ),
        ],
        vec![
            DiagnosticRun {
                report: 0,
                first: 0,
                len: 1,
            },
            DiagnosticRun {
                report: 1,
                first: 0,
                len: 1,
            },
            DiagnosticRun {
                report: 0,
                first: 1,
                len: 1,
            },
            DiagnosticRun {
                report: 1,
                first: 1,
                len: 1,
            },
        ],
    );

    let before = reports.memory_ledger_metrics();
    assert!(before.extra_rows > 0);
    assert!(before.extra_capacity_bytes > 0);
    assert!(before.label_rows > 0);
    assert!(before.label_capacity_bytes > 0);
    assert!(before.label_data_rows > 0);
    assert!(before.label_data_capacity_bytes > 0);
    assert!(before.substitution_rows > 0);
    assert!(before.substitution_capacity_bytes > 0);
    assert!(before.foreign_site_rows > 0);
    assert!(before.foreign_site_capacity_bytes > 0);
    assert!(before.foreign_context_rows > 0);
    assert!(before.foreign_context_capacity_bytes > 0);
    assert!(before.type_display_record_rows > 0);
    assert!(before.type_display_record_capacity_bytes > 0);

    let diagnosed = reports.into_errors();

    assert_eq!(
        diagnosed.counts(),
        SeverityCounts {
            errors: 2,
            warnings: 0,
            notes: 0,
        }
    );
    let retained = diagnosed.iter().collect::<Vec<_>>();
    assert_eq!(retained.len(), 2);
    for (index, (report, view)) in retained.iter().enumerate() {
        assert_eq!(view.descriptor().code, "MOTH-SYNTAX-0037");
        assert_eq!(view.severity(), DiagnosticSeverity::Error);
        let expected_context = if index == 0 {
            project_context.as_ref()
        } else {
            package_context.as_ref()
        };
        assert!(std::ptr::eq(report.context(), expected_context));
        let expected_span = if index == 0 {
            project_capacity_span
        } else {
            package_capacity_span
        };
        assert!(matches!(
            view.primary_site(),
            SiteRef::Local(actual) if actual == expected_span
        ));
        let facts = view
            .family_view::<CompilerCapacityExceededView>()
            .expect("the retained record has capacity facts");
        assert!(matches!(
            facts.resource,
            CapacityResource::Store(DiagnosticCapacityResource::Records)
        ));
        let render = view.family().render.expect("capacity facts render");
        let rendered = render(view, DiagnosticRenderContext::for_report(report));
        assert_eq!(
            rendered.message,
            "This source exceeds the compiler's capacity for diagnostic records."
        );
    }

    let after = diagnosed.memory_ledger_metrics();
    assert_eq!(after.extra_rows, 0);
    assert_eq!(after.extra_capacity_bytes, 0);
    assert_eq!(after.label_rows, 0);
    assert_eq!(after.label_capacity_bytes, 0);
    assert_eq!(after.label_data_rows, 0);
    assert_eq!(after.label_data_capacity_bytes, 0);
    assert_eq!(after.string_list_rows, 0);
    assert_eq!(after.string_list_capacity_bytes, 0);
    assert_eq!(after.substitution_rows, 0);
    assert_eq!(after.substitution_capacity_bytes, 0);
    assert_eq!(after.foreign_site_rows, 0);
    assert_eq!(after.foreign_site_capacity_bytes, 0);
    assert_eq!(after.foreign_context_rows, 0);
    assert_eq!(after.foreign_context_capacity_bytes, 0);
    assert_eq!(after.type_display_record_rows, 0);
    assert_eq!(after.type_display_record_capacity_bytes, 0);
    assert_eq!(after.type_display_auxiliary_rows, 0);
    assert_eq!(after.type_display_auxiliary_capacity_bytes, 0);
}
