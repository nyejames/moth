//! Focused tests for build-config resolution diagnostics at the build boundary.
//!
//! WHAT: exercises the real resolver and config-boundary diagnostic mapper for contract
//!       conflicts, unknown explicit inputs and typed value mismatches.
//! WHY: these failures cross a StringTable boundary before they reach renderers, so tests must
//!      inspect structured payloads, identities, labels and source locations rather than prose.

use super::config_boundary::build_config_resolution_messages;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigContractFact, BuildConfigInputEntry, BuildConfigInputSet,
    BuildConfigResolutionError, BuildConfigValueLocation, BuildInputName, BuildInputType,
    BuilderConfigGlobalSet, PrimitiveBuildInputType, PrimitiveBuildValue,
    resolve_build_config_values,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload, InvalidConfigReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn source_location(
    string_table: &mut StringTable,
    path: &str,
    line: i32,
    start_column: i32,
    end_column: i32,
) -> SourceLocation {
    SourceLocation::new(
        InternedPath::from_single_str(path, string_table),
        CharPosition {
            line_number: line,
            char_column: start_column,
        },
        CharPosition {
            line_number: line,
            char_column: end_column,
        },
    )
}

/// Build one source location in a worker-local table, then remap it into the boundary table.
///
/// Seeding the destination with a different string makes the remap non-identity. This mirrors
/// the production merge-before-map handoff and makes a foreign StringId resolve to the wrong path
/// if the handoff is accidentally omitted.
fn location_in_boundary_table(
    boundary_table: &mut StringTable,
    path: &str,
    line: i32,
    start_column: i32,
    end_column: i32,
) -> SourceLocation {
    let mut local_table = StringTable::new();
    let mut location = source_location(&mut local_table, path, line, start_column, end_column);
    let remap = boundary_table.merge_from(&local_table);
    assert!(
        !remap.is_identity(),
        "the distinct local and boundary tables must exercise a non-identity remap"
    );
    location.remap_string_ids(&remap);
    location
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

fn assert_config_identity_and_location<'a>(
    messages: &'a crate::compiler_frontend::compiler_errors::CompilerMessages,
    expected_location: &SourceLocation,
    expected_reason_key: &str,
) -> &'a crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    assert_eq!(messages.error_count(), 1);
    let diagnostic = messages
        .first_error()
        .expect("mapped config failure should contain one error");
    let identity = diagnostic.identity();
    assert_eq!(identity.code, "MOTH-CONFIG-0001");
    assert_eq!(identity.reason_key, Some(expected_reason_key));
    assert_eq!(&diagnostic.primary_location, expected_location);
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Primary);
    assert_eq!(diagnostic.labels[0].location, *expected_location);
    assert_eq!(diagnostic.labels[0].message, None);
    diagnostic
}

#[test]
fn mapped_config_contract_conflict_preserves_payload_locations_and_labels() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");

    let first_location =
        location_in_boundary_table(&mut boundary_table, "first-contract.moth", 3, 2, 9);
    let conflicting_location =
        location_in_boundary_table(&mut boundary_table, "conflicting-contract.moth", 8, 4, 12);
    let fallback_location =
        location_in_boundary_table(&mut boundary_table, "config-fallback.moth", 1, 0, 1);

    let name = BuildInputName::new("setting").expect("test input name should be valid");
    let source_facts = [
        BuildConfigContractFact::new(
            name.clone(),
            BuildInputType::Primitive(PrimitiveBuildInputType::Int),
            true,
            None,
            first_location.clone(),
        ),
        BuildConfigContractFact::new(
            name,
            BuildInputType::Primitive(PrimitiveBuildInputType::String),
            true,
            None,
            conflicting_location.clone(),
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

    let messages = build_config_resolution_messages(error, fallback_location, &mut boundary_table);
    let diagnostic = messages
        .first_error()
        .expect("mapped conflict should contain one error");
    let identity = diagnostic.identity();
    assert_eq!(identity.code, "MOTH-CONFIG-0001");
    assert_eq!(
        identity.reason_key,
        Some("invalid_config.config_contract_conflict")
    );
    assert_eq!(diagnostic.primary_location, conflicting_location);
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_portable_string(&messages.string_table),
        "conflicting-contract.moth"
    );
    assert_eq!(diagnostic.labels.len(), 2);
    assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Primary);
    assert_eq!(diagnostic.labels[0].location, conflicting_location);
    assert_eq!(diagnostic.labels[0].message, None);
    assert_eq!(diagnostic.labels[1].style, DiagnosticLabelStyle::Secondary);
    assert_eq!(diagnostic.labels[1].location, first_location);
    assert_eq!(
        diagnostic.labels[1].message,
        Some(DiagnosticLabelMessage::PreviousDeclaration)
    );
    assert_eq!(
        diagnostic.labels[1]
            .location
            .scope
            .to_portable_string(&messages.string_table),
        "first-contract.moth"
    );
    assert_eq!(diagnostic.labels[1].location.start_pos.line_number, 3);
    assert_eq!(diagnostic.labels[1].location.start_pos.char_column, 2);

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
fn mapped_unknown_build_config_input_preserves_fallback_location_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let fallback_location =
        location_in_boundary_table(&mut boundary_table, "unknown-fallback.moth", 6, 1, 6);

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

    let messages =
        build_config_resolution_messages(error, fallback_location.clone(), &mut boundary_table);
    let diagnostic = assert_config_identity_and_location(
        &messages,
        &fallback_location,
        "invalid_config.unknown_build_config_input",
    );
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_portable_string(&messages.string_table),
        "unknown-fallback.moth"
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
fn mapped_config_input_type_mismatch_preserves_contract_location_and_argument_index() {
    let mut boundary_table = StringTable::new();
    boundary_table.intern("boundary-table-prefix");
    let contract_location =
        location_in_boundary_table(&mut boundary_table, "typed-contract.moth", 11, 3, 10);
    let fallback_location =
        location_in_boundary_table(&mut boundary_table, "typed-fallback.moth", 1, 0, 1);

    let source_facts = [BuildConfigContractFact::new(
        BuildInputName::new("count").expect("test input name should be valid"),
        BuildInputType::Primitive(PrimitiveBuildInputType::Int),
        true,
        None,
        contract_location.clone(),
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

    let messages = build_config_resolution_messages(error, fallback_location, &mut boundary_table);
    let diagnostic = assert_config_identity_and_location(
        &messages,
        &contract_location,
        "invalid_config.config_input_type_mismatch",
    );
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_portable_string(&messages.string_table),
        "typed-contract.moth"
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 11);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 3);

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
