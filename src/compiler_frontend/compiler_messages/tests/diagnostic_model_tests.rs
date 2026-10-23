use super::{
    BorrowAccessKind, BorrowDiagnosticKind, BuiltinFailureOriginKind, BuiltinFailureWitness,
    CapacityResource, CompileTimeEvaluationErrorReason, CompilerDiagnostic, ConfigDiagnosticKind,
    DeferredFeatureDiagnosticKind, DeferredFeatureReason, DependencyClauseKind, DiagnosticBag,
    DiagnosticCategory, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, DiagnosticSeverity, DiagnosticToken,
    GenericApplicationErrorReason, HirCapacityResource, ImportDiagnosticKind,
    ImportPublicSurfaceType, IncompatibleChoiceComparisonReason, InfrastructureDiagnosticKind,
    InvalidAssignmentTargetReason, InvalidCallShapeReason, InvalidCastReason,
    InvalidChoiceVariantReason, InvalidCollectionTypeReason, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidExpressionReason, InvalidFallibleHandlingReason,
    InvalidFunctionSignatureReason, InvalidGenericParameterReason, InvalidImportPathReason,
    InvalidMapTypeReason, InvalidOutputFolderReason, InvalidReceiverCallReason,
    InvalidSignatureMemberReason, InvalidStandaloneStatementReason, InvalidStatementPositionReason,
    InvalidStringEscapeReason, InvalidTemplateDirectiveReason, InvalidTemplateStructureReason,
    InvalidTraitConformanceReason, InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason,
    NameNamespace, NamespaceTypeValueMisuseKind, PathKind, ReceiverCallKind, RuleDiagnosticKind,
    SyntaxDiagnosticKind, TokenTag, TypeAnnotationContext, TypeDiagnosticKind, TypeMismatchContext,
    UnsupportedBackendFeatureReason, ValueReceiverContext, is_well_formed_reason_key,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data::{InvalidCallShapeView, InvalidReceiverCallView},
    choices_control_flow::CompileTimeEvaluationErrorView,
    generics_traits_api::{
        InvalidReceiverDeclarationView, InvalidTraitConformanceShape, InvalidTraitConformanceView,
    },
    imports::NotExportedByPublicSurfaceView,
    names_declarations::DuplicateDeclarationView,
    project_config::LegacyDependencyClauseView,
    token_lexical::ExpectedTokenView,
    types_borrows_targets::{SharedMutableConflictView, TypeMismatchView},
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server, invalid_config_message, terminal, terse,
};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, builtin_type_ids};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

fn span<T>(_path: T) -> Option<SourceSpan> {
    None
}
fn exact_span(
    source: SourceId,
    start: u32,
    length: u32,
    builder: &mut ExtendedSpanBuilder,
) -> SourceSpan {
    SourceSpan::new(
        source,
        LocalSpan::exact(start, length, builder).expect("test span should fit"),
    )
}

fn unknown_name_diagnostic(
    name: StringId,
    namespace: NameNamespace,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        span,
        DiagnosticPayload::UnknownName { name, namespace },
    )
}

fn capture_rebased_batch(
    diagnostics: Vec<CompilerDiagnostic>,
    local_table: StringTable,
    root_table: &mut StringTable,
    environment: Option<&TypeEnvironment>,
    path_table_is_available: bool,
    path_table: Option<Arc<PathTable>>,
) -> PreparedDiagnosticBatch {
    let remap = root_table.merge_from(&local_table);
    assert!(
        !remap.is_identity(),
        "the fixture must exercise a shifting string-table remap"
    );
    let mut prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        local_table,
        environment,
        path_table_is_available,
        path_table,
    );
    prepared.rebase_strings(&remap);
    prepared
}

fn finish_prepared_batch(
    prepared: PreparedDiagnosticBatch,
    string_table: StringTable,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(CompilerMessages::empty(string_table));
    accumulator.append_prepared(prepared, None);
    accumulator.finish(InstallMode::Frozen)
}

#[test]
fn descriptor_codes_are_stable_and_non_empty() {
    let cases = [
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
            "MOTH-SYNTAX-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            "MOTH-SYNTAX-0002",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTrailingComma),
            "MOTH-SYNTAX-0003",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidDependencyClause),
            "MOTH-SYNTAX-0019",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::CompilerCapacityExceeded),
            "MOTH-SYNTAX-0037",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            "MOTH-RULE-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::IdentifierNamingConvention),
            "MOTH-RULE-0021",
            DiagnosticSeverity::Warning,
        ),
        (
            DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
            "MOTH-TYPE-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            "MOTH-IMPORT-0005",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Borrow(BorrowDiagnosticKind::BorrowConflict),
            "MOTH-BORROW-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Config(ConfigDiagnosticKind::InvalidConfig),
            "MOTH-CONFIG-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure),
            "MOTH-INFRA-0001",
            DiagnosticSeverity::Error,
        ),
        (
            DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::DeferredFeature),
            "MOTH-DEFERRED-0001",
            DiagnosticSeverity::Error,
        ),
    ];

    for (kind, expected_code, expected_severity) in cases {
        let descriptor = kind.descriptor();
        assert_eq!(descriptor.code, expected_code);
        assert_eq!(descriptor.default_severity, expected_severity);
        assert!(!descriptor.title.is_empty());
        assert!(!kind.code().is_empty());
    }
}

#[test]
fn compiler_capacity_diagnostic_keeps_the_triggering_span_and_resource() {
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let primary_span = Some(exact_span(source, 3, 4, &mut span_builder));
    let diagnostic = CompilerDiagnostic::compiler_capacity_exceeded(
        HirCapacityResource::ExpressionRows,
        primary_span,
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::CompilerCapacityExceeded)
    );
    assert_eq!(diagnostic.primary_span, primary_span);
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::CompilerCapacityExceeded {
            resource: CapacityResource::Hir(HirCapacityResource::ExpressionRows),
        }
    );

    let string_table = StringTable::new();
    let terse = terse::format_terse_diagnostic_with_context(
        &diagnostic,
        DiagnosticRenderContext::new(&string_table),
    );
    assert!(terse.contains("MOTH-SYNTAX-0037"));
    assert!(terse.contains("HIR expression rows"));
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::with_severity(
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCollectionType),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern("Wasm"),
        UnsupportedBackendFeatureReason::HashmapOperation,
        span(source_path),
    );

    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0064");
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("unsupported_backend_feature.hashmap_operation")
    );
}

#[test]
fn body_declaration_placement_preserves_source_name_and_stable_reason() {
    use super::InvalidDeclarationReason;

    let mut string_table = StringTable::new();
    let name = string_table.intern("nested");
    let mut span_builder = ExtendedSpanBuilder::new();
    let name_span = Some(exact_span(
        SourceId::from_index(1),
        12,
        6,
        &mut span_builder,
    ));
    let diagnostic = CompilerDiagnostic::invalid_declaration(
        InvalidDeclarationReason::DeclarationRequiresModuleTopLevel,
        Some(name),
        name_span,
    );

    assert_eq!(diagnostic.primary_span, name_span);
    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0043");
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("invalid_declaration.declaration_requires_module_top_level"),
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidDeclaration {
            reason: InvalidDeclarationReason::DeclarationRequiresModuleTopLevel,
            name: Some(source_name),
        } if *source_name == name
    ));
    let guidance = terminal::format_diagnostic_guidance(
        &diagnostic,
        DiagnosticRenderContext::new(&string_table),
    )
    .join("\n");
    assert!(guidance.contains("Declaration 'nested' cannot appear inside an executable body"));
    assert!(guidance.contains("Declare it at module top level"));
}

#[test]
fn mutable_function_parameter_feature_exposes_stable_reason_key() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern("Wasm"),
        UnsupportedBackendFeatureReason::MutableFunctionParameters,
        span(source_path),
    );

    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0064");
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("unsupported_backend_feature.mutable_function_parameters")
    );
}

#[test]
fn non_utf8_output_folder_reason_has_stable_identity_and_rendering() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("config.moth", &mut string_table)
        .expect("test path fits");
    let reason = InvalidConfigReason::InvalidOutputFolder {
        folder: None,
        reason: InvalidOutputFolderReason::NonUtf8,
    };
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Config(ConfigDiagnosticKind::InvalidConfig),
        span(source_path),
        DiagnosticPayload::InvalidConfig {
            key: None,
            reason: reason.clone(),
        },
    );

    assert_eq!(
        diagnostic.identity().reason_key,
        Some("invalid_config.invalid_output_folder.non_utf8")
    );
    assert_eq!(
        invalid_config_message(None, &reason, &string_table),
        "'config' must use valid UTF-8 portable path components."
    );
}

#[test]
fn output_folder_collision_rendering_preserves_both_authored_spellings() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let reason = InvalidConfigReason::OutputFoldersNotDistinct {
        dev_folder: string_table.intern("Build"),
        release_folder: string_table.intern("build"),
    };

    assert_eq!(
        invalid_config_message(
            Some(string_table.intern("dev_folder")),
            &reason,
            &string_table
        ),
        "Development and release output folders 'Build' and 'build' resolve to the same output root and must be distinct."
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        span(source_path),
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let span = span(source_path);
    let diagnostics = [
        CompilerDiagnostic::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::NoneNotAllowed,
            span,
        ),
        CompilerDiagnostic::invalid_map_type(InvalidMapTypeReason::FixedCapacityNotAllowed, span),
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityOverflow,
            span,
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let first = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        span(source_path),
    );
    let second = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let error = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        span(source_path),
    );
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    assert!(messages.infrastructure_errors().is_none());
}

#[test]
fn compiler_messages_with_warnings_keep_typed_diagnostics_off_error_mirrors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let error = unknown_name_diagnostic(
        string_table.intern("missing"),
        NameNamespace::Value,
        span(source_path),
    );
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    assert!(messages.infrastructure_errors().is_none());
}

