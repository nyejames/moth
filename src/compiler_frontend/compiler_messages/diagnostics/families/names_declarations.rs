//! Family rows for name and declaration diagnostics.
//!
//! WHAT: compact rows, constructors, typed views and renderers for name and declaration diagnostics.
//! WHY: the group keeps each family beside its stable code, facts, labels, and renderer.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptStringFact, OptTokenFact, StringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_const_record_used_as_value, render_deferred_feature, render_duplicate_declaration,
    render_identifier_naming_convention, render_invalid_assignment_target,
    render_invalid_declaration, render_invalid_signature_member,
    render_invalid_struct_default_value, render_invalid_type_annotation,
    render_missing_declaration_initializer, render_reserved_builtin_name,
    render_reserved_name_collision, render_shadowed_name, render_title_only,
    render_type_used_as_value, render_unknown_name, render_unknown_type_name,
    render_unknown_value_name, render_value_used_as_type,
};
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureDiagnosticKind, DeferredFeatureReason, DiagnosticCategory, DiagnosticKind,
    DiagnosticPayload, DiagnosticSeverity, DiagnosticToken, InvalidAssignmentTargetReason,
    InvalidDeclarationReason, InvalidSignatureMemberReason, InvalidTypeAnnotationReason,
    NameNamespace, NamingConvention, ReservedNameOwner, RuleDiagnosticKind, SyntaxDiagnosticKind,
    TypeAnnotationContext,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl NameNamespace {
        Value = 1,
        Type = 2,
        Module = 3,
        Field = 4,
        Variant = 5,
        Function = 6,
        Method = 7,
        TemplateSlot = 8,
        ConfigKey = 9,
    }
}
unit_code! {
    impl ReservedNameOwner {
        BuiltinType = 1,
        Keyword = 2,
        CoreTrait = 3,
        ImplicitStart = 4,
        ParameterContract = 5,
    }
}
unit_code! {
    impl InvalidAssignmentTargetReason {
        TemporaryNotAssignable = 1,
        ImmutableBinding = 2,
        ImmutableFieldRoot = 3,
        UnavailableInCatchRecovery = 4,
        CollectionGetTargetNotWritable = 5,
        MapGetTargetNotWritable = 6,
        ReadOnlyMapProperty = 7,
        ExpectedAssignmentOperator = 8,
        MutableMarkerOnAssignmentTarget = 9,
    }
}
unit_code! {
    pub(crate) enum InvalidDeclarationCode {
        ReservedBuiltinName = 1,
        DeclarationRequiresModuleTopLevel = 2,
        ConstantCannotBeMutable = 3,
        ExternalTypeLiteralConstruction = 4,
        ParameterizedGenericTypeAlias = 5,
        UnusedGenericParameter = 6,
        RecursiveGenericType = 7,
        RecursiveRuntimeStruct = 8,
        ExternalTypeAlias = 9,
        InvalidGenericParameterName = 10,
        DuplicateGenericParameter = 11,
        GenericParameterNameCollision = 12,
        ReservedGenericParameterName = 13,
        GenericTraitsUnsupported = 14,
        InvalidTraitName = 15,
        TraitConformanceMissingTrait = 16,
        TraitConformanceSemicolon = 17,
        TraitIncompatibilityMissingTrait = 18,
        TraitIncompatibilitySemicolon = 19,
        MissingInitializerExpression = 20,
    }
}

impl InvalidDeclarationCode {
    pub(crate) fn of(reason: &InvalidDeclarationReason) -> Self {
        Self::split(reason).0
    }

