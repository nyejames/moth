//! Focused tests for build-config resolution diagnostics at the build boundary.
//!
//! WHAT: exercises the real resolver and config-boundary diagnostic mapper for contract
//!       conflicts, unknown explicit inputs and typed value mismatches.
//! WHY: these failures cross a StringTable boundary before they reach renderers, so tests must
//!      inspect structured payloads, identities, exact spans and payload facts rather than prose.

use super::config_boundary::build_config_resolution_failure;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigContractFact, BuildConfigInputEntry, BuildConfigInputSet,
    BuildConfigResolutionError, BuildConfigValueLocation, BuildInputName, BuildInputType,
    BuilderConfigGlobalSet, PrimitiveBuildInputType, PrimitiveBuildValue,
    resolve_build_config_values,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticPayload, InvalidConfigReason, PremergeFailure,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn exact_span(source_index: usize, start: u32, length: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, length, &mut builder).expect("test span should fit");
    SourceSpan::new(SourceId::from_index(source_index), local)
}

fn command_location(argument_index: usize) -> BuildConfigValueLocation {
    BuildConfigValueLocation::Command(BuildCommandLocation::new(argument_index))
}

fn input(name: &str, value: PrimitiveBuildValue, argument_index: usize) -> BuildConfigInputEntry {
    BuildConfigInputEntry::new(
        BuildInputName::new(name).expect("test input name should be lower_snake_case"),
        value,
        command_location(argument_index),
    )
}

fn assert_config_identity_and_span<'a>(
    messages: &'a crate::compiler_frontend::compiler_errors::CompilerMessages,
    expected_span: Option<SourceSpan>,
    expected_reason_key: &str,
) -> &'a crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    assert_eq!(messages.error_count(), 1);
    let diagnostic = messages
        .first_error()
        .expect("mapped config failure should contain one error");
    let identity = diagnostic.identity();
    assert_eq!(identity.code, "MOTH-CONFIG-0001");
    assert_eq!(identity.reason_key, Some(expected_reason_key));
    assert_eq!(diagnostic.primary_span, expected_span);
    assert!(diagnostic.labels.is_empty());
    diagnostic
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
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
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

    let failure = build_config_resolution_failure(error, None, &mut boundary_table);
    let PremergeFailure::Diagnosed(batch) = failure else {
        panic!("mapped config failure should be diagnosed, not infrastructure");
    };
    let messages = batch.into_messages();
    let diagnostic = messages
        .first_error()
        .expect("mapped conflict should contain one error");
    let identity = diagnostic.identity();
    assert_eq!(identity.code, "MOTH-CONFIG-0001");
    assert_eq!(
        identity.reason_key,
        Some("invalid_config.config_contract_conflict")
    );
    assert_eq!(diagnostic.primary_span, Some(conflicting_span));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(first_span));
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::PreviousDeclaration)
    );

    let DiagnosticPayload::InvalidConfig {
        key: Some(key),
        reason: InvalidConfigReason::ConfigContractConflict { first, conflicting },
    } = &diagnostic.payload
    else {
        panic!("expected a structured config contract conflict payload");
    };
    assert_eq!(messages.string_table.resolve(*key), "setting");
    assert_eq!(
        messages.string_table.resolve(*first),
        "Int; required; no default"
    );
    assert_eq!(
        messages.string_table.resolve(*conflicting),
        "String; required; no default"
    );
}

#[test]
fn mapped_unknown_build_config_input_preserves_spanlessness_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");

    let mut explicit_inputs = BuildConfigInputSet::new();
    explicit_inputs
        .insert(input("mystery_input", PrimitiveBuildValue::Int(42), 17))
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

    let failure = build_config_resolution_failure(error, None, &mut boundary_table);
    let PremergeFailure::Diagnosed(batch) = failure else {
        panic!("mapped config failure should be diagnosed, not infrastructure");
    };
    let messages = batch.into_messages();
    let diagnostic = assert_config_identity_and_span(
        &messages,
        None,
        "invalid_config.unknown_build_config_input",
    );

    let DiagnosticPayload::InvalidConfig {
        key: Some(diagnostic_key),
        reason:
            InvalidConfigReason::UnknownBuildConfigInput {
                key: reason_key,
                provided_argument_index,
            },
    } = &diagnostic.payload
    else {
        panic!("expected a structured unknown build-config input payload");
    };
    assert_eq!(
        messages.string_table.resolve(*diagnostic_key),
        "mystery_input"
    );
    assert_eq!(messages.string_table.resolve(*reason_key), "mystery_input");
    assert_eq!(diagnostic_key, reason_key);
    assert_eq!(*provided_argument_index, Some(17));
}

#[test]
fn mapped_config_input_type_mismatch_preserves_contract_span_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let contract_span = exact_span(2, 41, 7);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
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
    .expect_err("a String should not satisfy an Int contract");
    assert!(matches!(
        &error,
        BuildConfigResolutionError::ValueTypeMismatch { .. }
    ));

    let failure = build_config_resolution_failure(error, None, &mut boundary_table);
    let PremergeFailure::Diagnosed(batch) = failure else {
        panic!("mapped config failure should be diagnosed, not infrastructure");
    };
    let messages = batch.into_messages();
    let diagnostic = assert_config_identity_and_span(
        &messages,
        Some(contract_span),
        "invalid_config.config_input_type_mismatch",
    );

    let DiagnosticPayload::InvalidConfig {
        key: Some(key),
        reason:
            InvalidConfigReason::ConfigInputTypeMismatch {
                provided,
                expected,
                provided_argument_index,
            },
    } = &diagnostic.payload
    else {
        panic!("expected a structured config input type mismatch payload");
    };
    assert_eq!(messages.string_table.resolve(*key), "count");
    assert_eq!(messages.string_table.resolve(*provided), "String");
    assert_eq!(messages.string_table.resolve(*expected), "Int");
    assert_eq!(*provided_argument_index, Some(23));
}

#[test]
fn mapped_missing_source_contract_publishes_exact_primary_span() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let source_span = exact_span(3, 37, 12);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
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

    let failure = build_config_resolution_failure(error, None, &mut boundary_table);
    let PremergeFailure::Diagnosed(batch) = failure else {
        panic!("mapped config failure should be diagnosed, not infrastructure");
    };
    let messages = batch.into_messages();
    let diagnostic = messages
        .first_error()
        .expect("mapped missing config input should contain one error");
    assert_eq!(diagnostic.primary_span, Some(source_span));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::MissingConfigInput,
            ..
        }
    ));
}