#[test]
fn compiler_messages_with_infrastructure_error_preserve_warning_production_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let warning = CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        DiagnosticSeverity::Warning,
        span(source_path),
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
    assert_eq!(messages.diagnostics.to_vec(), vec![warning]);
    let error = messages
        .primary_infrastructure_error()
        .expect("outer infrastructure failure must be preserved");
    assert_eq!(error.msg.as_str(), "backend failed after warnings");
}

fn assert_infrastructure_causes(messages: &CompilerMessages, expected: &[CompilerError]) {
    let causes = messages
        .infrastructure_errors()
        .expect("infrastructure causes should remain")
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(causes.len(), expected.len());
    for (cause, expected) in causes.into_iter().zip(expected) {
        assert_eq!(cause.error_type, expected.error_type);
        assert_eq!(cause.msg, expected.msg);
        assert_eq!(cause.host_path, expected.host_path);
        assert_eq!(cause.source_span, expected.source_span);
        assert_eq!(cause.metadata, expected.metadata);
    }
    assert_eq!(
        messages.error_count(),
        1,
        "causes describe one aborted compilation"
    );
}

#[test]
fn infrastructure_setter_and_message_append_preserve_every_ordered_cause() {
    for (first_type, second_type) in [
        (ErrorType::Compiler, ErrorType::File),
        (ErrorType::File, ErrorType::Compiler),
        (ErrorType::File, ErrorType::File),
    ] {
        let mut span_builder = ExtendedSpanBuilder::new();
        let mut make_error = |index, error_type| {
            let mut error = CompilerError::new(
                format!("cause {index}"),
                Some(exact_span(
                    SourceId::from_index(1),
                    index,
                    1,
                    &mut span_builder,
                )),
                error_type,
            );
            error.host_path = Some(format!("cause_{index}.moth").into());
            error.new_metadata_entry(
                CompilerErrorMetadataKey::CompilationStage,
                format!("stage {index}"),
            );
            error
        };
        let expected = [
            make_error(0, first_type.clone()),
            make_error(1, second_type.clone()),
            make_error(2, second_type),
            make_error(3, first_type),
        ];
        let mut messages = CompilerMessages::empty(StringTable::new());
        messages.set_infrastructure_error(expected[0].clone());
        messages.set_infrastructure_error(expected[1].clone());
        assert_infrastructure_causes(&messages, &expected[..2]);

        let mut incoming = CompilerMessages::from_error(expected[2].clone(), StringTable::new());
        incoming.set_infrastructure_error(expected[3].clone());
        messages.append_messages_preserving_context(incoming);
        assert_infrastructure_causes(&messages, &expected);
    }
}

#[test]
fn capture_with_required_paths_rejects_foreign_domain_table_behind_rendered_path_payload() {
    // The candidate table was issued by an independent domain whose component IDs exceed the
    // batch's next ID, so the rendered leaf's walk resolves nowhere and pairing would silently
    // render wrong spellings. The foreign table interns padding strings to avoid ID collisions.
    let mut batch_table = StringTable::new();
    batch_table.intern("batch");
    batch_table.intern("second");
    let mut batch_fork = PathInternerFork::empty();
    let batch_leaf = batch_fork
        .try_intern_portable_path("batch/import", &mut batch_table)
        .expect("batch path fits");
    let diagnostic = CompilerDiagnostic::missing_import_target(batch_leaf, None);

    let mut foreign_strings = StringTable::new();
    foreign_strings.intern("foreign-pad-a");
    foreign_strings.intern("foreign-pad-b");
    foreign_strings.intern("foreign-pad-c");
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());

    let error = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(diagnostic),
        batch_table,
        None,
        || Some(foreign_table),
    )
    .expect_err("a foreign table behind a rendered path must be rejected");
    assert!(
        error.msg.contains("path node"),
        "pairing should identify the unresolved path node: {}",
        error.msg
    );
}

#[test]
fn capture_with_required_paths_rejects_rendered_path_beyond_candidate_table() {
    // A retained diagnostic whose PathId points past the candidate table has lost its issuing
    // table: rendering would dereference a node the table cannot address. Pairing must fail
    // visibly instead of silently attaching a context that cannot spell the payload's path.
    let batch_table = StringTable::new();
    let orphan_path = PathId::try_from_index(9).expect("index 9 fits the compact path domain");
    let diagnostic = CompilerDiagnostic::missing_import_target(orphan_path, None);

    let mut local_strings = StringTable::new();
    let mut local_fork = PathInternerFork::empty();
    local_fork
        .try_intern_portable_path("local/import", &mut local_strings)
        .expect("local path fits");
    let local_table = Arc::new(local_fork.snapshot_table());

    assert!(
        PreparedDiagnosticBatch::capture_with_required_paths(
            DiagnosticBag::from(diagnostic),
            batch_table,
            None,
            || Some(local_table),
        )
        .is_err(),
        "a retained path outside the candidate table must fail pairing"
    );
}

#[test]
fn capture_with_required_paths_skips_foreign_table_without_path_payloads() {
    // A path-free diagnostic must not retain an unrelated candidate table; its string identity
    // must still survive the boundary's non-identity merge.
    let mut batch_table = StringTable::new();
    let name = batch_table.intern("local");
    let diagnostic = CompilerDiagnostic::unknown_value_name(name, None);

    let mut foreign_strings = StringTable::new();
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());
    let mut requested_path_table = false;
    let prepared = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(diagnostic),
        batch_table,
        None,
        || {
            requested_path_table = true;
            Some(foreign_table)
        },
    )
    .expect("a path-free diagnostic does not need a candidate table");
    assert!(
        !requested_path_table,
        "path-free diagnostics must not request or retain a caller-supplied path table"
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(CompilerMessages::empty(aggregate_table));
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Frozen);
    let rendered = terse::format_terse_boundary_messages(&failure);
    assert!(
        rendered.iter().any(|line| line.contains("local")),
        "path-free rendering lost its authored name after aggregation: {rendered:?}"
    );
}

#[test]
fn capture_with_required_paths_rejects_unresolvable_unused_path_table_node() {
    // The required path is valid in the batch string domain, but the retained table also carries
    // an unused node whose component ID is outside that domain. The complete table must validate
    // before it can be retained and remapped.
    let mut batch_table = StringTable::new();
    batch_table.intern("local");
    batch_table.intern("leaf");
    let mut batch_fork = PathInternerFork::empty();
    let required_path = batch_fork
        .try_intern_portable_path("local/leaf", &mut batch_table)
        .expect("required path fits");
    let diagnostic = CompilerDiagnostic::missing_import_target(required_path, None);

    let mut candidate_strings = StringTable::new();
    candidate_strings.intern("local");
    candidate_strings.intern("leaf");
    candidate_strings.intern("unused");
    candidate_strings.intern("extra");
    let mut candidate_fork = PathInternerFork::empty();
    let candidate_path = candidate_fork
        .try_intern_portable_path("local/leaf", &mut candidate_strings)
        .expect("candidate required path fits");
    candidate_fork
        .try_intern_portable_path("unused/extra", &mut candidate_strings)
        .expect("candidate unused path fits");
    assert_eq!(
        candidate_path, required_path,
        "the valid required path must exercise the same node identity"
    );

    let error = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(diagnostic),
        batch_table,
        None,
        || Some(Arc::new(candidate_fork.snapshot_table())),
    )
    .expect_err("an unresolvable unused node must reject a retained table");
    assert!(
        error.msg.contains("path node"),
        "the producer boundary should identify the invalid retained node: {}",
        error.msg
    );
}

#[test]
fn not_exported_by_public_surface_remaps_path_and_surface_name_after_prepared_capture() {
    // Both values must survive a non-identity aggregation and frozen boundary conversion.
    let mut local_table = StringTable::new();
    let surface_name = local_table.intern("surface-name");
    let mut local_fork = PathInternerFork::empty();
    let requested_path = local_fork
        .try_intern_portable_path("module/root", &mut local_table)
        .expect("requested path fits");
    let diagnostic = CompilerDiagnostic::not_exported_by_public_surface(
        requested_path,
        surface_name,
        ImportPublicSurfaceType::ModuleRoot,
        None,
    );
    let prepared = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(diagnostic),
        local_table,
        None,
        || Some(Arc::new(local_fork.snapshot_table())),
    )
    .expect("requested path table must pair with the local string table");

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("collision-padding");
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(CompilerMessages::empty(aggregate_table));
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Frozen);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let frozen = view
        .family_view::<NotExportedByPublicSurfaceView>()
        .expect("the frozen diagnostic must retain public-surface facts");
    assert_eq!(frozen.requested_path, requested_path);
    assert_eq!(
        frozen.public_surface_type,
        ImportPublicSurfaceType::ModuleRoot
    );
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(frozen.public_surface_name),
        "surface-name",
        "the public-surface name must follow the non-identity string merge"
    );
    let rendered = terse::format_terse_boundary_messages(&failure).join("\n");
    assert!(
        rendered.contains("Cannot bind 'module/root' from module 'surface-name'"),
        "public-surface path/name remapping rendered incorrectly: {rendered}"
    );
}