    fn split(reason: &InvalidDeclarationReason) -> (Self, Option<StringId>) {
        match reason {
            InvalidDeclarationReason::ReservedBuiltinName => (Self::ReservedBuiltinName, None),
            InvalidDeclarationReason::DeclarationRequiresModuleTopLevel => {
                (Self::DeclarationRequiresModuleTopLevel, None)
            }
            InvalidDeclarationReason::ConstantCannotBeMutable => {
                (Self::ConstantCannotBeMutable, None)
            }
            InvalidDeclarationReason::ExternalTypeLiteralConstruction => {
                (Self::ExternalTypeLiteralConstruction, None)
            }
            InvalidDeclarationReason::ParameterizedGenericTypeAlias => {
                (Self::ParameterizedGenericTypeAlias, None)
            }
            InvalidDeclarationReason::UnusedGenericParameter { parameter_name } => {
                (Self::UnusedGenericParameter, Some(*parameter_name))
            }
            InvalidDeclarationReason::RecursiveGenericType => (Self::RecursiveGenericType, None),
            InvalidDeclarationReason::RecursiveRuntimeStruct { .. } => {
                (Self::RecursiveRuntimeStruct, None)
            }
            InvalidDeclarationReason::ExternalTypeAlias { type_name } => {
                (Self::ExternalTypeAlias, Some(*type_name))
            }
            InvalidDeclarationReason::InvalidGenericParameterName { parameter_name } => {
                (Self::InvalidGenericParameterName, Some(*parameter_name))
            }
            InvalidDeclarationReason::DuplicateGenericParameter { parameter_name } => {
                (Self::DuplicateGenericParameter, Some(*parameter_name))
            }
            InvalidDeclarationReason::GenericParameterNameCollision { parameter_name } => {
                (Self::GenericParameterNameCollision, Some(*parameter_name))
            }
            InvalidDeclarationReason::ReservedGenericParameterName { parameter_name } => {
                (Self::ReservedGenericParameterName, Some(*parameter_name))
            }
            InvalidDeclarationReason::GenericTraitsUnsupported => {
                (Self::GenericTraitsUnsupported, None)
            }
            InvalidDeclarationReason::InvalidTraitName => (Self::InvalidTraitName, None),
            InvalidDeclarationReason::TraitConformanceMissingTrait => {
                (Self::TraitConformanceMissingTrait, None)
            }
            InvalidDeclarationReason::TraitConformanceSemicolon => {
                (Self::TraitConformanceSemicolon, None)
            }
            InvalidDeclarationReason::TraitIncompatibilityMissingTrait => {
                (Self::TraitIncompatibilityMissingTrait, None)
            }
            InvalidDeclarationReason::TraitIncompatibilitySemicolon => {
                (Self::TraitIncompatibilitySemicolon, None)
            }
            InvalidDeclarationReason::MissingInitializerExpression => {
                (Self::MissingInitializerExpression, None)
            }
        }
    }

    fn to_reason(self, detail: Option<StringId>) -> Option<InvalidDeclarationReason> {
        Some(match self {
            Self::ReservedBuiltinName => InvalidDeclarationReason::ReservedBuiltinName,
            Self::DeclarationRequiresModuleTopLevel => {
                InvalidDeclarationReason::DeclarationRequiresModuleTopLevel
            }
            Self::ConstantCannotBeMutable => InvalidDeclarationReason::ConstantCannotBeMutable,
            Self::ExternalTypeLiteralConstruction => {
                InvalidDeclarationReason::ExternalTypeLiteralConstruction
            }
            Self::ParameterizedGenericTypeAlias => {
                InvalidDeclarationReason::ParameterizedGenericTypeAlias
            }
            Self::UnusedGenericParameter => InvalidDeclarationReason::UnusedGenericParameter {
                parameter_name: detail?,
            },
            Self::RecursiveGenericType => InvalidDeclarationReason::RecursiveGenericType,
            // The cycle is carried separately in this family's `StringIds` extra.
            Self::RecursiveRuntimeStruct => {
                InvalidDeclarationReason::RecursiveRuntimeStruct { cycle: Vec::new() }
            }
            Self::ExternalTypeAlias => {
                InvalidDeclarationReason::ExternalTypeAlias { type_name: detail? }
            }
            Self::InvalidGenericParameterName => {
                InvalidDeclarationReason::InvalidGenericParameterName {
                    parameter_name: detail?,
                }
            }
            Self::DuplicateGenericParameter => {
                InvalidDeclarationReason::DuplicateGenericParameter {
                    parameter_name: detail?,
                }
            }
            Self::GenericParameterNameCollision => {
                InvalidDeclarationReason::GenericParameterNameCollision {
                    parameter_name: detail?,
                }
            }
            Self::ReservedGenericParameterName => {
                InvalidDeclarationReason::ReservedGenericParameterName {
                    parameter_name: detail?,
                }
            }
            Self::GenericTraitsUnsupported => InvalidDeclarationReason::GenericTraitsUnsupported,
            Self::InvalidTraitName => InvalidDeclarationReason::InvalidTraitName,
            Self::TraitConformanceMissingTrait => {
                InvalidDeclarationReason::TraitConformanceMissingTrait
            }
            Self::TraitConformanceSemicolon => InvalidDeclarationReason::TraitConformanceSemicolon,
            Self::TraitIncompatibilityMissingTrait => {
                InvalidDeclarationReason::TraitIncompatibilityMissingTrait
            }
            Self::TraitIncompatibilitySemicolon => {
                InvalidDeclarationReason::TraitIncompatibilitySemicolon
            }
            Self::MissingInitializerExpression => {
                InvalidDeclarationReason::MissingInitializerExpression
            }
        })
    }
}

