use super::{
    BorrowAccessKind, BorrowDiagnosticKind, CommonSyntaxMistakeReason,
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, ConfigDiagnosticKind,
    DeferredFeatureDiagnosticKind, DeferredFeatureReason, DiagnosticBag, DiagnosticCategory,
    DiagnosticCompoundAssignmentOperator, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, DiagnosticSeverity,
    GenericApplicationErrorReason, ImportClauseKind, ImportDiagnosticKind,
    IncompatibleChoiceComparisonReason, InfrastructureDiagnosticKind,
    InvalidAssignmentTargetReason, InvalidCallShapeReason, InvalidCastReason,
    InvalidChoiceVariantReason, InvalidCollectionTypeReason, InvalidConfigReason,
    InvalidExpressionReason, InvalidFallibleHandlingReason, InvalidFallibleOperandReason,
    InvalidFunctionSignatureReason, InvalidGenericParameterReason, InvalidImportClauseReason,
    InvalidMapTypeReason, InvalidReceiverCallReason, InvalidSignatureMemberReason,
    InvalidStandaloneStatementReason, InvalidStatementPositionReason, InvalidStringEscapeReason,
    InvalidTemplateDirectiveReason, InvalidTemplateStructureReason, InvalidTraitKeywordUsageReason,
    InvalidTypeAnnotationReason, MissingWhitespace, NameNamespace, NumberLiteralErrorReason,
    PathKind, ReceiverCallKind, RuleDiagnosticKind, SymbolicSpacingConstruct, SymbolicSpacingError,
    SyntaxDiagnosticKind, TypeAnnotationContext, TypeDiagnosticKind, TypeMismatchContext,
    UnsupportedBackendFeatureReason, UnsupportedOperatorCategory, is_well_formed_reason_key,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, builtin_type_ids};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{PathTokenItem, TokenKind};
use std::collections::HashSet;
use std::path::Path;

const DIAGNOSTIC_PAYLOAD_SOURCE: &str = include_str!("../diagnostic_payload/mod.rs");

fn location(path: InternedPath) -> SourceLocation {
    SourceLocation::new(
        path,
        CharPosition {
            line_number: 1,
            char_column: 2,
        },
        CharPosition {
            line_number: 1,
            char_column: 4,
        },
    )
}

fn unknown_name_diagnostic(
    name: StringId,
    namespace: NameNamespace,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        location,
        DiagnosticPayload::UnknownName { name, namespace },
    )
}

fn borrow_conflict_diagnostic(
    place: DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Borrow(BorrowDiagnosticKind::BorrowConflict),
        location,
        DiagnosticPayload::BorrowConflict {
            place,
            existing_access,
            requested_access,
        },
    )
}

#[test]
fn descriptor_codes_are_stable_and_non_empty() {
    let cases = [
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
            "MOTH-SYNTAX-0001",
            "Expected token",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            "MOTH-SYNTAX-0002",
            "Unexpected token",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTrailingComma),
            "MOTH-SYNTAX-0003",
            "Unexpected trailing comma",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            "MOTH-RULE-0001",
            "Unknown name",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnusedVariable),
            "MOTH-RULE-0010",
            "Unused variable",
            DiagnosticSeverity::Warning,
        ),
        (
            DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
            "MOTH-TYPE-0001",
            "Type mismatch",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            "MOTH-IMPORT-0005",
            "Missing import target",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Borrow(BorrowDiagnosticKind::BorrowConflict),
            "MOTH-BORROW-0001",
            "Access conflict",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Config(ConfigDiagnosticKind::InvalidConfig),
            "MOTH-CONFIG-0001",
            "Invalid config",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure),
            "MOTH-INFRA-0001",
            "Infrastructure failure",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::DeferredFeature),
            "MOTH-DEFERRED-0001",
            "Deferred feature",
            DiagnosticSeverity::Error,
        ),
    ];

    for (kind, expected_code, expected_title, expected_severity) in cases {
        let descriptor = kind.descriptor();
        assert_eq!(descriptor.code, expected_code);
        assert_eq!(descriptor.title, expected_title);
        assert_eq!(descriptor.default_severity, expected_severity);
        assert!(!descriptor.title.is_empty());
        assert!(!kind.code().is_empty());
    }
}

#[test]
fn every_diagnostic_descriptor_has_a_unique_category_code() {
    let mut codes = HashSet::new();

    for kind in DiagnosticKind::all() {
        let descriptor = kind.descriptor();

        assert!(
            !descriptor.code.is_empty(),
            "{kind:?} must have a stable diagnostic code",
        );
        assert!(
            !descriptor.title.is_empty(),
            "{kind:?} must have a user-facing descriptor title",
        );
        assert!(
            descriptor
                .code
                .starts_with(expected_code_prefix(kind.category())),
            "{kind:?} code '{}' does not match its category {:?}",
            descriptor.code,
            kind.category(),
        );
        assert!(
            codes.insert(descriptor.code),
            "{kind:?} reuses diagnostic code '{}'",
            descriptor.code,
        );
    }
}

#[test]
fn old_error_payload_has_been_removed_from_diagnostic_payloads() {
    assert!(
        !DIAGNOSTIC_PAYLOAD_SOURCE.contains(concat!("Legacy", "Error")),
        "diagnostic payloads should no longer expose the old error variant",
    );
}

fn expected_code_prefix(category: DiagnosticCategory) -> &'static str {
    match category {
        DiagnosticCategory::Syntax => "MOTH-SYNTAX-",
        DiagnosticCategory::Type => "MOTH-TYPE-",
        DiagnosticCategory::Rule => "MOTH-RULE-",
        DiagnosticCategory::Import => "MOTH-IMPORT-",
        DiagnosticCategory::Borrow => "MOTH-BORROW-",
        DiagnosticCategory::Config => "MOTH-CONFIG-",
        DiagnosticCategory::Infrastructure => "MOTH-INFRA-",
        DiagnosticCategory::DeferredFeature => "MOTH-DEFERRED-",
    }
}

#[test]
fn category_and_default_severity_derive_from_kind() {
    let syntax = DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken);
    let type_mismatch = DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch);
    let import = DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget);

    assert_eq!(syntax.category(), DiagnosticCategory::Syntax);
    assert_eq!(type_mismatch.category(), DiagnosticCategory::Type);
    assert_eq!(import.category(), DiagnosticCategory::Import);
    assert_eq!(syntax.default_severity(), DiagnosticSeverity::Error);
    assert_eq!(type_mismatch.default_severity(), DiagnosticSeverity::Error);
}

#[test]
fn explicit_severity_can_override_descriptor_default() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostic = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("unused_name"),
            namespace: NameNamespace::Value,
        },
    );

    assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
}

#[test]
fn diagnostic_identity_uses_descriptor_code_actual_severity_and_typed_reason() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostic = CompilerDiagnostic::with_severity(
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCollectionType),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::InvalidCollectionType {
            reason: InvalidCollectionTypeReason::ZeroCapacity,
        },
    );

    let identity = diagnostic.identity();

    assert_eq!(identity.code, "MOTH-SYNTAX-0016");
    assert_eq!(identity.severity, DiagnosticSeverity::Warning);
    assert_eq!(
        identity.reason_key,
        Some("invalid_collection_type.zero_capacity")
    );
}

#[test]
fn unsupported_backend_feature_exposes_stable_reason_key() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern("Wasm"),
        UnsupportedBackendFeatureReason::HashmapOperation,
        location(source_path),
    );

    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0064");
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("unsupported_backend_feature.hashmap_operation")
    );
}

#[test]
fn every_authored_stable_reason_key_is_unique_and_well_formed() {
    let keys = super::diagnostic_payload::stable_reason_keys_for_tests();
    let unique_keys = keys.iter().copied().collect::<HashSet<_>>();

    assert!(
        !keys.is_empty(),
        "the stable reason-key inventory must not be empty"
    );
    assert_eq!(
        keys.len(),
        unique_keys.len(),
        "stable reason keys must be globally unique",
    );

    for key in keys {
        assert!(
            is_well_formed_reason_key(key),
            "stable reason key '{key}' must use the qualified lower-snake-case format",
        );
    }
}

