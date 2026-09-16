use super::{
    BorrowAccessKind, BorrowDiagnosticKind, CompileTimeEvaluationErrorReason, CompilerDiagnostic,
    ConfigDiagnosticKind, DeferredFeatureDiagnosticKind, DeferredFeatureReason,
    DependencyClauseKind, DiagnosticBag, DiagnosticCategory, DiagnosticKind, DiagnosticLabel,
    DiagnosticLabelMessage, DiagnosticOperator, DiagnosticPayload, DiagnosticPlace,
    DiagnosticSeverity, GenericApplicationErrorReason, ImportDiagnosticKind,
    ImportPublicSurfaceType, IncompatibleChoiceComparisonReason, InfrastructureDiagnosticKind,
    InvalidAssignmentTargetReason, InvalidCallShapeReason, InvalidCastReason,
    InvalidChoiceVariantReason, InvalidCollectionTypeReason, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidExpressionReason, InvalidFallibleHandlingReason,
    InvalidFallibleOperandReason, InvalidFunctionSignatureReason, InvalidGenericParameterReason,
    InvalidImportPathReason, InvalidMapTypeReason, InvalidOutputFolderReason,
    InvalidReceiverCallReason, InvalidSignatureMemberReason, InvalidStandaloneStatementReason,
    InvalidStatementPositionReason, InvalidStringEscapeReason, InvalidTemplateDirectiveReason,
    InvalidTemplateStructureReason, InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason,
    NameNamespace, NamespaceTypeValueMisuseKind, NumberLiteralErrorReason, PathKind,
    ReceiverCallKind, RuleDiagnosticKind, SyntaxDiagnosticKind, TokenTag, TypeAnnotationContext,
    TypeDiagnosticKind, TypeMismatchContext, UnsupportedBackendFeatureReason,
    UnsupportedOperatorCategory, is_well_formed_reason_key,
};
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerMessages, ErrorType, RenderFrozenContext, RenderTypeContext,
};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server, invalid_config_message, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::{ModuleDiagnostics, PremergeDiagnosticBatch};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, builtin_type_ids};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceKind, SourceRegistrationIndex, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
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

fn borrow_conflict_diagnostic(
    place: DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::new(
        DiagnosticKind::Borrow(BorrowDiagnosticKind::BorrowConflict),
        span,
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
    assert!(messages.infrastructure_error().is_none());
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
    assert!(messages.infrastructure_error().is_none());
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
        .infrastructure_error()
        .expect("outer infrastructure failure must be preserved");
    assert_eq!(error.msg.as_str(), "backend failed after warnings");
}

#[test]
fn attach_rejects_foreign_domain_table_behind_rendered_path_payload() {
    // The batch's string table only knows its own domain. The candidate table was issued by
    // an independent domain whose component IDs exceed the batch's next ID, so the rendered
    // leaf's walk resolves nowhere and pairing would silently render wrong spellings. The
    // foreign table interns padding strings so no foreign component ID collides with a
    // batch-table ID.
    let mut batch_table = StringTable::new();
    batch_table.intern("batch");
    batch_table.intern("second");
    let mut batch_fork = PathInternerFork::empty();
    let batch_leaf = batch_fork
        .try_intern_portable_path("batch/import", &mut batch_table)
        .expect("batch path fits");
    let diagnostic = CompilerDiagnostic::missing_import_target(batch_leaf, None);
    let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, batch_table);

    let mut foreign_strings = StringTable::new();
    foreign_strings.intern("foreign-pad-a");
    foreign_strings.intern("foreign-pad-b");
    foreign_strings.intern("foreign-pad-c");
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());

    assert!(
        batch.attach_path_table_if_missing(foreign_table).is_err(),
        "a foreign table behind a rendered path must be rejected"
    );
}

#[test]
fn attach_rejects_rendered_path_beyond_the_candidate_table() {
    // A retained diagnostic whose PathId points past the candidate table has lost its issuing
    // table: rendering would dereference a node the table cannot address. Pairing must fail
    // visibly instead of silently attaching a context that cannot spell the payload's path.
    let batch_table = StringTable::new();
    let orphan_path = PathId::try_from_index(9).expect("index 9 fits the compact path domain");
    let diagnostic = CompilerDiagnostic::missing_import_target(orphan_path, None);
    let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, batch_table);

    let mut local_strings = StringTable::new();
    let mut local_fork = PathInternerFork::empty();
    local_fork
        .try_intern_portable_path("local/import", &mut local_strings)
        .expect("local path fits");
    let local_table = Arc::new(local_fork.snapshot_table());

    assert!(
        batch.attach_path_table_if_missing(local_table).is_err(),
        "a retained path outside the candidate table must fail pairing"
    );
}

