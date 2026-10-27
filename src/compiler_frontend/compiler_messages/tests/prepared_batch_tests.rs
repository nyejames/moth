//! Focused behavioural tests for direct prepared-batch capture and boundary installation.

use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, UnknownValueNameView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    self, TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{DiagnosticSeverity, TypeMismatchContext};
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
    let diagnostic = names_declarations::unknown_value_name(name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![diagnostic]),
        string_table,
        None,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let result = accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::<()>::Diagnosed);
    let Ok(CompilationOutcome::Diagnosed(reports)) = result else {
        panic!("the captured error should produce a diagnosed result");
    };
    assert_eq!(reports.counts().errors, 1);
    assert_eq!(reports.counts().warnings, 0);

    let mut diagnostic_reports = reports.iter();
    let (report, view) = diagnostic_reports
        .next()
        .expect("the captured diagnostic is retained");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("the typed family view matches");
    assert_eq!(
        report.context().strings().resolve(facts.name),
        "unknown_name"
    );
    assert_eq!(view.severity(), DiagnosticSeverity::Error);
    assert!(diagnostic_reports.next().is_none());
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
    let diagnostic = types_borrows_targets::type_mismatch(
        nominal_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        TypeMismatchContext::Assignment,
        None,
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![diagnostic]),
        string_table,
        Some(&type_environment),
        Some(path_table),
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let result = accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::<()>::Diagnosed);
    let Ok(CompilationOutcome::Diagnosed(reports)) = result else {
        panic!("the captured type mismatch should produce a diagnosed result");
    };
    let (report, view) = reports
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
fn captured_warning_is_not_carried_with_diagnosed_error() {
    let mut string_table = StringTable::new();
    let warning_name = string_table.intern("unused");
    let error_name = string_table.intern("missing");
    let warning = names_declarations::unknown_value_name(warning_name, None)
        .with_severity(DiagnosticSeverity::Warning);
    let error = names_declarations::unknown_value_name(error_name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning, error]),
        string_table,
        None,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    let result = accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::<()>::Diagnosed);
    let Ok(CompilationOutcome::Diagnosed(reports)) = result else {
        panic!("the captured error should produce a diagnosed result");
    };
    assert_eq!(reports.counts().warnings, 0);
    assert_eq!(reports.counts().errors, 1);
    let (report, view) = reports
        .iter()
        .next()
        .expect("the captured error is retained");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("the retained diagnostic uses the unknown-value family");
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
fn prepared_classifier_keeps_empty_and_warning_only_invariants() {
    let empty = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::new(),
        StringTable::new(),
        None,
        None,
    );
    let empty_error = match empty.classify_premerge() {
        Ok(_) => panic!("an empty prepared batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        &*empty_error.message,
        "module semantic stage returned a premerge batch with no diagnostics"
    );

    let mut strings = StringTable::new();
    let warning = names_declarations::unknown_value_name(strings.intern("unused"), None)
        .with_severity(DiagnosticSeverity::Warning);
    let warning_only = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning]),
        strings,
        None,
        None,
    );
    let warning_error = match warning_only.classify_premerge() {
        Ok(_) => panic!("a warning-only prepared batch must be an invariant error"),
        Err(error) => error,
    };
    assert_eq!(
        &*warning_error.message,
        "module semantic stage returned a premerge batch with no user-facing error diagnostic"
    );
}

#[test]
fn root_domain_capture_renders_against_the_seeded_root_table() {
    let mut root_string_table = StringTable::new();
    let name = root_string_table.intern("unknown_name");
    let diagnostic = names_declarations::unknown_value_name(name, None);
    let root_domain_batch = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from_diagnostics(vec![diagnostic]),
        &root_string_table,
        None,
        || None,
    )
    .expect("root-domain capture without path payloads cannot fail");

    // Root batches append directly into the run accumulator without a local-table seed batch.
    let mut accumulator = BoundaryAccumulator::with_root_string_table(root_string_table);
    accumulator.append_prepared(root_domain_batch, None);
    let result = accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::<()>::Diagnosed);
    let Ok(CompilationOutcome::Diagnosed(reports)) = result else {
        panic!("the root-domain error should produce a diagnosed result");
    };
    assert_eq!(reports.counts().errors, 1);
    assert_eq!(reports.counts().warnings, 0);

    let mut diagnostic_reports = reports.iter();
    let (report, view) = diagnostic_reports
        .next()
        .expect("the root-domain diagnostic is retained");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("the typed family view matches");
    assert_eq!(
        report.context().strings().resolve(facts.name),
        "unknown_name"
    );
    assert_eq!(view.severity(), DiagnosticSeverity::Error);
    assert!(diagnostic_reports.next().is_none());
}