#[test]
fn capture_from_bag_preserves_nominal_type_rendering_without_path_payloads() {
    // TypeMismatch stores TypeIds rather than PathIds, but its nominal type name is rendered by
    // the captured type display using the paired path table.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let nominal_path = path_fork
        .try_intern_portable_path("NominalType", &mut string_table)
        .expect("nominal path fits");
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
        DiagnosticBag::from(diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(CompilerMessages::empty(aggregate_table));
    accumulator.append_prepared(prepared, None);
    let failure = accumulator.finish(InstallMode::Frozen);
    let (_, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let mismatch = view
        .family_view::<TypeMismatchView>()
        .expect("the frozen diagnostic must retain type-mismatch facts");
    assert_eq!(mismatch.context, TypeMismatchContext::Assignment);
    assert_eq!(
        view.types().shape(mismatch.expected),
        TypeDisplayShape::Struct {
            path: nominal_path,
            const_record: false,
        }
    );
    assert_eq!(
        view.types().shape(mismatch.found),
        TypeDisplayShape::FixedScalar(FixedScalar::I32)
    );
    let rendered = terse::format_terse_boundary_messages(&failure);
    assert!(
        rendered
            .iter()
            .any(|line| line
                .contains("Type mismatch in assignment: expected NominalType, found I32")),
        "frozen type rendering lost its nominal context after a non-identity table remap: {rendered:?}"
    );
}

#[test]
fn capture_with_required_paths_rejects_unresolved_rendered_ancestor() {
    // The leaf component resolves in the batch string table, but its rendered ancestor does not.
    // Pairing must walk the entire referenced chain rather than validating only the leaf.
    let mut batch_table = StringTable::new();
    batch_table.intern("batch-padding");
    batch_table.intern("leaf");

    let mut foreign_strings = StringTable::new();
    foreign_strings.intern("foreign-padding");
    foreign_strings.intern("leaf");
    foreign_strings.intern("ancestor");
    let mut foreign_fork = PathInternerFork::empty();
    let rendered_leaf = foreign_fork
        .try_intern_portable_path("ancestor/leaf", &mut foreign_strings)
        .expect("foreign path fits");
    foreign_fork
        .try_intern_portable_path("ancestor/leaf/unreferenced", &mut foreign_strings)
        .expect("unreferenced foreign path fits");
    let diagnostic = CompilerDiagnostic::missing_import_target(rendered_leaf, None);

    assert!(
        PreparedDiagnosticBatch::capture_with_required_paths(
            DiagnosticBag::from(diagnostic),
            batch_table,
            None,
            || Some(Arc::new(foreign_fork.snapshot_table())),
        )
        .is_err(),
        "an unresolved rendered ancestor must fail pairing"
    );
}

#[test]
fn prepared_rebase_updates_payloads_labels_and_tokens() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let import_path = path_fork
        .try_intern_portable_path("lib.moth", &mut local_table)
        .expect("test path fits");
    let rendered_import = path_fork.render_portable(import_path, &local_table, &mut Vec::new());
    let name = local_table.intern("Button");
    let alias = local_table.intern("AliasButton");
    let label_text = local_table.intern("temporary label");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let primary_span = exact_span(source, 4, 3, &mut span_builder);
    let first_span = exact_span(source, 10, 2, &mut span_builder);

    let expected_token = CompilerDiagnostic::expected_token_from_projections(
        DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name),
        Some(DiagnosticToken::from_static_tag(TokenTag::PATH)),
        Some(primary_span),
    );
    let duplicate =
        CompilerDiagnostic::duplicate_declaration(name, Some(first_span), Some(primary_span));
    let conformance = CompilerDiagnostic::invalid_trait_conformance(
        alias,
        None,
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(primary_span),
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(first_span),
        Some(DiagnosticLabelMessage::RenderedText(label_text)),
    )]);
    let borrow = CompilerDiagnostic::shared_mutable_conflict(
        DiagnosticPlace::Local(name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        None,
        Some(first_span),
        Some(primary_span),
    );

    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(
        vec![expected_token, duplicate, conformance, borrow],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let mut diagnostics = failure.reports.iter();

    let (token_report, token_view) = diagnostics.next().expect("expected-token record");
    let token = token_view
        .family_view::<ExpectedTokenView>()
        .expect("the first frozen record must be an expected-token diagnostic");
    assert_eq!(token.expected.tag(), TokenTag::SYMBOL);
    assert_eq!(
        token_report
            .context()
            .strings()
            .resolve(StringId::from_index(token.expected.data())),
        "Button"
    );
    let found = token.found.expect("the test token has a found-token fact");
    assert_eq!(found.tag(), TokenTag::PATH);
    assert_eq!(found.data(), 0);
    assert_eq!(rendered_import, "lib.moth");
    assert!(matches!(
        token_view.primary_site(),
        SiteRef::Local(span) if span == primary_span
    ));

    let (duplicate_report, duplicate_view) =
        diagnostics.next().expect("duplicate-declaration record");
    let duplicate = duplicate_view
        .family_view::<DuplicateDeclarationView>()
        .expect("the second frozen record must be a duplicate declaration");
    assert_eq!(
        duplicate_report.context().strings().resolve(duplicate.name),
        "Button"
    );
    let previous = duplicate_view
        .durable_labels()
        .next()
        .expect("duplicate declaration should retain its previous-location label");
    assert!(matches!(
        previous.message,
        DurableLabelMessage::PreviousDeclaration
    ));
    assert!(matches!(
        previous.site,
        SiteRef::Local(span) if span == first_span
    ));
    assert!(matches!(
        duplicate_view.primary_site(),
        SiteRef::Local(span) if span == primary_span
    ));

    let (conformance_report, conformance_view) = diagnostics
        .next()
        .expect("invalid-trait-conformance record");
    let conformance = conformance_view
        .family_view::<InvalidTraitConformanceView>()
        .expect("the third frozen record must be an invalid trait conformance");
    assert!(matches!(
        conformance.shape,
        InvalidTraitConformanceShape::ImportedModuleRoot
    ));
    let target_name = conformance_view
        .string_ids()
        .first()
        .expect("trait conformance should retain its target name");
    assert_eq!(
        conformance_report.context().strings().resolve(*target_name),
        "AliasButton"
    );
    let label = conformance_view
        .durable_labels()
        .next()
        .expect("trait conformance should retain its authored label");
    match label.message {
        DurableLabelMessage::RenderedText(text) => {
            assert_eq!(
                conformance_report.context().strings().resolve(text),
                "temporary label"
            );
        }
        message => panic!("unexpected frozen trait-conformance label: {message:?}"),
    }
    assert!(matches!(
        label.site,
        SiteRef::Local(span) if span == first_span
    ));
    assert!(matches!(
        conformance_view.primary_site(),
        SiteRef::Local(span) if span == primary_span
    ));

    let (borrow_report, borrow_view) = diagnostics.next().expect("borrow-conflict record");
    let borrow = borrow_view
        .family_view::<SharedMutableConflictView>()
        .expect("the fourth frozen record must be a shared/mutable conflict");
    match borrow.place.diagnostic_place() {
        DiagnosticPlace::Local(name) => {
            assert_eq!(borrow_report.context().strings().resolve(name), "Button");
        }
        place => panic!("unexpected frozen borrow place: {place:?}"),
    }
    assert_eq!(borrow.existing_access, BorrowAccessKind::Shared);
    assert_eq!(borrow.requested_access, BorrowAccessKind::Mutable);
    let conflict = borrow_view
        .durable_labels()
        .next()
        .expect("shared/mutable conflict should retain its access label");
    assert!(matches!(
        conflict.message,
        DurableLabelMessage::ConflictingAccess
    ));
    assert!(matches!(
        conflict.site,
        SiteRef::Local(span) if span == first_span
    ));
    assert!(matches!(
        borrow_view.primary_site(),
        SiteRef::Local(span) if span == primary_span
    ));
    assert!(
        diagnostics.next().is_none(),
        "expected exactly four frozen records"
    );
}

#[test]
fn prepared_rebase_updates_legacy_dependency_replacement() {
    let mut local_table = StringTable::new();
    let replacement = local_table.intern("@vendor/drawing.js as drawing");
    let diagnostic = CompilerDiagnostic::legacy_dependency_clause(Some(replacement), None);

    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let dependency = view
        .family_view::<LegacyDependencyClauseView>()
        .expect("the frozen diagnostic must retain legacy dependency facts");
    let replacement = dependency
        .replacement
        .expect("legacy dependency migration should retain its replacement");
    assert_eq!(
        report.context().strings().resolve(replacement),
        "@vendor/drawing.js as drawing",
        "migration replacement StringId must remain valid after table rebase"
    );
}

#[test]
fn prepared_rebase_updates_compile_time_evaluation_operation() {
    let mut local_table = StringTable::new();
    let operation = local_table.intern("string equality comparison");
    let diagnostic = CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        Some(operation),
        None,
    );

    let mut merged_table = StringTable::new();
    merged_table.intern("a string owned by another module");
    merged_table.intern("a second string owned by another module");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let evaluation = view
        .family_view::<CompileTimeEvaluationErrorView>()
        .expect("the frozen diagnostic must retain compile-time evaluation facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(evaluation.operation.expect("operation name")),
        "string equality comparison",
        "the operation name must survive a shifting table rebase"
    );
}

#[test]
fn prepared_rebase_updates_receiver_declaration_function_name_and_type() {
    use crate::compiler_frontend::compiler_messages::InvalidReceiverDeclarationReason;

    let mut local_table = StringTable::new();
    let function_name = local_table.intern("describe");
    let type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let diagnostic = CompilerDiagnostic::invalid_receiver_declaration(
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name,
            type_id,
        },
        None,
    );
    let type_environment = TypeEnvironment::new();
    let path_table = Arc::new(PathInternerFork::empty().snapshot_table());
    let mut merged_table = StringTable::new();
    merged_table.intern("a string owned by another module");
    merged_table.intern("a second string owned by another module");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        Some(&type_environment),
        true,
        Some(path_table),
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let receiver = view
        .family_view::<InvalidReceiverDeclarationView>()
        .expect("the frozen diagnostic must retain receiver-declaration facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(receiver.function_name.expect("function name")),
        "describe"
    );
    assert_eq!(
        view.types().shape(receiver.type_id.expect("receiver type")),
        TypeDisplayShape::FixedScalar(FixedScalar::I32),
        "string rebasing must not alter the receiver's semantic type"
    );
}