#[test]
fn attach_ignores_foreign_domain_table_for_diagnostics_without_path_payloads() {
    // A diagnostic with no PathId payload never dereferences the attached table, so a table
    // issued by a foreign string domain must not be retained merely because a caller supplies
    // one. Retaining it would expose foreign component IDs to the complete-table rewrite at the
    // next aggregation. The path-free diagnostic must still render its own name after a
    // non-identity string merge.
    let mut batch_table = StringTable::new();
    let name = batch_table.intern("local");
    let diagnostic = CompilerDiagnostic::unknown_value_name(name, None);
    let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, batch_table);

    let mut foreign_strings = StringTable::new();
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());

    batch
        .attach_path_table_if_missing(foreign_table)
        .expect("a path-free diagnostic must ignore a foreign table");

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut messages = CompilerMessages::empty(aggregate_table);
    messages.append_messages_preserving_context(batch.into_messages());

    assert!(
        messages
            .render_path_contexts
            .as_ref()
            .is_none_or(|contexts| contexts.is_empty()),
        "a path-free diagnostic must not retain a caller-supplied path table"
    );
    let DiagnosticPayload::UnknownName { name, .. } = &messages.diagnostic_slice()[0].payload
    else {
        panic!("expected unknown-name diagnostic");
    };
    assert_eq!(
        messages.string_table.resolve(*name),
        "local",
        "the incoming name must follow the non-identity string merge"
    );
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        )
        .join("\n");
    assert!(
        rendered.contains("local"),
        "path-free rendering lost its authored name after aggregation: {rendered}"
    );
}

#[test]
fn attach_rejects_unresolvable_unused_path_table_node() {
    // The required path is valid in the batch string domain, but the retained table also carries
    // an unused node whose component ID is outside that domain. Retained tables are remapped in
    // full during aggregation, so this must fail at attachment rather than panic later.
    let mut batch_table = StringTable::new();
    batch_table.intern("local");
    batch_table.intern("leaf");
    let mut batch_fork = PathInternerFork::empty();
    let required_path = batch_fork
        .try_intern_portable_path("local/leaf", &mut batch_table)
        .expect("required path fits");
    let diagnostic = CompilerDiagnostic::missing_import_target(required_path, None);
    let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, batch_table);

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

    let error = batch
        .attach_path_table_if_missing(Arc::new(candidate_fork.snapshot_table()))
        .expect_err("an unresolvable unused node must reject a retained table");
    assert!(
        error.msg.contains("path node"),
        "the producer boundary should identify the invalid retained node: {}",
        error.msg
    );
}

#[test]
fn not_exported_by_public_surface_remaps_path_and_surface_name_after_aggregation() {
    // The destination already owns a string at the source surface-name index. The diagnostic's
    // requested path and public surface name must both survive the non-identity aggregation and
    // the final frozen render boundary: the source context freezes into a FrozenIdentityContext
    // that consumes the aggregate table, so the exact user-facing names must be asserted through
    // the frozen owner rather than the mutable aggregate table after it is moved.
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
    let mut local_messages = CompilerMessages::from_diagnostic(diagnostic, local_table);
    local_messages
        .attach_path_table_if_missing(Arc::new(local_fork.snapshot_table()))
        .expect("requested path table must pair with the local string table");

    // The aggregate table already owns the string at the source surface-name index, so the merge
    // into it is a genuinely shifting remap. Building the frozen source domain from the aggregate
    // table keeps the frozen path trie inside the frozen string domain.
    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("collision-padding");
    let mut messages = CompilerMessages::empty(aggregate_table);
    messages.append_messages_preserving_context(local_messages);

    let project_path = Path::new("/project/main.moth");
    let registration = SourceRegistrationIndex::from_rows(std::iter::once((
        project_path,
        SourceKind::Compiler(SourceFileKind::Moth),
    )));
    let source_database = {
        let mut frozen_table = messages.string_table.as_ref().clone();
        let path_fork = PathInternerFork::empty();
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &registration,
            project_path,
            None,
            &mut frozen_table,
            path_fork.clone_path_builder(),
        )
        .expect("frozen source identity should build")
    };
    messages.set_source_database(Arc::new(source_database));
    let messages = messages
        .freeze_source_contexts()
        .expect("aggregation should freeze the attached source context");
    assert!(
        messages.frozen_identity_context_for_diagnostic(0).is_some(),
        "a real source context must freeze into an immutable identity owner"
    );

    let DiagnosticPayload::NotExportedByPublicSurface {
        requested_path,
        public_surface_name,
        ..
    } = &messages.diagnostic_slice()[0].payload
    else {
        panic!("expected public-surface diagnostic");
    };
    // The frozen context consumed the aggregate table during freezing; the surface name must
    // resolve through that immutable owner under the destination-colliding remapped ID.
    let frozen_identity = messages
        .frozen_identity_context_for_diagnostic(0)
        .expect("the frozen render boundary owns this diagnostic's strings");
    assert_eq!(
        frozen_identity.try_resolve_string(*public_surface_name),
        Some("surface-name"),
        "the surface name must follow the colliding destination string remap"
    );
    // Freezing consumes only string/source ownership; the remapped premerge path table remains
    // the renderer's path owner, now resolving components through the frozen strings.
    assert!(
        messages.path_table_for_diagnostic(0).is_some(),
        "the retained premerge path table must survive freezing to serve the rendered path"
    );
    assert_eq!(
        messages
            .diagnostic_render_context(0)
            .render_path(*requested_path),
        "module/root",
        "the requested path must render exactly through the frozen identity owner"
    );
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        )
        .join("\n");
    assert!(
        rendered.contains("Cannot bind 'module/root' from module 'surface-name'"),
        "public-surface path/name remapping rendered incorrectly: {rendered}"
    );
}