#[test]
fn stable_reason_key_format_rejects_unqualified_or_noncanonical_keys() {
    for key in [
        "invalid_expression.expected_operator",
        "invalid_expression.operand_2_missing",
    ] {
        assert!(is_well_formed_reason_key(key), "key should be valid: {key}");
    }

    for key in [
        "unqualified",
        "Invalid_expression.expected_operator",
        "invalid_expression.Expected_operator",
        "invalid_expression._expected_operator",
        "invalid_expression.expected__operator",
        "invalid_expression.expected_operator_",
        "invalid-expression.expected_operator",
        "invalid_expression.expected operator",
        "invalid_expression.",
    ] {
        assert!(
            !is_well_formed_reason_key(key),
            "key should be rejected: {key}"
        );
    }
}

#[test]
fn reasonless_payloads_have_no_reason_key() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("missing"),
            namespace: NameNamespace::Value,
        },
    );

    assert_eq!(diagnostic.identity().reason_key, None);
}

#[test]
fn reason_key_dispatch_covers_distinct_typed_reason_families() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let location = location(source_path);
    let diagnostics = [
        CompilerDiagnostic::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::NoneNotAllowed,
            location.clone(),
        ),
        CompilerDiagnostic::invalid_map_type(
            InvalidMapTypeReason::FixedCapacityNotAllowed,
            location.clone(),
        ),
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            location,
        ),
    ];

    assert_eq!(
        diagnostics[0].identity().reason_key,
        Some("invalid_type_annotation.none_not_allowed")
    );
    assert_eq!(
        diagnostics[1].identity().reason_key,
        Some("invalid_map_type.fixed_capacity_not_allowed")
    );
    assert_eq!(
        diagnostics[2].identity().reason_key,
        Some("invalid_collection_type.capacity_overflow")
    );
}

#[test]
fn diagnostic_bag_tracks_errors_warnings_and_order() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let first = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        location(source_path.clone()),
    );
    let second = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("warning_name"),
            namespace: NameNamespace::Value,
        },
    );

    let mut bag = DiagnosticBag::new();
    bag.push(first.clone());
    bag.push(second.clone());

    assert!(bag.has_errors());
    assert!(bag.has_warnings());
    assert_eq!(bag.errors().count(), 1);
    assert_eq!(bag.warnings().count(), 1);
    assert_eq!(bag.diagnostics(), &[first, second]);
    assert_eq!(bag.into_diagnostics().len(), 2);
}

#[test]
fn compiler_messages_counts_and_order_come_from_structured_diagnostics() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let error = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        location(source_path.clone()),
    );
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("maybe_unused"),
            namespace: NameNamespace::Value,
        },
    );

    let messages =
        CompilerMessages::from_diagnostics(vec![warning.clone(), error.clone()], string_table);

    assert!(messages.has_errors());
    assert!(messages.has_warnings());
    assert_eq!(messages.error_count(), 1);
    assert_eq!(messages.warning_count(), 1);

    let diagnostics = messages.diagnostics.to_vec();
    assert_eq!(diagnostics, vec![warning, error]);
    assert!(messages.first_infrastructure_error_for_tests().is_none());
}

#[test]
fn compiler_messages_with_warnings_keep_typed_diagnostics_off_error_mirrors() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let error = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        location(source_path.clone()),
    );
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("maybe_unused"),
            namespace: NameNamespace::Value,
        },
    );

    let messages = CompilerMessages::from_diagnostic_with_warnings(
        error.clone(),
        vec![warning.clone()],
        &string_table,
    );

    assert_eq!(messages.error_count(), 1);
    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.diagnostics.to_vec(), vec![warning, error]);
    assert!(messages.first_infrastructure_error_for_tests().is_none());
}

#[test]
fn compiler_messages_with_infrastructure_error_preserve_warning_production_order() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("maybe_unused"),
            namespace: NameNamespace::Value,
        },
    );
    let error = CompilerError::compiler_error("backend failed after warnings");

    let messages =
        CompilerMessages::from_error_with_warnings(error, vec![warning.clone()], &string_table);

    assert_eq!(messages.error_count(), 1);
    assert_eq!(messages.warning_count(), 1);
    assert_eq!(messages.diagnostics[0], warning);
    assert_eq!(
        messages.diagnostics[1].kind,
        DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure),
    );
    assert_eq!(messages.diagnostics[1].kind.code(), "MOTH-INFRA-0001");
}

#[test]
fn compiler_messages_preserve_type_context_ranges_when_prepending_and_appending() {
    let mut string_table = StringTable::new();
    let point_path = InternedPath::from_single_str("Point", &mut string_table);
    let status_path = InternedPath::from_single_str("Status", &mut string_table);

    let mut first_environment = TypeEnvironment::new();
    let (_, point_type) = first_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let first_error = CompilerDiagnostic::type_mismatch(
        point_type,
        first_environment.builtins().int,
        TypeMismatchContext::Assignment,
        SourceLocation::default(),
    );
    let warning = CompilerDiagnostic::unreachable_match_arm(SourceLocation::default());
    let mut first_messages =
        CompilerMessages::from_diagnostics(vec![first_error], string_table.clone())
            .with_type_context_for_all_diagnostics(first_environment);
    first_messages.prepend_diagnostics_preserving_context(vec![warning]);

    let mut second_environment = TypeEnvironment::new();
    let (_, status_type) = second_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: status_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let second_error = CompilerDiagnostic::type_mismatch(
        status_type,
        second_environment.builtins().string,
        TypeMismatchContext::ReturnValue,
        SourceLocation::default(),
    );
    let second_messages = CompilerMessages::from_diagnostics(vec![second_error], string_table)
        .with_type_context_for_all_diagnostics(second_environment);

    first_messages.append_messages_preserving_context(second_messages);
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &first_messages,
        );

    assert_eq!(
        first_messages.render_type_contexts()[0].diagnostic_range,
        1..2
    );
    assert_eq!(
        first_messages.render_type_contexts()[1].diagnostic_range,
        2..3
    );
    assert!(
        rendered[0].contains("expected Point, found Int"),
        "errors should render before warnings; first rendered line should be the Point error, got: {}",
        rendered[0]
    );
    assert!(
        rendered[1].contains("expected Status, found String"),
        "second rendered line should be the Status error, got: {}",
        rendered[1]
    );
}