#[test]
fn duplicate_declaration_with_previous_location_keeps_secondary_label() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let declaration_name = string_table.intern("Button");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let previous_location = Some(exact_span(source, 0, 6, &mut span_builder));
    let duplicate_location = Some(exact_span(source, 10, 6, &mut span_builder));

    let diagnostic = CompilerDiagnostic::duplicate_declaration(
        declaration_name,
        previous_location,
        duplicate_location,
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    );
    match &diagnostic.payload {
        DiagnosticPayload::DuplicateDeclaration { name } => {
            assert_eq!(string_table.resolve(*name), "Button");
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
    assert_eq!(diagnostic.primary_span, duplicate_location);
    assert_eq!(
        diagnostic.labels.len(),
        1,
        "only the secondary label is kept"
    );
    assert_eq!(diagnostic.labels[0].span, previous_location);
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
    let _path_fork = PathInternerFork::empty();
    let declaration_name = string_table.intern("print");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let duplicate_location = Some(exact_span(source, 0, 5, &mut span_builder));

    // Prelude-injected symbols have no authored previous span.
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(declaration_name, None, duplicate_location);

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration)
    );
    match &diagnostic.payload {
        DiagnosticPayload::DuplicateDeclaration { name } => {
            assert_eq!(string_table.resolve(*name), "print");
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
    assert_eq!(diagnostic.primary_span, duplicate_location);
    assert!(
        diagnostic.labels.is_empty(),
        "no secondary label for prelude symbols"
    );
}

#[test]
fn value_receiver_diagnostic_preserves_factual_arity_and_producer_span() {
    let mut spans = ExtendedSpanBuilder::new();
    let producer_span = Some(exact_span(SourceId::from_index(1), 12, 6, &mut spans));
    let local_table = StringTable::new();
    let mut merged_table = StringTable::new();
    merged_table.intern("unrelated");
    let remap = merged_table.merge_from(&local_table);

    for produced_count in [0, 2] {
        let mut diagnostic = CompilerDiagnostic::invalid_value_receiver(
            ValueReceiverContext::Declaration,
            1,
            produced_count,
            producer_span,
        );
        diagnostic.payload.remap_string_ids(&remap);
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidValueReceiver)
        );
        assert_eq!(diagnostic.identity().code, "MOTH-RULE-0087");
        assert_eq!(diagnostic.identity().reason_key, None);
        assert_eq!(diagnostic.primary_span, producer_span);
        assert_eq!(
            diagnostic.payload,
            DiagnosticPayload::InvalidValueReceiver {
                receiver: ValueReceiverContext::Declaration,
                target_count: 1,
                produced_count,
            }
        );
        let context = DiagnosticRenderContext::new(&merged_table);
        let rendered = terse::format_terse_diagnostic_with_context(&diagnostic, context);
        assert!(rendered.contains(&format!(
            "Invalid declaration receiver: expected 1 success slot(s), but this expression produces {produced_count}."
        )));
        let guidance = terminal::format_diagnostic_guidance(&diagnostic, context).join("\n");
        if produced_count == 0 {
            assert!(
                guidance.contains("Use an expression that produces the required success values.")
            );
        } else {
            assert!(guidance.contains("Use a multi-bind with one target per success slot."));
        }
    }
}

#[test]
fn type_mismatch_constructor_carries_type_ids_without_rendering() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        builtin_type_ids::STRING,
        TypeMismatchContext::Declaration,
        span(source_path),
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch)
    );

    match diagnostic.payload {
        DiagnosticPayload::TypeMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, builtin_type_ids::fixed_scalar(FixedScalar::I32));
            assert_eq!(found, builtin_type_ids::STRING);
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
}

#[test]
fn type_mismatch_terminal_guidance_renders_captured_type_names() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_environment.builtins().string,
        TypeMismatchContext::Declaration,
        span(source_path),
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let (report, view) = failure.reports.iter().next().expect("captured mismatch");
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::for_report(report));

    assert!(guidance.iter().any(|line| line == "Expected: I32"));
    assert!(guidance.iter().any(|line| line == "Found: String"));
    assert!(!guidance.iter().any(|line| line.contains("type id")));
}

#[test]
fn type_mismatch_terse_renderer_renders_captured_type_names() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_environment.builtins().string,
        TypeMismatchContext::FunctionArgument,
        span(source_path),
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let line = terse::format_terse_boundary_messages(&failure)
        .into_iter()
        .next()
        .expect("captured mismatch");

    assert!(line.contains("expected I32"));
    assert!(line.contains("found String"));
    assert!(!line.contains("Expected type id"));
    assert!(!line.contains("Found type id"));
}

#[test]
fn invalid_string_escape_renderer_preserves_the_authored_escape_spelling() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (escaped, expected) in [
        ('q', "Unsupported string escape '\\q'."),
        ('\t', "Unsupported string escape '\\\\t'."),
    ] {
        let diagnostic = CompilerDiagnostic::invalid_string_escape(
            InvalidStringEscapeReason::UnsupportedEscape { escaped },
            span(source_path),
        );
        let message = terminal::format_diagnostic_guidance(&diagnostic, render_context).join("\n");

        assert!(message.contains(expected), "unexpected message: {message}");
    }
}

#[test]
fn invalid_string_escape_renderer_distinguishes_physical_newlines_and_trailing_backslashes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
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
        let diagnostic = CompilerDiagnostic::invalid_string_escape(reason, span(source_path));
        let message = terminal::format_diagnostic_guidance(&diagnostic, render_context).join("\n");

        assert_eq!(message, expected);
    }
}

#[test]
fn rule_renderers_use_user_facing_messages_not_reason_debug_names() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let value_name = string_table.intern("value");

    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(value_name),
        None,
        None,
        None,
        None,
        span(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);

    let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);
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
    let _path_fork = PathInternerFork::empty();
    let source_path = Path::new("/project/main.moth");
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .expect("test source identity should build");
    let source = source_database
        .get_by_canonical_path(source_path)
        .expect("test source should be registered")
        .id;
    assert_eq!(source, SourceId::from_index(1));
    source_database
        .retain_text(source, "value = value\n".to_owned())
        .expect("test source text should be retained");

    let value_name = string_table.intern("value");
    let mut span_builder = ExtendedSpanBuilder::new();
    let declaration_location = Some(exact_span(source, 0, 5, &mut span_builder));
    let assignment_location = Some(exact_span(source, 8, 5, &mut span_builder));

    let diagnostic = CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(value_name),
        None,
        None,
        None,
        declaration_location,
        assignment_location,
    );

    let labels = terminal::format_label_messages_with_context(
        &diagnostic,
        DiagnosticRenderContext::new(&string_table)
            .with_optional_source_database(Some(&source_database)),
    );

    assert_eq!(diagnostic.primary_span, assignment_location);
    assert_eq!(
        diagnostic.labels.len(),
        1,
        "expected only the secondary label"
    );
    assert_eq!(diagnostic.labels[0].span, declaration_location);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::ImmutableBindingDeclaration),
    );
    assert_eq!(
        labels,
        vec!["info: 1:1 - immutable binding declared here"],
        "secondary declaration label should render through the retained source context",
    );
}