#[test]
fn attach_retains_table_for_nominal_type_render_context_without_path_payloads() {
    // TypeMismatch stores TypeIds rather than PathIds, but its nominal type name is rendered by
    // walking the attached path table. Type-render contexts therefore remain legitimate path
    // consumers even when payload-only classification sees no path identity.
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
        type_environment.builtins().int,
        TypeMismatchContext::Assignment,
        None,
    );
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table)
        .with_type_context_for_all_diagnostics(type_environment);
    messages
        .attach_path_table_if_missing(Arc::new(path_fork.snapshot_table()))
        .expect("type rendering must retain its paired path table");
    assert!(
        messages.path_table_for_diagnostic(0).is_some(),
        "type-render context must retain a path table even without a path payload"
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut aggregate = CompilerMessages::empty(aggregate_table);
    aggregate.append_messages_preserving_context(messages);
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &aggregate,
        )
        .join("\n");
    assert!(
        rendered.contains("expected NominalType, found Int"),
        "nominal type rendering lost its path context after aggregation: {rendered}"
    );
}

#[test]
fn batch_attaches_table_for_nominal_type_context_and_survives_canonical_round_trip() {
    // The batch-level attach gate duplicates the message-set retention rule: a table is retained
    // only when a payload renders a path or a type context resolves nominal names. A nominal
    // TypeMismatch payload stores TypeIds, so without the type-context arm the batch would drop
    // the table the renderer needs. The batch must retain it, survive the canonical
    // batch/diagnosed round trip, and still render the exact nominal name after a non-identity
    // append.
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
        type_environment.builtins().int,
        TypeMismatchContext::Assignment,
        None,
    );
    let batch = PremergeDiagnosticBatch::from_parts(
        vec![diagnostic],
        string_table,
        vec![RenderTypeContext {
            diagnostic_range: 0..1,
            type_environment,
        }],
        Vec::new(),
    );
    let mut batch = batch;
    batch
        .attach_path_table_if_missing(Arc::new(path_fork.snapshot_table()))
        .expect("type rendering must retain its paired path table at the batch boundary");

    let diagnosed = ModuleDiagnostics::from_batch(batch)
        .expect("a nominal type-context batch must classify as a diagnosed module");
    assert!(
        diagnosed.render_type_contexts().len() == 1,
        "the canonical round trip must keep the nominal type context"
    );
    let batch = diagnosed
        .into_batch()
        .expect("a diagnosed module without source contexts must return to the batch lane");
    let messages = batch.into_messages();
    assert!(
        messages.path_table_for_diagnostic(0).is_some(),
        "the batch's retained path table must move into the final vessel"
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut aggregate = CompilerMessages::empty(aggregate_table);
    aggregate.append_messages_preserving_context(messages);
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &aggregate,
        )
        .join("\n");
    assert!(
        rendered.contains("Type mismatch in assignment: expected NominalType, found Int"),
        "batch-level nominal type rendering lost its path context through the canonical \
         round trip and append: {rendered}"
    );
}

#[test]
fn attach_rejects_unresolved_rendered_ancestor() {
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
    let foreign_table = Arc::new(foreign_fork.snapshot_table());
    let diagnostic = CompilerDiagnostic::missing_import_target(rendered_leaf, None);
    let mut batch = PremergeDiagnosticBatch::from_diagnostic(diagnostic, batch_table);

    assert!(
        batch.attach_path_table_if_missing(foreign_table).is_err(),
        "an unresolved rendered ancestor must fail pairing"
    );
}

