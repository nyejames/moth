//! Focused tests for build-config resolution diagnostics at the build boundary.
//!
//! WHAT: exercises the real resolver and config-boundary diagnostic mapper for contract
//!       conflicts, unknown explicit inputs and typed value mismatches.
//! WHY: these failures cross a StringTable boundary before they reach renderers, so tests must
//!      inspect structured payloads, identities, exact spans and payload facts rather than prose.

use super::config_boundary::build_config_resolution_failure;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigContractFact, BuildConfigInputEntry, BuildConfigInputSet,
    BuildConfigInputValue, BuildConfigResolutionError, BuildConfigValueLocation, BuildInputName,
    BuildInputType, BuilderConfigGlobalSet, PrimitiveBuildInputType, PrimitiveBuildValue,
    resolve_build_config_values,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::{
    boundary_accumulator::BoundaryAccumulator,
    families::project_config::{InvalidConfigCode, InvalidConfigView},
    foreign::SiteRef,
    freeze::InstallMode,
    labels::DurableLabelMessage,
    report::DiagnosticReport,
    view::DiagnosticView,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn exact_span(source_index: usize, start: u32, length: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, length, &mut builder).expect("test span should fit");
    SourceSpan::new(SourceId::from_index(source_index), local)
}

fn command_location(argument_index: usize) -> BuildConfigValueLocation {
    BuildConfigValueLocation::Command(BuildCommandLocation::new(argument_index))
}

fn input(
    name: &str,
    value: impl Into<BuildConfigInputValue>,
    argument_index: usize,
) -> BuildConfigInputEntry {
    BuildConfigInputEntry::new(
        BuildInputName::new(name).expect("test input name should be lower_snake_case"),
        value,
        command_location(argument_index),
    )
}

fn i32_value(value: i64) -> PrimitiveBuildValue {
    let fixed = FixedScalarValue::signed(FixedScalar::I32, value);
    PrimitiveBuildValue::Fixed(fixed.expect("test integer should fit I32"))
}

fn diagnosed_reports(
    failure: PreparedFailure,
) -> crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("mapped config failure should be diagnosed, not infrastructure");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    accumulator
        .finish(InstallMode::Unfrozen)
        .expect("mapped config diagnostics should freeze")
}

fn assert_config_identity_and_span<'a>(
    reports: &'a crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet,
    expected_span: Option<SourceSpan>,
    expected_reason_key: &str,
) -> (&'a DiagnosticReport, DiagnosticView<'a>) {
    assert_eq!(reports.counts().errors, 1);
    let (report, diagnostic) = reports
        .iter()
        .next()
        .expect("mapped config failure should contain one diagnostic");
    assert_eq!(diagnostic.severity(), DiagnosticSeverity::Error);
    assert_eq!(diagnostic.descriptor().code, "MOTH-CONFIG-0001");
    assert_eq!(diagnostic.reason_key(), Some(expected_reason_key));
    match (diagnostic.primary_site(), expected_span) {
        (SiteRef::Absent, None) => {}
        (SiteRef::Local(actual), Some(expected)) => assert_eq!(actual, expected),
        (actual, expected) => panic!("unexpected primary site {actual:?} for {expected:?}"),
    }
    (report, diagnostic)
}

fn diagnostic_strings(report: &DiagnosticReport, diagnostic: &DiagnosticView<'_>) -> Vec<String> {
    diagnostic
        .string_ids()
        .iter()
        .map(|id| report.context().strings().resolve(*id).to_owned())
        .collect()
}

#[test]
fn mapped_config_contract_conflict_preserves_payload_spans_and_labels() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");

    let first_span = exact_span(0, 12, 7);
    let conflicting_span = exact_span(1, 33, 8);
    let name = BuildInputName::new("setting").expect("test input name should be valid");
    let source_facts = [
        BuildConfigContractFact::new(
            name.clone(),
            BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
            true,
            None,
            Some(first_span),
        ),
        BuildConfigContractFact::new(
            name,
            BuildInputType::Primitive(PrimitiveBuildInputType::String),
            true,
            None,
            Some(conflicting_span),
        ),
    ];

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("different source contracts should produce a conflict");
    assert!(matches!(
        &error,
        BuildConfigResolutionError::SourceContractConflict { .. }
    ));

    let failure = diagnosed_reports(build_config_resolution_failure(
        error,
        None,
        &mut boundary_table,
    ));
    let (report, diagnostic) = assert_config_identity_and_span(
        &failure,
        Some(conflicting_span),
        "invalid_config.config_contract_conflict",
    );
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 1);
    assert!(labels[0].spanned);
    assert!(matches!(
        labels[0].site,
        SiteRef::Local(span) if span == first_span
    ));
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::PreviousDeclaration
    ));
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("the config failure should retain typed conflict facts");
    assert_eq!(facts.reason, InvalidConfigCode::ConfigContractConflict);
    assert_eq!(
        diagnostic_strings(report, &diagnostic),
        [
            "I32; required; no default",
            "String; required; no default",
            "setting"
        ]
    );
}