#[test]
fn syntax_and_choice_renderers_use_user_facing_messages_not_reason_debug_names() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let choice_name = string_table.intern("Status");
    let variant_name = string_table.intern("Ready");

    let diagnostics = [
        (
            CompilerDiagnostic::invalid_path(PathKind::WhitespaceMustBeQuoted, span(source_path)),
            "Path components with whitespace must be quoted",
            "WhitespaceMustBeQuoted",
        ),
        (
            CompilerDiagnostic::invalid_dependency_clause(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::ExpectedAliasName,
                span(source_path),
            ),
            "Expected alias name after `as`",
            "ExpectedAliasName",
        ),
        (
            CompilerDiagnostic::invalid_choice_variant(
                InvalidChoiceVariantReason::UnitVariantWithParentheses,
                Some(choice_name),
                Some(variant_name),
                Vec::new(),
                span(source_path),
            ),
            "Unit variant 'Status::Ready' cannot be called with empty parentheses",
            "UnitVariantWithParentheses",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
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
        span(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);

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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
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
        span(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);

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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let literal = string_table.intern("1.");
    let fixed_unsigned_literal = string_table.intern("300");
    let fixed_negative_literal = string_table.intern("-1");
    let fixed_signed_literal = string_table.intern("-129");
    let style_directive = string_table.intern("unknown");
    let supported_directives = string_table.intern("'$html', '$css'");
    let declaration_name = string_table.intern("Card");
    let template_directive = string_table.intern("insert");
    let namespace_name = string_table.intern("card");

    let diagnostics = [
        (
            CompilerDiagnostic::duplicate_declaration(
                declaration_name,
                span(source_path),
                span(source_path),
            ),
            "Cannot declare 'Card' because that name is already visible in this scope",
            "StringId",
        ),
        (
            CompilerDiagnostic::invalid_number_literal(
                literal,
                NumberLiteralErrorReason::MultipleDecimalPoints,
                span(source_path),
            ),
            "Can't have more than one decimal point in numeric literal '1.'",
            "MultipleDecimalPoints",
        ),
        (
            CompilerDiagnostic::invalid_number_literal(
                fixed_unsigned_literal,
                NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U8),
                span(source_path),
            ),
            "Integer literal '300' is outside the U8 range 0 to 255.",
            "OutsideFixedScalarRange",
        ),
        (
            CompilerDiagnostic::invalid_number_literal(
                fixed_signed_literal,
                NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I8),
                span(source_path),
            ),
            "Integer literal '-129' is outside the I8 range -128 to 127.",
            "OutsideFixedScalarRange",
        ),
        (
            CompilerDiagnostic::invalid_number_literal(
                fixed_negative_literal,
                NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::Byte),
                span(source_path),
            ),
            "Numeric literal '-1' is negative, but Byte cannot hold negative values.",
            "NegativeUnsignedLiteral",
        ),
        (
            CompilerDiagnostic::invalid_style_directive(
                style_directive,
                supported_directives,
                span(source_path),
            ),
            "Style directive '$unknown' is unsupported here",
            "InvalidStyleDirective",
        ),
        (
            CompilerDiagnostic::invalid_type_annotation(
                TypeAnnotationContext::DeclarationTarget,
                InvalidTypeAnnotationReason::UnexpectedColon,
                span(source_path),
            ),
            "Unexpected ':' after declaration name",
            "UnexpectedColon",
        ),
        (
            CompilerDiagnostic::invalid_signature_member(
                InvalidSignatureMemberReason::ChoicePayloadDefaultValue,
                span(source_path),
            ),
            "Choice payload fields cannot have default values.",
            "ChoicePayloadDefaultValue",
        ),
        (
            CompilerDiagnostic::invalid_signature_member(
                InvalidSignatureMemberReason::MissingDefaultValue,
                span(source_path),
            ),
            "Expected a default value after '='.",
            "MissingDefaultValue",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingColonAfterReturns,
                span(source_path),
            ),
            "Function return declarations must end with ':'",
            "MissingColonAfterReturns",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingReturnType,
                span(source_path),
            ),
            "Function signature is missing a return type after '->'. Add a type followed by ':', or remove '->' for a no-value function.",
            "MissingReturnType",
        ),
        (
            CompilerDiagnostic::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingTraitRequirementReturnType,
                span(source_path),
            ),
            "Trait requirement is missing a return type after '->'. Add a type, or remove '->' for a no-value requirement.",
            "MissingTraitRequirementReturnType",
        ),
        (
            CompilerDiagnostic::invalid_generic_application(
                GenericApplicationErrorReason::NestedApplication,
                span(source_path),
            ),
            "Nested generic type applications are not supported",
            "NestedApplication",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::NegativeCapacity,
                span(source_path),
            ),
            "Fixed collection capacity must be greater than zero.",
            "NegativeCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ZeroCapacity,
                span(source_path),
            ),
            "Fixed collection capacity must be greater than zero.",
            "ZeroCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityNotInt,
                span(source_path),
            ),
            "Collection capacity must be an integer.",
            "CapacityNotInt",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityOverflow,
                span(source_path),
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
                span(source_path),
            ),
            "Collection literal has more items than the fixed collection capacity allows.",
            "InitializerExceedsFixedCapacity",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection,
                span(source_path),
            ),
            "Immutable binding initialized with an empty fixed collection literal is not allowed.",
            "EmptyImmutableFixedCollection",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous,
                span(source_path),
            ),
            "Capacity-only shorthand requires a non-empty collection literal so the element type can be inferred.",
            "ShorthandEmptyLiteralAmbiguous",
        ),
        (
            CompilerDiagnostic::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                span(source_path),
            ),
            "Capacity-only shorthand requires a collection literal initializer.",
            "ShorthandNonLiteralRhs",
        ),
        (
            CompilerDiagnostic::invalid_generic_parameter(
                InvalidGenericParameterReason::BoundsMustUseIs,
                span(source_path),
            ),
            "Generic parameter bounds use `is`.",
            "BoundsMustUseIs",
        ),
        (
            CompilerDiagnostic::invalid_trait_keyword_usage(
                InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
                span(source_path),
            ),
            "Keyword 'must' is trait-only syntax",
            "MustOutsideTraitSyntax",
        ),
        (
            CompilerDiagnostic::invalid_template_directive(
                Some(template_directive),
                InvalidTemplateDirectiveReason::MissingArgument,
                span(source_path),
            ),
            "Template directive 'insert' is missing a required argument.",
            "MissingArgument",
        ),
        (
            CompilerDiagnostic::invalid_template_directive(
                Some(template_directive),
                InvalidTemplateDirectiveReason::invalid_argument_with_detail(
                    string_table.get_or_intern(
                        "Unsupported language \"rustt\". Supported aliases are \"rs\"/\"rust\"."
                            .to_owned(),
                    ),
                ),
                span(source_path),
            ),
            "Invalid argument for template directive 'insert'. Unsupported language \"rustt\". Supported aliases are \"rs\"/\"rust\".",
            "InvalidArgument",
        ),
        (
            CompilerDiagnostic::namespace_misuse(
                namespace_name,
                NameNamespace::Type,
                NameNamespace::Value,
                span(source_path),
            ),
            "'card' is a value and cannot be used as a type.",
            "NamespaceMisuse",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);
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
    let type_environment = TypeEnvironment::new();
    let operator_diagnostic = CompilerDiagnostic::unsupported_operator_types(
        DiagnosticOperator::Add,
        type_environment.builtins().string,
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
        span(source_path),
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(operator_diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("captured operator diagnostic");
    let render_context = DiagnosticRenderContext::for_report(report);
    let guidance = terminal::format_guidance_view(view, render_context);
    let terse_line = terse::format_terse_boundary_messages(&failure)
        .into_iter()
        .next()
        .expect("captured operator diagnostic");

    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Operator `+` cannot concatenate")),
        "Guidance should retain typed operator prose: {guidance:?}",
    );
    assert!(
        terse_line.contains("Operator `+` cannot concatenate"),
        "Terse output should retain typed operator prose: {terse_line}",
    );
    assert!(
        !guidance.iter().any(|line| line.contains("Add")),
        "Guidance should not expose the reason code: {guidance:?}",
    );
    assert!(
        !terse_line.contains("Add"),
        "Terse output should not expose the reason code: {terse_line}",
    );
}

#[test]
fn invalid_expression_renderers_keep_structured_reason_prose() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");

    let diagnostics = [
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::ExpectedOperatorBeforeExpression,
                span(source_path),
            ),
            "Expected an operator before this expression.",
            "ExpectedOperatorBeforeExpression",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::UnresolvedStackShape,
                span(source_path),
            ),
            "This expression does not resolve to exactly one value.",
            "UnresolvedStackShape",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::MothFileHasNoValue,
                span(source_path),
            ),
            "A `.moth` file has no file value. Bind its declarations through a dependency clause.",
            "MothFileHasNoValue",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::ExtensionlessFileValue,
                span(source_path),
            ),
            "A file value needs an explicit extension. Write the path with a file extension, or use a dependency clause to bind declarations.",
            "ExtensionlessFileValue",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::EmptyRuntimeAnonymousRecord,
                span(source_path),
            ),
            "Empty anonymous records are outside the initial runtime surface.",
            "EmptyRuntimeAnonymousRecord",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                span(source_path),
            ),
            "Each const-record parameter needs a value. Write `name = value`.",
            "AnonymousRecordFieldNotNamed",
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);

    for (diagnostic, expected_message, debug_name) in diagnostics {
        let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);
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
fn failure_handling_diagnostics_preserve_producer_types_and_spans() {
    let mut span_builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let catch_span = Some(exact_span(source, 0, 30, &mut span_builder));
    let first_span = Some(exact_span(source, 0, 8, &mut span_builder));
    let second_span = Some(exact_span(source, 11, 9, &mut span_builder));
    let reasons = [
        (
            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count: 3 },
            "invalid_fallible_handling.function_has_multiple_success_returns",
            vec![],
        ),
        (
            InvalidFallibleHandlingReason::EntryHasIncompatibleError {
                produced_error_type: builtin_type_ids::STRING,
                entry_error_type: builtin_type_ids::BOOL,
            },
            "invalid_fallible_handling.entry_has_incompatible_error",
            vec![],
        ),
        (
            InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                first_error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                second_error_type_id: builtin_type_ids::STRING,
                first_producer_span: first_span,
                second_producer_span: second_span,
            },
            "invalid_fallible_handling.incompatible_catch_error_types",
            vec![first_span, second_span],
        ),
        (
            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                typed_producer_span: first_span,
                implicit_producer_span: second_span,
            },
            "invalid_fallible_handling.custom_error_mixed_with_implicit_failure",
            vec![first_span, second_span],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                witness: Box::new(BuiltinFailureWitness {
                    codes: vec![BuiltinErrorCode::IntOverflow],
                    call_spans: vec![first_span, None],
                    origin_span: second_span,
                    origin: BuiltinFailureOriginKind::Operation,
                    elided_call_hops: 2,
                }),
            },
            "invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function",
            vec![first_span, second_span],
        ),
        (
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness: Box::new(BuiltinFailureWitness {
                    codes: vec![
                        BuiltinErrorCode::DivideByZero,
                        BuiltinErrorCode::IntOverflow,
                    ],
                    call_spans: vec![first_span],
                    origin_span: second_span,
                    origin: BuiltinFailureOriginKind::Operation,
                    elided_call_hops: 0,
                }),
            },
            "invalid_fallible_handling.unhandled_builtin_failure_in_exported_function",
            vec![first_span, second_span],
        ),
    ];

    for (reason, reason_key, producer_spans) in reasons {
        let diagnostic = CompilerDiagnostic::invalid_fallible_handling(reason.clone(), catch_span);
        assert_eq!(diagnostic.primary_span, catch_span);
        assert_eq!(diagnostic.identity().code, "MOTH-RULE-0051");
        assert_eq!(diagnostic.identity().reason_key, Some(reason_key));
        assert_eq!(
            diagnostic.payload,
            DiagnosticPayload::InvalidFallibleHandling { reason },
        );
        assert_eq!(
            diagnostic
                .labels
                .iter()
                .map(|label| label.span)
                .collect::<Vec<_>>(),
            producer_spans,
        );
    }
}

