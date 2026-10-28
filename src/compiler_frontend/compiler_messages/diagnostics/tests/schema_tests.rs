//! Schema rows: declared layouts, reserved flags, registry lookup and stable reason keys.
//!
//! WHAT: checks family layouts, registry lookup and stable keys shared by multi-bind codes and facts.
//! WHY: a layout or key drift would otherwise surface only at render time, far from the producer
//! that caused it.

use crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::UnitCode;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    self, INVALID_MULTI_BIND, InvalidMultiBindCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    NON_EXHAUSTIVE_MATCH, UNREACHABLE_MATCH_ARM,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    self, INVALID_EXTERNAL_MODULE, InvalidExternalModuleCode, InvalidExternalModuleView,
    MISSING_IMPORT_TARGET, UNUSED_IMPORT,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, InvalidDeclarationView, UnknownValueNameView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    self, InvalidConfigView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    COMPILER_CAPACITY_EXCEEDED, EXPECTED_TOKEN,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    self, TYPE_MISMATCH, USE_OF_UNINITIALIZED_LOCAL, UnsupportedBackendFeatureView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::reason_keys::StableReasonKey;
use crate::compiler_frontend::compiler_messages::diagnostics::reason_keys::stable_reason_keys_for_tests;
use crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode;
use crate::compiler_frontend::compiler_messages::diagnostics::registry::family_for_code;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    compact_bag_for_test, exact_span_in_source,
};
use crate::compiler_frontend::compiler_messages::is_well_formed_reason_key;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticSeverity, InvalidCollectionTypeReason, InvalidConfigReason,
    InvalidDeclarationReason, InvalidExternalModuleReason, InvalidMapTypeReason,
    InvalidMultiBindReason, InvalidOutputFolderReason, InvalidTypeAnnotationReason,
    NonExhaustiveMatchReason, TypeAnnotationContext, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::collections::HashSet;

#[test]
fn family_layouts_declare_their_word_counts() {
    assert_eq!(EXPECTED_TOKEN.word_count(), 4);
    assert_eq!(EXPECTED_TOKEN.pending_mask(), 0);
    assert_eq!(TYPE_MISMATCH.word_count(), 3);
    assert_eq!(TYPE_MISMATCH.pending_mask(), 0b11);
    assert_eq!(INVALID_MULTI_BIND.word_count(), 4);
    assert_eq!(USE_OF_UNINITIALIZED_LOCAL.word_count(), 1);
    assert_eq!(COMPILER_CAPACITY_EXCEEDED.word_count(), 1);
    assert_eq!(UNREACHABLE_MATCH_ARM.word_count(), 0);
}

#[test]
fn label_carrying_families_declare_their_extra_data_kind() {
    assert!(matches!(
        NON_EXHAUSTIVE_MATCH.extra,
        ExtraDataKind::StringIds
    ));
    assert!(matches!(UNREACHABLE_MATCH_ARM.extra, ExtraDataKind::None));
}

#[test]
fn reserved_and_active_rows_are_flagged() {
    assert!(UNUSED_IMPORT.is_reserved());
    assert!(!MISSING_IMPORT_TARGET.is_reserved());
}

#[test]
fn registry_finds_sampled_family_codes_and_no_other() {
    for code in [
        1001, 1037, 2001, 3002, 3022, 3034, 3035, 3045, 3050, 4002, 4005, 5009,
    ] {
        assert!(
            family_for_code(DiagnosticCode::new(code)).is_some(),
            "family code {code} is registered"
        );
    }
    assert!(family_for_code(DiagnosticCode::new(9999)).is_none());
}

#[test]
fn multi_bind_codes_are_dense_unique_and_well_formed() {
    let mut keys = HashSet::new();
    for code in 1..=15 {
        let key = InvalidMultiBindCode::from_code(code)
            .expect("codes 1 to 15 are assigned")
            .stable_reason_key();
        assert!(is_well_formed_reason_key(key), "{key} is well formed");
        assert!(
            stable_reason_keys_for_tests().contains(&key),
            "{key} is in the shared key table"
        );
        keys.insert(key);
    }
    assert_eq!(keys.len(), 15);
    assert!(InvalidMultiBindCode::from_code(0).is_none());
    assert!(InvalidMultiBindCode::from_code(16).is_none());
}