#[test]
fn remap_string_ids_updates_locations_payloads_labels_and_tokens() {
    let mut local_table = StringTable::new();
    let main_path = InternedPath::from_single_str("main.moth", &mut local_table);
    let import_path = InternedPath::from_single_str("lib.moth", &mut local_table);
    let name = local_table.intern("Button");
    let alias = local_table.intern("AliasButton");
    let label_text = local_table.intern("temporary label");

    let primary_location = location(main_path.clone());
    let first_location = location(import_path.clone());

    let expected_token = CompilerDiagnostic::expected_token(
        TokenKind::Symbol(name),
        Some(TokenKind::Path(vec![PathTokenItem {
            path: import_path.clone(),
            alias: Some(alias),
            path_location: first_location.clone(),
            alias_location: Some(primary_location.clone()),
            from_grouped: true,
        }])),
        primary_location.clone(),
    );
    let duplicate = CompilerDiagnostic::duplicate_declaration(
        name,
        Some(first_location.clone()),
        primary_location.clone(),
    );
    let import = CompilerDiagnostic::import_name_collision(
        alias,
        Some(first_location.clone()),
        primary_location.clone(),
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        first_location,
        Some(DiagnosticLabelMessage::RenderedText(label_text)),
    )]);
    let borrow = borrow_conflict_diagnostic(
        DiagnosticPlace::Local(name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        primary_location,
    );

    let mut bag = DiagnosticBag::from_diagnostics(vec![expected_token, duplicate, import, borrow]);

    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let diagnostics = bag.diagnostics();
    match &diagnostics[0].payload {
        DiagnosticPayload::ExpectedToken {
            expected,
            found: Some(TokenKind::Path(items)),
        } => {
            assert!(matches!(expected, TokenKind::Symbol(_)));
            let item = items
                .first()
                .expect("path token item should remain present");
            assert_eq!(item.path.to_string(&merged_table), "lib.moth");
            assert_eq!(
                item.alias.map(|id| merged_table.resolve(id)),
                Some("AliasButton")
            );
        }
        payload => panic!("unexpected expected-token payload: {payload:?}"),
    }

    match &diagnostics[1].payload {
        DiagnosticPayload::DuplicateDeclaration {
            name,
            first_location,
        } => {
            assert_eq!(merged_table.resolve(*name), "Button");
            let previous_location = first_location
                .as_ref()
                .expect("explicit import should carry a previous location");
            assert_eq!(
                previous_location.scope.to_string(&merged_table),
                String::from("lib.moth")
            );
        }
        payload => panic!("unexpected duplicate payload: {payload:?}"),
    }

    match &diagnostics[2].payload {
        DiagnosticPayload::ImportNameCollision {
            name,
            previous_location,
        } => {
            assert_eq!(merged_table.resolve(*name), "AliasButton");
            assert!(previous_location.is_some());
        }
        payload => panic!("unexpected import payload: {payload:?}"),
    }

    match &diagnostics[2].labels[0].message {
        Some(DiagnosticLabelMessage::RenderedText(message)) => {
            assert_eq!(merged_table.resolve(*message), "temporary label");
        }
        message => panic!("unexpected label message: {message:?}"),
    }

    match &diagnostics[3].payload {
        DiagnosticPayload::BorrowConflict {
            place: DiagnosticPlace::Local(name),
            ..
        } => assert_eq!(merged_table.resolve(*name), "Button"),
        payload => panic!("unexpected borrow payload: {payload:?}"),
    }
}

#[test]
fn remap_string_ids_updates_missing_at_prefix_authored_path() {
    let mut local_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut local_table);
    let authored_path = local_table.intern("vendor/drawing.js");

    let diagnostic = CompilerDiagnostic::common_syntax_mistake(
        CommonSyntaxMistakeReason::ImportPathMissingAtPrefix { authored_path },
        location(source_path),
    );

    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    match &bag.diagnostics()[0].payload {
        DiagnosticPayload::CommonSyntaxMistake {
            reason: CommonSyntaxMistakeReason::ImportPathMissingAtPrefix { authored_path },
        } => {
            assert_eq!(
                merged_table.resolve(*authored_path),
                "vendor/drawing.js",
                "authored import path StringId must remain valid after table merge/remap"
            );
        }
        payload => panic!("unexpected missing-@ payload after remap: {payload:?}"),
    }
}

#[test]
fn duplicate_declaration_with_previous_location_keeps_secondary_label() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let declaration_name = string_table.intern("Button");
    let previous_location = location(source_path);
    let duplicate_location = location(InternedPath::from_single_str(
        "other.moth",
        &mut string_table,
    ));

    let diagnostic = CompilerDiagnostic::duplicate_declaration(
        declaration_name,
        Some(previous_location),
        duplicate_location,
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    );
    match &diagnostic.payload {
        DiagnosticPayload::DuplicateDeclaration {
            name,
            first_location,
        } => {
            assert_eq!(string_table.resolve(*name), "Button");
            assert!(
                first_location.is_some(),
                "explicit import carries a previous location"
            );
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
    assert_eq!(diagnostic.labels.len(), 2, "primary and secondary labels");
    assert!(
        diagnostic
            .labels
            .iter()
            .any(|label| label.message == Some(DiagnosticLabelMessage::PreviousDeclaration))
    );
}

#[test]
fn duplicate_declaration_without_previous_location_omits_secondary_label() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let declaration_name = string_table.intern("print");
    let duplicate_location = location(source_path);

    // Prelude-injected symbols have no authored previous location.
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(declaration_name, None, duplicate_location);

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    );
    match &diagnostic.payload {
        DiagnosticPayload::DuplicateDeclaration {
            name,
            first_location,
        } => {
            assert_eq!(string_table.resolve(*name), "print");
            assert!(
                first_location.is_none(),
                "prelude symbol has no previous location"
            );
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
    assert_eq!(diagnostic.labels.len(), 1, "only the primary label is kept");
    assert!(
        !diagnostic
            .labels
            .iter()
            .any(|label| label.message == Some(DiagnosticLabelMessage::PreviousDeclaration)),
        "no secondary label for prelude symbols"
    );
}

#[test]
fn type_mismatch_constructor_carries_type_ids_without_rendering() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::Declaration,
        location(source_path),
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch)
    );

    match diagnostic.payload {
        DiagnosticPayload::TypeMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, builtin_type_ids::INT);
            assert_eq!(found, builtin_type_ids::STRING);
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
}

#[test]
fn type_mismatch_terminal_guidance_renders_type_names_with_context() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::Declaration,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);

    assert!(guidance.iter().any(|line| line == "Expected: Int"));
    assert!(guidance.iter().any(|line| line == "Found: String"));
    assert!(!guidance.iter().any(|line| line.contains("type id")));
}

#[test]
fn type_mismatch_terse_renderer_renders_type_names_with_context() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::FunctionArgument,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

    let line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(line.contains("expected Int"));
    assert!(line.contains("found String"));
    assert!(!line.contains("Expected type id"));
    assert!(!line.contains("Found type id"));
}

#[test]
fn invalid_string_escape_renderer_preserves_the_authored_escape_spelling() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (escaped, expected) in [
        ('q', "Unsupported string escape '\\q'."),
        ('\t', "Unsupported string escape '\\\\t'."),
    ] {
        let diagnostic = CompilerDiagnostic::invalid_string_escape(
            InvalidStringEscapeReason::UnsupportedEscape { escaped },
            location(source_path.clone()),
        );
        let message =
            terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");

        assert!(message.contains(expected), "unexpected message: {message}");
    }
}

#[test]
fn invalid_string_escape_renderer_distinguishes_physical_newlines_and_trailing_backslashes() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (reason, expected) in [
        (
            InvalidStringEscapeReason::PhysicalNewline,
            "A backslash cannot continue a quoted string across a physical newline. Remove the backslash or use the two-character '\\n' escape.",
        ),
        (
            InvalidStringEscapeReason::TrailingBackslash,
            "The string ends with a backslash. Add a supported escaped character or remove the backslash.",
        ),
    ] {
        let diagnostic =
            CompilerDiagnostic::invalid_string_escape(reason, location(source_path.clone()));
        let message =
            terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");

        assert_eq!(message, expected);
    }
}

#[test]
fn rule_renderers_use_user_facing_messages_not_reason_debug_names() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let value_name = string_table.intern("value");

    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(value_name),
        None,
        None,
        None,
        None,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(guidance.iter().any(|line| line
        == "Cannot reassign `value` because its binding is immutable. Make the original binding mutable, then reassign it with ordinary `=`."));
    assert!(terse_line.contains("Cannot reassign `value`"));
    assert!(
        !guidance
            .iter()
            .any(|line| line.contains("ImmutableBinding"))
    );
    assert!(!terse_line.contains("ImmutableBinding"));
}

#[test]
fn immutable_binding_diagnostic_carries_secondary_declaration_label() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let value_name = string_table.intern("value");
    let declaration_location = location(source_path.clone());

    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(value_name),
        None,
        None,
        None,
        Some(declaration_location),
        location(source_path.clone()),
    );

    let labels = terminal::format_label_messages(&diagnostic, &string_table);

    assert!(
        labels
            .iter()
            .any(|label| label.contains("immutable binding declared here")),
        "expected secondary declaration label, got: {labels:?}"
    );
    assert_eq!(
        diagnostic.labels.len(),
        2,
        "expected primary and secondary labels"
    );
    assert_eq!(
        diagnostic.labels[1].message,
        Some(DiagnosticLabelMessage::ImmutableBindingDeclaration),
    );
}