#[test]
fn propagation_boundary_renderers_preserve_authored_count_and_semantic_error_types() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let error_path = path_fork
        .try_intern_portable_path("ParseFailure", &mut string_table)
        .expect("test path fits");
    let (_, produced_error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: error_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let entry_error_path = path_fork
        .try_intern_portable_path("Error", &mut string_table)
        .expect("test path fits");
    let (_, entry_error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: entry_error_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let path_table = Arc::new(path_fork.snapshot_table());
    let render_captured = |diagnostic| {
        let prepared = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            string_table.clone(),
            Some(&type_environment),
            true,
            Some(Arc::clone(&path_table)),
        );
        finish_prepared_batch(prepared, StringTable::new())
    };

    for count in [2, 5] {
        let diagnostic = CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count },
            None,
        );
        let failure = render_captured(diagnostic);
        let (report, view) = failure.reports.iter().next().expect("captured diagnostic");
        let guidance =
            terminal::format_guidance_view(view, DiagnosticRenderContext::for_report(report))
                .join("\n");
        let terse = terse::format_terse_boundary_messages(&failure).join("\n");
        let count_fact = format!("{count} authored success return slots");
        for message in [&guidance, &terse] {
            assert!(message.contains(&count_fact), "{message}");
            assert!(message.contains("Inspect the option locally"), "{message}");
            assert!(
                message.contains("use a single optional success return slot"),
                "{message}",
            );
            assert!(!message.contains("FunctionHasMultipleSuccessReturns"));
        }
    }

    let diagnostic = CompilerDiagnostic::invalid_fallible_handling(
        InvalidFallibleHandlingReason::EntryHasIncompatibleError {
            produced_error_type,
            entry_error_type,
        },
        None,
    );
    let failure = render_captured(diagnostic);
    let (report, view) = failure.reports.iter().next().expect("captured diagnostic");
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::for_report(report))
            .join("\n");
    let terse = terse::format_terse_boundary_messages(&failure).join("\n");
    for message in [&guidance, &terse] {
        assert!(message.contains("produces `ParseFailure!`"), "{message}");
        assert!(message.contains("`start` `Error!` boundary"), "{message}");
        assert!(
            message.contains("Handle `ParseFailure` locally with `catch` or recover explicitly"),
            "{message}",
        );
        assert!(!message.contains("EntryHasIncompatibleError"));
        assert!(!message.contains("TypeId"));
        assert!(!message.contains("declare"));
        assert!(!message.contains("add"));
    }
}

#[test]
fn builtin_failure_witness_labels_render_calls_before_the_original_operation() {
    let mut string_table = StringTable::new();
    let source_path = Path::new("/project/failure.moth");
    let mut sources = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .expect("test source identity should build");
    let source = sources
        .get_by_canonical_path(source_path)
        .expect("registered source")
        .id;
    sources
        .retain_text(
            source,
            "export\nfirst\nsecond\nthird\nmultiply\n".to_owned(),
        )
        .expect("test source text should be retained");
    let mut builder = ExtendedSpanBuilder::new();
    let boundary = Some(exact_span(source, 0, 6, &mut builder));
    let call_spans = vec![
        Some(exact_span(source, 7, 5, &mut builder)),
        Some(exact_span(source, 13, 6, &mut builder)),
        Some(exact_span(source, 20, 5, &mut builder)),
    ];
    let origin_span = Some(exact_span(source, 26, 8, &mut builder));
    let witness = Box::new(BuiltinFailureWitness {
        codes: vec![BuiltinErrorCode::IntOverflow],
        call_spans,
        origin_span,
        origin: BuiltinFailureOriginKind::Operation,
        elided_call_hops: 2,
    });
    let mut diagnostic = CompilerDiagnostic::invalid_fallible_handling(
        InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
        boundary,
    );
    assert_eq!(diagnostic.primary_span, boundary);
    assert_eq!(diagnostic.labels.len(), 4);
    assert!(
        diagnostic.labels[..3]
            .iter()
            .all(|label| { label.message == Some(DiagnosticLabelMessage::BuiltinFailureCall) })
    );
    assert_eq!(
        diagnostic.labels[3].message,
        Some(DiagnosticLabelMessage::BuiltinFailureOrigin)
    );
    assert_eq!(diagnostic.labels[3].span, origin_span);
    let context =
        DiagnosticRenderContext::new(&string_table).with_optional_source_database(Some(&sources));
    assert_eq!(
        terminal::format_label_messages_with_context(&diagnostic, context),
        vec![
            "info: 2:1 - implicit built-in failure propagates through this call",
            "info: 3:1 - implicit built-in failure propagates through this call",
            "info: 4:1 - implicit built-in failure propagates through this call",
            "info: 5:1 - original failing numeric or range operation here",
        ],
    );

    let owner = FrozenIdentityHandle::new();
    owner
        .install(Arc::new(FrozenIdentityContext::from_parts(
            string_table,
            sources,
        )))
        .expect("witness source identity should install once");
    diagnostic.attach_frozen_identity_handle_if_missing(owner.clone());
    assert_eq!(
        diagnostic.primary_frozen_identity_handle,
        Some(owner.clone())
    );
    assert!(
        diagnostic
            .labels
            .iter()
            .all(|label| { label.frozen_identity_handle.as_ref() == Some(&owner) })
    );
}

#[test]
fn phase_1_2_renderers_keep_source_language_terminology() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let config_key = string_table.intern("homepage");
    let diagnostics = vec![
        CompilerDiagnostic::invalid_standalone_statement(
            InvalidStandaloneStatementReason::StandaloneTemplate,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::FallibleValueInTemplateHead,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateIfConditionNotConst,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnOptional,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::ValueCouldNotFold,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_cast(
            InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
            None,
            None,
            span(source_path),
        ),
        CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
            None,
            span(source_path),
        ),
        CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::AsyncBlock,
            span(source_path),
        ),
    ];
    let render_context = DiagnosticRenderContext::new(&string_table);
    let rendered = diagnostics
        .iter()
        .flat_map(|diagnostic| terminal::format_diagnostic_guidance(diagnostic, render_context))
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
fn source_dependency_renderers_use_current_language_terminology() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let dependency_path = path_fork
        .try_intern_portable_path("missing.moth", &mut string_table)
        .expect("test path fits");
    let namespace_name = string_table.intern("docs");
    let alias_name = string_table.intern("readFile");
    let symbol_name = string_table.intern("read_file");
    let member_name = string_table.intern("member");
    let mut diagnostics = vec![
        CompilerDiagnostic::missing_import_target(dependency_path, span(source_path)),
        CompilerDiagnostic::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::ParentDirectorySegment,
            span(source_path),
        ),
        CompilerDiagnostic::dependency_namespace_used_as_value(namespace_name, span(source_path)),
        CompilerDiagnostic::nested_dependency_traversal(namespace_name, span(source_path)),
        CompilerDiagnostic::dependency_alias_case_mismatch(
            alias_name,
            symbol_name,
            span(source_path),
        ),
    ];

    for (expected, found) in [
        (
            NamespaceTypeValueMisuseKind::Type,
            NamespaceTypeValueMisuseKind::Value,
        ),
        (
            NamespaceTypeValueMisuseKind::Value,
            NamespaceTypeValueMisuseKind::Type,
        ),
        (
            NamespaceTypeValueMisuseKind::Value,
            NamespaceTypeValueMisuseKind::Namespace,
        ),
        (
            NamespaceTypeValueMisuseKind::Type,
            NamespaceTypeValueMisuseKind::Namespace,
        ),
        (
            NamespaceTypeValueMisuseKind::Namespace,
            NamespaceTypeValueMisuseKind::Value,
        ),
        (
            NamespaceTypeValueMisuseKind::Namespace,
            NamespaceTypeValueMisuseKind::Type,
        ),
    ] {
        diagnostics.push(CompilerDiagnostic::namespace_type_value_misuse(
            member_name,
            expected,
            found,
            span(source_path),
        ));
    }

    let render_context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);
    let rendered_lines = terse::format_terse_diagnostics_with_context(&diagnostics, render_context);
    assert_eq!(
        rendered_lines.len(),
        diagnostics.len(),
        "the complete terse renderer must emit one record per diagnostic"
    );
    let rendered = rendered_lines.join("\n");

    for code in [
        "MOTH-IMPORT-0005",
        "MOTH-IMPORT-0016",
        "MOTH-RULE-0065",
        "MOTH-RULE-0066",
        "MOTH-IMPORT-0003",
        "MOTH-RULE-0067",
    ] {
        assert!(
            rendered.contains(code),
            "expected dependency diagnostic code '{code}' in: {rendered}",
        );
    }
    for expected in [
        "Cannot resolve dependency",
        "Dependency paths containing '..'",
        "dependency namespace binding",
        "Dependency namespace records do not expose nested filesystem paths",
        "Dependency alias 'readFile' case mismatch with symbol 'read_file'",
        "member of the dependency namespace",
    ] {
        assert!(
            rendered.contains(expected),
            "expected current dependency terminology '{expected}' in: {rendered}",
        );
    }

    for stale in [
        "Cannot resolve import",
        "Import paths",
        "Import alias",
        "import namespace",
        "import record",
        "Import record",
        "Nested import-record traversal",
    ] {
        assert!(
            !rendered.contains(stale),
            "source dependency renderer retained stale term '{stale}': {rendered}",
        );
    }
}

