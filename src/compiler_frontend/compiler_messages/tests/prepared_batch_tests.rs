//! Focused behavioural tests for direct prepared-batch capture and boundary installation.

use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::UnknownValueNameView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticSeverity, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, builtin_type_ids};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::sync::Arc;

#[test]
fn captured_bag_keeps_local_strings_and_typed_family_facts() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("unknown_name");
    let diagnostic = CompilerDiagnostic::unknown_value_name(name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![diagnostic]),
        string_table,
        None,
        false,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Unfrozen);
    assert_eq!(failure.reports.counts().errors, 1);
    assert_eq!(failure.reports.counts().warnings, 0);

    let mut reports = failure.reports.iter();
    let (report, view) = reports.next().expect("the captured diagnostic is retained");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("the typed family view matches");
    assert_eq!(
        report.context().strings().resolve(facts.name),
        "unknown_name"
    );
    assert_eq!(view.severity(), DiagnosticSeverity::Error);
    assert!(reports.next().is_none());
}

#[test]
fn captured_bag_snapshots_type_facts_from_its_live_environment() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let nominal_path = path_fork
        .try_intern_portable_path("NominalType", &mut string_table)
        .expect("nominal path fits");
    let path_table = Arc::new(path_fork.snapshot_table());
    let mut type_environment = TypeEnvironment::new();
    let (_, nominal_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: nominal_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let diagnostic = CompilerDiagnostic::type_mismatch(
        nominal_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        TypeMismatchContext::Assignment,
        None,
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![diagnostic]),
        string_table,
        Some(&type_environment),
        true,
        Some(path_table),
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Unfrozen);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the captured type mismatch is retained");
    let facts = view
        .family_view::<TypeMismatchView>()
        .expect("the typed family view matches");
    assert_eq!(facts.context, TypeMismatchContext::Assignment);
    assert_eq!(
        view.types().display_for_render(
            facts.expected,
            report.context().strings(),
            Some(report.context().paths()),
        ),
        "NominalType"
    );
}

#[test]
fn captured_warning_and_error_keep_push_order_and_severity() {
    let mut string_table = StringTable::new();
    let warning_name = string_table.intern("unused");
    let error_name = string_table.intern("missing");
    let mut warning = CompilerDiagnostic::unknown_value_name(warning_name, None);
    warning.severity = DiagnosticSeverity::Warning;
    let error = CompilerDiagnostic::unknown_value_name(error_name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning, error]),
        string_table,
        None,
        false,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Unfrozen);
    assert_eq!(failure.reports.counts().warnings, 1);
    assert_eq!(failure.reports.counts().errors, 1);

    let ordered: Vec<_> = failure
        .reports
        .iter()
        .map(|(report, view)| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("both records use the unknown-value family");
            (
                view.severity(),
                report.context().strings().resolve(facts.name).to_owned(),
            )
        })
        .collect();
    assert_eq!(
        ordered,
        [
            (DiagnosticSeverity::Warning, String::from("unused")),
            (DiagnosticSeverity::Error, String::from("missing")),
        ]
    );
}

#[test]
fn prepared_classifier_keeps_empty_and_warning_only_invariants() {
    let empty = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::new(),
        StringTable::new(),
        None,
        false,
        None,
    );
    let empty_error = match empty.classify_premerge() {
        Ok(_) => panic!("an empty prepared batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        empty_error.msg,
        "module semantic stage returned a premerge batch with no diagnostics"
    );

    let mut strings = StringTable::new();
    let warning = CompilerDiagnostic::unknown_value_name(strings.intern("unused"), None);
    let mut warning = warning;
    warning.severity = DiagnosticSeverity::Warning;
    let warning_only = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning]),
        strings,
        None,
        false,
        None,
    );
    let warning_error = match warning_only.classify_premerge() {
        Ok(_) => panic!("a warning-only prepared batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        warning_error.msg,
        "module semantic stage returned a premerge batch with no user-facing error diagnostic"
    );
}