#[test]
fn compiler_messages_preserve_type_context_ranges_when_prepending_and_appending() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let point_path = path_fork
        .try_intern_portable_path("Point", &mut string_table)
        .expect("test path fits");
    let status_path = path_fork
        .try_intern_portable_path("Status", &mut string_table)
        .expect("test path fits");
    let path_table = Arc::new(path_fork.snapshot_table());

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
        None,
    );
    let warning = CompilerDiagnostic::unreachable_match_arm(None);
    let mut first_messages =
        CompilerMessages::from_diagnostics(vec![first_error], string_table.clone())
            .with_type_context_for_all_diagnostics(first_environment);
    first_messages
        .attach_path_table_if_missing(Arc::clone(&path_table))
        .expect("test path tables must pair with the test string table");
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
        None,
    );
    let mut second_messages = CompilerMessages::from_diagnostics(vec![second_error], string_table)
        .with_type_context_for_all_diagnostics(second_environment);
    second_messages
        .attach_path_table_if_missing(path_table)
        .expect("test path tables must pair with the test string table");

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
fn append_preserves_frozen_and_remaps_unfrozen_string_and_type_owners() {
    let mut frozen_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let frozen_name = frozen_table.intern("frozen-name");
    let frozen_path = path_fork
        .try_intern_portable_path("FrozenType", &mut frozen_table)
        .expect("test path fits");
    let frozen_registration =
        SourceRegistrationIndex::from_rows(std::iter::empty::<(&Path, SourceKind)>());
    let frozen_sources =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &frozen_registration,
            Path::new("frozen.moth"),
            None,
            &mut frozen_table,
            path_fork.clone_path_builder(),
        )
        .expect("frozen identity path table should build");
    let frozen_identity = Arc::new(FrozenIdentityContext::from_parts(
        frozen_table.clone(),
        frozen_sources,
    ));

    let make_frozen_messages = || {
        let local_table = frozen_table.clone();
        let mut type_environment = TypeEnvironment::new();
        let (_, frozen_type) = type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: frozen_path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });
        let diagnostics = vec![
            CompilerDiagnostic::unknown_value_name(frozen_name, None),
            CompilerDiagnostic::type_mismatch(
                frozen_type,
                type_environment.builtins().int,
                TypeMismatchContext::Assignment,
                None,
            ),
        ];
        let mut messages = CompilerMessages::from_diagnostics(diagnostics, local_table)
            .with_type_context_for_all_diagnostics(type_environment);
        messages.render_frozen_contexts.push(RenderFrozenContext {
            diagnostic_range: 0..2,
            identity: Arc::clone(&frozen_identity),
        });
        messages
    };

    let make_unfrozen_messages = || {
        let mut local_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let unfrozen_name = local_table.intern("unfrozen-name");
        let unfrozen_path = path_fork
            .try_intern_portable_path("UnfrozenType", &mut local_table)
            .expect("test path fits");
        let mut type_environment = TypeEnvironment::new();
        let (_, unfrozen_type) = type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: unfrozen_path,
            fields: Box::new([]),
            generic_parameters: None,
            const_record: false,
        });
        let mut messages = CompilerMessages::from_diagnostics(
            vec![
                CompilerDiagnostic::unknown_value_name(unfrozen_name, None),
                CompilerDiagnostic::type_mismatch(
                    unfrozen_type,
                    type_environment.builtins().int,
                    TypeMismatchContext::Assignment,
                    None,
                ),
            ],
            local_table,
        )
        .with_type_context_for_all_diagnostics(type_environment);
        messages
            .attach_path_table_if_missing(Arc::new(path_fork.snapshot_table()))
            .expect("test path tables must pair with the test string table");
        messages
    };

    let make_mixed_messages = || {
        let mut messages = make_frozen_messages();
        messages.append_messages_preserving_context(make_unfrozen_messages());
        messages
    };

    let mut standalone_messages = make_mixed_messages();
    let mut standalone_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    standalone_table.intern("standalone-collision");
    let standalone_remap = standalone_table.merge_from(standalone_messages.string_table.as_ref());
    standalone_messages.remap_string_ids(&standalone_remap);
    *standalone_messages.string_table = standalone_table;
    let standalone_rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &standalone_messages,
        )
        .join("\n");
    assert!(
        standalone_rendered.contains("frozen-name"),
        "{standalone_rendered}"
    );
    assert!(
        standalone_rendered.contains("unfrozen-name"),
        "{standalone_rendered}"
    );
    assert!(
        standalone_rendered.contains("expected FrozenType"),
        "{standalone_rendered}"
    );
    assert!(
        standalone_rendered.contains("expected UnfrozenType"),
        "{standalone_rendered}"
    );

    let mut aggregate_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    aggregate_table.intern("aggregate-collision");
    let mut messages = CompilerMessages::empty(aggregate_table);
    messages.append_messages_preserving_context(standalone_messages);
    // Repeat the same mixed-owner append. The second merge is non-identity for both local
    // tables, so this checks that ownership survives more than one aggregation step.
    messages.append_messages_preserving_context(make_mixed_messages());

    assert_eq!(
        messages
            .render_type_contexts()
            .iter()
            .map(|context| context.diagnostic_range.clone())
            .collect::<Vec<_>>(),
        vec![0..2, 2..4, 4..6, 6..8]
    );
    assert_eq!(
        messages
            .render_frozen_contexts
            .iter()
            .map(|context| context.diagnostic_range.clone())
            .collect::<Vec<_>>(),
        vec![0..2, 4..6]
    );

    let frozen_name_id = StringId::from_index(0);
    assert_eq!(
        messages.string_table.resolve(frozen_name_id),
        "aggregate-collision"
    );
    for diagnostic_index in [0, 4] {
        let DiagnosticPayload::UnknownName { name, .. } =
            &messages.diagnostic_slice()[diagnostic_index].payload
        else {
            panic!("expected frozen unknown-name diagnostic");
        };
        assert_eq!(*name, frozen_name_id);
        assert_eq!(
            messages
                .frozen_identity_context_for_diagnostic(diagnostic_index)
                .expect("frozen diagnostic owner")
                .try_resolve_string(*name),
            Some("frozen-name")
        );
    }
    for diagnostic_index in [2, 6] {
        let DiagnosticPayload::UnknownName { name, .. } =
            &messages.diagnostic_slice()[diagnostic_index].payload
        else {
            panic!("expected unfrozen unknown-name diagnostic");
        };
        assert_eq!(messages.string_table.resolve(*name), "unfrozen-name");
    }

    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        )
        .join("\n");
    assert!(rendered.contains("frozen-name"), "{rendered}");
    assert!(rendered.contains("unfrozen-name"), "{rendered}");
    assert!(rendered.contains("expected FrozenType"), "{rendered}");
    assert!(rendered.contains("expected UnfrozenType"), "{rendered}");
}