#[test]
fn render_boundary_smoke_coverage_hides_internal_debug_names_by_family() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let import_path = path_fork
        .try_intern_portable_path("missing.moth", &mut string_table)
        .expect("test path fits");
    let value_name = string_table.intern("value");
    let config_key = string_table.intern("homepage");
    let feature_name = string_table.intern("traits");
    let type_environment = TypeEnvironment::new();

    let diagnostics = vec![
        CompilerDiagnostic::invalid_path(PathKind::WhitespaceMustBeQuoted, span(source_path)),
        CompilerDiagnostic::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
            TypeMismatchContext::Declaration,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::ImmutableBinding,
            Some(value_name),
            None,
            None,
            None,
            None,
            span(source_path),
        ),
        CompilerDiagnostic::missing_import_target(import_path, span(source_path)),
        CompilerDiagnostic::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::UnsupportedScalarValue,
            span(source_path),
        ),
        CompilerDiagnostic::deferred_feature(feature_name, span(source_path)),
    ];
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let html = dev_server::render_boundary_messages_html(&failure, Path::new("/tmp"));
    assert_rendered_diagnostic_hides_internal_names(&html);

    let terse_lines = terse::format_terse_boundary_messages(&failure);
    assert_eq!(terse_lines.len(), 6);
    for (report, view) in failure.reports.iter() {
        let render_context = DiagnosticRenderContext::for_report(report);
        let terminal_guidance = terminal::format_guidance_view(view, render_context).join("\n");
        let terse_line = terse::format_terse_diagnostic_view(view, render_context);
        assert_rendered_diagnostic_hides_internal_names(&terminal_guidance);
        assert_rendered_diagnostic_hides_internal_names(&terse_line);
        assert!(terse_line.contains(view.descriptor().code));
        assert!(html.contains(view.descriptor().code));
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::incompatible_choice_comparison(
        IncompatibleChoiceComparisonReason::ChoiceWithNonChoice,
        builtin_type_ids::BOOL,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        span(source_path),
    );
    let render_context = DiagnosticRenderContext::new(&string_table);

    let guidance = terminal::format_diagnostic_guidance(&diagnostic, render_context);
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", string_table)
        .expect("test path fits");
    let callee = string_table.intern(callee_name);
    let diagnostic =
        CompilerDiagnostic::invalid_call_shape(reason, Some(callee), span(source_path));
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_diagnostic_guidance(&diagnostic, render_context)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[test]
fn mutable_access_required_renders_explicit_marker_guidance() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
fn prepared_rebase_updates_invalid_call_shape_names() {
    let mut local_table = StringTable::new();
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
        None,
    );

    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let call = view
        .family_view::<InvalidCallShapeView>()
        .expect("the frozen diagnostic must retain invalid-call-shape facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.callee_name.expect("callee name")),
        "consume"
    );
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.parameter_name.expect("parameter name")),
        "values"
    );
    let [binding] = view.string_ids() else {
        panic!("expected one frozen binding-name string ID");
    };
    assert_eq!(report.context().strings().resolve(*binding), "values");
}

fn render_invalid_receiver_call(
    string_table: &mut StringTable,
    reason: InvalidReceiverCallReason,
    method_name: &str,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<&str>,
) -> String {
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", string_table)
        .expect("test path fits");
    let method = string_table.intern(method_name);
    let binding = receiver_binding_name.map(|name| string_table.intern(name));
    let diagnostic = CompilerDiagnostic::invalid_receiver_call(
        reason,
        None,
        Some(method),
        receiver_kind,
        binding,
        span(source_path),
    );
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_diagnostic_guidance(&diagnostic, render_context)
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[test]
fn source_method_missing_marker_renders_named_receiver_example() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
fn prepared_rebase_updates_invalid_receiver_call_binding_name() {
    let mut local_table = StringTable::new();
    let method = local_table.intern("move");
    let binding = local_table.intern("p");
    let diagnostic = CompilerDiagnostic::invalid_receiver_call(
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        None,
        Some(method),
        Some(ReceiverCallKind::SourceMethod),
        Some(binding),
        None,
    );

    let mut merged_table = StringTable::new();
    merged_table.intern("another module's string");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let call = view
        .family_view::<InvalidReceiverCallView>()
        .expect("the frozen diagnostic must retain receiver-call facts");
    assert_eq!(
        report
            .context()
            .strings()
            .resolve(call.method_name.expect("method name")),
        "move"
    );
    assert_eq!(
        crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::invalid_receiver_call_binding_name(
            view.string_ids(),
        )
        .map(|binding| report.context().strings().resolve(binding)),
        Some("p")
    );
}

#[test]
fn generic_instantiation_rendering_resolves_type_name() {
    use crate::compiler_frontend::compiler_messages::{
        CompilerDiagnostic, InvalidGenericInstantiationReason,
    };
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let box_name = string_table.intern("Box");
    let span = None;

    let diagnostic = CompilerDiagnostic::invalid_generic_instantiation(
        Some(box_name),
        InvalidGenericInstantiationReason::WrongArgumentCount {
            expected: 1,
            found: 2,
        },
        span,
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

    let mut string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let function_name = string_table.intern("first");
    let parameter_name = string_table.intern("T");
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::invalid_generic_instantiation(
        Some(function_name),
        InvalidGenericInstantiationReason::ConflictingInference {
            subject: crate::compiler_frontend::compiler_messages::GenericInferenceSubject::Function,
            parameter_id: GenericParameterId(0),
            parameter_name,
            existing_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            replacement_type_id: type_environment.builtins().string,
        },
        None,
    );

    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("captured inference conflict");
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::for_report(report));

    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Generic parameter 'T'")
                && line.contains("I32")
                && line.contains("String")),
        "expected captured type names in guidance, got: {guidance:?}"
    );
}

#[test]
fn builtin_cast_shape_diagnostic_preserves_stable_code_and_rendering() {
    use crate::compiler_frontend::compiler_messages::InvalidBuiltinCallReason;

    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let scalar_name = string_table.intern("I32");

    let diagnostic = CompilerDiagnostic::invalid_builtin_call(
        InvalidBuiltinCallReason::CastMissingArgument,
        Some(scalar_name),
        None,
    );

    let render_context = DiagnosticRenderContext::new(&string_table);
    let terse_line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert_eq!(diagnostic.kind.descriptor().code, "MOTH-RULE-0046");
    assert!(
        terse_line.contains("'I32' cast requires exactly one argument"),
        "{terse_line}"
    );
}

#[test]
fn token_diagnostics_render_source_spelling_not_token_debug_names() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let name = string_table.intern("item");
    let span = None;

    let expected = CompilerDiagnostic::expected_token_from_tags(
        TokenTag::OPEN_PARENTHESIS,
        Some(DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name)),
        span,
    );
    let unexpected = CompilerDiagnostic::unexpected_token_from_tag(
        DiagnosticToken::from_static_tag(TokenTag::OPEN_CURLY),
        span,
    );

    let render_context = DiagnosticRenderContext::new(&string_table);
    let expected_terse = terse::format_terse_diagnostic_with_context(&expected, render_context);
    let unexpected_terminal =
        terminal::format_diagnostic_guidance(&unexpected, render_context).join("\n");

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
fn borrow_labels_preserve_order_and_exact_source_spans() {
    let primary_source = SourceId::from_index(1);
    let related_source = SourceId::from_index(2);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span(primary_source, 40, 8, &mut builder);
    let related = exact_span(related_source, 11, 7, &mut builder);
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let value_name = strings.intern("value");

    let diagnostic = CompilerDiagnostic::multiple_mutable_borrows(
        DiagnosticPlace::Local(value_name),
        None,
        Some(related),
        Some(primary),
    );

    assert_eq!(diagnostic.primary_span, Some(primary));
    assert_eq!(diagnostic.labels.len(), 1);
    assert_eq!(diagnostic.labels[0].span, Some(related));
    assert_eq!(
        diagnostic.labels[0].style,
        super::DiagnosticLabelStyle::Secondary
    );
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::ConflictingAccess)
    );
}