#[test]
fn syntax_and_choice_renderers_use_user_facing_messages_not_reason_debug_names() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let choice_name = string_table.intern("Status");
    let variant_name = string_table.intern("Ready");

    let diagnostics = [
        (
            CompilerDiagnostic::invalid_path(
                PathKind::WhitespaceMustBeQuoted,
                location(source_path.clone()),
            ),
            "Path components with whitespace must be quoted",
            "WhitespaceMustBeQuoted",
        ),
        (
            CompilerDiagnostic::invalid_import_clause(
                ImportClauseKind::Alias,
                InvalidImportClauseReason::AliasNotValidIdentifier,
                location(source_path.clone()),
            ),
            "Import alias must be a valid local binding name",
            "AliasNotValidIdentifier",
        ),
        (
            CompilerDiagnostic::invalid_choice_variant(
                InvalidChoiceVariantReason::UnitVariantWithParentheses,
                Some(choice_name),
                Some(variant_name),
                Vec::new(),
                location(source_path),
            ),
            "Unit variant 'Status::Ready' cannot be called with empty parentheses",
            "UnitVariantWithParentheses",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
        let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "Expected guidance to contain '{expected_message}', got {guidance:?}",
        );
        assert!(
            terse_line.contains(expected_message),
            "Expected terse line to contain '{expected_message}', got {terse_line}",
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "Guidance should not expose enum variant '{debug_name}': {guidance:?}",
        );
        assert!(
            !terse_line.contains(debug_name),
            "Terse line should not expose enum variant '{debug_name}': {terse_line}",
        );
    }
}

#[test]
fn choice_variant_unknown_variant_suggests_close_candidate() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let choice_name = string_table.intern("Status");
    let misspelled = string_table.intern("Reay");
    let ready = string_table.intern("Ready");
    let error = string_table.intern("Error");
    let available = vec![ready, error];

    let diagnostic = CompilerDiagnostic::invalid_choice_variant(
        InvalidChoiceVariantReason::UnknownVariant,
        Some(choice_name),
        Some(misspelled),
        available,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);

    let message = guidance
        .iter()
        .find(|line| line.contains("Unknown variant"))
        .expect("expected an unknown-variant message");

    assert!(
        message.contains("Status::Reay"),
        "message should name the full misspelled variant: {message}",
    );
    assert!(
        message.contains("Did you mean 'Ready'?"),
        "message should suggest the closest existing variant: {message}",
    );
    assert!(
        message.contains("Available variants: [Ready, Error]"),
        "message must retain the available-variants list: {message}",
    );
}

#[test]
fn choice_variant_unknown_variant_no_suggestion_for_unrelated_name() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let choice_name = string_table.intern("Status");
    let unrelated = string_table.intern("Xyzzy");
    let ready = string_table.intern("Ready");
    let error = string_table.intern("Error");
    let available = vec![ready, error];

    let diagnostic = CompilerDiagnostic::invalid_choice_variant(
        InvalidChoiceVariantReason::UnknownVariant,
        Some(choice_name),
        Some(unrelated),
        available,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);

    let message = guidance
        .iter()
        .find(|line| line.contains("Unknown variant"))
        .expect("expected an unknown-variant message");

    assert!(
        message.contains("Status::Xyzzy"),
        "message should name the full misspelled variant: {message}",
    );
    assert!(
        !message.contains("Did you mean"),
        "an unrelated name must not get a bogus suggestion: {message}",
    );
    assert!(
        message.contains("Available variants: [Ready, Error]"),
        "message must retain the available-variants list: {message}",
    );
}

#[test]
fn syntax_renderers_keep_typed_prose_without_error_conversion() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let literal = string_table.intern("1.");
    let style_directive = string_table.intern("unknown");
    let supported_directives = string_table.intern("'$html', '$css'");
    let declaration_name = string_table.intern("Card");
    let template_directive = string_table.intern("insert");
    let namespace_name = string_table.intern("card");

    let diagnostics = [
        (
            CompilerDiagnostic::duplicate_declaration(
                declaration_name,
                Some(location(source_path.clone())),
                location(source_path.clone()),
            ),
            "Cannot declare 'Card' because that name is already visible in this scope",
            "StringId",
        ),
        (
            CompilerDiagnostic::invalid_number_literal(
                literal,
                NumberLiteralErrorReason::MultipleDecimalPoints,
                location(source_path.clone()),
            ),
            "Can't have more than one decimal point in numeric literal '1.'",
            "MultipleDecimalPoints",
        ),
        (
            CompilerDiagnostic::invalid_style_directive(
                style_directive,
                supported_directives,
                location(source_path.clone()),
            ),
            "Style directive '$unknown' is unsupported here",
            "InvalidStyleDirective",
        ),
        (
            CompilerDiagnostic::invalid_type_annotation(
                TypeAnnotationContext::DeclarationTarget,
                InvalidTypeAnnotationReason::UnexpectedColon,
                location(source_path.clone()),
            ),
            "Unexpected ':' after declaration name",
            "UnexpectedColon",
        ),
        (
            CompilerDiagnostic::invalid_signature_member(
                InvalidSignatureMemberReason::ChoicePayloadDefaultValue,
                location(source_path.clone()),
            ),
            "Choice payload fields cannot have default values.",
            "ChoicePayloadDefaultValue",
        ),
        (
            CompilerDiagnostic::invalid_signature_member(
                InvalidSignatureMemberReason::MissingDefaultValue,
                location(source_path.clone()),
            ),
            "Expected a default value after '='.",
            "MissingDefaultValue",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingColonAfterReturns,
                location(source_path.clone()),
            ),
            "Function return declarations must end with ':'",
            "MissingColonAfterReturns",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingReturnType,
                location(source_path.clone()),
            ),
            "Function signature is missing a return type after '->'. Add a type followed by ':', or remove '->' for a no-value function.",
            "MissingReturnType",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingTraitRequirementReturnType,
                location(source_path.clone()),
            ),
            "Trait requirement is missing a return type after '->'. Add a type, or remove '->' for a no-value requirement.",
            "MissingTraitRequirementReturnType",
        ),
        (
            CompilerDiagnostic::invalid_generic_application(
                GenericApplicationErrorReason::NestedApplication,
                location(source_path.clone()),
            ),
            "Nested generic type applications are not supported",
            "NestedApplication",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::NegativeCapacity,
                location(source_path.clone()),
            ),
            "Fixed collection capacity must be greater than zero.",
            "NegativeCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ZeroCapacity,
                location(source_path.clone()),
            ),
            "Fixed collection capacity must be greater than zero.",
            "ZeroCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityNotInt,
                location(source_path.clone()),
            ),
            "Collection capacity must be an integer.",
            "CapacityNotInt",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityNotConstant,
                location(source_path.clone()),
            ),
            "Collection capacity must be a positive integer literal or the bare name of a visible compile-time `Int` constant.",
            "CapacityNotConstant",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityOverflow,
                location(source_path.clone()),
            ),
            "Collection capacity is too large.",
            "CapacityOverflow",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::InitializerExceedsFixedCapacity {
                    capacity: 2,
                    length: 3,
                },
                location(source_path.clone()),
            ),
            "Collection literal has more items than the fixed collection capacity allows.",
            "InitializerExceedsFixedCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection,
                location(source_path.clone()),
            ),
            "Immutable binding initialized with an empty fixed collection literal is not allowed.",
            "EmptyImmutableFixedCollection",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous,
                location(source_path.clone()),
            ),
            "Capacity-only shorthand requires a non-empty collection literal so the element type can be inferred.",
            "ShorthandEmptyLiteralAmbiguous",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                location(source_path.clone()),
            ),
            "Capacity-only shorthand requires a collection literal initializer.",
            "ShorthandNonLiteralRhs",
        ),
        (
            CompilerDiagnostic::invalid_generic_parameter(
                InvalidGenericParameterReason::BoundsMustUseIs,
                location(source_path.clone()),
            ),
            "Generic parameter bounds use `is`.",
            "BoundsMustUseIs",
        ),
        (
            CompilerDiagnostic::invalid_trait_keyword_usage(
                InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
                location(source_path.clone()),
            ),
            "Keyword 'must' is trait-only syntax",
            "MustOutsideTraitSyntax",
        ),
        (
            CompilerDiagnostic::invalid_template_directive(
                Some(template_directive),
                InvalidTemplateDirectiveReason::MissingArgument,
                location(source_path.clone()),
            ),
            "Template directive 'insert' is missing a required argument.",
            "MissingArgument",
        ),
        (
            CompilerDiagnostic::namespace_misuse(
                namespace_name,
                NameNamespace::Type,
                NameNamespace::Value,
                location(source_path.clone()),
            ),
            "'card' is a value and cannot be used as a type.",
            "NamespaceMisuse",
        ),
        (
            CompilerDiagnostic::unsupported_operator_types(
                DiagnosticOperator::Add,
                builtin_type_ids::STRING,
                Some(builtin_type_ids::INT),
                location(source_path.clone()),
            ),
            "Operator `+` cannot concatenate",
            "Add",
        ),
        (
            CompilerDiagnostic::invalid_fallible_operand(
                InvalidFallibleOperandReason::FallibleValueNotHandled,
                UnsupportedOperatorCategory::Arithmetic,
                builtin_type_ids::STRING,
                location(source_path),
            ),
            "arithmetic operator cannot use a fallible value that has not been handled",
            "FallibleValueNotHandled",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
        let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "Expected guidance to contain '{expected_message}', got {guidance:?}",
        );
        assert!(
            terse_line.contains(expected_message),
            "Expected terse line to contain '{expected_message}', got {terse_line}",
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "Guidance should not expose '{debug_name}': {guidance:?}",
        );
        assert!(
            !terse_line.contains(debug_name),
            "Terse output should not expose '{debug_name}': {terse_line}",
        );
    }
}