pub(crate) fn decode_invalid_declaration_reason(
    code: InvalidDeclarationCode,
    detail: Option<StringId>,
) -> Option<InvalidDeclarationReason> {
    code.to_reason(detail)
}

unit_code! {
    pub(crate) enum DeferredFeatureCode {
        NamedFeature = 1,
        CaptureTaggedPattern = 2,
        NegatedMatchPattern = 3,
        NamedPayloadPatternAssignment = 4,
        NestedPayloadPattern = 5,
        ChoiceVariantDefaultValue = 6,
        GenericReceiverMethod = 7,
        DeclaredRegion = 8,
        CheckedBlock = 9,
        AsyncBlock = 10,
        RuntimeAnonymousRecord = 11,
    }
}

impl DeferredFeatureCode {
    pub(crate) fn of(reason: &DeferredFeatureReason) -> Self {
        Self::split(reason).0
    }

    fn split(reason: &DeferredFeatureReason) -> (Self, Option<StringId>) {
        match reason {
            DeferredFeatureReason::NamedFeature { feature } => (Self::NamedFeature, Some(*feature)),
            DeferredFeatureReason::CaptureTaggedPattern => (Self::CaptureTaggedPattern, None),
            DeferredFeatureReason::NegatedMatchPattern => (Self::NegatedMatchPattern, None),
            DeferredFeatureReason::NamedPayloadPatternAssignment => {
                (Self::NamedPayloadPatternAssignment, None)
            }
            DeferredFeatureReason::NestedPayloadPattern => (Self::NestedPayloadPattern, None),
            DeferredFeatureReason::ChoiceVariantDefaultValue => {
                (Self::ChoiceVariantDefaultValue, None)
            }
            DeferredFeatureReason::GenericReceiverMethod => (Self::GenericReceiverMethod, None),
            DeferredFeatureReason::DeclaredRegion => (Self::DeclaredRegion, None),
            DeferredFeatureReason::CheckedBlock => (Self::CheckedBlock, None),
            DeferredFeatureReason::AsyncBlock => (Self::AsyncBlock, None),
            DeferredFeatureReason::RuntimeAnonymousRecord => (Self::RuntimeAnonymousRecord, None),
        }
    }

