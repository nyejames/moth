//! Stable reason keys for compact diagnostic facts.
//!
//! WHAT: maps family reason values to their qualified identity and exposes the authored key inventory.
//! WHY: reason keys are compiler-owned identity independent of prose and family storage.

use super::reasons::*;

use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidCallShapeCode, InvalidCollectionTypeCode, InvalidMapTypeCode, InvalidMultiBindCode,
    InvalidReturnShapeCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidFallibleHandlingCode, InvalidLoopHeaderCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    GenericInstantiationShape, InvalidTraitConformanceShape,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    InvalidFunctionSignatureCode, InvalidGenericInstantiationCode, InvalidGenericParameterCode,
    InvalidParameterContractCode, InvalidReceiverDeclarationCode, InvalidThisUsageCode,
    InvalidTraitConformanceCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    InvalidExternalModuleCode, InvalidImportPathCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    DeferredFeatureCode, InvalidDeclarationCode, InvalidTypeAnnotationCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidCompileTimePathCode, InvalidConfigCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    CssTemplateWarningCode, HtmlTemplateWarningCode, InvalidTemplateDirectiveCode,
    InvalidTemplateStructureCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::UnexpectedTokenShape;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    CommonSyntaxMistakeCode, InvalidStringEscapeCode, NumberLiteralCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::IncompatibleChoiceComparisonCode;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

pub(crate) trait StableReasonKey {
    fn stable_reason_key(&self) -> &'static str;
}

/// A decoded diagnostic fact that may have no stable reason key for some variants.
pub(crate) trait OptionalStableReasonKey {
    fn optional_stable_reason_key(&self) -> Option<&'static str>;
}

impl<T: StableReasonKey> OptionalStableReasonKey for T {
    fn optional_stable_reason_key(&self) -> Option<&'static str> {
        Some(StableReasonKey::stable_reason_key(self))
    }
}

impl<Type> StableReasonKey for GenericInstantiationShape<Type> {
    fn stable_reason_key(&self) -> &'static str {
        self.code().stable_reason_key()
    }
}

impl<Type> StableReasonKey for InvalidTraitConformanceShape<Type> {
    fn stable_reason_key(&self) -> &'static str {
        self.code().stable_reason_key()
    }
}

impl OptionalStableReasonKey for UnexpectedTokenShape {
    fn optional_stable_reason_key(&self) -> Option<&'static str> {
        match self {
            UnexpectedTokenShape::Token(_) => None,
            UnexpectedTokenShape::MultiBind { code, .. } => Some(code.stable_reason_key()),
        }
    }
}

macro_rules! define_stable_reason_keys {
    (
        $(
            $reason_type:ident => {
                $(@delegate($delegate_pattern:pat => $delegate_expression:expr);)*
                $($pattern:pat => $key:literal),* $(,)?
            }
        ),+ $(,)?
    ) => {
        $(
            impl StableReasonKey for $reason_type {
                fn stable_reason_key(&self) -> &'static str {
                    match self {
                        $($delegate_pattern => $delegate_expression,)*
                        $($pattern => $key,)*
                    }
                }
            }
        )+

        #[cfg(test)]
        pub(super) fn stable_reason_keys_for_tests() -> &'static [&'static str] {
            &[
                $(
                    $(
                        $key,
                    )*
                )+
            ]
        }
    };
}