#[test]
fn invalid_expression_renderers_keep_structured_reason_prose() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);

    let diagnostics = [
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::ExpectedOperatorBeforeExpression,
                location(source_path.clone()),
            ),
            "Expected an operator before this expression.",
            "ExpectedOperatorBeforeExpression",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::UnresolvedStackShape,
                location(source_path),
            ),
            "This expression does not resolve to exactly one value.",
            "UnresolvedStackShape",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
        let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "Expected guidance to contain '{expected_message}', got {guidance:?}",
        );
        assert!(
            terse_line.contains(expected_message),
            "Expected terse line to contain '{expected_message}', got {terse_line}",
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "Guidance should not expose '{debug_name}': {guidance:?}",
        );
        assert!(
            !terse_line.contains(debug_name),
            "Terse output should not expose '{debug_name}': {terse_line}",
        );
    }
}

#[test]
fn phase_1_2_renderers_keep_source_language_terminology() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let config_key = string_table.intern("homepage");
    let diagnostics = vec![
        CompilerDiagnostic::invalid_standalone_statement(
            InvalidStandaloneStatementReason::StandaloneTemplate,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::FallibleValueInTemplateHead,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateIfConditionNotConst,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_fallible_operand(
            InvalidFallibleOperandReason::FallibleValueNotHandled,
            UnsupportedOperatorCategory::Arithmetic,
            builtin_type_ids::STRING,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnOptional,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::ValueCouldNotFold,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_cast(
            InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
            None,
            None,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
            None,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::AsyncBlock,
            location(source_path),
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);
    let rendered = diagnostics
        .iter()
        .flat_map(|diagnostic| {
            terminal::format_payload_guidance(&diagnostic.payload, render_context)
        })
        .collect::<Vec<_>>()
        .join("\n");

    for expected in [
        "A standalone template is not a valid statement here",
        "top-level generic declaration header",
        "`Box of String`",
        "compatible fallible function",
        "`if ... is |present| ... else ...`",
        "`!` propagates an `Error!` return, but this expression is not fallible",
        "Config declarations cannot depend on runtime evaluation",
        "User-defined cast evidence must be fully evaluable at compile time",
        "`value String? = none`",
        "future language support",
        "This template must be fully evaluated at compile time",
        "optional value's presence cannot be determined at compile time",
    ] {
        assert!(
            rendered.contains(expected),
            "expected source-language diagnostic text '{expected}' in: {rendered}",
        );
    }
}

#[test]
fn render_boundary_smoke_coverage_hides_internal_debug_names_by_family() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let import_path = InternedPath::from_single_str("missing.moth", &mut string_table);
    let value_name = string_table.intern("value");
    let config_key = string_table.intern("homepage");
    let feature_name = string_table.intern("traits");
    let type_environment = TypeEnvironment::new();

    let diagnostics = vec![
        CompilerDiagnostic::invalid_path(
            PathKind::WhitespaceMustBeQuoted,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::type_mismatch(
            type_environment.builtins().int,
            type_environment.builtins().string,
            TypeMismatchContext::Declaration,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::ImmutableBinding,
            Some(value_name),
            None,
            None,
            None,
            None,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::missing_import_target(import_path, location(source_path.clone())),
        borrow_conflict_diagnostic(
            DiagnosticPlace::Local(value_name),
            BorrowAccessKind::Shared,
            BorrowAccessKind::Mutable,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::UnsupportedScalarValue,
            location(source_path.clone()),
        ),
        CompilerDiagnostic::deferred_feature(feature_name, location(source_path)),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));

    for diagnostic in &diagnostics {
        let terminal_guidance =
            terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");
        let terse_line = terse::format_terse_diagnostic_with_context(diagnostic, render_context);
        let dev_server_html = dev_server::render_diagnostics_html_with_context(
            std::slice::from_ref(diagnostic),
            Path::new("/tmp"),
            render_context,
        );

        assert_rendered_diagnostic_hides_internal_names(&terminal_guidance);
        assert_rendered_diagnostic_hides_internal_names(&terse_line);
        assert_rendered_diagnostic_hides_internal_names(&dev_server_html);
        assert!(terse_line.contains(diagnostic.kind.code()));
        assert!(dev_server_html.contains(diagnostic.kind.code()));
    }
}

fn assert_rendered_diagnostic_hides_internal_names(rendered: &str) {
    for internal_name in [
        "StringId(",
        "TypeId(",
        "DiagnosticPlace",
        "BorrowAccessKind",
        "InvalidConfigReason",
        "DeferredFeatureReason",
        "WhitespaceMustBeQuoted",
        "ImmutableBinding",
    ] {
        assert!(
            !rendered.contains(internal_name),
            "renderer leaked internal name '{internal_name}' in: {rendered}",
        );
    }
}

#[test]
fn incompatible_choice_comparison_renderer_hides_reason_debug_names() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let diagnostic = CompilerDiagnostic::incompatible_choice_comparison(
        IncompatibleChoiceComparisonReason::ChoiceWithNonChoice,
        builtin_type_ids::BOOL,
        builtin_type_ids::INT,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Cannot compare choice")),
        "{guidance:?}"
    );
    assert!(terse_line.contains("Cannot compare choice"), "{terse_line}");
    assert!(
        !guidance
            .iter()
            .any(|line| line.contains("ChoiceWithNonChoice"))
    );
    assert!(!terse_line.contains("ChoiceWithNonChoice"));
}

fn render_invalid_call_shape(
    string_table: &mut StringTable,
    reason: InvalidCallShapeReason,
    callee_name: &str,
) -> String {
    let source_path = InternedPath::from_single_str("main.moth", string_table);
    let callee = string_table.intern(callee_name);
    let diagnostic =
        CompilerDiagnostic::invalid_call_shape(reason, Some(callee), location(source_path));
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_payload_guidance(&diagnostic.payload, render_context)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[test]
fn mutable_access_required_renders_explicit_marker_guidance() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "consume",
    );

    assert!(
        message
            .contains("Call to 'consume' requires explicit mutable access for parameter 'value'."),
        "{message}"
    );
    assert!(
        message.contains("Prefix the existing mutable place with `~`"),
        "{message}"
    );
}