    pub(crate) fn to_reason(self, feature: Option<StringId>) -> Option<DeferredFeatureReason> {
        Some(match self {
            Self::NamedFeature => DeferredFeatureReason::NamedFeature { feature: feature? },
            Self::CaptureTaggedPattern => DeferredFeatureReason::CaptureTaggedPattern,
            Self::NegatedMatchPattern => DeferredFeatureReason::NegatedMatchPattern,
            Self::NamedPayloadPatternAssignment => {
                DeferredFeatureReason::NamedPayloadPatternAssignment
            }
            Self::NestedPayloadPattern => DeferredFeatureReason::NestedPayloadPattern,
            Self::ChoiceVariantDefaultValue => DeferredFeatureReason::ChoiceVariantDefaultValue,
            Self::GenericReceiverMethod => DeferredFeatureReason::GenericReceiverMethod,
            Self::DeclaredRegion => DeferredFeatureReason::DeclaredRegion,
            Self::CheckedBlock => DeferredFeatureReason::CheckedBlock,
            Self::AsyncBlock => DeferredFeatureReason::AsyncBlock,
            Self::RuntimeAnonymousRecord => DeferredFeatureReason::RuntimeAnonymousRecord,
        })
    }
}
unit_code! {
    impl NamingConvention {
        CamelCase = 1,
        LowercaseWithUnderscores = 2,
        UppercaseWithUnderscores = 3,
        LowercaseOrUppercaseWithUnderscores = 4,
    }
}

unit_code! {
    impl InvalidSignatureMemberReason {
        ChoicePayloadMutable = 1,
        ChoicePayloadDefaultValue = 2,
        CompileTimeParameterDeferred = 3,
        ThisNotAllowed = 4,
        TrailingComma = 5,
        TraitReceiverMustBeThis = 6,
        TraitMutableThisOnlyFirstParameter = 7,
        TraitBareThisOnlyReceiver = 8,
        TraitRequirementDefaultValue = 9,
        MissingDefaultValue = 10,
    }
}

unit_code! {
    impl TypeAnnotationContext {
        DeclarationTarget = 1,
        BuildConfigContract = 2,
        SignatureParameter = 3,
        SignatureReturn = 4,
        TypeAliasTarget = 5,
        TraitRequirement = 6,
    }
}

unit_code! {
    /// WHAT: codes invalid type-annotation reasons separately from their optional found token.
    /// WHY: the code owns the stable reason key, keeping the context and token within four words.
    pub(crate) enum InvalidTypeAnnotationCode {
        NoneNotAllowed = 1,
        ReservedTraitKeyword = 2,
        TraitThisMustBeDirect = 3,
        AsNotValidHere = 4,
        UnexpectedColon = 5,
        InvalidTokenAfterName = 6,
        ExpectedTypeAnnotation = 7,
        DuplicateOptional = 8,
        NestedOptional = 9,
    }
}

impl InvalidTypeAnnotationCode {
    /// Splits a legacy reason into its compact code and optional token fact.
    pub(crate) fn split(reason: &InvalidTypeAnnotationReason) -> (Self, Option<DiagnosticToken>) {
        match reason {
            InvalidTypeAnnotationReason::NoneNotAllowed => (Self::NoneNotAllowed, None),
            InvalidTypeAnnotationReason::ReservedTraitKeyword => (Self::ReservedTraitKeyword, None),
            InvalidTypeAnnotationReason::TraitThisMustBeDirect => {
                (Self::TraitThisMustBeDirect, None)
            }
            InvalidTypeAnnotationReason::AsNotValidHere => (Self::AsNotValidHere, None),
            InvalidTypeAnnotationReason::UnexpectedColon => (Self::UnexpectedColon, None),
            InvalidTypeAnnotationReason::InvalidTokenAfterName { token } => {
                (Self::InvalidTokenAfterName, Some(*token))
            }
            InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found } => {
                (Self::ExpectedTypeAnnotation, Some(*found))
            }
            InvalidTypeAnnotationReason::DuplicateOptional => (Self::DuplicateOptional, None),
            InvalidTypeAnnotationReason::NestedOptional => (Self::NestedOptional, None),
        }
    }

    /// Rebuilds the legacy reason for the existing prose helper.
    pub(crate) fn to_reason(
        self,
        token: Option<DiagnosticToken>,
    ) -> Option<InvalidTypeAnnotationReason> {
        Some(match self {
            Self::NoneNotAllowed => InvalidTypeAnnotationReason::NoneNotAllowed,
            Self::ReservedTraitKeyword => InvalidTypeAnnotationReason::ReservedTraitKeyword,
            Self::TraitThisMustBeDirect => InvalidTypeAnnotationReason::TraitThisMustBeDirect,
            Self::AsNotValidHere => InvalidTypeAnnotationReason::AsNotValidHere,
            Self::UnexpectedColon => InvalidTypeAnnotationReason::UnexpectedColon,
            Self::InvalidTokenAfterName => {
                InvalidTypeAnnotationReason::InvalidTokenAfterName { token: token? }
            }
            Self::ExpectedTypeAnnotation => {
                InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found: token? }
            }
            Self::DuplicateOptional => InvalidTypeAnnotationReason::DuplicateOptional,
            Self::NestedOptional => InvalidTypeAnnotationReason::NestedOptional,
        })
    }
}