#[test]
fn append_and_remap_keep_frozen_path_tables_in_owner_domain() {
    let mut frozen_table = StringTable::new();
    let frozen_name = frozen_table.intern("frozen-name");
    let mut frozen_path_fork = PathInternerFork::empty();
    frozen_path_fork
        .try_intern_portable_path("FrozenType", &mut frozen_table)
        .expect("test path fits");
    let frozen_registration =
        SourceRegistrationIndex::from_rows(std::iter::empty::<(&Path, SourceKind)>());
    let frozen_sources =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &frozen_registration,
            Path::new("frozen.moth"),
            None,
            &mut frozen_table,
            frozen_path_fork.clone_path_builder(),
        )
        .expect("frozen identity path table should build");
    let frozen_identity = Arc::new(FrozenIdentityContext::from_parts(
        frozen_table.clone(),
        frozen_sources,
    ));

    // One local message set whose single path table serves both owner domains: rows 0..2 are
    // frozen facts and rows 2..4 are premerge facts extending the same cloned string table.
    let make_mixed_messages = || {
        let mut local_table = frozen_table.clone();
        let mut path_fork = PathInternerFork::empty();
        let frozen_path = path_fork
            .try_intern_portable_path("FrozenType", &mut local_table)
            .expect("test path fits");
        let unfrozen_name = local_table.intern("unfrozen-name");
        let unfrozen_path = path_fork
            .try_intern_portable_path("UnfrozenType", &mut local_table)
            .expect("test path fits");
        let diagnostics = vec![
            CompilerDiagnostic::missing_import_target(frozen_path, None),
            CompilerDiagnostic::unknown_value_name(frozen_name, None),
            CompilerDiagnostic::missing_import_target(unfrozen_path, None),
            CompilerDiagnostic::unknown_value_name(unfrozen_name, None),
        ];
        let mut messages = CompilerMessages::from_diagnostics(diagnostics, local_table);
        messages
            .attach_path_table_if_missing(Arc::new(path_fork.snapshot_table()))
            .expect("test path tables must pair with the test string table");
        messages.render_frozen_contexts.push(RenderFrozenContext {
            diagnostic_range: 0..2,
            identity: Arc::clone(&frozen_identity),
        });
        messages
    };

    let assert_owner_paths_render = |rendered: &str| {
        assert!(
            rendered.contains("Cannot resolve dependency 'FrozenType'."),
            "{rendered}"
        );
        assert!(
            rendered.contains("Cannot resolve dependency 'UnfrozenType'."),
            "{rendered}"
        );
        assert!(rendered.contains("frozen-name"), "{rendered}");
        assert!(rendered.contains("unfrozen-name"), "{rendered}");
    };

    let mut standalone_messages = make_mixed_messages();
    let mut standalone_table = StringTable::new();
    standalone_table.intern("standalone-collision");
    let standalone_remap = standalone_table.merge_from(standalone_messages.string_table.as_ref());
    standalone_messages.remap_string_ids(&standalone_remap);
    *standalone_messages.string_table = standalone_table;
    let standalone_rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &standalone_messages,
        )
        .join("\n");
    assert_owner_paths_render(&standalone_rendered);

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-collision");
    let mut messages = CompilerMessages::empty(aggregate_table);
    messages.append_messages_preserving_context(make_mixed_messages());
    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        )
        .join("\n");
    assert_owner_paths_render(&rendered);
}

fn colliding_owner_messages(
    name: &str,
    type_name: &str,
    primary: &Path,
    primary_text: &str,
    foreign_text: &str,
) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let name_id = string_table.intern(name);
    assert_eq!(name_id, StringId::from_index(0));
    let type_path = path_fork
        .try_intern_portable_path(type_name, &mut string_table)
        .expect("test path fits");

    let registration = SourceRegistrationIndex::from_rows(std::iter::once((
        primary,
        SourceKind::Compiler(SourceFileKind::Moth),
    )));
    let mut source_database =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &registration,
            primary,
            None,
            &mut string_table,
            path_fork.clone_path_builder(),
        )
        .expect("colliding owner source identities should build");
    let primary_source = source_database
        .get_by_canonical_path(primary)
        .expect("colliding owner entry should be registered")
        .id;
    assert_eq!(primary_source, SourceId::from_index(1));
    source_database
        .retain_text(primary_source, primary_text.to_owned())
        .expect("colliding owner source text should be retained");

    let mut foreign_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let foreign_path = Path::new("/package/foreign.moth");
    let foreign_registration = SourceRegistrationIndex::from_rows(std::iter::once((
        foreign_path,
        SourceKind::Compiler(SourceFileKind::Moth),
    )));
    let mut foreign_database =
        SourceDatabase::from_registration_index_sorted_by_logical_path_with_path_builder(
            &foreign_registration,
            foreign_path,
            None,
            &mut foreign_table,
            path_fork.clone_path_builder(),
        )
        .expect("foreign source identities should build");
    let foreign_source = foreign_database
        .get_by_canonical_path(foreign_path)
        .expect("foreign source should be registered")
        .id;
    assert_eq!(
        foreign_source, primary_source,
        "independently built domains should exercise colliding SourceIds"
    );
    foreign_database
        .retain_text(foreign_source, foreign_text.to_owned())
        .expect("foreign source text should be retained");
    let foreign_handle = FrozenIdentityHandle::new();
    foreign_handle
        .install(Arc::new(FrozenIdentityContext::from_parts(
            foreign_table,
            foreign_database,
        )))
        .expect("foreign identity should install once");

    let mut span_builder = ExtendedSpanBuilder::new();
    let primary_span = Some(exact_span(primary_source, 0, 1, &mut span_builder));
    let foreign_span = exact_span(foreign_source, 0, 1, &mut span_builder);

    let mut type_environment = TypeEnvironment::new();
    let (_, expected_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: type_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let unknown_name =
        CompilerDiagnostic::unknown_value_name(name_id, primary_span).with_labels(vec![
            DiagnosticLabel::secondary_with_frozen_identity(
                Some(foreign_span),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
                foreign_handle,
            ),
        ]);
    let type_mismatch = CompilerDiagnostic::type_mismatch(
        expected_type,
        type_environment.builtins().int,
        TypeMismatchContext::Assignment,
        primary_span,
    );

    let mut messages =
        CompilerMessages::from_diagnostics(vec![unknown_name, type_mismatch], string_table)
            .with_type_context_for_all_diagnostics(type_environment);
    messages.set_source_database(Arc::new(source_database));
    messages
}