#[test]
fn immutable_place_mutable_access_renders_binding_name_for_missing_marker() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("values");
    let binding = string_table.intern("values");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        "consume",
    );

    assert!(
        message
            .contains("requires mutable access for parameter 'values', but `values` is immutable."),
        "{message}"
    );
    assert!(
        message.contains("Declare the binding as mutable, then pass `~values`."),
        "{message}"
    );
}

#[test]
fn immutable_place_mutable_access_renders_authored_marker_with_binding_name() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let binding = string_table.intern("x");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        "mutate",
    );

    assert!(
        message.contains("requires mutable access for parameter 'value', but `x` is immutable."),
        "{message}"
    );
    assert!(message.contains("then pass `~x`."), "{message}");
}

#[test]
fn immutable_place_mutable_access_uses_generic_fallback_without_binding_name() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let missing_marker = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: None,
        },
        "mutate",
    );
    let authored_marker = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: None,
        },
        "mutate",
    );

    let expected_fallback = "but this argument comes from an immutable binding or field.";
    assert!(
        missing_marker.contains(expected_fallback),
        "{missing_marker}"
    );
    assert!(
        authored_marker.contains(expected_fallback),
        "{authored_marker}"
    );

    // The fallback must not expose compiler-facing place terminology.
    for message in [missing_marker.as_str(), authored_marker.as_str()] {
        assert!(
            !message.contains("place"),
            "fallback must not say place: {message}"
        );
        assert!(
            !message.contains("non-place"),
            "fallback must not say non-place: {message}"
        );
        assert!(
            !message.contains("rvalue"),
            "fallback must not say rvalue: {message}"
        );
        assert!(
            !message.contains("variable"),
            "fallback must not say variable: {message}"
        );
    }
}

#[test]
fn mutable_access_on_non_place_renders_fresh_value_guidance() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnNonPlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "mutate",
    );

    assert!(
        message.contains(
            "Call to 'mutate' cannot use `~` on a fresh or computed value for parameter 'value'."
        ),
        "{message}"
    );
    assert!(
        message.contains("Remove `~` and pass the value directly."),
        "{message}"
    );
    // The fresh/computed branch must not expose compiler-facing terminology.
    assert!(
        !message.contains("place"),
        "non-place branch must not say place: {message}"
    );
    assert!(
        !message.contains("non-place"),
        "non-place branch must not say non-place: {message}"
    );
    assert!(
        !message.contains("rvalue"),
        "non-place branch must not say rvalue: {message}"
    );
    assert!(
        !message.contains("variable"),
        "non-place branch must not say variable: {message}"
    );
}

#[test]
fn unnamed_parameter_renders_one_based_position_without_internal_slot() {
    let mut string_table = StringTable::new();
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessRequired {
            parameter_name: None,
            parameter_index: 0,
        },
        "consume",
    );

    // The source-facing label is one-based: the first parameter is "parameter 1".
    assert!(message.contains("parameter 1"), "{message}");
    // The internal zero-based slot, parenthetical conversion and #N must never be rendered.
    assert!(
        !message.contains("parameter 0"),
        "must not render zero-based slot: {message}"
    );
    assert!(
        !message.contains("1-based"),
        "must not render conversion note: {message}"
    );
    assert!(!message.contains("#1"), "must not render #N: {message}");
}

#[test]
fn mutable_access_not_allowed_tells_author_to_remove_authored_marker() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessNotAllowed {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "consume",
    );

    assert!(
        message.contains("Call to 'consume' does not accept mutable access for parameter 'value'."),
        "{message}"
    );
    assert!(
        message.contains("Remove the authored `~` from this argument."),
        "{message}"
    );
    // The marker must be rendered with consistent backtick punctuation, not a bare tilde.
    assert!(
        !message.contains("(~)"),
        "must not render parenthetical marker: {message}"
    );
    assert!(
        !message.contains("Remove the ~ "),
        "must not render bare tilde: {message}"
    );
}

#[test]
fn invalid_call_shape_remap_updates_binding_name_and_parameter_name() {
    let mut local_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut local_table);
    let parameter = local_table.intern("values");
    let binding = local_table.intern("values");
    let callee = local_table.intern("consume");

    let diagnostic = CompilerDiagnostic::invalid_call_shape(
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        Some(callee),
        location(source_path),
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let DiagnosticPayload::InvalidCallShape {
        reason:
            InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
                parameter_name,
                binding_name,
                ..
            },
        callee_name,
    } = &bag.diagnostics()[0].payload
    else {
        panic!("expected remapped ImmutablePlaceMutableAccessRequired payload");
    };

    assert_eq!(merged_table.resolve(callee_name.unwrap()), "consume");
    assert_eq!(merged_table.resolve(parameter_name.unwrap()), "values");
    assert_eq!(merged_table.resolve(binding_name.unwrap()), "values");
}

fn render_invalid_receiver_call(
    string_table: &mut StringTable,
    reason: InvalidReceiverCallReason,
    method_name: &str,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<&str>,
) -> String {
    let source_path = InternedPath::from_single_str("main.moth", string_table);
    let method = string_table.intern(method_name);
    let binding = receiver_binding_name.map(|name| string_table.intern(name));
    let diagnostic = CompilerDiagnostic::invalid_receiver_call(
        reason,
        None,
        Some(method),
        receiver_kind,
        binding,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_payload_guidance(&diagnostic.payload, render_context)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[test]
fn source_method_missing_marker_renders_named_receiver_example() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );

    assert!(
        message.contains("Mutable receiver method `move` requires explicit mutable access."),
        "{message}"
    );
    assert!(
        message.contains("for example `~p.move(...)`"),
        "named receiver example must use the factual binding name: {message}"
    );
}

#[test]
fn source_method_missing_marker_omits_example_without_binding_name() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );

    assert!(
        message.contains("Prefix the receiver with `~`."),
        "{message}"
    );
    assert!(
        !message.contains("~this receiver"),
        "must not render an internal placeholder: {message}"
    );
    assert!(!message.contains("for example"), "{message}");
}

#[test]
fn source_method_immutable_receiver_names_binding_to_declare_mutable() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );

    assert!(
        message.contains("`p` is immutable. Declare `p` as mutable, then call it with `~`."),
        "{message}"
    );
    assert!(!message.contains("temporary"), "{message}");
}

#[test]
fn source_method_non_place_receiver_requires_mutable_place() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );

    assert!(
        message.contains("requires a mutable place receiver."),
        "{message}"
    );
    // A temporary must not be described as immutable or share an existing-binding repair.
    assert!(!message.contains("immutable"), "{message}");
    assert!(!message.contains("Declare"), "{message}");
}

#[test]
fn collection_missing_marker_names_kind_and_explicit_access() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        Some("values"),
    );

    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn collection_immutable_receiver_names_binding_to_declare_mutable() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        Some("values"),
    );

    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Declare `values` as mutable, then call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn collection_non_place_receiver_requires_mutable_binding() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        None,
    );

    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Bind this value in a mutable binding first, then call it with `~`."),
        "{message}"
    );
    assert!(!message.contains("immutable"), "{message}");
}

#[test]
fn map_immutable_receiver_names_kind_and_binding() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "set",
        Some(ReceiverCallKind::MapBuiltin),
        Some("scores"),
    );

    assert!(
        message.contains("`set` requires a mutable map receiver"),
        "{message}"
    );
    assert!(
        message.contains("Declare `scores` as mutable, then call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn authored_marker_on_immutable_receiver_keeps_marker_wording() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );

    assert!(
        message.contains("`~` accepts only an existing mutable place."),
        "{message}"
    );
    assert!(
        message.contains("Declare `p` as mutable before calling it with `~`."),
        "{message}"
    );
    assert!(message.contains("`p` is immutable"), "{message}");
}

#[test]
fn authored_marker_on_non_place_receiver_explains_temporary() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableMarkerOnNonPlaceReceiver,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );

    assert!(
        message.contains("`~` accepts only an existing mutable place."),
        "{message}"
    );
    assert!(
        message.contains("cannot be called on a temporary value."),
        "{message}"
    );
    assert!(!message.contains("immutable"), "{message}");
}