diagnostic_family! {
    DUPLICATE_DECLARATION {
        internal: 3002,
        stable: "MOTH-RULE-0002",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate declaration",
        facts: [name: StringFact],
        labels: [LabelMessageCode::PREVIOUS_DECLARATION],
        extra: None,
        ctor: manual,
        view: DuplicateDeclarationView,
        render: render_duplicate_declaration,
    }
}

diagnostic_family! {
    UNKNOWN_VALUE_NAME {
        internal: 3034,
        stable: "MOTH-RULE-0034",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unknown value name",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: generated unknown_value_name,
        view: UnknownValueNameView,
        render: render_unknown_value_name,
    }
}

diagnostic_family! {
    UNKNOWN_TYPE_NAME {
        internal: 3035,
        stable: "MOTH-RULE-0035",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unknown type name",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: generated unknown_type_name,
        view: UnknownTypeNameView,
        render: render_unknown_type_name,
    }
}

/// The duplicate-declaration draft. The primary site is the duplicate. The earlier declaration is not part of this
/// constructor: the legacy adapter adds it as a label, and only when its span is known.
pub(crate) fn duplicate_declaration(
    name: StringId,
    duplicate_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&DUPLICATE_DECLARATION, duplicate_span, |cursor| {
        cursor.push::<StringFact>(&name)
    })
}

pub(crate) fn shadowed_name(name: StringId, span: Option<SourceSpan>) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&SHADOWED_NAME, span, |cursor| {
        cursor.push::<StringFact>(&name)
    })
}

pub(crate) fn invalid_assignment_target(
    reason: InvalidAssignmentTargetReason,
    target_name: Option<StringId>,
    _target_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
    field_name: Option<StringId>,
    root_binding_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_ASSIGNMENT_TARGET, span, |cursor| {
        cursor.push::<UnitFact<InvalidAssignmentTargetReason>>(&reason)?;
        cursor.push::<OptStringFact>(&target_name)?;
        cursor.push::<OptStringFact>(&field_name)?;
        cursor.push::<OptStringFact>(&root_binding_name)
    })
}

pub(crate) fn invalid_declaration(
    reason: InvalidDeclarationReason,
    name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, detail) = InvalidDeclarationCode::split(&reason);
    let draft = DiagnosticDraft::with_facts(&INVALID_DECLARATION, span, |cursor| {
        cursor.push::<UnitFact<InvalidDeclarationCode>>(&code)?;
        cursor.push::<OptStringFact>(&name)?;
        cursor.push::<OptStringFact>(&detail)
    });
    match reason {
        InvalidDeclarationReason::RecursiveRuntimeStruct { cycle } => draft.with_string_ids(cycle),
        _ => draft,
    }
}

pub(crate) fn deferred_feature(feature: StringId, span: Option<SourceSpan>) -> DiagnosticDraft {
    deferred_feature_reason(DeferredFeatureReason::NamedFeature { feature }, span)
}