#[test]
fn mapped_unknown_build_config_input_preserves_spanlessness_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");

    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(input("mystery_input", i32_value(42), 17))
        .expect("test input should insert");
    let error = resolve_build_config_values(
        &[],
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("an input without a selected contract should be unknown");
    assert!(matches!(
        &error,
        BuildConfigResolutionError::UnknownExplicitInput { .. }
    ));

    let failure = diagnosed_reports(build_config_resolution_failure(
        error,
        None,
        &mut boundary_table,
    ));
    let (report, diagnostic) = assert_config_identity_and_span(
        &failure,
        None,
        "invalid_config.unknown_build_config_input",
    );
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("the config failure should retain typed unknown-input facts");
    assert_eq!(facts.reason, InvalidConfigCode::UnknownBuildConfigInput);
    assert_eq!(facts.provided_argument_index, Some(17));
    assert_eq!(diagnostic_strings(report, &diagnostic), ["mystery_input"]);
}

#[test]
fn mapped_config_input_type_mismatch_preserves_contract_span_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let contract_span = exact_span(2, 41, 7);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        None,
        Some(contract_span),
    )];
    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(input(
            "count",
            PrimitiveBuildValue::String("four".to_owned()),
            23,
        ))
        .expect("test input should insert");

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("a String should not satisfy an I32 contract");
    assert!(matches!(
        &error,
        BuildConfigResolutionError::ValueTypeMismatch { .. }
    ));

    let failure = diagnosed_reports(build_config_resolution_failure(
        error,
        None,
        &mut boundary_table,
    ));
    let (report, diagnostic) = assert_config_identity_and_span(
        &failure,
        Some(contract_span),
        "invalid_config.config_input_type_mismatch",
    );
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("the config failure should retain typed mismatch facts");
    assert_eq!(facts.reason, InvalidConfigCode::ConfigInputTypeMismatch);
    assert_eq!(facts.provided_argument_index, Some(23));
    assert_eq!(
        diagnostic_strings(report, &diagnostic),
        ["String", "I32", "count"]
    );
}

#[test]
fn mapped_command_literal_outside_its_contract_scalar_keeps_the_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let contract_span = exact_span(2, 41, 7);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        None,
        Some(contract_span),
    )];
    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(input(
            "count",
            BuildConfigInputValue::from_command_text("4_294_967_296")
                .expect("whole-number command text should classify"),
            5,
        ))
        .expect("test input should insert");

    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &explicit_inputs,
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("a value above the I32 maximum should not satisfy an I32 contract");

    let failure = diagnosed_reports(build_config_resolution_failure(
        error,
        None,
        &mut boundary_table,
    ));
    let (report, diagnostic) = assert_config_identity_and_span(
        &failure,
        Some(contract_span),
        "invalid_config.config_input_out_of_range",
    );
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("the config failure should retain typed out-of-range facts");
    assert_eq!(
        facts.reason,
        InvalidConfigCode::ConfigInputOutOfRangeOutsideFixedScalarRange
    );
    assert_eq!(facts.provided_argument_index, Some(5));
    assert_eq!(facts.fixed_scalar, Some(FixedScalar::I32));
    assert_eq!(
        diagnostic_strings(report, &diagnostic),
        ["4_294_967_296", "count"]
    );
}

#[test]
fn mapped_missing_source_contract_publishes_exact_primary_span() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let source_span = exact_span(3, 37, 12);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Fixed(FixedScalar::I32)),
        true,
        None,
        Some(source_span),
    )];
    let error = resolve_build_config_values(
        &source_facts,
        &[],
        &[],
        &BuildConfigInputSet::new(),
        &BuilderConfigGlobalSet::new(),
    )
    .expect_err("a required source contract without a value should fail");
    assert!(matches!(
        &error,
        BuildConfigResolutionError::MissingRequiredValue { .. }
    ));

    let failure = diagnosed_reports(build_config_resolution_failure(
        error,
        None,
        &mut boundary_table,
    ));
    let (_, diagnostic) = assert_config_identity_and_span(
        &failure,
        Some(source_span),
        "invalid_config.missing_config_input",
    );
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("the config failure should retain typed missing-value facts");
    assert_eq!(facts.reason, InvalidConfigCode::MissingConfigInput);
}