#[test]
fn prepared_rebase_preserves_source_spans_and_strings() {
    let mut local_table = StringTable::new();
    let name = local_table.intern("Button");
    let label_text = local_table.intern("previous declaration");
    let primary_source = SourceId::from_index(3);
    let related_source = SourceId::from_index(4);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span(primary_source, 4, 6, &mut builder);
    let related = exact_span(related_source, 18, 5, &mut builder);
    let diagnostic = CompilerDiagnostic::invalid_trait_conformance(
        name,
        None,
        InvalidTraitConformanceReason::ImportedModuleRoot,
        Some(primary),
    )
    .with_labels(vec![DiagnosticLabel::secondary(
        Some(related),
        Some(DiagnosticLabelMessage::RenderedText(label_text)),
    )]);

    let mut merged_table = StringTable::new();
    merged_table.intern("preexisting string");
    let prepared = capture_rebased_batch(
        vec![diagnostic],
        local_table,
        &mut merged_table,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batch(prepared, merged_table);
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the prepared batch must freeze one diagnostic");
    let conformance = view
        .family_view::<InvalidTraitConformanceView>()
        .expect("the frozen diagnostic must retain invalid trait-conformance facts");
    assert!(matches!(
        conformance.shape,
        InvalidTraitConformanceShape::ImportedModuleRoot
    ));
    let target_name = view
        .string_ids()
        .first()
        .expect("trait conformance should retain its target name");
    assert_eq!(report.context().strings().resolve(*target_name), "Button");
    assert!(matches!(
        view.primary_site(),
        SiteRef::Local(span) if span == primary
    ));
    let label = view
        .durable_labels()
        .next()
        .expect("the frozen trait-conformance label must remain present");
    match label.message {
        DurableLabelMessage::RenderedText(text) => {
            assert_eq!(
                report.context().strings().resolve(text),
                "previous declaration"
            );
        }
        message => panic!("unexpected frozen trait-conformance label: {message:?}"),
    }
    assert!(matches!(
        label.site,
        SiteRef::Local(span) if span == related
    ));
}

#[test]
fn preparation_capture_validates_source_identity_without_encoding_or_rebinding() {
    let source = SourceId::from_index(5);
    let other_source = SourceId::from_index(6);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span(source, 10, 4, &mut builder);
    let related = exact_span(source, 30, 3, &mut builder);
    let mut diagnostic = CompilerDiagnostic::unterminated_string_literal(Some(primary));
    diagnostic
        .labels
        .push(DiagnosticLabel::secondary(Some(related), None));

    diagnostic
        .capture_preparation_span(source)
        .expect("spans from the prepared source should validate");
    assert_eq!(diagnostic.primary_span, Some(primary));
    assert_eq!(diagnostic.labels[0].span, Some(related));

    let error = diagnostic
        .capture_preparation_span(other_source)
        .expect_err("a foreign source span must be rejected");
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert_eq!(error.source_span, None);
    assert_eq!(diagnostic.primary_span, Some(primary));
    assert_eq!(diagnostic.labels[0].span, Some(related));
}

#[test]
fn preparation_capture_allows_spanless_diagnostics() {
    let mut diagnostic = CompilerDiagnostic::unterminated_string_literal(None);
    diagnostic
        .capture_preparation_span(SourceId::from_index(7))
        .expect("spanless diagnostics need no synthesized provenance");
    assert_eq!(diagnostic.primary_span, None);
    assert!(diagnostic.labels.is_empty());
}
// WHY: These inventories provide exhaustive descriptor-registry coverage for the reflection test.
impl DiagnosticKind {
    #[cfg(test)]
    pub(crate) fn all() -> Vec<Self> {
        let mut kinds = Vec::new();

        kinds.extend(SyntaxDiagnosticKind::all().map(DiagnosticKind::Syntax));
        kinds.extend(TypeDiagnosticKind::all().map(DiagnosticKind::Type));
        kinds.extend(RuleDiagnosticKind::all().map(DiagnosticKind::Rule));
        kinds.extend(ImportDiagnosticKind::all().map(DiagnosticKind::Import));
        kinds.extend(BorrowDiagnosticKind::all().map(DiagnosticKind::Borrow));
        kinds.extend(ConfigDiagnosticKind::all().map(DiagnosticKind::Config));
        kinds.extend(InfrastructureDiagnosticKind::all().map(DiagnosticKind::Infrastructure));
        kinds.extend(DeferredFeatureDiagnosticKind::all().map(DiagnosticKind::DeferredFeature));

        kinds
    }
}
#[cfg(test)]
impl SyntaxDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::ExpectedToken,
            Self::UnexpectedToken,
            Self::UnexpectedTrailingComma,
            Self::MalformedCssTemplate,
            Self::MalformedHtmlTemplate,
            Self::UnterminatedStringLiteral,
            Self::InvalidCharacter,
            Self::InvalidNumberLiteral,
            Self::InvalidCharLiteral,
            Self::InvalidStyleDirective,
            Self::InvalidIdentifier,
            Self::MissingClosingDelimiter,
            Self::UnexpectedTokenInDeclaration,
            Self::InvalidTypeAnnotation,
            Self::InvalidGenericApplication,
            Self::InvalidParameterContract,
            Self::InvalidCollectionType,
            Self::InvalidMapType,
            Self::InvalidMapLiteral,
            Self::UnexpectedEndOfFile,
            Self::InvalidPath,
            Self::InvalidDependencyClause,
            Self::LegacyDependencyClause,
            Self::InvalidGenericParameter,
            Self::InvalidTemplateDirective,
            Self::InvalidTemplateStructure,
            Self::InvalidExpression,
            Self::MissingOperatorOperand,
            Self::InvalidStandaloneStatement,
            Self::ExpectedSymbolStatement,
            Self::MissingCollectionItem,
            Self::InvalidMatchArm,
            Self::InvalidLoopHeader,
            Self::InvalidStatementPosition,
            Self::CommonSyntaxMistake,
            Self::UnescapedImplicitTemplateClose,
            Self::SourceSpanCapacity,
            Self::CompilerCapacityExceeded,
            Self::InvalidStringEscape,
        ]
        .into_iter()
    }
}
#[cfg(test)]
impl TypeDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::TypeMismatch,
            Self::EmptyCollectionTypeAmbiguity,
            Self::UnsupportedOperatorTypes,
            Self::IncompatibleChoiceComparison,
            Self::InvalidRouteOutput,
        ]
        .into_iter()
    }
}
#[cfg(test)]
impl RuleDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::UnknownName,
            Self::DuplicateDeclaration,
            Self::IdentifierNamingConvention,
            Self::UnreachableMatchArm,
            Self::InvalidTopLevelRuntimeStatement,
            Self::ReservedBuiltinName,
            Self::InvalidSignatureMember,
            Self::InvalidChoiceVariant,
            Self::InvalidStructDefaultValue,
            Self::MissingDeclarationInitializer,
            Self::CircularDependency,
            Self::UnknownValueName,
            Self::UnknownTypeName,
            Self::ValueUsedAsType,
            Self::TypeUsedAsValue,
            Self::ShadowedName,
            Self::ReservedNameCollision,
            Self::InvalidThisUsage,
            Self::InvalidReceiverDeclaration,
            Self::InvalidControlFlowStatement,
            Self::InvalidDeclaration,
            Self::InvalidAssignmentTarget,
            Self::InvalidMultiBind,
            Self::InvalidValueReceiver,
            Self::InvalidBuiltinCall,
            Self::InvalidCast,
            Self::InvalidReceiverCall,
            Self::InvalidCopyTarget,
            Self::InvalidFieldAccess,
            Self::InvalidMatchPattern,
            Self::NonExhaustiveMatch,
            Self::InvalidFallibleHandling,
            Self::InvalidTemplateSlot,
            Self::CompileTimeEvaluationError,
            Self::InvalidCallShape,
            Self::InvalidReturnShape,
            Self::InvalidFunctionSignature,
            Self::InvalidGenericInstantiation,
            Self::UnsupportedExternalFunction,
            Self::InvalidRangeOperand,
            Self::UnsupportedBuilderPackage,
            Self::InvalidPageMetadata,
            Self::InvalidCompileTimePath,
            Self::DependencyNamespaceUsedAsValue,
            Self::ConstRecordUsedAsValue,
            Self::NestedDependencyTraversal,
            Self::NamespaceTypeValueMisuse,
            Self::UnknownTrait,
            Self::DuplicateTraitRequirement,
            Self::TraitPrivateSurfaceLeak,
            Self::UnsupportedTraitFeature,
            Self::InvalidTraitConformance,
            Self::InvalidTraitIncompatibility,
            Self::GenericBoundPrivateSurfaceLeak,
            Self::TraitNameUsedAsType,
            Self::InvalidTraitKeywordUsage,
            Self::ExportOutsideModuleRoot,
            Self::InvalidExportTarget,
            Self::DuplicatePublicExport,
            Self::DuplicateExportBlock,
            Self::PrivateTypeInExportedApi,
            Self::ProjectContextEscape,
            Self::UnsupportedBackendFeature,
        ]
        .into_iter()
    }
}
#[cfg(test)]
impl ImportDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::UnusedImport,
            Self::DependencyAliasCaseMismatch,
            Self::MissingImportTarget,
            Self::AmbiguousImportTarget,
            Self::BareFileImport,
            Self::DirectSpecialFileImport,
            Self::ImportNameCollision,
            Self::NotExportedBySourceFile,
            Self::NotExportedByPublicSurface,
            Self::MissingModuleRootPublicSurface,
            Self::MissingPackageSymbol,
            Self::CrossModuleImportNotExported,
            Self::InvalidImportPath,
            Self::DirectSymbolPathImport,
            Self::InvalidNamespaceDefaultName,
            Self::DuplicateImportSurfaceMember,
            Self::ExplicitMothExtension,
            Self::ExplicitSourceExtension,
            Self::UnsupportedSourceFileKind,
            Self::InvalidSourceFileEntry,
            Self::DuplicateMothTemplateInputPath,
            Self::UnsupportedExternalExtension,
            Self::InvalidExternalModule,
            Self::MothTemplateInputsShareNoCommonAncestor,
        ]
        .into_iter()
    }
}
#[cfg(test)]
impl BorrowDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::BorrowConflict,
            Self::MultipleMutableBorrows,
            Self::SharedMutableConflict,
            Self::UseAfterPossibleMove,
            Self::MoveWhileBorrowed,
            Self::WholeObjectBorrowConflict,
            Self::InvalidMutableAccess,
            Self::UseOfUninitializedLocal,
        ]
        .into_iter()
    }
}
#[cfg(test)]
impl ConfigDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::InvalidConfig].into_iter()
    }
}
#[cfg(test)]
impl InfrastructureDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::InfrastructureFailure].into_iter()
    }
}
#[cfg(test)]
impl DeferredFeatureDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::DeferredFeature].into_iter()
    }
}

#[test]
fn failure_writeback_label_keeps_target_type_in_frozen_prepared_output() {
    let mut string_table = StringTable::new();
    let source_path = Path::new("/project/writeback.moth");
    let mut sources = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .unwrap();
    let source = sources.get_by_canonical_path(source_path).unwrap().id;
    sources
        .retain_text(source, "total += amount\n".to_owned())
        .unwrap();
    let mut builder = ExtendedSpanBuilder::new();
    let origin_span = Some(exact_span(source, 0, 5, &mut builder));
    let type_environment = TypeEnvironment::new();
    let diagnostic = CompilerDiagnostic::invalid_fallible_handling(
        InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
            witness: Box::new(BuiltinFailureWitness {
                codes: vec![BuiltinErrorCode::IntOverflow],
                call_spans: vec![],
                origin_span,
                origin: BuiltinFailureOriginKind::CompoundWriteBack {
                    target: builtin_type_ids::fixed_scalar(FixedScalar::U8),
                },
                elided_call_hops: 0,
            }),
        },
        None,
    );
    // Pass A captures the label's type against the live environment and the paired source path
    // table; the boundary then rebases the local string table and freezes the typed label fact.
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        string_table,
        Some(&type_environment),
        true,
        Some(Arc::new(sources.paths().clone())),
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(CompilerMessages::empty(aggregate_table));
    accumulator.append_prepared(prepared, Some((Arc::new(sources), None)));
    let failure = accumulator.finish(InstallMode::Frozen);
    let mut found_writeback_type = false;
    for (_, view) in failure.reports.iter() {
        for label in view.durable_labels() {
            if let DurableLabelMessage::BuiltinWriteBackOrigin { target_type } = label.message {
                assert_eq!(
                    view.types().shape(target_type),
                    TypeDisplayShape::FixedScalar(FixedScalar::U8)
                );
                found_writeback_type = true;
            }
        }
    }
    assert!(
        found_writeback_type,
        "the frozen diagnostic must retain the write-back target type"
    );
    let labels = failure
        .reports
        .iter()
        .flat_map(|(report, view)| {
            terminal::format_label_messages_view(view, DiagnosticRenderContext::for_report(report))
        })
        .collect::<Vec<_>>();
    assert!(
        labels.iter().any(|label| label.contains("U8")),
        "{labels:?}"
    );
}