pub(crate) fn deferred_feature_reason(
    reason: DeferredFeatureReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, feature) = DeferredFeatureCode::split(&reason);
    DiagnosticDraft::with_facts(&DEFERRED_FEATURE, span, |cursor| {
        cursor.push::<UnitFact<DeferredFeatureCode>>(&code)?;
        cursor.push::<OptStringFact>(&feature)
    })
}
/// The invalid-type-annotation draft preserves its context, reason code and optional found token.
pub(crate) fn invalid_type_annotation(
    context: TypeAnnotationContext,
    reason: InvalidTypeAnnotationReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, token) = InvalidTypeAnnotationCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_TYPE_ANNOTATION, span, |cursor| {
        cursor.push::<UnitFact<TypeAnnotationContext>>(&context)?;
        cursor.push::<UnitFact<InvalidTypeAnnotationCode>>(&code)?;
        cursor.push::<OptTokenFact>(&token)
    })
}
/// Mirrors the legacy constructor by selecting the compact family from the namespace pair.
pub(crate) fn namespace_misuse(
    name: StringId,
    expected: NameNamespace,
    found: NameNamespace,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    match (expected, found) {
        (NameNamespace::Type, NameNamespace::Value) => {
            value_used_as_type_row(name, expected, found, span)
        }
        (NameNamespace::Value, NameNamespace::Type) => {
            type_used_as_value_row(name, expected, found, span)
        }
        _ => unknown_name_row(name, expected, found, span),
    }
}