#[test]
fn unneeded_mutable_marker_tells_author_to_remove_it() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::UnneededMutableAccessMarker,
        "length",
        Some(ReceiverCallKind::CollectionBuiltin),
        None,
    );

    assert!(
        message.contains("`length` does not accept an explicit mutable access marker `~`."),
        "{message}"
    );
    assert!(
        message.contains("Remove the `~` from this call."),
        "{message}"
    );
}

#[test]
fn const_record_runtime_call_renders_current_source_term() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ConstRecordNoRuntimeCalls,
        "length",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );

    assert!(
        message.contains("Const records are data-only"),
        "must use the current `const record` source term: {message}"
    );
    assert!(
        !message.contains("const struct record"),
        "must not render the stale `const struct record` term: {message}"
    );
}

#[test]
fn invalid_receiver_call_remap_updates_receiver_binding_name() {
    let mut local_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut local_table);
    let method = local_table.intern("move");
    let binding = local_table.intern("p");

    let diagnostic = CompilerDiagnostic::invalid_receiver_call(
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        None,
        Some(method),
        Some(ReceiverCallKind::SourceMethod),
        Some(binding),
        location(source_path),
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let DiagnosticPayload::InvalidReceiverCall {
        method_name,
        receiver_binding_name,
        ..
    } = &bag.diagnostics()[0].payload
    else {
        panic!("expected remapped InvalidReceiverCall payload");
    };

    assert_eq!(merged_table.resolve(method_name.unwrap()), "move");
    assert_eq!(merged_table.resolve(receiver_binding_name.unwrap()), "p");
}

#[test]
fn type_mismatch_renderer_fallback_uses_stable_type_id_text() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::Declaration,
        location(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(guidance.iter().any(|line| line == "Expected: TypeId(1)"));
    assert!(guidance.iter().any(|line| line == "Found: TypeId(4)"));
    assert!(terse_line.contains("expected TypeId(1)"));
    assert!(terse_line.contains("found TypeId(4)"));
    assert!(
        !guidance
            .iter()
            .any(|line| line.contains("Expected type id"))
    );
    assert!(!guidance.iter().any(|line| line.contains("Found type id")));
}

#[test]
fn generic_instantiation_rendering_resolves_type_name() {
    use crate::compiler_frontend::compiler_messages::{
        CompilerDiagnostic, InvalidGenericInstantiationReason,
    };
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

    let mut string_table = StringTable::new();
    let box_name = string_table.intern("Box");
    let location = SourceLocation::default();

    let diagnostic = CompilerDiagnostic::invalid_generic_instantiation(
        Some(box_name),
        InvalidGenericInstantiationReason::WrongArgumentCount {
            expected: 1,
            found: 2,
        },
        location,
    );

    let render_context = DiagnosticRenderContext::new(&string_table);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(
        terse_line.contains("'Box'"),
        "Expected 'Box' in message, got: {terse_line}"
    );
}

#[test]
fn generic_conflict_rendering_resolves_concrete_type_names() {
    use crate::compiler_frontend::compiler_messages::{
        CompilerDiagnostic, InvalidGenericInstantiationReason,
    };
    use crate::compiler_frontend::datatypes::ids::GenericParameterId;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

    let mut string_table = StringTable::new();
    let function_name = string_table.intern("first");
    let parameter_name = string_table.intern("T");
    let location = SourceLocation::default();
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::invalid_generic_instantiation(
        Some(function_name),
        InvalidGenericInstantiationReason::ConflictingInference {
            subject: crate::compiler_frontend::compiler_messages::GenericInferenceSubject::Function,
            parameter_id: GenericParameterId(0),
            parameter_name,
            existing_type_id: type_environment.builtins().int,
            replacement_type_id: type_environment.builtins().string,
            current_evidence_location: location.clone(),
            previous_evidence_location: None,
        },
        location,
    );

    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment));
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);

    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Generic parameter 'T'")
                && line.contains("Int")
                && line.contains("String")),
        "expected rendered type names in guidance, got: {guidance:?}"
    );
}

#[test]
fn builtin_cast_shape_diagnostic_preserves_stable_code_and_rendering() {
    use crate::compiler_frontend::compiler_messages::InvalidBuiltinCallReason;

    let mut string_table = StringTable::new();
    let int_name = string_table.intern("Int");

    let diagnostic = CompilerDiagnostic::invalid_builtin_call(
        InvalidBuiltinCallReason::CastMissingArgument,
        Some(int_name),
        SourceLocation::default(),
    );

    let render_context = DiagnosticRenderContext::new(&string_table);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert_eq!(diagnostic.kind.descriptor().code, "MOTH-RULE-0046");
    assert!(
        terse_line.contains("'Int' cast requires exactly one argument"),
        "{terse_line}"
    );
}

#[test]
fn token_diagnostics_render_source_spelling_not_token_debug_names() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("item");
    let location = SourceLocation::default();

    let expected = CompilerDiagnostic::expected_token(
        TokenKind::OpenParenthesis,
        Some(TokenKind::Symbol(name)),
        location.clone(),
    );
    let unexpected = CompilerDiagnostic::unexpected_token(TokenKind::OpenCurly, location);

    let render_context = DiagnosticRenderContext::new(&string_table);
    let expected_terse = terse::format_terse_diagnostic_with_context(&expected, render_context);
    let unexpected_terminal =
        terminal::format_payload_guidance(&unexpected.payload, render_context).join("\n");

    assert!(expected_terse.contains("Expected `(`"), "{expected_terse}");
    assert!(expected_terse.contains("name `item`"), "{expected_terse}");
    assert!(
        unexpected_terminal.contains("Unexpected token `{`"),
        "{unexpected_terminal}"
    );
    assert!(
        !expected_terse.contains("OpenParenthesis") && !expected_terse.contains("Symbol"),
        "{expected_terse}"
    );
}

#[test]
fn borrow_conflict_rendering_hides_payload_debug_names() {
    let mut string_table = StringTable::new();
    let value_name = string_table.intern("value");
    let location = SourceLocation::default();

    let diagnostic = borrow_conflict_diagnostic(
        DiagnosticPlace::Local(value_name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        location,
    );

    let render_context = DiagnosticRenderContext::new(&string_table);
    let terminal_guidance =
        terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(
        terminal_guidance
            .contains("existing shared access conflicts with requested mutable access"),
        "{terminal_guidance}"
    );
    assert!(
        !terminal_guidance.contains("DiagnosticPlace") && !terminal_guidance.contains("Shared"),
        "{terminal_guidance}"
    );
    assert!(!terse_line.contains("BorrowConflict"), "{terse_line}");
}

#[test]
fn diagnostic_display_order_buckets_errors_before_warnings_before_notes() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);

    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        location(source_path.clone()),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("warning"),
            namespace: NameNamespace::Value,
        },
    );
    let error = unknown_name_diagnostic(
        string_table.intern("error"),
        NameNamespace::Value,
        location(source_path.clone()),
    );
    let note = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Note,
        location(source_path),
        DiagnosticPayload::UnknownName {
            name: string_table.intern("note"),
            namespace: NameNamespace::Value,
        },
    );

    let messages = CompilerMessages::from_diagnostics(vec![warning, error, note], string_table);

    assert_eq!(
        messages.diagnostic_display_order(),
        vec![1, 0, 2],
        "errors should render first, then warnings, then notes"
    );
}