#[test]
fn multi_bind_reason_keys_remain_stable() {
    let cases = [
        (
            InvalidMultiBindReason::ThisTargetReserved,
            "invalid_multi_bind.this_target_reserved",
        ),
        (
            InvalidMultiBindReason::ExpectedTargetName,
            "invalid_multi_bind.expected_target_name",
        ),
        (
            InvalidMultiBindReason::MissingTargetAfterComma,
            "invalid_multi_bind.missing_target_after_comma",
        ),
        (
            InvalidMultiBindReason::MissingAssignmentOperator,
            "invalid_multi_bind.missing_assignment_operator",
        ),
        (
            InvalidMultiBindReason::InvalidTokenAfterTarget,
            "invalid_multi_bind.invalid_token_after_target",
        ),
        (
            InvalidMultiBindReason::MissingRightHandExpression,
            "invalid_multi_bind.missing_right_hand_expression",
        ),
        (
            InvalidMultiBindReason::MultipleRightHandExpressions,
            "invalid_multi_bind.multiple_right_hand_expressions",
        ),
        (
            InvalidMultiBindReason::MutableTargetNeedsExplicitType,
            "invalid_multi_bind.mutable_target_needs_explicit_type",
        ),
        (
            InvalidMultiBindReason::CompileTimeMarkerOnTarget,
            "invalid_multi_bind.compile_time_marker_on_target",
        ),
        (
            InvalidMultiBindReason::DuplicateTarget,
            "invalid_multi_bind.duplicate_target",
        ),
        (
            InvalidMultiBindReason::UnsupportedRhs,
            "invalid_multi_bind.unsupported_rhs",
        ),
        (
            InvalidMultiBindReason::ExistingTargetMutableMarker,
            "invalid_multi_bind.existing_target_mutable_marker",
        ),
        (
            InvalidMultiBindReason::ExistingTargetImmutable,
            "invalid_multi_bind.existing_target_immutable",
        ),
        (
            InvalidMultiBindReason::ArityMismatch {
                expected: 2,
                found: 1,
            },
            "invalid_multi_bind.arity_mismatch",
        ),
        (
            InvalidMultiBindReason::RhsNotMultiValue,
            "invalid_multi_bind.rhs_not_multi_value",
        ),
    ];

    assert_eq!(cases.len(), 15, "one case per multi-bind reason");
    for (reason, expected_key) in &cases {
        assert_eq!(reason.stable_reason_key(), *expected_key);
        assert_eq!(
            InvalidMultiBindCode::of(reason).stable_reason_key(),
            *expected_key
        );
    }
}

#[test]
fn invalid_external_module_reason_codec_and_key_are_stable() {
    let detail = StringId::from_index(7);
    let cases = [
        (
            InvalidExternalModuleReason::ReceiverMethod { moth_name: detail },
            Some(detail),
        ),
        (
            InvalidExternalModuleReason::ParserDiagnostic {
                parser_detail: detail,
            },
            Some(detail),
        ),
        (
            InvalidExternalModuleReason::NonUtf8Source {
                utf8_detail: detail,
            },
            Some(detail),
        ),
        (
            InvalidExternalModuleReason::PathContainsPackageSeparator,
            None,
        ),
    ];

    assert_eq!(INVALID_EXTERNAL_MODULE.word_count(), 3);
    for (reason, expected_detail) in cases {
        let code = InvalidExternalModuleCode::of(&reason);
        let (split_code, detail) = InvalidExternalModuleCode::split(&reason);
        assert_eq!(split_code, code);
        assert_eq!(detail, expected_detail);
        assert_eq!(code.to_reason(detail), Some(reason));
    }

    let expected_key = "invalid_external_module.path_contains_package_separator";
    let path_reason = InvalidExternalModuleReason::PathContainsPackageSeparator;
    let path_code = InvalidExternalModuleCode::of(&path_reason);
    assert_eq!(path_reason.stable_reason_key(), expected_key);
    assert_eq!(path_code.stable_reason_key(), expected_key);
    assert_eq!(
        InvalidExternalModuleCode::ReceiverMethod.to_reason(None),
        None
    );
    assert_eq!(
        InvalidExternalModuleCode::PathContainsPackageSeparator.to_reason(Some(detail)),
        None
    );
    let compact = compact_bag_for_test(
        DiagnosticBag::from(imports::invalid_external_module(
            PathId::ROOT,
            path_reason,
            None,
        )),
        None,
    );
    let view = compact
        .first()
        .expect("path-separator external-module draft compacts");
    assert_eq!(view.descriptor().code, "MOTH-IMPORT-0022");
    assert_eq!(view.reason_key(), Some(expected_key));
    let facts = view
        .family_view::<InvalidExternalModuleView>()
        .expect("external-module reason facts remain typed");
    assert_eq!(facts.reason_code, path_code);
    assert_eq!(facts.reason_detail, None);
}