fn assert_foreign_secondary_site(messages: &CompilerMessages, diagnostic_index: usize, line: &str) {
    let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
    let position = messages
        .diagnostic_render_context(diagnostic_index)
        .label_position(&diagnostic.labels[0])
        .expect("foreign secondary site should resolve through its installed owner");
    assert_eq!(position.line, line);
}

struct MixedFreezeExpectation<'a> {
    first_name: &'a str,
    second_name: &'a str,
    first_type: &'a str,
    second_type: &'a str,
    first_foreign_line: &'a str,
    second_foreign_line: &'a str,
}

fn assert_mixed_freeze_keeps_originating_owners(
    first: CompilerMessages,
    second: CompilerMessages,
    expected: MixedFreezeExpectation<'_>,
) {
    let first_len = first.diagnostic_slice().len();
    let mut messages = first;
    messages.append_messages_preserving_context(second);
    let messages = messages
        .freeze_source_contexts()
        .expect("mixed frozen and transitional owners should freeze together");

    let first_identity = messages
        .frozen_identity_context_for_diagnostic(0)
        .expect("first diagnostic should keep its originating frozen owner");
    assert_eq!(
        first_identity.try_resolve_string(StringId::from_index(0)),
        Some(expected.first_name)
    );
    assert_foreign_secondary_site(&messages, 0, expected.first_foreign_line);

    let second_identity = messages
        .frozen_identity_context_for_diagnostic(first_len)
        .expect("appended diagnostic should keep its originating frozen owner");
    assert_eq!(
        second_identity.try_resolve_string(StringId::from_index(0)),
        Some(expected.second_name)
    );
    assert_foreign_secondary_site(&messages, first_len, expected.second_foreign_line);

    let rendered =
        crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages(
            &messages,
        )
        .join("\n");
    assert!(rendered.contains(expected.first_name), "{rendered}");
    assert!(rendered.contains(expected.second_name), "{rendered}");
    assert!(rendered.contains(expected.first_type), "{rendered}");
    assert!(rendered.contains(expected.second_type), "{rendered}");
}

#[test]
fn freeze_keeps_existing_frozen_owners_when_later_rows_still_need_conversion() {
    let primary = Path::new("/project/main.moth");

    assert_mixed_freeze_keeps_originating_owners(
        colliding_owner_messages(
            "frozen-name",
            "FrozenType",
            primary,
            "frozen_primary_alpha",
            "frozen_foreign_omega",
        )
        .freeze_source_contexts()
        .expect("frozen owner should freeze before mixed aggregation"),
        colliding_owner_messages(
            "mutable-name",
            "MutableType",
            primary,
            "mutable_primary_gamma",
            "mutable_foreign_delta",
        ),
        MixedFreezeExpectation {
            first_name: "frozen-name",
            second_name: "mutable-name",
            first_type: "FrozenType",
            second_type: "MutableType",
            first_foreign_line: "frozen_foreign_omega",
            second_foreign_line: "mutable_foreign_delta",
        },
    );
    assert_mixed_freeze_keeps_originating_owners(
        colliding_owner_messages(
            "mutable-name",
            "MutableType",
            primary,
            "mutable_primary_gamma",
            "mutable_foreign_delta",
        ),
        colliding_owner_messages(
            "frozen-name",
            "FrozenType",
            primary,
            "frozen_primary_alpha",
            "frozen_foreign_omega",
        )
        .freeze_source_contexts()
        .expect("frozen owner should freeze before mixed aggregation"),
        MixedFreezeExpectation {
            first_name: "mutable-name",
            second_name: "frozen-name",
            first_type: "MutableType",
            second_type: "FrozenType",
            first_foreign_line: "mutable_foreign_delta",
            second_foreign_line: "frozen_foreign_omega",
        },
    );
}

#[test]
fn remap_string_ids_updates_payloads_labels_and_tokens() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let import_path = path_fork
        .try_intern_portable_path("lib.moth", &mut local_table)
        .expect("test path fits");
    let name = local_table.intern("Button");
    let alias = local_table.intern("AliasButton");
    let label_text = local_table.intern("temporary label");
    let source = SourceId::from_index(1);
    let mut span_builder = ExtendedSpanBuilder::new();
    let primary_span = Some(exact_span(source, 4, 3, &mut span_builder));
    let first_span = exact_span(source, 10, 2, &mut span_builder);

    let mut path_syntax = PathSyntaxTable::new();
    let path_id = path_syntax.push(import_path, first_span);
    let expected_token = CompilerDiagnostic::expected_token(
        TokenKind::Symbol(name),
        Some(TokenKind::Path(path_id)),
        primary_span,
    );
    let duplicate = CompilerDiagnostic::duplicate_declaration(name, Some(first_span), primary_span);
    let import = CompilerDiagnostic::import_name_collision(alias, Some(first_span), primary_span)
        .with_labels(vec![DiagnosticLabel::secondary(
            Some(first_span),
            Some(DiagnosticLabelMessage::RenderedText(label_text)),
        )]);
    let borrow = borrow_conflict_diagnostic(
        DiagnosticPlace::Local(name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        primary_span,
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![expected_token, duplicate, import, borrow]);

    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let diagnostics = bag.diagnostics();
    match &diagnostics[0].payload {
        DiagnosticPayload::ExpectedToken {
            expected,
            found: Some(found),
        } => {
            assert_eq!(expected.tag(), TokenTag::SYMBOL);
            assert_eq!(found.tag(), TokenTag::PATH);
            assert_eq!(found.data(), 0);
            assert_eq!(
                path_fork.render_portable(import_path, &local_table, &mut Vec::new()),
                "lib.moth"
            );
        }
        payload => panic!("unexpected expected-token payload: {payload:?}"),
    }

    match &diagnostics[1].payload {
        DiagnosticPayload::DuplicateDeclaration { name } => {
            assert_eq!(merged_table.resolve(*name), "Button");
        }
        payload => panic!("unexpected duplicate payload: {payload:?}"),
    }
    let previous_label = diagnostics[1]
        .labels
        .iter()
        .find(|label| label.message == Some(DiagnosticLabelMessage::PreviousDeclaration))
        .expect("duplicate declaration should carry a previous declaration label");
    assert_eq!(previous_label.span, Some(first_span));

    match &diagnostics[2].payload {
        DiagnosticPayload::ImportNameCollision { name } => {
            assert_eq!(merged_table.resolve(*name), "AliasButton");
        }
        payload => panic!("unexpected import payload: {payload:?}"),
    }
    assert_eq!(diagnostics[2].labels[0].span, Some(first_span));
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
    assert_eq!(diagnostics[0].primary_span, primary_span);
    assert_eq!(diagnostics[1].primary_span, primary_span);
    assert_eq!(diagnostics[2].primary_span, primary_span);
    assert_eq!(diagnostics[3].primary_span, primary_span);
}