define_stable_reason_keys! {
    InvalidConfigReason => {
        @delegate(reason => InvalidConfigCode::of(reason).stable_reason_key());
    },

    InvalidConfigCode => {
        &InvalidConfigCode::MissingKey => "invalid_config.missing_key",
        &InvalidConfigCode::MissingConfigFile => "invalid_config.missing_config_file",
        &InvalidConfigCode::MissingProjectRecord => "invalid_config.missing_project_record",
        &InvalidConfigCode::MissingActiveBuilderSection => "invalid_config.missing_active_builder_section",
        &InvalidConfigCode::DuplicateKey => "invalid_config.duplicate_key",
        &InvalidConfigCode::FunctionUnsupported => "invalid_config.function_unsupported",
        &InvalidConfigCode::TraitDeclarationUnsupported => "invalid_config.trait_declaration_unsupported",
        &InvalidConfigCode::TraitConformanceUnsupported => "invalid_config.trait_conformance_unsupported",
        &InvalidConfigCode::TraitIncompatibilityUnsupported => "invalid_config.trait_incompatibility_unsupported",
        &InvalidConfigCode::NamedTypeUnsupported => "invalid_config.named_type_unsupported",
        &InvalidConfigCode::MutableBindingUnsupported => "invalid_config.mutable_binding_unsupported",
        &InvalidConfigCode::PlainBindingUnsupported => "invalid_config.plain_binding_unsupported",
        &InvalidConfigCode::UnsupportedStatement => "invalid_config.unsupported_statement",
        &InvalidConfigCode::StandaloneTemplateUnsupported => "invalid_config.standalone_template_unsupported",
        &InvalidConfigCode::MissingValue => "invalid_config.missing_value",
        &InvalidConfigCode::UnsupportedScalarValue => "invalid_config.unsupported_scalar_value",
        &InvalidConfigCode::NotCompileTimeConstant => "invalid_config.not_compile_time_constant",
        &InvalidConfigCode::ValueCouldNotFold => "invalid_config.value_could_not_fold",
        &InvalidConfigCode::EmptyProjectSetting => "invalid_config.empty_project_setting",
        &InvalidConfigCode::ConfigQualifierFixedField => "invalid_config.config_qualifier_fixed_field",
        &InvalidConfigCode::ProjectGlobalsNameReserved => "invalid_config.project_globals_name_reserved",
        &InvalidConfigCode::ConfigContractNameInvalid => "invalid_config.config_contract_name_invalid",
        &InvalidConfigCode::ConfigQualifierInvalidPlacement => "invalid_config.config_qualifier_invalid_placement",
        &InvalidConfigCode::ConfigQualifierUnsupportedType => "invalid_config.config_qualifier_unsupported_type",
        &InvalidConfigCode::ConfigContractConflict => "invalid_config.config_contract_conflict",
        &InvalidConfigCode::UnknownBuildConfigInput => "invalid_config.unknown_build_config_input",
        &InvalidConfigCode::ConfigInputTypeMismatch => "invalid_config.config_input_type_mismatch",
        &InvalidConfigCode::ConfigInputOutOfRangeSeparatorNotBetweenDigits
        | &InvalidConfigCode::ConfigInputOutOfRangeMultipleDecimalPoints
        | &InvalidConfigCode::ConfigInputOutOfRangeDecimalPointNotAfterDigit
        | &InvalidConfigCode::ConfigInputOutOfRangeEndsWithSeparator
        | &InvalidConfigCode::ConfigInputOutOfRangeMissingFractionalDigits
        | &InvalidConfigCode::ConfigInputOutOfRangeUppercaseExponentMarker
        | &InvalidConfigCode::ConfigInputOutOfRangeMissingExponentDigits
        | &InvalidConfigCode::ConfigInputOutOfRangeInvalidExponentSignPlacement
        | &InvalidConfigCode::ConfigInputOutOfRangeInvalidSeparatorPlacement
        | &InvalidConfigCode::ConfigInputOutOfRangeParseOverflow
        | &InvalidConfigCode::ConfigInputOutOfRangeOutsideFixedScalarRange
        | &InvalidConfigCode::ConfigInputOutOfRangeNegativeUnsignedLiteral
        | &InvalidConfigCode::ConfigInputOutOfRangeNonFiniteFixedFloat
        | &InvalidConfigCode::ConfigInputOutOfRangeInexactNumberScale => "invalid_config.config_input_out_of_range",
        &InvalidConfigCode::MissingConfigInput => "invalid_config.missing_config_input",
        &InvalidConfigCode::UnknownKey => "invalid_config.unknown_key",
        &InvalidConfigCode::UnknownRecordField => "invalid_config.unknown_record_field",
        &InvalidConfigCode::MissingRequiredRecordField => "invalid_config.missing_required_record_field",
        &InvalidConfigCode::InvalidConfigValueShape => "invalid_config.invalid_config_value_shape",
        &InvalidConfigCode::InvalidProjectSettingValue => "invalid_config.invalid_project_setting_value",
        &InvalidConfigCode::MissingHtmlHomepage => "invalid_config.missing_html_homepage",
        &InvalidConfigCode::DuplicateHtmlOutputPath => "invalid_config.duplicate_html_output_path",
        &InvalidConfigCode::ResourceOutputPathCollision => "invalid_config.resource_output_path_collision",
        &InvalidConfigCode::ResourceOutputPathReserved => "invalid_config.resource_output_path_reserved",
        &InvalidConfigCode::ConfiguredEntryRootMissing => "invalid_config.configured_entry_root_missing",
        &InvalidConfigCode::EntryRootPackagePrefixCollision => "invalid_config.entry_root_package_prefix_collision",
        &InvalidConfigCode::SourcePackageMissingRoot => "invalid_config.source_package_missing_root",
        &InvalidConfigCode::SourcePackageMultipleRoots => "invalid_config.source_package_multiple_roots",
        &InvalidConfigCode::NoRootModuleEntries => "invalid_config.no_root_module_entries",
        &InvalidConfigCode::MultipleModuleRootFiles => "invalid_config.multiple_module_root_files",
        &InvalidConfigCode::ConfigImportUnsupported => "invalid_config.config_import_unsupported",
        &InvalidConfigCode::FileValuePathUnsupported => "invalid_config.file_value_path_unsupported",
        &InvalidConfigCode::SourceFileFolderCollision => "invalid_config.source_file_folder_collision",
        &InvalidConfigCode::LegacyModuleRootFileName => "invalid_config.legacy_module_root_file_name",
        &InvalidConfigCode::InvalidOutputFolderEmpty => "invalid_config.invalid_output_folder.empty",
        &InvalidConfigCode::InvalidOutputFolderNonUtf8 => "invalid_config.invalid_output_folder.non_utf8",
        &InvalidConfigCode::InvalidOutputFolderAbsolutePathWithoutFolder
        | &InvalidConfigCode::InvalidOutputFolderAbsolutePathWithFolder => "invalid_config.invalid_output_folder.absolute_path",
        &InvalidConfigCode::InvalidOutputFolderRootOrPrefix => "invalid_config.invalid_output_folder.root_or_prefix",
        &InvalidConfigCode::InvalidOutputFolderParentDirectorySegmentWithoutFolder
        | &InvalidConfigCode::InvalidOutputFolderParentDirectorySegmentWithFolder => "invalid_config.invalid_output_folder.parent_directory_segment",
        &InvalidConfigCode::InvalidOutputFolderCurrentDirectory => "invalid_config.invalid_output_folder.current_directory",
        &InvalidConfigCode::InvalidOutputFolderInvalidPathComponentWithoutFolder
        | &InvalidConfigCode::InvalidOutputFolderInvalidPathComponentWithFolder => "invalid_config.invalid_output_folder.invalid_path_component",
        &InvalidConfigCode::InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder
        | &InvalidConfigCode::InvalidOutputFolderInsideOrEqualToEntryRootWithFolder => "invalid_config.invalid_output_folder.inside_or_equal_to_entry_root",
        &InvalidConfigCode::InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder
        | &InvalidConfigCode::InvalidOutputFolderResolvesOutsideProjectRootWithFolder => "invalid_config.invalid_output_folder.resolves_outside_project_root",
        &InvalidConfigCode::OutputFoldersNotDistinct => "invalid_config.output_folders_not_distinct",
        &InvalidConfigCode::OutputManifestOwnerConflict => "invalid_config.output_manifest_owner_conflict",
        &InvalidConfigCode::InvalidArtefactOutputPathEmpty => "invalid_config.invalid_artefact_output_path.empty",
        &InvalidConfigCode::InvalidArtefactOutputPathNonUtf8 => "invalid_config.invalid_artefact_output_path.non_utf8",
        &InvalidConfigCode::InvalidArtefactOutputPathAbsolutePath => "invalid_config.invalid_artefact_output_path.absolute_path",
        &InvalidConfigCode::InvalidArtefactOutputPathRootOrPrefix => "invalid_config.invalid_artefact_output_path.root_or_prefix",
        &InvalidConfigCode::InvalidArtefactOutputPathParentDirectorySegment => "invalid_config.invalid_artefact_output_path.parent_directory_segment",
        &InvalidConfigCode::InvalidArtefactOutputPathCurrentDirectory => "invalid_config.invalid_artefact_output_path.current_directory",
        &InvalidConfigCode::InvalidArtefactOutputPathInvalidPathComponent => "invalid_config.invalid_artefact_output_path.invalid_path_component",
        &InvalidConfigCode::InvalidArtefactOutputPathInsideOrEqualToEntryRoot => "invalid_config.invalid_artefact_output_path.inside_or_equal_to_entry_root",
        &InvalidConfigCode::InvalidArtefactOutputPathResolvesOutsideProjectRoot => "invalid_config.invalid_artefact_output_path.resolves_outside_project_root",
    },

    InvalidOutputFolderReason => {
        @delegate(reason => InvalidConfigCode::output_folder_code(*reason).stable_reason_key());
    },

    InvalidMutableAccessReason => {
    &InvalidMutableAccessReason::ImmutablePlace => "invalid_mutable_access.immutable_place",
    &InvalidMutableAccessReason::OverlappingAccess => "invalid_mutable_access.overlapping_access",
    &InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess => "invalid_mutable_access.aliased_value_requires_exclusive_access",
    },

    UnsupportedBackendFeatureReason => {
    &UnsupportedBackendFeatureReason::ConstructorRoutes => "unsupported_backend_feature.constructor_routes",
    &UnsupportedBackendFeatureReason::HashmapConstruction => "unsupported_backend_feature.hashmap_construction",
    &UnsupportedBackendFeatureReason::HashmapOperation => "unsupported_backend_feature.hashmap_operation",
    &UnsupportedBackendFeatureReason::RuntimeCasts => "unsupported_backend_feature.runtime_casts",
    &UnsupportedBackendFeatureReason::CheckedNumericOperations => "unsupported_backend_feature.checked_numeric_operations",
    &UnsupportedBackendFeatureReason::RecoverableNumericFailure => "unsupported_backend_feature.recoverable_numeric_failure",
    &UnsupportedBackendFeatureReason::FloatFormatting => "unsupported_backend_feature.float_formatting",
    &UnsupportedBackendFeatureReason::FloatBoundaryValidation => "unsupported_backend_feature.float_boundary_validation",
    &UnsupportedBackendFeatureReason::GenericRuntimeValues => "unsupported_backend_feature.generic_runtime_values",
    &UnsupportedBackendFeatureReason::FixedWidthScalarValues => "unsupported_backend_feature.fixed_width_scalar_values",
    &UnsupportedBackendFeatureReason::CrossModuleCalls => "unsupported_backend_feature.cross_module_calls",
    &UnsupportedBackendFeatureReason::RuntimeAssertionMessages => "unsupported_backend_feature.runtime_assertion_messages",
    &UnsupportedBackendFeatureReason::ErrorValues => "unsupported_backend_feature.error_values",
    &UnsupportedBackendFeatureReason::FallibleControlFlow => "unsupported_backend_feature.fallible_control_flow",
    &UnsupportedBackendFeatureReason::MutableFunctionParameters => "unsupported_backend_feature.mutable_function_parameters",
    &UnsupportedBackendFeatureReason::MutableLocalBindingAliases => "unsupported_backend_feature.mutable_local_binding_aliases",
    &UnsupportedBackendFeatureReason::NumberValues => "unsupported_backend_feature.number_values",
    &UnsupportedBackendFeatureReason::VariantPayloadValues => "unsupported_backend_feature.variant_payload_values",
    &UnsupportedBackendFeatureReason::UnaryOperators => "unsupported_backend_feature.unary_operators",
    &UnsupportedBackendFeatureReason::NonEmptyCollections => "unsupported_backend_feature.non_empty_collections",
    &UnsupportedBackendFeatureReason::StructRangeVariantPayloadExpressions => "unsupported_backend_feature.struct_range_variant_payload_expressions",
    &UnsupportedBackendFeatureReason::TemplateChunkTypes => "unsupported_backend_feature.template_chunk_types",
    &UnsupportedBackendFeatureReason::ProjectedPlaces => "unsupported_backend_feature.projected_places",
    &UnsupportedBackendFeatureReason::LinkedFunctionTargets => "unsupported_backend_feature.linked_function_targets",
    &UnsupportedBackendFeatureReason::MatchTerminators => "unsupported_backend_feature.match_terminators",
    &UnsupportedBackendFeatureReason::ProjectedAssignmentTargets => "unsupported_backend_feature.projected_assignment_targets",
    },

    NumberLiteralErrorReason => {
        @delegate(reason => NumberLiteralCode::split(reason).0.stable_reason_key());
    },

    NumberLiteralCode => {
    &NumberLiteralCode::SeparatorNotBetweenDigits => "invalid_number_literal.separator_not_between_digits",
    &NumberLiteralCode::MultipleDecimalPoints => "invalid_number_literal.multiple_decimal_points",
    &NumberLiteralCode::DecimalPointNotAfterDigit => "invalid_number_literal.decimal_point_not_after_digit",
    &NumberLiteralCode::EndsWithSeparator => "invalid_number_literal.ends_with_separator",
    &NumberLiteralCode::MissingFractionalDigits => "invalid_number_literal.missing_fractional_digits",
    &NumberLiteralCode::UppercaseExponentMarker => "invalid_number_literal.uppercase_exponent_marker",
    &NumberLiteralCode::MissingExponentDigits => "invalid_number_literal.missing_exponent_digits",
    &NumberLiteralCode::InvalidExponentSignPlacement => "invalid_number_literal.invalid_exponent_sign_placement",
    &NumberLiteralCode::InvalidSeparatorPlacement => "invalid_number_literal.invalid_separator_placement",
    &NumberLiteralCode::OutsideFixedScalarRange => "invalid_number_literal.outside_fixed_scalar_range",
    &NumberLiteralCode::NegativeUnsignedLiteral => "invalid_number_literal.negative_unsigned_literal",
    &NumberLiteralCode::NonFiniteFixedFloat => "invalid_number_literal.non_finite_float",
    &NumberLiteralCode::InexactNumberScale => "invalid_number_literal.inexact_number_scale",
    &NumberLiteralCode::ParseOverflow => "invalid_number_literal.parse_overflow",
    },
    InvalidStringEscapeReason => {
        @delegate(reason => InvalidStringEscapeCode::of(reason).stable_reason_key());
    },
    InvalidStringEscapeCode => {
    &InvalidStringEscapeCode::UnsupportedEscape => "invalid_string_escape.unsupported_escape",
    &InvalidStringEscapeCode::PhysicalNewline => "invalid_string_escape.physical_newline",
    &InvalidStringEscapeCode::TrailingBackslash => "invalid_string_escape.trailing_backslash",
    },

    GenericApplicationErrorReason => {
    &GenericApplicationErrorReason::OnNonNamedType => "invalid_generic_application.on_non_named_type",
    &GenericApplicationErrorReason::EmptyArgumentList => "invalid_generic_application.empty_argument_list",
    &GenericApplicationErrorReason::MissingArgumentAfterComma => "invalid_generic_application.missing_argument_after_comma",
    &GenericApplicationErrorReason::NestedApplication => "invalid_generic_application.nested_application",
    },
    InvalidParameterContractReason => {
        @delegate(reason => InvalidParameterContractCode::of(reason).stable_reason_key());
    },
    InvalidParameterContractCode => {
        &InvalidParameterContractCode::MissingSpecification => "invalid_parameter_contract.missing_specification",
        &InvalidParameterContractCode::InvalidPosition => "invalid_parameter_contract.invalid_position",
        &InvalidParameterContractCode::WireArrowNotAllowed => "invalid_parameter_contract.wire_arrow_not_allowed",
        &InvalidParameterContractCode::RouteOutputMissing => "invalid_parameter_contract.route_output_missing",
        &InvalidParameterContractCode::RouteRepeatedArrow => "invalid_parameter_contract.route_repeated_arrow",
        &InvalidParameterContractCode::NestedContract => "invalid_parameter_contract.nested_contract",
        &InvalidParameterContractCode::MutableParameter => "invalid_parameter_contract.mutable_parameter",
        &InvalidParameterContractCode::ParameterDefault => "invalid_parameter_contract.parameter_default",
        &InvalidParameterContractCode::ReceiverContract => "invalid_parameter_contract.receiver_contract",
        &InvalidParameterContractCode::MultipleSpecificationTypes => "invalid_parameter_contract.multiple_specification_types",
    },

    InvalidDependencyClauseReason => {
    &InvalidDependencyClauseReason::MissingPath => "invalid_dependency_clause.missing_path",
    &InvalidDependencyClauseReason::ExpectedPath => "invalid_dependency_clause.expected_path",
    &InvalidDependencyClauseReason::ExpectedSelectionName => "invalid_dependency_clause.expected_selection_name",
    &InvalidDependencyClauseReason::MissingAlias => "invalid_dependency_clause.missing_alias",
    &InvalidDependencyClauseReason::ExpectedAliasName => "invalid_dependency_clause.expected_alias_name",
    &InvalidDependencyClauseReason::DuplicateSelectionName => "invalid_dependency_clause.duplicate_selection_name",
    &InvalidDependencyClauseReason::DuplicateSelectionLocalName => "invalid_dependency_clause.duplicate_selection_local_name",
    &InvalidDependencyClauseReason::LegacyBraceSelections => "invalid_dependency_clause.legacy_brace_selections",
    &InvalidDependencyClauseReason::MissingSelectionAfterComma => "invalid_dependency_clause.missing_selection_after_comma",
    &InvalidDependencyClauseReason::MissingCommaBetweenSelections => "invalid_dependency_clause.missing_comma_between_selections",
    &InvalidDependencyClauseReason::NamespaceAliasWithSelections => "invalid_dependency_clause.namespace_alias_with_selections",
    &InvalidDependencyClauseReason::InvalidSelectionDelimiter => "invalid_dependency_clause.invalid_selection_delimiter",
    &InvalidDependencyClauseReason::DependencyClauseNotAllowed => "invalid_dependency_clause.dependency_clause_not_allowed",
    &InvalidDependencyClauseReason::ProjectGlobalsPathReserved => "invalid_dependency_clause.project_globals_path_reserved",
    &InvalidDependencyClauseReason::ProjectGlobalsFacadeDependencyNotAllowed => "invalid_dependency_clause.project_globals_facade_dependency_not_allowed",
    &InvalidDependencyClauseReason::ProjectGlobalsReexportNotAllowed => "invalid_dependency_clause.project_globals_reexport_not_allowed",
    &InvalidDependencyClauseReason::ProviderRequiresBinding => "invalid_dependency_clause.provider_requires_binding",
    &InvalidDependencyClauseReason::ContinuationEnteredStatement => "invalid_dependency_clause.continuation_entered_statement",
    },

    LegacyDependencyClauseReason => {
    &LegacyDependencyClauseReason::ImportKeyword => "legacy_dependency_clause.import_keyword",
    },

    InvalidImportPathReason => {
        @delegate(reason => InvalidImportPathCode::of(reason).stable_reason_key());
    },
    InvalidImportPathCode => {
        &InvalidImportPathCode::PublicRoot => "invalid_import_path.public_root",
        &InvalidImportPathCode::CurrentDirectorySegment => "invalid_import_path.current_directory_segment",
        &InvalidImportPathCode::ParentDirectorySegment => "invalid_import_path.parent_directory_segment",
        &InvalidImportPathCode::EscapesProjectRoot => "invalid_import_path.escapes_project_root",
        &InvalidImportPathCode::EscapesSourcePackageRoot => "invalid_import_path.escapes_source_package_root",
        &InvalidImportPathCode::CaseMismatch => "invalid_import_path.case_mismatch",
    },
    InvalidExternalModuleReason => {
        @delegate(reason => InvalidExternalModuleCode::of(reason).stable_reason_key());
    },
    InvalidExternalModuleCode => {
        &InvalidExternalModuleCode::ReceiverMethod => "invalid_external_module.receiver_method",
        &InvalidExternalModuleCode::ParserDiagnostic => "invalid_external_module.parser_diagnostic",
        &InvalidExternalModuleCode::NonUtf8Source => "invalid_external_module.non_utf8_source",
        &InvalidExternalModuleCode::PathContainsPackageSeparator => "invalid_external_module.path_contains_package_separator",
    },


    InvalidCompileTimePathReason => {
        @delegate(reason => InvalidCompileTimePathCode::of(reason).stable_reason_key());
    },

    InvalidCompileTimePathCode => {
        &InvalidCompileTimePathCode::MissingTarget => "invalid_compile_time_path.missing_target",
        &InvalidCompileTimePathCode::TargetIsDirectory => "invalid_compile_time_path.target_is_directory",
        &InvalidCompileTimePathCode::TargetNotRegular => "invalid_compile_time_path.target_not_regular",
        &InvalidCompileTimePathCode::CurrentDirectorySegment => "invalid_compile_time_path.current_directory_segment",
        &InvalidCompileTimePathCode::ParentDirectorySegment => "invalid_compile_time_path.parent_directory_segment",
        &InvalidCompileTimePathCode::CaseMismatch => "invalid_compile_time_path.case_mismatch",
        &InvalidCompileTimePathCode::EscapesModuleBoundary => "invalid_compile_time_path.escapes_module_boundary",
        &InvalidCompileTimePathCode::EscapesSymlink => "invalid_compile_time_path.escapes_symlink",
        &InvalidCompileTimePathCode::EscapesProjectRoot => "invalid_compile_time_path.escapes_project_root",
    },

    InvalidTypeAnnotationReason => {
        @delegate(reason => InvalidTypeAnnotationCode::split(reason).0.stable_reason_key());
    },

    InvalidTypeAnnotationCode => {
    &InvalidTypeAnnotationCode::NoneNotAllowed => "invalid_type_annotation.none_not_allowed",
    &InvalidTypeAnnotationCode::ReservedTraitKeyword => "invalid_type_annotation.reserved_trait_keyword",
    &InvalidTypeAnnotationCode::TraitThisMustBeDirect => "invalid_type_annotation.trait_this_must_be_direct",
    &InvalidTypeAnnotationCode::AsNotValidHere => "invalid_type_annotation.as_not_valid_here",
    &InvalidTypeAnnotationCode::UnexpectedColon => "invalid_type_annotation.unexpected_colon",
    &InvalidTypeAnnotationCode::InvalidTokenAfterName => "invalid_type_annotation.invalid_token_after_name",
    &InvalidTypeAnnotationCode::ExpectedTypeAnnotation => "invalid_type_annotation.expected_type_annotation",
    &InvalidTypeAnnotationCode::DuplicateOptional => "invalid_type_annotation.duplicate_optional",
    &InvalidTypeAnnotationCode::NestedOptional => "invalid_type_annotation.nested_optional",
    },

    InvalidCollectionTypeReason => {
        @delegate(reason => InvalidCollectionTypeCode::of(reason).stable_reason_key());
    },
    InvalidCollectionTypeCode => {
        &InvalidCollectionTypeCode::NegativeCapacity => "invalid_collection_type.negative_capacity",
        &InvalidCollectionTypeCode::ShorthandCapacityNotAllowed => "invalid_collection_type.shorthand_capacity_not_allowed",
        &InvalidCollectionTypeCode::ZeroCapacity => "invalid_collection_type.zero_capacity",
        &InvalidCollectionTypeCode::CapacityNotInt => "invalid_collection_type.capacity_not_int",
        &InvalidCollectionTypeCode::CapacityNotConstant => "invalid_collection_type.capacity_not_constant",
        &InvalidCollectionTypeCode::CapacityOverflow => "invalid_collection_type.capacity_overflow",
        &InvalidCollectionTypeCode::InitializerExceedsFixedCapacity => "invalid_collection_type.initializer_exceeds_fixed_capacity",
        &InvalidCollectionTypeCode::EmptyImmutableFixedCollection => "invalid_collection_type.empty_immutable_fixed_collection",
        &InvalidCollectionTypeCode::ShorthandEmptyLiteralAmbiguous => "invalid_collection_type.shorthand_empty_literal_ambiguous",
        &InvalidCollectionTypeCode::ShorthandNonLiteralRhs => "invalid_collection_type.shorthand_non_literal_rhs",
    },

    InvalidMapTypeReason => {
        @delegate(reason => InvalidMapTypeCode::of(reason).stable_reason_key());
    },
    InvalidMapTypeCode => {
        &InvalidMapTypeCode::UnsupportedKeyType => "invalid_map_type.unsupported_key_type",
        &InvalidMapTypeCode::ExcessiveInlineNesting => "invalid_map_type.excessive_inline_nesting",
        &InvalidMapTypeCode::EmptyMapKeyType => "invalid_map_type.empty_map_key_type",
        &InvalidMapTypeCode::EmptyMapValueType => "invalid_map_type.empty_map_value_type",
        &InvalidMapTypeCode::MultipleMapSeparators => "invalid_map_type.multiple_map_separators",
        &InvalidMapTypeCode::FixedCapacityNotAllowed => "invalid_map_type.fixed_capacity_not_allowed",
    },

    InvalidMapLiteralReason => {
    &InvalidMapLiteralReason::MixedCollectionMapEntries => "invalid_map_literal.mixed_collection_map_entries",
    &InvalidMapLiteralReason::DuplicateKnownKey => "invalid_map_literal.duplicate_known_key",
    &InvalidMapLiteralReason::MissingKeyExpression => "invalid_map_literal.missing_key_expression",
    &InvalidMapLiteralReason::MissingValueExpression => "invalid_map_literal.missing_value_expression",
    },

    InvalidGenericParameterReason => {
        @delegate(reason => InvalidGenericParameterCode::of(reason).stable_reason_key());
    },
    InvalidGenericParameterCode => {
        &InvalidGenericParameterCode::EmptyParameterList => "invalid_generic_parameter.empty_parameter_list",
        &InvalidGenericParameterCode::BoundsMustUseIs => "invalid_generic_parameter.bounds_must_use_is",
        &InvalidGenericParameterCode::ListMustStayWithHeader => "invalid_generic_parameter.list_must_stay_with_header",
        &InvalidGenericParameterCode::InvalidToken => "invalid_generic_parameter.invalid_token",
        &InvalidGenericParameterCode::DuplicateTraitBound => "invalid_generic_parameter.duplicate_trait_bound",
    },

    InvalidTemplateDirectiveReason => {
        @delegate(reason => InvalidTemplateDirectiveCode::of(reason).stable_reason_key());
    },
    InvalidTemplateDirectiveCode => {
        &InvalidTemplateDirectiveCode::UnknownDirective => "invalid_template_directive.unknown_directive",
        &InvalidTemplateDirectiveCode::MissingArgument => "invalid_template_directive.missing_argument",
        &InvalidTemplateDirectiveCode::InvalidArgument => "invalid_template_directive.invalid_argument",
        &InvalidTemplateDirectiveCode::DirectiveNotAllowedHere => "invalid_template_directive.directive_not_allowed_here",
        &InvalidTemplateDirectiveCode::UnexpectedArguments => "invalid_template_directive.unexpected_arguments",
        &InvalidTemplateDirectiveCode::EmptyArguments => "invalid_template_directive.empty_arguments",
        &InvalidTemplateDirectiveCode::InvalidSlotTarget => "invalid_template_directive.invalid_slot_target",
        &InvalidTemplateDirectiveCode::InvalidInsertTarget => "invalid_template_directive.invalid_insert_target",
        &InvalidTemplateDirectiveCode::InvalidChildrenArgument => "invalid_template_directive.invalid_children_argument",
    },
    CssTemplateWarning => {
        @delegate(reason => CssTemplateWarningCode::of(reason).stable_reason_key());
    },
    CssTemplateWarningCode => {
        &CssTemplateWarningCode::MismatchedClosingBrace => "malformed_template.css.mismatched_closing_brace",
        &CssTemplateWarningCode::UnexpectedClosingBrace => "malformed_template.css.unexpected_closing_brace",
        &CssTemplateWarningCode::MismatchedClosingParenthesis => "malformed_template.css.mismatched_closing_parenthesis",
        &CssTemplateWarningCode::UnexpectedClosingParenthesis => "malformed_template.css.unexpected_closing_parenthesis",
        &CssTemplateWarningCode::UnclosedDelimiter => "malformed_template.css.unclosed_delimiter",
        &CssTemplateWarningCode::InlineSelectorBlock => "malformed_template.css.inline_selector_block",
        &CssTemplateWarningCode::MissingSelectorOrAtRulePrelude => "malformed_template.css.missing_selector_or_at_rule_prelude",
        &CssTemplateWarningCode::NestedBlock => "malformed_template.css.nested_block",
        &CssTemplateWarningCode::TopLevelContent => "malformed_template.css.top_level_content",
        &CssTemplateWarningCode::MalformedDeclaration => "malformed_template.css.malformed_declaration",
        &CssTemplateWarningCode::InvalidPropertyName => "malformed_template.css.invalid_property_name",
        &CssTemplateWarningCode::MissingDeclarationValue => "malformed_template.css.missing_declaration_value",
    },

    HtmlTemplateWarning => {
        @delegate(reason => HtmlTemplateWarningCode::of(reason).stable_reason_key());
    },
    HtmlTemplateWarningCode => {
        &HtmlTemplateWarningCode::ScriptTag => "malformed_template.html.unsafe_script_tag",
        &HtmlTemplateWarningCode::JavascriptUrl => "malformed_template.html.unsafe_javascript_url",
        &HtmlTemplateWarningCode::InlineEventHandler => "malformed_template.html.unsafe_inline_event_handler",
    },

    MalformedTemplateReason => {
        @delegate(&MalformedTemplateReason::Css(reason) => reason.stable_reason_key());
        @delegate(&MalformedTemplateReason::Html(reason) => reason.stable_reason_key());
    },


    InvalidTemplateStructureReason => {
        @delegate(reason => InvalidTemplateStructureCode::of(reason).stable_reason_key());
    },
    InvalidTemplateStructureCode => {
        &InvalidTemplateStructureCode::MissingClosingBracket => "invalid_template_structure.missing_closing_bracket",
        &InvalidTemplateStructureCode::SlotInHead => "invalid_template_structure.slot_in_head",
        &InvalidTemplateStructureCode::MissingHandlerBody => "invalid_template_structure.missing_handler_body",
        &InvalidTemplateStructureCode::InvalidChildDirective => "invalid_template_structure.invalid_child_directive",
        &InvalidTemplateStructureCode::NestedTemplateNotAllowed => "invalid_template_structure.nested_template_not_allowed",
        &InvalidTemplateStructureCode::HelperInConstTemplate => "invalid_template_structure.helper_in_const_template",
        &InvalidTemplateStructureCode::NonFoldableConstTemplate => "invalid_template_structure.non_foldable_const_template",
        &InvalidTemplateStructureCode::NonFoldableDocComment => "invalid_template_structure.non_foldable_doc_comment",
        &InvalidTemplateStructureCode::FallibleValueInTemplateHead => "invalid_template_structure.fallible_value_in_template_head",
        &InvalidTemplateStructureCode::UnsupportedTypeInTemplateHead => "invalid_template_structure.unsupported_type_in_template_head",
        &InvalidTemplateStructureCode::RuntimeTemplateInConst => "invalid_template_structure.runtime_template_in_const",
        &InvalidTemplateStructureCode::RuntimeValueInConstTemplateHead => "invalid_template_structure.runtime_value_in_const_template_head",
        &InvalidTemplateStructureCode::EmptyPathInTemplateHead => "invalid_template_structure.empty_path_in_template_head",
        &InvalidTemplateStructureCode::IncompatibleHeadItem => "invalid_template_structure.incompatible_head_item",
        &InvalidTemplateStructureCode::HelperOutsideWrapperSlot => "invalid_template_structure.helper_outside_wrapper_slot",
        &InvalidTemplateStructureCode::RuntimeControlFlowUnresolvedInsert => "invalid_template_structure.runtime_control_flow_unresolved_insert",
        &InvalidTemplateStructureCode::MissingCommaBeforeControlFlowSuffix => "invalid_template_structure.missing_comma_before_control_flow_suffix",
        &InvalidTemplateStructureCode::ControlFlowSuffixNotFinal => "invalid_template_structure.control_flow_suffix_not_final",
        &InvalidTemplateStructureCode::MissingTemplateIfCondition => "invalid_template_structure.missing_template_if_condition",
        &InvalidTemplateStructureCode::MissingTemplateLoopHeader => "invalid_template_structure.missing_template_loop_header",
        &InvalidTemplateStructureCode::UnexpectedTokenAfterControlFlowSuffix => "invalid_template_structure.unexpected_token_after_control_flow_suffix",
        &InvalidTemplateStructureCode::TemplateMatchStyleControlFlowUnsupported => "invalid_template_structure.template_match_style_control_flow_unsupported",
        &InvalidTemplateStructureCode::TemplateIfConditionNotConst => "invalid_template_structure.template_if_condition_not_const",
        &InvalidTemplateStructureCode::TemplateIfBranchNotConst => "invalid_template_structure.template_if_branch_not_const",
        &InvalidTemplateStructureCode::TemplateOptionCaptureConstDeferred => "invalid_template_structure.template_option_capture_const_deferred",
        &InvalidTemplateStructureCode::TemplateLoopRangeBoundsNotConst => "invalid_template_structure.template_loop_range_bounds_not_const",
        &InvalidTemplateStructureCode::TemplateLoopSourceNotConst => "invalid_template_structure.template_loop_source_not_const",
        &InvalidTemplateStructureCode::TemplateLoopConditionNotConst => "invalid_template_structure.template_loop_condition_not_const",
        &InvalidTemplateStructureCode::TemplateConditionalLoopConstTrue => "invalid_template_structure.template_conditional_loop_const_true",
        &InvalidTemplateStructureCode::TemplateLoopBodyNotConst => "invalid_template_structure.template_loop_body_not_const",
        &InvalidTemplateStructureCode::TemplateConstLoopExpansionLimitExceeded => "invalid_template_structure.template_const_loop_expansion_limit_exceeded",
    },

    InvalidSignatureMemberReason => {
    &InvalidSignatureMemberReason::ChoicePayloadMutable => "invalid_signature_member.choice_payload_mutable",
    &InvalidSignatureMemberReason::ChoicePayloadDefaultValue => "invalid_signature_member.choice_payload_default_value",
    &InvalidSignatureMemberReason::CompileTimeParameterDeferred => "invalid_signature_member.compile_time_parameter_deferred",
    &InvalidSignatureMemberReason::ThisNotAllowed => "invalid_signature_member.this_not_allowed",
    &InvalidSignatureMemberReason::TrailingComma => "invalid_signature_member.trailing_comma",
    &InvalidSignatureMemberReason::TraitReceiverMustBeThis => "invalid_signature_member.trait_receiver_must_be_this",
    &InvalidSignatureMemberReason::TraitMutableThisOnlyFirstParameter => "invalid_signature_member.trait_mutable_this_only_first_parameter",
    &InvalidSignatureMemberReason::TraitBareThisOnlyReceiver => "invalid_signature_member.trait_bare_this_only_receiver",
    &InvalidSignatureMemberReason::TraitRequirementDefaultValue => "invalid_signature_member.trait_requirement_default_value",
    &InvalidSignatureMemberReason::MissingDefaultValue => "invalid_signature_member.missing_default_value",
    },

    InvalidFunctionSignatureReason => {
        @delegate(reason => InvalidFunctionSignatureCode::of(reason).stable_reason_key());
    },
    InvalidFunctionSignatureCode => {
        &InvalidFunctionSignatureCode::MissingArrowOrColon => "invalid_function_signature.missing_arrow_or_colon",
        &InvalidFunctionSignatureCode::UnexpectedEndAfterParameters => "invalid_function_signature.unexpected_end_after_parameters",
        &InvalidFunctionSignatureCode::MissingReturnType => "invalid_function_signature.missing_return_type",
        &InvalidFunctionSignatureCode::MissingTraitRequirementReturnType => "invalid_function_signature.missing_trait_requirement_return_type",
        &InvalidFunctionSignatureCode::TrailingCommaInReturns => "invalid_function_signature.trailing_comma_in_returns",
        &InvalidFunctionSignatureCode::UnexpectedEndAfterComma => "invalid_function_signature.unexpected_end_after_comma",
        &InvalidFunctionSignatureCode::UnexpectedEndInReturns => "invalid_function_signature.unexpected_end_in_returns",
        &InvalidFunctionSignatureCode::MissingColonAfterReturns => "invalid_function_signature.missing_colon_after_returns",
        &InvalidFunctionSignatureCode::UnexpectedArrowInReturns => "invalid_function_signature.unexpected_arrow_in_returns",
        &InvalidFunctionSignatureCode::MissingCommaOrColon => "invalid_function_signature.missing_comma_or_colon",
        &InvalidFunctionSignatureCode::VoidNotAllowed => "invalid_function_signature.void_not_allowed",
        &InvalidFunctionSignatureCode::MultipleErrorReturnSlots => "invalid_function_signature.multiple_error_return_slots",
        &InvalidFunctionSignatureCode::ErrorSlotNotLast => "invalid_function_signature.error_slot_not_last",
        &InvalidFunctionSignatureCode::GenericWhereConstraintsUnsupported => "invalid_function_signature.generic_where_constraints_unsupported",
    },

    InvalidChoiceVariantReason => {
    &InvalidChoiceVariantReason::EmptyRecordBody => "invalid_choice_variant.empty_record_body",
    &InvalidChoiceVariantReason::RecursiveDeclaration => "invalid_choice_variant.recursive_declaration",
    &InvalidChoiceVariantReason::ConstructorStyleNotSupported => "invalid_choice_variant.constructor_style_not_supported",
    &InvalidChoiceVariantReason::PayloadShorthandNotSupported => "invalid_choice_variant.payload_shorthand_not_supported",
    &InvalidChoiceVariantReason::UnexpectedSeparator => "invalid_choice_variant.unexpected_separator",
    &InvalidChoiceVariantReason::MissingVariants => "invalid_choice_variant.missing_variants",
    &InvalidChoiceVariantReason::UnknownVariant => "invalid_choice_variant.unknown_variant",
    &InvalidChoiceVariantReason::UnitVariantWithParentheses => "invalid_choice_variant.unit_variant_with_parentheses",
    &InvalidChoiceVariantReason::UnitVariantAsConstructor => "invalid_choice_variant.unit_variant_as_constructor",
    &InvalidChoiceVariantReason::PayloadVariantMissingArguments => "invalid_choice_variant.payload_variant_missing_arguments",
    },

    InvalidThisUsageReason => {
        @delegate(reason => InvalidThisUsageCode::of(reason).stable_reason_key());
    },
    InvalidThisUsageCode => {
        &InvalidThisUsageCode::NotInReceiverMethod => "invalid_this_usage.not_in_receiver_method",
        &InvalidThisUsageCode::Reassignment => "invalid_this_usage.reassignment",
        &InvalidThisUsageCode::LoopBinding => "invalid_this_usage.loop_binding",
        &InvalidThisUsageCode::DeclarationBinding => "invalid_this_usage.declaration_binding",
        &InvalidThisUsageCode::DuplicateThis => "invalid_this_usage.duplicate_this",
        &InvalidThisUsageCode::NotFirstParameter => "invalid_this_usage.not_first_parameter",
        &InvalidThisUsageCode::OutsideTraitDeclaration => "invalid_this_usage.outside_trait_declaration",
    },

    InvalidTraitKeywordUsageReason => {
    &InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax => "invalid_trait_keyword_usage.must_outside_trait_syntax",
    &InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax => "invalid_trait_keyword_usage.this_outside_trait_syntax",
    },

    InvalidReceiverDeclarationReason => {
        @delegate(reason => InvalidReceiverDeclarationCode::of(reason).stable_reason_key());
    },
    InvalidReceiverDeclarationCode => {
        &InvalidReceiverDeclarationCode::UnknownStructTarget => "invalid_receiver_declaration.unknown_struct_target",
        &InvalidReceiverDeclarationCode::NonlocalSourceType => "invalid_receiver_declaration.nonlocal_source_type",
        &InvalidReceiverDeclarationCode::BuiltinScalarType => "invalid_receiver_declaration.builtin_scalar_type",
        &InvalidReceiverDeclarationCode::ExternalOpaqueType => "invalid_receiver_declaration.external_opaque_type",
        &InvalidReceiverDeclarationCode::FieldNameConflict => "invalid_receiver_declaration.field_name_conflict",
        &InvalidReceiverDeclarationCode::DuplicateMethod => "invalid_receiver_declaration.duplicate_method",
        &InvalidReceiverDeclarationCode::DuplicateVisibleMethod => "invalid_receiver_declaration.duplicate_visible_method",
        &InvalidReceiverDeclarationCode::GenericReceiverType => "invalid_receiver_declaration.generic_receiver_type",
        &InvalidReceiverDeclarationCode::UnsupportedType => "invalid_receiver_declaration.unsupported_type",
        &InvalidReceiverDeclarationCode::ReceiverMethodImportOrExportNotAllowed => "invalid_receiver_declaration.receiver_method_import_or_export_not_allowed",
        &InvalidReceiverDeclarationCode::GenericOwnerAlignment => "invalid_receiver_declaration.generic_owner_alignment",
        &InvalidReceiverDeclarationCode::GenericParameterAlignment => "invalid_receiver_declaration.generic_parameter_alignment",
    },

    InvalidControlFlowStatementReason => {
    &InvalidControlFlowStatementReason::ElseOutsideIfOrMatch => "invalid_control_flow_statement.else_outside_if_or_match",
    &InvalidControlFlowStatementReason::ElseIfUnsupported => "invalid_control_flow_statement.else_if_unsupported",
    &InvalidControlFlowStatementReason::BreakOutsideLoop => "invalid_control_flow_statement.break_outside_loop",
    &InvalidControlFlowStatementReason::ContinueOutsideLoop => "invalid_control_flow_statement.continue_outside_loop",
    &InvalidControlFlowStatementReason::ReturnOutsideFunction => "invalid_control_flow_statement.return_outside_function",
    &InvalidControlFlowStatementReason::ReturnBangOutsideErrorFunction => "invalid_control_flow_statement.return_bang_outside_error_function",
    &InvalidControlFlowStatementReason::ExpectedColonAfterCondition => "invalid_control_flow_statement.expected_colon_after_condition",
    &InvalidControlFlowStatementReason::ExpectedConditionAfterIf => "invalid_control_flow_statement.expected_condition_after_if",
    &InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch => "invalid_control_flow_statement.unexpected_end_of_file_in_match",
    &InvalidControlFlowStatementReason::CaseRequiredBeforeElse => "invalid_control_flow_statement.case_required_before_else",
    &InvalidControlFlowStatementReason::DuplicateElseArm => "invalid_control_flow_statement.duplicate_else_arm",
    &InvalidControlFlowStatementReason::ExpectedFatArrow => "invalid_control_flow_statement.expected_fat_arrow",
    &InvalidControlFlowStatementReason::InlineValueIfMultiline => "invalid_control_flow_statement.inline_value_if_multiline",
    &InvalidControlFlowStatementReason::InlineValueIfElseThen => "invalid_control_flow_statement.inline_value_if_else_then",
    &InvalidControlFlowStatementReason::ValueIfMissingElse => "invalid_control_flow_statement.value_if_missing_else",
    &InvalidControlFlowStatementReason::ValueIfBranchFallsThrough => "invalid_control_flow_statement.value_if_branch_falls_through",
    &InvalidControlFlowStatementReason::ValueIfNoProducingPath => "invalid_control_flow_statement.value_if_no_producing_path",
    &InvalidControlFlowStatementReason::ValueBlockOutsideReceiver => "invalid_control_flow_statement.value_block_outside_receiver",
    &InvalidControlFlowStatementReason::ValueIfOptionNonePredicate => "invalid_control_flow_statement.value_if_option_none_predicate",
    &InvalidControlFlowStatementReason::ValueIfOptionLiteralPredicate => "invalid_control_flow_statement.value_if_option_literal_predicate",
    &InvalidControlFlowStatementReason::ExpectedValueAfterThen => "invalid_control_flow_statement.expected_value_after_then",
    &InvalidControlFlowStatementReason::ExpectedValueAfterElse => "invalid_control_flow_statement.expected_value_after_else",
    },

    InvalidDeclarationReason => {
        @delegate(reason => InvalidDeclarationCode::of(reason).stable_reason_key());
    },

    InvalidDeclarationCode => {
        &InvalidDeclarationCode::ReservedBuiltinName => "invalid_declaration.reserved_builtin_name",
        &InvalidDeclarationCode::DeclarationRequiresModuleTopLevel => "invalid_declaration.declaration_requires_module_top_level",
        &InvalidDeclarationCode::ConstantCannotBeMutable => "invalid_declaration.constant_cannot_be_mutable",
        &InvalidDeclarationCode::ExternalTypeLiteralConstruction => "invalid_declaration.external_type_literal_construction",
        &InvalidDeclarationCode::ParameterizedGenericTypeAlias => "invalid_declaration.parameterized_generic_type_alias",
        &InvalidDeclarationCode::UnusedGenericParameter => "invalid_declaration.unused_generic_parameter",
        &InvalidDeclarationCode::RecursiveGenericType => "invalid_declaration.recursive_generic_type",
        &InvalidDeclarationCode::RecursiveRuntimeStruct => "invalid_declaration.recursive_runtime_struct",
        &InvalidDeclarationCode::ExternalTypeAlias => "invalid_declaration.external_type_alias",
        &InvalidDeclarationCode::InvalidGenericParameterName => "invalid_declaration.invalid_generic_parameter_name",
        &InvalidDeclarationCode::DuplicateGenericParameter => "invalid_declaration.duplicate_generic_parameter",
        &InvalidDeclarationCode::GenericParameterNameCollision => "invalid_declaration.generic_parameter_name_collision",
        &InvalidDeclarationCode::ReservedGenericParameterName => "invalid_declaration.reserved_generic_parameter_name",
        &InvalidDeclarationCode::GenericTraitsUnsupported => "invalid_declaration.generic_traits_unsupported",
        &InvalidDeclarationCode::InvalidTraitName => "invalid_declaration.invalid_trait_name",
        &InvalidDeclarationCode::TraitConformanceMissingTrait => "invalid_declaration.trait_conformance_missing_trait",
        &InvalidDeclarationCode::TraitConformanceSemicolon => "invalid_declaration.trait_conformance_semicolon",
        &InvalidDeclarationCode::TraitIncompatibilityMissingTrait => "invalid_declaration.trait_incompatibility_missing_trait",
        &InvalidDeclarationCode::TraitIncompatibilitySemicolon => "invalid_declaration.trait_incompatibility_semicolon",
        &InvalidDeclarationCode::MissingInitializerExpression => "invalid_declaration.missing_initializer_expression",
    },

    InvalidTraitIncompatibilityReason => {
    &InvalidTraitIncompatibilityReason::SelfIncompatible => "invalid_trait_incompatibility.self_incompatible",
    &InvalidTraitIncompatibilityReason::UnknownTrait => "invalid_trait_incompatibility.unknown_trait",
    &InvalidTraitIncompatibilityReason::DuplicateRelation => "invalid_trait_incompatibility.duplicate_relation",
    &InvalidTraitIncompatibilityReason::PrivateTraitSurfaceLeak => "invalid_trait_incompatibility.private_trait_surface_leak",
    },

    InvalidTraitConformanceReason => {
        @delegate(reason => InvalidTraitConformanceCode::of(reason).stable_reason_key());
    },

    InvalidTraitConformanceCode => {
        &InvalidTraitConformanceCode::ImportedModuleRoot => "invalid_trait_conformance.imported_module_root",
        &InvalidTraitConformanceCode::AliasTarget => "invalid_trait_conformance.alias_target",
        &InvalidTraitConformanceCode::NonCanonicalTarget => "invalid_trait_conformance.non_canonical_target",
        &InvalidTraitConformanceCode::NonlocalSourceTarget => "invalid_trait_conformance.nonlocal_source_target",
        &InvalidTraitConformanceCode::BuiltinTarget => "invalid_trait_conformance.builtin_target",
        &InvalidTraitConformanceCode::ExternalOpaqueTarget => "invalid_trait_conformance.external_opaque_target",
        &InvalidTraitConformanceCode::DuplicateCanonicalEvidence => "invalid_trait_conformance.duplicate_canonical_evidence",
        &InvalidTraitConformanceCode::BuiltinEvidenceOverride => "invalid_trait_conformance.builtin_evidence_override",
        &InvalidTraitConformanceCode::IncompatibleTraitEvidence => "invalid_trait_conformance.incompatible_trait_evidence",
        &InvalidTraitConformanceCode::MissingMethod => "invalid_trait_conformance.missing_method",
        &InvalidTraitConformanceCode::ReceiverMutabilityMismatch => "invalid_trait_conformance.receiver_mutability_mismatch",
        &InvalidTraitConformanceCode::ParameterCountMismatch => "invalid_trait_conformance.parameter_count_mismatch",
        &InvalidTraitConformanceCode::ParameterModeMismatch => "invalid_trait_conformance.parameter_mode_mismatch",
        &InvalidTraitConformanceCode::ParameterContractMismatch => "invalid_trait_conformance.parameter_contract_mismatch",
        &InvalidTraitConformanceCode::ParameterTypeMismatch => "invalid_trait_conformance.parameter_type_mismatch",
        &InvalidTraitConformanceCode::ReturnCountMismatch => "invalid_trait_conformance.return_count_mismatch",
        &InvalidTraitConformanceCode::ReturnTypeMismatch => "invalid_trait_conformance.return_type_mismatch",
        &InvalidTraitConformanceCode::ReturnChannelMismatch => "invalid_trait_conformance.return_channel_mismatch",
        &InvalidTraitConformanceCode::InferredFailureInInfallibleCast => "invalid_trait_conformance.inferred_failure_in_infallible_cast",
    },

    InvalidAssignmentTargetReason => {
    &InvalidAssignmentTargetReason::TemporaryNotAssignable => "invalid_assignment_target.temporary_not_assignable",
    &InvalidAssignmentTargetReason::ImmutableBinding => "invalid_assignment_target.immutable_binding",
    &InvalidAssignmentTargetReason::ImmutableFieldRoot => "invalid_assignment_target.immutable_field_root",
    &InvalidAssignmentTargetReason::UnavailableInCatchRecovery => "invalid_assignment_target.unavailable_in_catch_recovery",
    &InvalidAssignmentTargetReason::CollectionGetTargetNotWritable => "invalid_assignment_target.collection_get_target_not_writable",
    &InvalidAssignmentTargetReason::MapGetTargetNotWritable => "invalid_assignment_target.map_get_target_not_writable",
    &InvalidAssignmentTargetReason::ReadOnlyMapProperty => "invalid_assignment_target.read_only_map_property",
    &InvalidAssignmentTargetReason::ExpectedAssignmentOperator => "invalid_assignment_target.expected_assignment_operator",
    &InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget => "invalid_assignment_target.mutable_marker_on_assignment_target",
    },

    InvalidMultiBindCode => {
    &InvalidMultiBindCode::ThisTargetReserved => "invalid_multi_bind.this_target_reserved",
    &InvalidMultiBindCode::ExpectedTargetName => "invalid_multi_bind.expected_target_name",
    &InvalidMultiBindCode::MissingTargetAfterComma => "invalid_multi_bind.missing_target_after_comma",
    &InvalidMultiBindCode::MissingAssignmentOperator => "invalid_multi_bind.missing_assignment_operator",
    &InvalidMultiBindCode::InvalidTokenAfterTarget => "invalid_multi_bind.invalid_token_after_target",
    &InvalidMultiBindCode::MissingRightHandExpression => "invalid_multi_bind.missing_right_hand_expression",
    &InvalidMultiBindCode::MultipleRightHandExpressions => "invalid_multi_bind.multiple_right_hand_expressions",
    &InvalidMultiBindCode::MutableTargetNeedsExplicitType => "invalid_multi_bind.mutable_target_needs_explicit_type",
    &InvalidMultiBindCode::CompileTimeMarkerOnTarget => "invalid_multi_bind.compile_time_marker_on_target",
    &InvalidMultiBindCode::DuplicateTarget => "invalid_multi_bind.duplicate_target",
    &InvalidMultiBindCode::UnsupportedRhs => "invalid_multi_bind.unsupported_rhs",
    &InvalidMultiBindCode::ExistingTargetMutableMarker => "invalid_multi_bind.existing_target_mutable_marker",
    &InvalidMultiBindCode::ExistingTargetImmutable => "invalid_multi_bind.existing_target_immutable",
    &InvalidMultiBindCode::ArityMismatch => "invalid_multi_bind.arity_mismatch",
    &InvalidMultiBindCode::RhsNotMultiValue => "invalid_multi_bind.rhs_not_multi_value",
    },

    InvalidMultiBindReason => {
        @delegate(reason => InvalidMultiBindCode::of(reason).stable_reason_key());
    },

    InvalidBuiltinCallReason => {
    &InvalidBuiltinCallReason::MissingParentheses => "invalid_builtin_call.missing_parentheses",
    &InvalidBuiltinCallReason::TakesNoArguments => "invalid_builtin_call.takes_no_arguments",
    &InvalidBuiltinCallReason::NamedArgumentsNotSupported => "invalid_builtin_call.named_arguments_not_supported",
    &InvalidBuiltinCallReason::UnhandledFallibleCall => "invalid_builtin_call.unhandled_fallible_call",
    &InvalidBuiltinCallReason::DoesNotAcceptMutableAccess => "invalid_builtin_call.does_not_accept_mutable_access",
    &InvalidBuiltinCallReason::CastMissingParentheses => "invalid_builtin_call.cast_missing_parentheses",
    &InvalidBuiltinCallReason::CastMissingArgument => "invalid_builtin_call.cast_missing_argument",
    &InvalidBuiltinCallReason::CastTooManyArguments => "invalid_builtin_call.cast_too_many_arguments",
    &InvalidBuiltinCallReason::CastMissingClosingParenthesis => "invalid_builtin_call.cast_missing_closing_parenthesis",
    &InvalidBuiltinCallReason::MissingArgument => "invalid_builtin_call.missing_argument",
    &InvalidBuiltinCallReason::TooManyArguments => "invalid_builtin_call.too_many_arguments",
    &InvalidBuiltinCallReason::ExpressionPositionNotAllowed => "invalid_builtin_call.expression_position_not_allowed",
    &InvalidBuiltinCallReason::MapLengthIsProperty => "invalid_builtin_call.map_length_is_property",
    &InvalidBuiltinCallReason::ScalarConstructorRemoved => "invalid_builtin_call.scalar_constructor_removed",
    },

    InvalidCastReason => {
    &InvalidCastReason::MissingExplicitTarget => "invalid_cast.missing_explicit_target",
    &InvalidCastReason::TargetNotBuiltin => "invalid_cast.target_not_builtin",
    &InvalidCastReason::TargetIsGenericParameter => "invalid_cast.target_is_generic_parameter",
    &InvalidCastReason::SameSourceAndTarget => "invalid_cast.same_source_and_target",
    &InvalidCastReason::SourceIsOptional => "invalid_cast.source_is_optional",
    &InvalidCastReason::OperandArityMismatch => "invalid_cast.operand_arity_mismatch",
    &InvalidCastReason::TargetArityMismatch => "invalid_cast.target_arity_mismatch",
    &InvalidCastReason::CastPropagationRemoved => "invalid_cast.cast_propagation_removed",
    &InvalidCastReason::ScalarConstructorRemoved => "invalid_cast.scalar_constructor_removed",
    &InvalidCastReason::NoEvidence => "invalid_cast.no_evidence",
    &InvalidCastReason::BuiltinEvidenceNotConstFoldable => "invalid_cast.builtin_evidence_not_const_foldable",
    &InvalidCastReason::UserDefinedEvidenceNotConstFoldable => "invalid_cast.user_defined_evidence_not_const_foldable",
    &InvalidCastReason::GenericBoundEvidenceNotConstFoldable => "invalid_cast.generic_bound_evidence_not_const_foldable",
    &InvalidCastReason::BuiltinCastFailedInConst => "invalid_cast.builtin_cast_failed_in_const",
    },

    InvalidReceiverCallReason => {
    &InvalidReceiverCallReason::CalledAsFreeFunction => "invalid_receiver_call.called_as_free_function",
    &InvalidReceiverCallReason::MustUseParentheses => "invalid_receiver_call.must_use_parentheses",
    &InvalidReceiverCallReason::ConstRecordNoRuntimeCalls => "invalid_receiver_call.const_record_no_runtime_calls",
    &InvalidReceiverCallReason::MutableReceiverMissingMarker => "invalid_receiver_call.mutable_receiver_missing_marker",
    &InvalidReceiverCallReason::ImmutableReceiverMutableMethod => "invalid_receiver_call.immutable_receiver_mutable_method",
    &InvalidReceiverCallReason::NonPlaceReceiverMutableMethod => "invalid_receiver_call.non_place_receiver_mutable_method",
    &InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver => "invalid_receiver_call.mutable_marker_on_immutable_receiver",
    &InvalidReceiverCallReason::MutableMarkerOnNonPlaceReceiver => "invalid_receiver_call.mutable_marker_on_non_place_receiver",
    &InvalidReceiverCallReason::UnneededMutableAccessMarker => "invalid_receiver_call.unneeded_mutable_access_marker",
    &InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall => "invalid_receiver_call.mutable_marker_on_non_receiver_call",
    &InvalidReceiverCallReason::AmbiguousGenericBoundMethod => "invalid_receiver_call.ambiguous_generic_bound_method",
    },

    InvalidCopyTargetReason => {
    &InvalidCopyTargetReason::FunctionName => "invalid_copy_target.function_name",
    &InvalidCopyTargetReason::FunctionCall => "invalid_copy_target.function_call",
    &InvalidCopyTargetReason::NonPlace => "invalid_copy_target.non_place",
    &InvalidCopyTargetReason::MutableMarkerNotAllowed => "invalid_copy_target.mutable_marker_not_allowed",
    },

    InvalidFieldAccessReason => {
    &InvalidFieldAccessReason::ExpectedNameAfterDot => "invalid_field_access.expected_name_after_dot",
    &InvalidFieldAccessReason::FieldNotMethod => "invalid_field_access.field_not_method",
    &InvalidFieldAccessReason::ChoicePayloadMutation => "invalid_field_access.choice_payload_mutation",
    &InvalidFieldAccessReason::ChoicePayloadDeferred => "invalid_field_access.choice_payload_deferred",
    &InvalidFieldAccessReason::UnknownExternalMember => "invalid_field_access.unknown_external_member",
    &InvalidFieldAccessReason::UnknownMember => "invalid_field_access.unknown_member",
    },

    InvalidMatchPatternReason => {
    &InvalidMatchPatternReason::WildcardNotSupported => "invalid_match_pattern.wildcard_not_supported",
    &InvalidMatchPatternReason::AsNotValid => "invalid_match_pattern.as_not_valid",
    &InvalidMatchPatternReason::NegativeLiteralNotNumeric => "invalid_match_pattern.negative_literal_not_numeric",
    &InvalidMatchPatternReason::LiteralTypeUnsupported => "invalid_match_pattern.literal_type_unsupported",
    &InvalidMatchPatternReason::ScrutineeTypeUnsupportedForRelational => "invalid_match_pattern.scrutinee_type_unsupported_for_relational",
    &InvalidMatchPatternReason::UnitVariantHasPayload => "invalid_match_pattern.unit_variant_has_payload",
    &InvalidMatchPatternReason::PayloadVariantNeedsBindings => "invalid_match_pattern.payload_variant_needs_bindings",
    &InvalidMatchPatternReason::CaptureBindingMustBeFieldName => "invalid_match_pattern.capture_binding_must_be_field_name",
    &InvalidMatchPatternReason::ExpectedLocalBindingAfterAs => "invalid_match_pattern.expected_local_binding_after_as",
    &InvalidMatchPatternReason::AliasMustBeLocalBinding => "invalid_match_pattern.alias_must_be_local_binding",
    &InvalidMatchPatternReason::DuplicateCaptureBinding => "invalid_match_pattern.duplicate_capture_binding",
    &InvalidMatchPatternReason::TooManyCaptureBindings => "invalid_match_pattern.too_many_capture_bindings",
    &InvalidMatchPatternReason::CaptureBindingNameMismatch => "invalid_match_pattern.capture_binding_name_mismatch",
    &InvalidMatchPatternReason::TooFewCaptureBindings => "invalid_match_pattern.too_few_capture_bindings",
    &InvalidMatchPatternReason::QualifierDoesNotMatchScrutinee => "invalid_match_pattern.qualifier_does_not_match_scrutinee",
    &InvalidMatchPatternReason::ExpectedVariantNameAfterQualifier => "invalid_match_pattern.expected_variant_name_after_qualifier",
    &InvalidMatchPatternReason::MustUseVariantNamesNotLiterals => "invalid_match_pattern.must_use_variant_names_not_literals",
    &InvalidMatchPatternReason::MustStartWithVariantName => "invalid_match_pattern.must_start_with_variant_name",
    &InvalidMatchPatternReason::UnknownVariant => "invalid_match_pattern.unknown_variant",
    &InvalidMatchPatternReason::CaptureBindingShadowsVariable => "invalid_match_pattern.capture_binding_shadows_variable",
    &InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee => "invalid_match_pattern.none_pattern_requires_optional_scrutinee",
    &InvalidMatchPatternReason::OptionValuePatternRequiresEquality => "invalid_match_pattern.option_value_pattern_requires_equality",
    &InvalidMatchPatternReason::BareCaptureOnOptionalScrutinee => "invalid_match_pattern.bare_capture_on_optional_scrutinee",
    &InvalidMatchPatternReason::OptionPresentCaptureOnNonOptional => "invalid_match_pattern.option_present_capture_on_non_optional",
    &InvalidMatchPatternReason::EmptyOptionPresentCapture => "invalid_match_pattern.empty_option_present_capture",
    &InvalidMatchPatternReason::OptionPresentCaptureTypeAnnotation => "invalid_match_pattern.option_present_capture_type_annotation",
    &InvalidMatchPatternReason::MissingClosingPipe => "invalid_match_pattern.missing_closing_pipe",
    &InvalidMatchPatternReason::ExpectedBindingInOptionPresentCapture => "invalid_match_pattern.expected_binding_in_option_present_capture",
    },

    NonExhaustiveMatchReason => {
    &NonExhaustiveMatchReason::MissingElseArm => "non_exhaustive_match.missing_else_arm",
    &NonExhaustiveMatchReason::MissingVariants => "non_exhaustive_match.missing_variants",
    &NonExhaustiveMatchReason::GuardedArmsRequireElse => "non_exhaustive_match.guarded_arms_require_else",
    &NonExhaustiveMatchReason::MissingOptionPatterns => "non_exhaustive_match.missing_option_patterns",
    },

    InvalidFallibleHandlingReason => {
        @delegate(reason => InvalidFallibleHandlingCode::of(reason).stable_reason_key());
    },
    InvalidFallibleHandlingCode => {
    &InvalidFallibleHandlingCode::CatchOutsideBoundary => "invalid_fallible_handling.catch_outside_boundary",
    &InvalidFallibleHandlingCode::ExpectedCatchBlockOrHandler => "invalid_fallible_handling.expected_catch_block_or_handler",
    &InvalidFallibleHandlingCode::ExpectedCatchHandlerOpeningPipe => "invalid_fallible_handling.expected_catch_handler_opening_pipe",
    &InvalidFallibleHandlingCode::ExpectedCatchHandlerIdentifier => "invalid_fallible_handling.expected_catch_handler_identifier",
    &InvalidFallibleHandlingCode::ExpectedCatchHandlerClosingPipe => "invalid_fallible_handling.expected_catch_handler_closing_pipe",
    &InvalidFallibleHandlingCode::ExpectedCatchHandlerColon => "invalid_fallible_handling.expected_catch_handler_colon",
    &InvalidFallibleHandlingCode::EmptyCatchHandlerBinding => "invalid_fallible_handling.empty_catch_handler_binding",
    &InvalidFallibleHandlingCode::MultipleCatchHandlerBindings => "invalid_fallible_handling.multiple_catch_handler_bindings",
    &InvalidFallibleHandlingCode::FallbackValuesForErrorOnlyResult => "invalid_fallible_handling.fallback_values_for_error_only_result",
    &InvalidFallibleHandlingCode::CatchOnNonFallible => "invalid_fallible_handling.catch_on_non_fallible",
    &InvalidFallibleHandlingCode::CatchOnOptional => "invalid_fallible_handling.catch_on_optional",
    &InvalidFallibleHandlingCode::BangOnNonFallible => "invalid_fallible_handling.bang_on_non_fallible",
    &InvalidFallibleHandlingCode::BangOnOptional => "invalid_fallible_handling.bang_on_optional",
    &InvalidFallibleHandlingCode::FunctionHasNoErrorSlot => "invalid_fallible_handling.function_has_no_error_slot",
    &InvalidFallibleHandlingCode::EntryHasIncompatibleError => "invalid_fallible_handling.entry_has_incompatible_error",
    &InvalidFallibleHandlingCode::NotOptionExpression => "invalid_fallible_handling.not_option_expression",
    &InvalidFallibleHandlingCode::FunctionHasNoOptionalReturn => "invalid_fallible_handling.function_has_no_optional_return",
    &InvalidFallibleHandlingCode::FunctionHasMultipleSuccessReturns => "invalid_fallible_handling.function_has_multiple_success_returns",
    &InvalidFallibleHandlingCode::OptionPropagationReturnTypeMismatch => "invalid_fallible_handling.option_propagation_return_type_mismatch",
    &InvalidFallibleHandlingCode::OptionPropagationCatchConflict => "invalid_fallible_handling.option_propagation_catch_conflict",
    &InvalidFallibleHandlingCode::ExplicitPropagationCatchConflict => "invalid_fallible_handling.explicit_propagation_catch_conflict",
    &InvalidFallibleHandlingCode::CatchHandlerConflicts => "invalid_fallible_handling.catch_handler_conflicts",
    &InvalidFallibleHandlingCode::CatchHandlerCanFallThrough => "invalid_fallible_handling.catch_handler_can_fall_through",
    &InvalidFallibleHandlingCode::InlineCatchMultiline => "invalid_fallible_handling.inline_catch_multiline",
    &InvalidFallibleHandlingCode::AssertionMessageCannotEscape => "invalid_fallible_handling.assertion_message_cannot_escape",
    &InvalidFallibleHandlingCode::ThenWithNoActiveValueTarget => "invalid_fallible_handling.then_with_no_active_value_target",
    &InvalidFallibleHandlingCode::ThenCrossesBlockedConstruct => "invalid_fallible_handling.then_crosses_blocked_construct",
    &InvalidFallibleHandlingCode::ThenRequiresValues => "invalid_fallible_handling.then_requires_values",
    &InvalidFallibleHandlingCode::DirectOptionFallbackSyntax => "invalid_fallible_handling.direct_option_fallback_syntax",
    &InvalidFallibleHandlingCode::UnhandledErrorReturn => "invalid_fallible_handling.unhandled_error_return",
    &InvalidFallibleHandlingCode::SuccessValueDiscarded => "invalid_fallible_handling.success_value_discarded",
    &InvalidFallibleHandlingCode::IncompatibleCatchErrorTypes => "invalid_fallible_handling.incompatible_catch_error_types",
    &InvalidFallibleHandlingCode::CustomErrorMixedWithImplicitFailure => "invalid_fallible_handling.custom_error_mixed_with_implicit_failure",
    &InvalidFallibleHandlingCode::UnhandledBuiltinFailureInCustomErrorFunction => "invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function",
    &InvalidFallibleHandlingCode::UnhandledBuiltinFailureInExportedFunction => "invalid_fallible_handling.unhandled_builtin_failure_in_exported_function",
    },

    InvalidTemplateSlotReason => {
    &InvalidTemplateSlotReason::InsertOutsideParentSlot => "invalid_template_slot.insert_outside_parent_slot",
    &InvalidTemplateSlotReason::ExtraLooseContentWithoutDefaultSlot => "invalid_template_slot.extra_loose_content_without_default_slot",
    &InvalidTemplateSlotReason::LooseContentWithoutDefaultSlot => "invalid_template_slot.loose_content_without_default_slot",
    &InvalidTemplateSlotReason::InsertCannotTargetDefaultSlot => "invalid_template_slot.insert_cannot_target_default_slot",
    &InvalidTemplateSlotReason::InsertTargetsUnknownNamedSlot => "invalid_template_slot.insert_targets_unknown_named_slot",
    &InvalidTemplateSlotReason::InsertTargetsUnknownPositionalSlot => "invalid_template_slot.insert_targets_unknown_positional_slot",
    &InvalidTemplateSlotReason::SlotDefinitionOutsideTemplateBody => "invalid_template_slot.slot_definition_outside_template_body",
    },

    CompileTimeEvaluationErrorReason => {
    &CompileTimeEvaluationErrorReason::IntegerOverflow => "compile_time_evaluation_error.integer_overflow",
    &CompileTimeEvaluationErrorReason::FloatOverflow => "compile_time_evaluation_error.float_overflow",
    &CompileTimeEvaluationErrorReason::DivideByZero => "compile_time_evaluation_error.divide_by_zero",
    &CompileTimeEvaluationErrorReason::InvalidExponent => "compile_time_evaluation_error.invalid_exponent",
    &CompileTimeEvaluationErrorReason::InvalidOperatorForType => "compile_time_evaluation_error.invalid_operator_for_type",
    &CompileTimeEvaluationErrorReason::ConstantSelfReference => "compile_time_evaluation_error.constant_self_reference",
    &CompileTimeEvaluationErrorReason::ConstantNotVisible => "compile_time_evaluation_error.constant_not_visible",
    &CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant => "compile_time_evaluation_error.non_constant_reference_in_constant",
    &CompileTimeEvaluationErrorReason::SameFileForwardConstantReference => "compile_time_evaluation_error.same_file_forward_constant_reference",
    &CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable => "compile_time_evaluation_error.constant_initializer_not_foldable",
    &CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext => "compile_time_evaluation_error.external_non_scalar_constant_in_constant_context",
    &CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext => "compile_time_evaluation_error.non_compile_time_field_in_constant_context",
    &CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext => "compile_time_evaluation_error.none_literal_requires_optional_type_context",
    &CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported => "compile_time_evaluation_error.external_type_construction_not_supported",
    &CompileTimeEvaluationErrorReason::StructFieldDefaultNotFoldable => "compile_time_evaluation_error.struct_field_default_not_foldable",
    &CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText => "compile_time_evaluation_error.structural_string_requires_final_text",
    &CompileTimeEvaluationErrorReason::EvaluationOutputBoundExceeded => "compile_time_evaluation_error.evaluation_output_bound_exceeded",
    },


    IncompatibleChoiceComparisonReason => {
        @delegate(reason => IncompatibleChoiceComparisonCode::split(reason).0.stable_reason_key());
    },

    IncompatibleChoiceComparisonCode => {
    &IncompatibleChoiceComparisonCode::DifferentChoiceTypes => "incompatible_choice_comparison.different_choice_types",
    &IncompatibleChoiceComparisonCode::ChoiceWithNonChoice => "incompatible_choice_comparison.choice_with_non_choice",
    &IncompatibleChoiceComparisonCode::PayloadEqualityNotSupported => "incompatible_choice_comparison.payload_equality_not_supported",
    },

    InvalidRouteOutputReason => {
    &InvalidRouteOutputReason::NotChoiceOrGeneric => "invalid_route_output.not_choice_or_generic",
    },

    InvalidCallShapeReason => {
        @delegate(reason => InvalidCallShapeCode::of(reason).stable_reason_key());
    },
    InvalidCallShapeCode => {
        &InvalidCallShapeCode::MissingArgument => "invalid_call_shape.missing_argument",
        &InvalidCallShapeCode::ExtraPositionalArgument => "invalid_call_shape.extra_positional_argument",
        &InvalidCallShapeCode::DuplicateArgument => "invalid_call_shape.duplicate_argument",
        &InvalidCallShapeCode::NamedArgumentNotFound => "invalid_call_shape.named_argument_not_found",
        &InvalidCallShapeCode::PositionalAfterNamed => "invalid_call_shape.positional_after_named",
        &InvalidCallShapeCode::NamedArgumentsNotSupported => "invalid_call_shape.named_arguments_not_supported",
        &InvalidCallShapeCode::WireArgumentOrigin => "invalid_call_shape.wire_argument_origin",
        &InvalidCallShapeCode::WireArgumentMutableMarker => "invalid_call_shape.wire_argument_mutable_marker",
        &InvalidCallShapeCode::RouteArgumentNotDirect => "invalid_call_shape.route_argument_not_direct",
        &InvalidCallShapeCode::RouteForwardingContractMismatch => "invalid_call_shape.route_forwarding_contract_mismatch",
        &InvalidCallShapeCode::RouteParameterRead => "invalid_call_shape.route_parameter_read",
        &InvalidCallShapeCode::RouteInputFieldMismatch => "invalid_call_shape.route_input_field_mismatch",
        &InvalidCallShapeCode::MutableAccessRequired => "invalid_call_shape.mutable_access_required",
        &InvalidCallShapeCode::MutableAccessNotAllowed => "invalid_call_shape.mutable_access_not_allowed",
        &InvalidCallShapeCode::MutableAccessOnNonPlace => "invalid_call_shape.mutable_access_on_non_place",
        &InvalidCallShapeCode::MutableAccessOnImmutablePlace => "invalid_call_shape.mutable_access_on_immutable_place",
        &InvalidCallShapeCode::ImmutablePlaceMutableAccessRequired => "invalid_call_shape.immutable_place_mutable_access_required",
    },

    InvalidReturnShapeReason => {
        @delegate(reason => InvalidReturnShapeCode::of(reason).stable_reason_key());
    },
    InvalidReturnShapeCode => {
        &InvalidReturnShapeCode::BareReturnWithExpectedValues => "invalid_return_shape.bare_return_with_expected_values",
        &InvalidReturnShapeCode::ReturnValuesWithBareSignature => "invalid_return_shape.return_values_with_bare_signature",
        &InvalidReturnShapeCode::TooManyReturnValues => "invalid_return_shape.too_many_return_values",
        &InvalidReturnShapeCode::TooFewReturnValues => "invalid_return_shape.too_few_return_values",
        &InvalidReturnShapeCode::MissingReturnBangValue => "invalid_return_shape.missing_return_bang_value",
        &InvalidReturnShapeCode::FunctionMayFallThrough => "invalid_return_shape.function_may_fall_through",
    },

    DeferredFeatureReason => {
        @delegate(reason => DeferredFeatureCode::of(reason).stable_reason_key());
    },
    DeferredFeatureCode => {
        &DeferredFeatureCode::NamedFeature => "deferred_feature.named_feature",
        &DeferredFeatureCode::CaptureTaggedPattern => "deferred_feature.capture_tagged_pattern",
        &DeferredFeatureCode::NegatedMatchPattern => "deferred_feature.negated_match_pattern",
        &DeferredFeatureCode::NamedPayloadPatternAssignment => "deferred_feature.named_payload_pattern_assignment",
        &DeferredFeatureCode::NestedPayloadPattern => "deferred_feature.nested_payload_pattern",
        &DeferredFeatureCode::ChoiceVariantDefaultValue => "deferred_feature.choice_variant_default_value",
        &DeferredFeatureCode::GenericReceiverMethod => "deferred_feature.generic_receiver_method",
        &DeferredFeatureCode::DeclaredRegion => "deferred_feature.declared_region",
        &DeferredFeatureCode::CheckedBlock => "deferred_feature.checked_block",
        &DeferredFeatureCode::AsyncBlock => "deferred_feature.async_block",
        &DeferredFeatureCode::RuntimeAnonymousRecord => "deferred_feature.runtime_anonymous_record",
    },

    InvalidPageMetadataReason => {
    &InvalidPageMetadataReason::NotAString => "invalid_page_metadata.not_a_string",
    &InvalidPageMetadataReason::DuplicateDeclaration => "invalid_page_metadata.duplicate_declaration",
    },

    InvalidRangeOperandReason => {
    &InvalidRangeOperandReason::NotNumeric => "invalid_range_operand.not_numeric",
    &InvalidRangeOperandReason::IncompatibleDomain => "invalid_range_operand.incompatible_domain",
    },

    InvalidExpressionReason => {
    &InvalidExpressionReason::ExpectedOperatorBeforeExpression => "invalid_expression.expected_operator_before_expression",
    &InvalidExpressionReason::UnresolvedStackShape => "invalid_expression.unresolved_stack_shape",
    &InvalidExpressionReason::MothFileHasNoValue => "invalid_expression.moth_file_has_no_value",
    &InvalidExpressionReason::ExtensionlessFileValue => "invalid_expression.extensionless_file_value",
    &InvalidExpressionReason::EmptyRuntimeAnonymousRecord => "invalid_expression.empty_runtime_anonymous_record",
    &InvalidExpressionReason::AnonymousRecordFieldNotNamed => "invalid_expression.anonymous_record_field_not_named",
    },

    InvalidStandaloneStatementReason => {
    &InvalidStandaloneStatementReason::FieldRead => "invalid_standalone_statement.field_read",
    &InvalidStandaloneStatementReason::Expression => "invalid_standalone_statement.expression",
    &InvalidStandaloneStatementReason::StandaloneTemplate => "invalid_standalone_statement.standalone_template",
    },

    InvalidMatchArmReason => {
    &InvalidMatchArmReason::SemicolonDelimiter => "invalid_match_arm.semicolon_delimiter",
    &InvalidMatchArmReason::LegacyColonSyntax => "invalid_match_arm.legacy_colon_syntax",
    &InvalidMatchArmReason::LegacyElseSyntax => "invalid_match_arm.legacy_else_syntax",
    &InvalidMatchArmReason::InvalidArrow => "invalid_match_arm.invalid_arrow",
    &InvalidMatchArmReason::ArmMustStartNewLine => "invalid_match_arm.arm_must_start_new_line",
    &InvalidMatchArmReason::ExpectedArmHeader => "invalid_match_arm.expected_arm_header",
    },
    InvalidMatchGuardReason => {
    &InvalidMatchGuardReason::SideEffectingPrelude => "invalid_match_guard.side_effecting_prelude",
    },

    InvalidLoopHeaderReason => {
        @delegate(reason => InvalidLoopHeaderCode::of(reason).stable_reason_key());
    },
    InvalidLoopHeaderCode => {
    &InvalidLoopHeaderCode::EmptyHeader => "invalid_loop_header.empty_header",
    &InvalidLoopHeaderCode::MissingColon => "invalid_loop_header.missing_colon",
    &InvalidLoopHeaderCode::RemovedInSyntax => "invalid_loop_header.removed_in_syntax",
    &InvalidLoopHeaderCode::MissingClosingPipe => "invalid_loop_header.missing_closing_pipe",
    &InvalidLoopHeaderCode::MalformedBindingPipes => "invalid_loop_header.malformed_binding_pipes",
    &InvalidLoopHeaderCode::MissingSourceBeforeBindings => "invalid_loop_header.missing_source_before_bindings",
    &InvalidLoopHeaderCode::EmptyBindingList => "invalid_loop_header.empty_binding_list",
    &InvalidLoopHeaderCode::ThisBinding => "invalid_loop_header.this_binding",
    &InvalidLoopHeaderCode::BindingMustBeSymbol => "invalid_loop_header.binding_must_be_symbol",
    &InvalidLoopHeaderCode::MissingBindingComma => "invalid_loop_header.missing_binding_comma",
    &InvalidLoopHeaderCode::TrailingBindingComma => "invalid_loop_header.trailing_binding_comma",
    &InvalidLoopHeaderCode::BareSingleBinding => "invalid_loop_header.bare_single_binding",
    &InvalidLoopHeaderCode::BareDualBinding => "invalid_loop_header.bare_dual_binding",
    &InvalidLoopHeaderCode::TooManyBindings => "invalid_loop_header.too_many_bindings",
    &InvalidLoopHeaderCode::DuplicateBindingName => "invalid_loop_header.duplicate_binding_name",
    &InvalidLoopHeaderCode::BindingAlreadyDeclared => "invalid_loop_header.binding_already_declared",
    &InvalidLoopHeaderCode::CollectionSourceNotCollection => "invalid_loop_header.collection_source_not_collection",
    &InvalidLoopHeaderCode::MissingRangeSeparator => "invalid_loop_header.missing_range_separator",
    &InvalidLoopHeaderCode::MissingRangeEndBound => "invalid_loop_header.missing_range_end_bound",
    &InvalidLoopHeaderCode::MissingRangeStep => "invalid_loop_header.missing_range_step",
    &InvalidLoopHeaderCode::FloatRangeMissingStep => "invalid_loop_header.float_range_missing_step",
    &InvalidLoopHeaderCode::ZeroRangeStep => "invalid_loop_header.zero_range_step",
    &InvalidLoopHeaderCode::ExpectedHeaderExpression => "invalid_loop_header.expected_header_expression",
    },

    InvalidStatementPositionReason => {
    &InvalidStatementPositionReason::UnexpectedComma => "invalid_statement_position.unexpected_comma",
    &InvalidStatementPositionReason::UnexpectedCloseParenthesis => "invalid_statement_position.unexpected_close_parenthesis",
    &InvalidStatementPositionReason::UnexpectedCloseCurly => "invalid_statement_position.unexpected_close_curly",
    &InvalidStatementPositionReason::UnexpectedPipe => "invalid_statement_position.unexpected_pipe",
    &InvalidStatementPositionReason::UnexpectedArrow => "invalid_statement_position.unexpected_arrow",
    &InvalidStatementPositionReason::UnexpectedWildcard => "invalid_statement_position.unexpected_wildcard",
    &InvalidStatementPositionReason::AnonymousDeclaredRegion => "invalid_statement_position.anonymous_declared_region",
    &InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader => "invalid_statement_position.generic_parameter_outside_declaration_header",
    &InvalidStatementPositionReason::UnexpectedOf => "invalid_statement_position.unexpected_of",
    &InvalidStatementPositionReason::UnexpectedScopeCloseInExpression => "invalid_statement_position.unexpected_scope_close_in_expression",
    &InvalidStatementPositionReason::UnexpectedScopeCloseInTemplate => "invalid_statement_position.unexpected_scope_close_in_template",
    },

    CommonSyntaxMistakeReason => {
        @delegate(reason => CommonSyntaxMistakeCode::of(reason).stable_reason_key());
    },
    CommonSyntaxMistakeCode => {
    &CommonSyntaxMistakeCode::EqualityOperator => "common_syntax_mistake.equality_operator",
    &CommonSyntaxMistakeCode::InequalityOperator => "common_syntax_mistake.inequality_operator",
    &CommonSyntaxMistakeCode::LogicalAndOperator => "common_syntax_mistake.logical_and_operator",
    &CommonSyntaxMistakeCode::LogicalOrOperator => "common_syntax_mistake.logical_or_operator",
    &CommonSyntaxMistakeCode::BooleanBangNegation => "common_syntax_mistake.boolean_bang_negation",
    &CommonSyntaxMistakeCode::ExpressionAssignment => "common_syntax_mistake.expression_assignment",
    &CommonSyntaxMistakeCode::RustBorrowPrefix => "common_syntax_mistake.rust_borrow_prefix",
    &CommonSyntaxMistakeCode::InvalidAsOperator => "common_syntax_mistake.invalid_as_operator",
    &CommonSyntaxMistakeCode::StatementLineComment => "common_syntax_mistake.statement_line_comment",
    &CommonSyntaxMistakeCode::FunctionKeyword => "common_syntax_mistake.function_keyword",
    &CommonSyntaxMistakeCode::LetOrVarKeyword => "common_syntax_mistake.let_or_var_keyword",
    &CommonSyntaxMistakeCode::ConstKeyword => "common_syntax_mistake.const_keyword",
    &CommonSyntaxMistakeCode::MatchKeyword => "common_syntax_mistake.match_keyword",
    &CommonSyntaxMistakeCode::StructKeyword => "common_syntax_mistake.struct_keyword",
    &CommonSyntaxMistakeCode::SignatureParenthesisDelimiter => "common_syntax_mistake.signature_parenthesis_delimiter",
    &CommonSyntaxMistakeCode::SignatureAsKeyword => "common_syntax_mistake.signature_as_keyword",
    &CommonSyntaxMistakeCode::InvalidCompileTimeBindingSpacing => "common_syntax_mistake.invalid_compile_time_binding_spacing",
    &CommonSyntaxMistakeCode::InvalidConfigQualifierSpacing => "common_syntax_mistake.invalid_config_qualifier_spacing",
    &CommonSyntaxMistakeCode::InvalidMutableBindingSpacing => "common_syntax_mistake.invalid_mutable_binding_spacing",
    &CommonSyntaxMistakeCode::InvalidSymbolicSpacing => "common_syntax_mistake.invalid_symbolic_spacing",
    &CommonSyntaxMistakeCode::InvalidChoiceVariantSpacing => "common_syntax_mistake.invalid_choice_variant_spacing",
    &CommonSyntaxMistakeCode::InvalidUnaryNegationSpacing => "common_syntax_mistake.invalid_unary_negation_spacing",
    &CommonSyntaxMistakeCode::UnsupportedUnaryPlus => "common_syntax_mistake.unsupported_unary_plus",
    },

    InvalidGenericInstantiationReason => {
        @delegate(reason => InvalidGenericInstantiationCode::of(reason).stable_reason_key());
    },
    InvalidGenericInstantiationCode => {
        &InvalidGenericInstantiationCode::WrongArgumentCount => "invalid_generic_instantiation.wrong_argument_count",
        &InvalidGenericInstantiationCode::TypeDoesNotAcceptArguments => "invalid_generic_instantiation.type_does_not_accept_arguments",
        &InvalidGenericInstantiationCode::OptionTypeSyntaxNotSupported => "invalid_generic_instantiation.option_type_syntax_not_supported",
        &InvalidGenericInstantiationCode::ResultTypeSyntaxNotSupported => "invalid_generic_instantiation.result_type_syntax_not_supported",
        &InvalidGenericInstantiationCode::ExternalTypeArgumentsUnsupported => "invalid_generic_instantiation.external_type_arguments_unsupported",
        &InvalidGenericInstantiationCode::MissingTypeArguments => "invalid_generic_instantiation.missing_type_arguments",
        &InvalidGenericInstantiationCode::CannotInferArguments => "invalid_generic_instantiation.cannot_infer_arguments",
        &InvalidGenericInstantiationCode::CannotInferFunctionArguments => "invalid_generic_instantiation.cannot_infer_function_arguments",
        &InvalidGenericInstantiationCode::ConflictingInference => "invalid_generic_instantiation.conflicting_inference",
        &InvalidGenericInstantiationCode::MissingTraitEvidence => "invalid_generic_instantiation.missing_trait_evidence",
        &InvalidGenericInstantiationCode::MissingNominalTraitEvidence => "invalid_generic_instantiation.missing_nominal_trait_evidence",
        &InvalidGenericInstantiationCode::RecursiveFunctionInstantiation => "invalid_generic_instantiation.recursive_function_instantiation",
        &InvalidGenericInstantiationCode::ExplicitCallTypeArgumentsUnsupported => "invalid_generic_instantiation.explicit_call_type_arguments_unsupported",
        &InvalidGenericInstantiationCode::GenericFunctionValueDeferred => "invalid_generic_instantiation.generic_function_value_deferred",
    },

    ProjectContextEscapeReason => {
        &ProjectContextEscapeReason::ExportedDeclaration => "project_context_escape.exported_declaration",
        &ProjectContextEscapeReason::ReachableExecutable => "project_context_escape.reachable_executable",
    },
}