#[test]
fn missing_option_patterns_reason_key_remains_stable() {
    assert_eq!(
        NonExhaustiveMatchReason::MissingOptionPatterns.stable_reason_key(),
        "non_exhaustive_match.missing_option_patterns"
    );
}

#[test]
fn every_authored_stable_reason_key_is_unique_and_well_formed() {
    let keys = stable_reason_keys_for_tests();
    let unique_keys = keys.iter().copied().collect::<HashSet<_>>();
    assert!(
        !keys.is_empty(),
        "the stable reason-key inventory must not be empty"
    );
    assert_eq!(
        keys.len(),
        unique_keys.len(),
        "stable reason keys must be globally unique"
    );
    for key in keys {
        assert!(
            is_well_formed_reason_key(key),
            "stable reason key '{key}' must use the qualified lower-snake-case format"
        );
    }
}

#[test]
fn diagnostic_identity_uses_descriptor_code_actual_severity_and_typed_reason() {
    let draft =
        calls_data::invalid_collection_type(InvalidCollectionTypeReason::ZeroCapacity, None)
            .with_severity(DiagnosticSeverity::Warning);
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let view = compact.first().expect("collection draft compacts");

    assert_eq!(view.descriptor().code, "MOTH-SYNTAX-0016");
    assert_eq!(view.severity(), DiagnosticSeverity::Warning);
    assert_eq!(
        view.reason_key(),
        Some("invalid_collection_type.zero_capacity")
    );
}

#[test]
fn unsupported_backend_feature_exposes_stable_reason_key() {
    let mut string_table = StringTable::new();
    let backend = string_table.intern("Wasm");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::unsupported_backend_feature(
            backend,
            UnsupportedBackendFeatureReason::HashmapOperation,
            None,
        )),
        None,
    );
    let view = compact.first().expect("unsupported-feature draft compacts");
    let facts = view
        .family_view::<UnsupportedBackendFeatureView>()
        .expect("backend feature facts remain typed");

    assert_eq!(view.descriptor().code, "MOTH-RULE-0064");
    assert_eq!(
        view.reason_key(),
        Some("unsupported_backend_feature.hashmap_operation")
    );
    assert_eq!(string_table.resolve(facts.backend_name), "Wasm");
}

#[test]
fn body_declaration_placement_preserves_source_name_and_stable_reason() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("nested");
    let mut span_builder = ExtendedSpanBuilder::new();
    let name_span = exact_span_in_source(SourceId::from_index(1), 12, 6, &mut span_builder);
    let compact = compact_bag_for_test(
        DiagnosticBag::from(names_declarations::invalid_declaration(
            InvalidDeclarationReason::DeclarationRequiresModuleTopLevel,
            Some(name),
            Some(name_span),
        )),
        None,
    );
    let view = compact.first().expect("invalid-declaration draft compacts");
    let facts = view
        .family_view::<InvalidDeclarationView>()
        .expect("declaration facts remain typed");

    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == name_span));
    assert_eq!(view.descriptor().code, "MOTH-RULE-0043");
    assert_eq!(
        view.reason_key(),
        Some("invalid_declaration.declaration_requires_module_top_level")
    );
    assert_eq!(
        string_table.resolve(facts.name.expect("source declaration name")),
        "nested"
    );
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::new(&string_table))
            .join("\n");
    assert!(guidance.contains("Declaration 'nested' cannot appear inside an executable body"));
    assert!(guidance.contains("Declare it at module top level"));
}