#[test]
fn remap_string_ids_updates_legacy_dependency_replacement() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut local_table)
        .expect("test path fits");
    let replacement = local_table.intern("@vendor/drawing.js as drawing");

    let diagnostic =
        CompilerDiagnostic::legacy_dependency_clause(Some(replacement), span(source_path));

    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    match &bag.diagnostics()[0].payload {
        DiagnosticPayload::LegacyDependencyClause {
            replacement: Some(replacement),
            ..
        } => {
            assert_eq!(
                merged_table.resolve(*replacement),
                "@vendor/drawing.js as drawing",
                "migration replacement StringId must remain valid after table merge/remap"
            );
        }
        payload => panic!("unexpected migration payload after remap: {payload:?}"),
    }
}

#[test]
fn remap_string_ids_updates_compile_time_evaluation_operation() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut local_table)
        .expect("test path fits");
    let operation = local_table.intern("string equality comparison");

    let diagnostic = CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        Some(operation),
        span(source_path),
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    // Occupy the merged table first so the merge cannot be an identity remap. A stale operation id
    // would then silently resolve to another module's text instead of naming the real operation.
    let mut merged_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    merged_table.intern("a string owned by another module");
    merged_table.intern("a second string owned by another module");
    let remap = merged_table.merge_from(&local_table);
    assert!(
        !remap.is_identity(),
        "the fixture must exercise a shifting remap"
    );
    bag.remap_string_ids(&remap);

    let DiagnosticPayload::CompileTimeEvaluationError {
        operation: Some(operation),
        ..
    } = &bag.diagnostics()[0].payload
    else {
        panic!(
            "unexpected compile-time evaluation payload after remap: {:?}",
            bag.diagnostics()[0].payload
        );
    };
    assert_eq!(
        merged_table.resolve(*operation),
        "string equality comparison",
        "the operation name must survive a module table merge"
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
fn type_mismatch_constructor_carries_type_ids_without_rendering() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
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
            assert_eq!(expected, builtin_type_ids::INT);
            assert_eq!(found, builtin_type_ids::STRING);
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
}

#[test]
fn type_mismatch_terminal_guidance_renders_type_names_with_context() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::Declaration,
        span(source_path),
    );
    let path_table = path_fork.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);

    assert!(guidance.iter().any(|line| line == "Expected: Int"));
    assert!(guidance.iter().any(|line| line == "Found: String"));
    assert!(!guidance.iter().any(|line| line.contains("type id")));
}