/// Maps this group's legacy diagnostics onto compact constructors. Declines other payloads.
pub(super) fn from_legacy(
    kind: DiagnosticKind,
    payload: DiagnosticPayload,
    span: Option<SourceSpan>,
) -> Result<DiagnosticDraft, DiagnosticPayload> {
    match (kind, payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
            DiagnosticPayload::DuplicateDeclaration { name },
        ) => Ok(duplicate_declaration(name, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTypeAnnotation),
            DiagnosticPayload::InvalidTypeAnnotation { context, reason },
        ) => Ok(invalid_type_annotation(context, reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::IdentifierNamingConvention),
            DiagnosticPayload::IdentifierNamingConvention {
                name,
                expected_style,
            },
        ) => Ok(identifier_naming_convention(name, expected_style, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName),
            DiagnosticPayload::ReservedBuiltinName { name },
        ) => Ok(reserved_builtin_name(name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidSignatureMember),
            DiagnosticPayload::InvalidSignatureMember { reason },
        ) => Ok(invalid_signature_member(reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if !matches!(
            (expected, found),
            (NameNamespace::Type, NameNamespace::Value)
                | (NameNamespace::Value, NameNamespace::Type)
        ) =>
        {
            Ok(namespace_misuse(name, expected, found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ValueUsedAsType),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if expected == NameNamespace::Type && found == NameNamespace::Value => {
            Ok(namespace_misuse(name, expected, found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::TypeUsedAsValue),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if expected == NameNamespace::Value && found == NameNamespace::Type => {
            Ok(namespace_misuse(name, expected, found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            },
        ) => Ok(unknown_value_name(name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTypeName),
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Type,
            },
        ) => Ok(unknown_type_name(name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement),
            DiagnosticPayload::None,
        ) => Ok(invalid_top_level_runtime_statement(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidStructDefaultValue),
            DiagnosticPayload::InvalidStructDefaultValue,
        ) => Ok(invalid_struct_default_value(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::MissingDeclarationInitializer),
            DiagnosticPayload::MissingDeclarationInitializer { name },
        ) => Ok(missing_declaration_initializer(name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ConstRecordUsedAsValue),
            DiagnosticPayload::ConstRecordUsedAsValue { record_name },
        ) => Ok(const_record_used_as_value(record_name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ShadowedName),
            DiagnosticPayload::ShadowedName { name },
        ) => Ok(shadowed_name(name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedNameCollision),
            DiagnosticPayload::ReservedNameCollision { name, reserved_by },
        ) => Ok(reserved_name_collision(name, reserved_by, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidAssignmentTarget),
            DiagnosticPayload::InvalidAssignmentTarget {
                reason,
                target_name,
                target_type,
                field_name,
                root_binding_name,
            },
        ) => Ok(invalid_assignment_target(
            reason,
            target_name,
            target_type,
            field_name,
            root_binding_name,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration),
            DiagnosticPayload::InvalidDeclaration { reason, name },
        ) => Ok(invalid_declaration(reason, name, span)),
        (
            DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::DeferredFeature),
            DiagnosticPayload::DeferredFeature { reason },
        ) => Ok(match reason {
            DeferredFeatureReason::NamedFeature { feature } => deferred_feature(feature, span),
            reason => deferred_feature_reason(reason, span),
        }),
        (_, payload) => Err(payload),
    }
}

diagnostic_family! {
    INVALID_TYPE_ANNOTATION {
        internal: 1014,
        stable: "MOTH-SYNTAX-0014",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid type annotation",
        facts: [
            context: UnitFact<TypeAnnotationContext>,
            code: UnitFact<InvalidTypeAnnotationCode>,
            token: OptTokenFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: code,
        view: InvalidTypeAnnotationView,
        render: render_invalid_type_annotation,
    }
}

diagnostic_family! {
    UNKNOWN_NAME {
        internal: 3001,
        stable: "MOTH-RULE-0001",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unknown name",
        facts: [
            name: StringFact,
            expected: UnitFact<NameNamespace>,
            found: UnitFact<NameNamespace>,
        ],
        labels: [],
        extra: None,
        ctor: generated unknown_name_row,
        view: UnknownNameView,
        render: render_unknown_name,
    }
}

diagnostic_family! {
    IDENTIFIER_NAMING_CONVENTION {
        internal: 3021,
        stable: "MOTH-RULE-0021",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Warning,
        title: "Identifier naming convention",
        facts: [
            name: StringFact,
            expected_style: UnitFact<NamingConvention>,
        ],
        labels: [],
        extra: None,
        ctor: generated identifier_naming_convention,
        view: IdentifierNamingConventionView,
        render: render_identifier_naming_convention,
    }
}

diagnostic_family! {
    INVALID_TOP_LEVEL_RUNTIME_STATEMENT {
        internal: 3023,
        stable: "MOTH-RULE-0023",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid top-level runtime statement",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated invalid_top_level_runtime_statement,
        view: InvalidTopLevelRuntimeStatementView,
        render: render_title_only,
    }
}

diagnostic_family! {
    RESERVED_BUILTIN_NAME {
        internal: 3027,
        stable: "MOTH-RULE-0027",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Reserved builtin name",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: generated reserved_builtin_name,
        view: ReservedBuiltinNameView,
        render: render_reserved_builtin_name,
    }
}

diagnostic_family! {
    INVALID_SIGNATURE_MEMBER {
        internal: 3028,
        stable: "MOTH-RULE-0028",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid signature member",
        facts: [reason: UnitFact<InvalidSignatureMemberReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_signature_member,
        reason_key: reason,
        view: InvalidSignatureMemberView,
        render: render_invalid_signature_member,
    }
}

diagnostic_family! {
    INVALID_STRUCT_DEFAULT_VALUE {
        internal: 3030,
        stable: "MOTH-RULE-0030",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid struct default value",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated invalid_struct_default_value,
        view: InvalidStructDefaultValueView,
        render: render_invalid_struct_default_value,
    }
}

diagnostic_family! {
    MISSING_DECLARATION_INITIALIZER {
        internal: 3031,
        stable: "MOTH-RULE-0031",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Missing declaration initializer",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: generated missing_declaration_initializer,
        view: MissingDeclarationInitializerView,
        render: render_missing_declaration_initializer,
    }
}

diagnostic_family! {
    VALUE_USED_AS_TYPE {
        internal: 3036,
        stable: "MOTH-RULE-0036",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Value used as type",
        facts: [
            name: StringFact,
            expected: UnitFact<NameNamespace>,
            found: UnitFact<NameNamespace>,
        ],
        labels: [],
        extra: None,
        ctor: generated value_used_as_type_row,
        view: ValueUsedAsTypeView,
        render: render_value_used_as_type,
    }
}

diagnostic_family! {
    TYPE_USED_AS_VALUE {
        internal: 3037,
        stable: "MOTH-RULE-0037",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Type used as value",
        facts: [
            name: StringFact,
            expected: UnitFact<NameNamespace>,
            found: UnitFact<NameNamespace>,
        ],
        labels: [],
        extra: None,
        ctor: generated type_used_as_value_row,
        view: TypeUsedAsValueView,
        render: render_type_used_as_value,
    }
}

diagnostic_family! {
    SHADOWED_NAME {
        internal: 3038,
        stable: "MOTH-RULE-0038",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Shadowed name",
        facts: [name: StringFact],
        labels: [LabelMessageCode::PREVIOUS_DECLARATION],
        extra: None,
        ctor: manual,
        view: ShadowedNameView,
        render: render_shadowed_name,
    }
}

diagnostic_family! {
    RESERVED_NAME_COLLISION {
        internal: 3039,
        stable: "MOTH-RULE-0039",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Reserved name collision",
        facts: [
            name: StringFact,
            reserved_by: UnitFact<ReservedNameOwner>,
        ],
        labels: [],
        extra: None,
        ctor: generated reserved_name_collision,
        view: ReservedNameCollisionView,
        render: render_reserved_name_collision,
    }
}

// For RecursiveRuntimeStruct, StringIds stores each cycle path's final component in traversal
// order, without a repeated closing entry.
diagnostic_family! {
    INVALID_DECLARATION {
        internal: 3043,
        stable: "MOTH-RULE-0043",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid declaration",
        facts: [
            code: UnitFact<InvalidDeclarationCode>,
            name: OptStringFact,
            detail: OptStringFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: code,
        view: InvalidDeclarationView,
        render: render_invalid_declaration,
    }
}

diagnostic_family! {
    INVALID_ASSIGNMENT_TARGET {
        internal: 3044,
        stable: "MOTH-RULE-0044",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid assignment target",
        facts: [
            reason: UnitFact<InvalidAssignmentTargetReason>,
            target_name: OptStringFact,
            field_name: OptStringFact,
            root_binding_name: OptStringFact,
        ],
        labels: [LabelMessageCode::IMMUTABLE_BINDING_DECLARATION],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidAssignmentTargetView,
        render: render_invalid_assignment_target,
    }
}

diagnostic_family! {
    CONST_RECORD_USED_AS_VALUE {
        internal: 3068,
        stable: "MOTH-RULE-0068",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Const record used as value",
        facts: [record_name: StringFact],
        labels: [],
        extra: None,
        ctor: generated const_record_used_as_value,
        view: ConstRecordUsedAsValueView,
        render: render_const_record_used_as_value,
    }
}

diagnostic_family! {
    DEFERRED_FEATURE {
        internal: 7001,
        stable: "MOTH-DEFERRED-0001",
        category: DiagnosticCategory::DeferredFeature,
        severity: DiagnosticSeverity::Error,
        title: "Deferred feature",
        facts: [
            code: UnitFact<DeferredFeatureCode>,
            feature: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: code,
        view: DeferredFeatureView,
        render: render_deferred_feature,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    DUPLICATE_DECLARATION,
    UNKNOWN_VALUE_NAME,
    UNKNOWN_TYPE_NAME,
    INVALID_TYPE_ANNOTATION,
    UNKNOWN_NAME,
    IDENTIFIER_NAMING_CONVENTION,
    INVALID_TOP_LEVEL_RUNTIME_STATEMENT,
    RESERVED_BUILTIN_NAME,
    INVALID_SIGNATURE_MEMBER,
    INVALID_STRUCT_DEFAULT_VALUE,
    MISSING_DECLARATION_INITIALIZER,
    VALUE_USED_AS_TYPE,
    TYPE_USED_AS_VALUE,
    SHADOWED_NAME,
    RESERVED_NAME_COLLISION,
    INVALID_DECLARATION,
    INVALID_ASSIGNMENT_TARGET,
    CONST_RECORD_USED_AS_VALUE,
    DEFERRED_FEATURE,
];