#[test]
fn diagnostic_display_order_preserves_original_order_within_each_severity_bucket() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let mut make = |name: &str, severity| {
        CompilerDiagnostic::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            severity,
            location(source_path.clone()),
            DiagnosticPayload::UnknownName {
                name: string_table.intern(name),
                namespace: NameNamespace::Value,
            },
        )
    };

    let error_first = make("error_first", DiagnosticSeverity::Error);
    let warning_first = make("warning_first", DiagnosticSeverity::Warning);
    let error_second = make("error_second", DiagnosticSeverity::Error);
    let warning_second = make("warning_second", DiagnosticSeverity::Warning);

    let messages = CompilerMessages::from_diagnostics(
        vec![error_first, warning_first, error_second, warning_second],
        string_table,
    );

    assert_eq!(
        messages.diagnostic_display_order(),
        vec![0, 2, 1, 3],
        "original order within each severity bucket must be stable"
    );
}

#[test]
fn diagnostic_display_order_keeps_type_context_lookups_aligned_with_original_indexes() {
    let mut string_table = StringTable::new();
    let point_path = InternedPath::from_single_str("Point", &mut string_table);
    let status_path = InternedPath::from_single_str("Status", &mut string_table);

    let mut point_environment = TypeEnvironment::new();
    let (_, point_type) = point_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let mut status_environment = TypeEnvironment::new();
    let (_, status_type) = status_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: status_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    // Stored order is warning first, error second, but display order must put the error first.
    let mut warning = CompilerDiagnostic::type_mismatch(
        status_type,
        status_environment.builtins().string,
        TypeMismatchContext::Assignment,
        SourceLocation::default(),
    );
    warning.severity = DiagnosticSeverity::Warning;
    let error = CompilerDiagnostic::type_mismatch(
        point_type,
        point_environment.builtins().int,
        TypeMismatchContext::Assignment,
        SourceLocation::default(),
    );

    let warning_messages = CompilerMessages::from_diagnostics(vec![warning], string_table.clone())
        .with_type_context_for_all_diagnostics(status_environment);
    let error_messages = CompilerMessages::from_diagnostics(vec![error], string_table)
        .with_type_context_for_all_diagnostics(point_environment);

    let mut messages = warning_messages;
    messages.append_messages_preserving_context(error_messages);

    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        );

    assert_eq!(
        messages.diagnostic_display_order(),
        vec![1, 0],
        "display order should use original indexes, not stored order"
    );
    assert!(
        rendered[0].contains("Point") && rendered[0].contains("Int"),
        "first rendered line should come from the error's point/int type context, got: {}",
        rendered[0]
    );
    assert!(
        rendered[1].contains("Status") && rendered[1].contains("String"),
        "second rendered line should come from the warning's status/string type context, got: {}",
        rendered[1]
    );
}

#[test]
fn terse_renderer_outputs_errors_before_warnings_in_display_order() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let mut make = |name: &str, severity| {
        CompilerDiagnostic::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            severity,
            location(source_path.clone()),
            DiagnosticPayload::UnknownName {
                name: string_table.intern(name),
                namespace: NameNamespace::Value,
            },
        )
    };

    let warning = make("warning", DiagnosticSeverity::Warning);
    let error = make("error", DiagnosticSeverity::Error);
    let messages = CompilerMessages::from_diagnostics(vec![warning, error], string_table);

    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        );

    assert_eq!(rendered.len(), 2);
    assert!(
        rendered[0].contains("MOTH-RULE-0001") && rendered[0].contains("error"),
        "first terse line should be the error, got: {}",
        rendered[0]
    );
    assert!(
        rendered[1].contains("MOTH-RULE-0001") && rendered[1].contains("warning"),
        "second terse line should be the warning, got: {}",
        rendered[1]
    );
}

#[test]
fn dev_server_html_renderer_outputs_error_card_before_warning_card() {
    use crate::compiler_frontend::compiler_messages::render::dev_server;

    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let mut make = |name: &str, severity| {
        CompilerDiagnostic::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            severity,
            location(source_path.clone()),
            DiagnosticPayload::UnknownName {
                name: string_table.intern(name),
                namespace: NameNamespace::Value,
            },
        )
    };

    let warning = make("warning", DiagnosticSeverity::Warning);
    let error = make("error", DiagnosticSeverity::Error);
    let messages = CompilerMessages::from_diagnostics(vec![warning, error], string_table);

    let html = dev_server::render_compiler_messages_html(&messages, std::path::Path::new("/tmp"));

    let error_pos = html.find("Error").expect("error badge should be present");
    let warning_pos = html
        .find("Warning")
        .expect("warning badge should be present");
    assert!(
        error_pos < warning_pos,
        "error card should appear before warning card in dev-server HTML"
    );
}

#[test]
fn terse_descriptor_only_diagnostics_use_descriptor_title() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let render_context = DiagnosticRenderContext::new(&string_table);

    let diagnostics = [
        (
            CompilerDiagnostic::unterminated_string_literal(location(source_path.clone())),
            "MOTH-SYNTAX-0006",
            "Unterminated string literal",
        ),
        (
            CompilerDiagnostic::invalid_char_literal(location(source_path.clone())),
            "MOTH-SYNTAX-0009",
            "Invalid character literal",
        ),
        (
            CompilerDiagnostic::export_outside_module_root(location(source_path)),
            "MOTH-RULE-0077",
            "`export:` is only valid in a module root file",
        ),
    ];

    for (diagnostic, expected_code, expected_title) in diagnostics {
        let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);
        assert!(
            terse_line.contains(expected_code),
            "Expected terse line to contain {expected_code}, got: {terse_line}",
        );
        assert!(
            terse_line.contains(expected_title),
            "Expected terse line to contain descriptor title {expected_title}, got: {terse_line}",
        );
        let message_field = terse_line.rsplit('|').next().unwrap_or_default();
        assert!(
            !message_field.is_empty(),
            "Terse message field should be non-empty for {expected_code}, got: {terse_line}",
        );
    }
}

#[test]
fn terse_message_field_is_never_empty() {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let render_context = DiagnosticRenderContext::new(&string_table);

    for kind in DiagnosticKind::all() {
        let diagnostic =
            CompilerDiagnostic::new(kind, location(source_path.clone()), DiagnosticPayload::None);
        let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);
        let message_field = terse_line.rsplit('|').next().unwrap_or_default();
        assert!(
            !message_field.is_empty(),
            "Terse message field should be non-empty for {:?}, got: {terse_line}",
            kind,
        );
    }
}

/// The symbolic spacing diagnostic must render the exact construct, spelling and
/// missing side without calling assignment, compound assignment or mutable
/// declaration a binary operator.
#[test]
fn symbolic_spacing_renders_exact_construct_and_side() {
    let string_table = StringTable::new();
    let render_context = DiagnosticRenderContext::new(&string_table);

    let cases = [
        (
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Add,
            },
            MissingWhitespace::After,
            "Binary operator '+' requires whitespace after it.",
        ),
        (
            SymbolicSpacingConstruct::Assignment,
            MissingWhitespace::Before,
            "Assignment '=' requires whitespace before it.",
        ),
        (
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Add,
            },
            MissingWhitespace::Before,
            "Compound assignment '+=' requires whitespace before it.",
        ),
        (
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::IntDivide,
            },
            MissingWhitespace::Both,
            "Compound assignment '//=' requires whitespace on both sides.",
        ),
        (
            SymbolicSpacingConstruct::MutableDeclaration,
            MissingWhitespace::After,
            "Mutable declaration '~=' requires whitespace after it.",
        ),
    ];

    for (construct, missing, expected_message) in cases {
        let diagnostic = CompilerDiagnostic::common_syntax_mistake(
            crate::compiler_frontend::compiler_messages::CommonSyntaxMistakeReason::InvalidSymbolicSpacing {
                error: SymbolicSpacingError { construct, missing },
            },
            SourceLocation::new(
                InternedPath::from_single_str("test.moth", &mut StringTable::new()),
                CharPosition { line_number: 1, char_column: 1 },
                CharPosition { line_number: 1, char_column: 2 },
            ),
        );
        let rendered = terminal::format_payload_guidance(&diagnostic.payload, render_context);
        assert!(
            rendered.iter().any(|line| line.contains(expected_message)),
            "expected message '{expected_message}' in rendered output {rendered:?} for {construct:?} {missing:?}"
        );
    }
}