#[test]
fn type_mismatch_terse_renderer_renders_type_names_with_context() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::type_mismatch(
        type_environment.builtins().int,
        type_environment.builtins().string,
        TypeMismatchContext::FunctionArgument,
        span(source_path),
    );
    let path_table = path_fork.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);
    let line = terse::format_terse_diagnostic_with_context(&diagnostic, render_context);

    assert!(line.contains("expected Int"));
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
        let message =
            terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");

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
        let message =
            terminal::format_payload_guidance(&diagnostic.payload, render_context).join("\n");

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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
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
                InvalidCollectionTypeReason::CapacityNotConstant,
                span(source_path),
            ),
            "Collection capacity must be a positive integer literal or the bare name of a visible compile-time `Int` constant.",
            "CapacityNotConstant",
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
        (
            CompilerDiagnostic::unsupported_operator_types(
                DiagnosticOperator::Add,
                builtin_type_ids::STRING,
                Some(builtin_type_ids::INT),
                span(source_path),
            ),
            "Operator `+` cannot concatenate",
            "Add",
        ),
        (
            CompilerDiagnostic::invalid_fallible_operand(
                InvalidFallibleOperandReason::FallibleValueNotHandled,
                UnsupportedOperatorCategory::Arithmetic,
                builtin_type_ids::STRING,
                span(source_path),
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
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                span(source_path),
            ),
            "Each const-record parameter needs a value. Write `name = value`.",
            "AnonymousRecordFieldNotNamed",
        ),
        (
            CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::NestedAnonymousConstRecord,
                span(source_path),
            ),
            "Declare the inner struct or record first, then use that name as a parameter value.",
            "NestedAnonymousConstRecord",
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
        CompilerDiagnostic::invalid_fallible_operand(
            InvalidFallibleOperandReason::FallibleValueNotHandled,
            UnsupportedOperatorCategory::Arithmetic,
            builtin_type_ids::STRING,
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
            type_environment.builtins().int,
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
        borrow_conflict_diagnostic(
            DiagnosticPlace::Local(value_name),
            BorrowAccessKind::Shared,
            BorrowAccessKind::Mutable,
            span(source_path),
        ),
        CompilerDiagnostic::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::UnsupportedScalarValue,
            span(source_path),
        ),
        CompilerDiagnostic::deferred_feature(feature_name, span(source_path)),
    ];
    let path_table = path_fork.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);

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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");
    let diagnostic = CompilerDiagnostic::incompatible_choice_comparison(
        IncompatibleChoiceComparisonReason::ChoiceWithNonChoice,
        builtin_type_ids::BOOL,
        builtin_type_ids::INT,
        span(source_path),
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", string_table)
        .expect("test path fits");
    let callee = string_table.intern(callee_name);
    let diagnostic =
        CompilerDiagnostic::invalid_call_shape(reason, Some(callee), span(source_path));
    let render_context = DiagnosticRenderContext::new(string_table);
    terminal::format_payload_guidance(&diagnostic.payload, render_context)
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
fn invalid_call_shape_remap_updates_binding_name_and_parameter_name() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut local_table)
        .expect("test path fits");
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
        span(source_path),
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    terminal::format_payload_guidance(&diagnostic.payload, render_context)
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
fn invalid_receiver_call_remap_updates_receiver_binding_name() {
    let mut local_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut local_table)
        .expect("test path fits");
    let method = local_table.intern("move");
    let binding = local_table.intern("p");

    let diagnostic = CompilerDiagnostic::invalid_receiver_call(
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        None,
        Some(method),
        Some(ReceiverCallKind::SourceMethod),
        Some(binding),
        span(source_path),
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("test path fits");

    let diagnostic = CompilerDiagnostic::type_mismatch(
        builtin_type_ids::INT,
        builtin_type_ids::STRING,
        TypeMismatchContext::Declaration,
        span(source_path),
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
    let span = None;
    let type_environment = TypeEnvironment::new();

    let diagnostic = CompilerDiagnostic::invalid_generic_instantiation(
        Some(function_name),
        InvalidGenericInstantiationReason::ConflictingInference {
            subject: crate::compiler_frontend::compiler_messages::GenericInferenceSubject::Function,
            parameter_id: GenericParameterId(0),
            parameter_name,
            existing_type_id: type_environment.builtins().int,
            replacement_type_id: type_environment.builtins().string,
        },
        span,
    );

    let path_table = path_fork.snapshot_table();
    let render_context = DiagnosticRenderContext::new(&string_table)
        .with_optional_type_environment(Some(&type_environment))
        .with_path_table(&path_table);
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
    let _path_fork = PathInternerFork::empty();
    let int_name = string_table.intern("Int");

    let diagnostic = CompilerDiagnostic::invalid_builtin_call(
        InvalidBuiltinCallReason::CastMissingArgument,
        Some(int_name),
        None,
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
    let _path_fork = PathInternerFork::empty();
    let name = string_table.intern("item");
    let span = None;

    let expected = CompilerDiagnostic::expected_token(
        TokenKind::OpenParenthesis,
        Some(TokenKind::Symbol(name)),
        span,
    );
    let unexpected = CompilerDiagnostic::unexpected_token(TokenKind::OpenCurly, span);

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
    let _path_fork = PathInternerFork::empty();
    let value_name = string_table.intern("value");
    let span = None;

    let diagnostic = borrow_conflict_diagnostic(
        DiagnosticPlace::Local(value_name),
        BorrowAccessKind::Shared,
        BorrowAccessKind::Mutable,
        span,
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
fn remapping_diagnostic_strings_leaves_source_spans_unchanged() {
    let mut local_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let name = local_table.intern("Button");
    let label_text = local_table.intern("previous declaration");
    let primary_source = SourceId::from_index(3);
    let related_source = SourceId::from_index(4);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span(primary_source, 4, 6, &mut builder);
    let related = exact_span(related_source, 18, 5, &mut builder);

    let diagnostic = CompilerDiagnostic::duplicate_declaration(name, Some(related), Some(primary))
        .with_labels(vec![DiagnosticLabel::secondary(
            Some(related),
            Some(DiagnosticLabelMessage::RenderedText(label_text)),
        )]);
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);

    let mut merged_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    merged_table.intern("preexisting string");
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let diagnostic = &bag.diagnostics()[0];
    assert_eq!(diagnostic.primary_span, Some(primary));
    assert_eq!(diagnostic.labels[0].span, Some(related));
    match &diagnostic.payload {
        DiagnosticPayload::DuplicateDeclaration {
            name: remapped_name,
        } => {
            assert_eq!(merged_table.resolve(*remapped_name), "Button");
        }
        payload => panic!("unexpected duplicate payload: {payload:?}"),
    }
    match diagnostic.labels[0].message {
        Some(DiagnosticLabelMessage::RenderedText(remapped_text)) => {
            assert_eq!(merged_table.resolve(remapped_text), "previous declaration");
        }
        ref message => panic!("unexpected label message: {message:?}"),
    }
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
            Self::InvalidFallibleOperand,
            Self::IncompatibleChoiceComparison,
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