#[test]
fn mutable_function_parameter_feature_exposes_stable_reason_key() {
    let mut string_table = StringTable::new();
    let backend = string_table.intern("Wasm");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::unsupported_backend_feature(
            backend,
            UnsupportedBackendFeatureReason::MutableFunctionParameters,
            None,
        )),
        None,
    );
    let view = compact.first().expect("unsupported-feature draft compacts");
    assert!(
        view.family_view::<UnsupportedBackendFeatureView>()
            .is_some()
    );
    assert_eq!(view.descriptor().code, "MOTH-RULE-0064");
    assert_eq!(
        view.reason_key(),
        Some("unsupported_backend_feature.mutable_function_parameters")
    );
}

#[test]
fn non_utf8_output_folder_reason_has_stable_identity_and_rendering() {
    let string_table = StringTable::new();
    let reason = InvalidConfigReason::InvalidOutputFolder {
        folder: None,
        reason: InvalidOutputFolderReason::NonUtf8,
    };
    let compact = compact_bag_for_test(
        DiagnosticBag::from(project_config::invalid_config_reason(None, reason, None)),
        None,
    );
    let view = compact.first().expect("invalid-config draft compacts");
    assert!(view.family_view::<InvalidConfigView>().is_some());
    assert_eq!(
        view.reason_key(),
        Some("invalid_config.invalid_output_folder.non_utf8")
    );
    let rendered =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(&string_table));
    assert!(rendered.contains("'config' must use valid UTF-8 portable path components."));
}

#[test]
fn output_folder_collision_rendering_preserves_both_authored_spellings() {
    let mut string_table = StringTable::new();
    let reason = InvalidConfigReason::OutputFoldersNotDistinct {
        dev_folder: string_table.intern("Build"),
        release_folder: string_table.intern("build"),
    };
    let key = string_table.intern("dev_folder");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(project_config::invalid_config_reason(
            Some(key),
            reason,
            None,
        )),
        None,
    );
    let view = compact.first().expect("invalid-config draft compacts");
    let rendered =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(&string_table));
    assert!(rendered.contains("Development and release output folders 'Build' and 'build' resolve to the same output root and must be distinct."));
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
fn reasonless_families_have_no_reason_key() {
    let mut string_table = StringTable::new();
    let missing = string_table.intern("missing");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(names_declarations::unknown_value_name(missing, None)),
        None,
    );
    let view = compact.first().expect("unknown-name draft compacts");
    assert!(view.family_view::<UnknownValueNameView>().is_some());
    assert_eq!(view.reason_key(), None);
}

#[test]
fn reason_key_dispatch_covers_distinct_typed_reason_families() {
    let drafts = [
        names_declarations::invalid_type_annotation(
            TypeAnnotationContext::DeclarationTarget,
            InvalidTypeAnnotationReason::NoneNotAllowed,
            None,
        ),
        calls_data::invalid_map_type(InvalidMapTypeReason::FixedCapacityNotAllowed, None),
        calls_data::invalid_collection_type(InvalidCollectionTypeReason::CapacityOverflow, None),
    ];
    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(drafts.into()), None);
    let mut views = compact.views();
    assert_eq!(
        views.next().expect("type annotation").reason_key(),
        Some("invalid_type_annotation.none_not_allowed")
    );
    assert_eq!(
        views.next().expect("map type").reason_key(),
        Some("invalid_map_type.fixed_capacity_not_allowed")
    );
    assert_eq!(
        views.next().expect("collection type").reason_key(),
        Some("invalid_collection_type.capacity_overflow")
    );
    assert!(views.next().is_none());
}
