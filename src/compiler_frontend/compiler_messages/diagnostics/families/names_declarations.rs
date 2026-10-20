//! Family rows for name and declaration diagnostics.
//!
//! WHAT: three existing name/declaration pilots and eight P4.4 families are full rows with mirror
//! constructors and typed views. The other eight rows remain identity declarations.
//! WHY: the group keeps the name and declaration families together, each with one code and category.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptTokenFact, StringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_duplicate_declaration, render_identifier_naming_convention,
    render_invalid_signature_member, render_invalid_type_annotation, render_reserved_builtin_name,
    render_title_only, render_type_used_as_value, render_unknown_name, render_unknown_type_name,
    render_unknown_value_name, render_value_used_as_type,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    DiagnosticToken, InvalidSignatureMemberReason, InvalidTypeAnnotationReason, NameNamespace,
    NamingConvention, RuleDiagnosticKind, SyntaxDiagnosticKind, TypeAnnotationContext,
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
    pilot DUPLICATE_DECLARATION {
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
    pilot UNKNOWN_VALUE_NAME {
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
    pilot UNKNOWN_TYPE_NAME {
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
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn duplicate_declaration(
    name: StringId,
    duplicate_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&DUPLICATE_DECLARATION, duplicate_span, |cursor| {
        cursor.push::<StringFact>(&name)
    })
}
/// The invalid-type-annotation draft preserves its context, reason code and optional found token.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_type_annotation(
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
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn namespace_misuse(
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
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
            DiagnosticPayload::DuplicateDeclaration { name },
        ) => Some(duplicate_declaration(*name, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTypeAnnotation),
            DiagnosticPayload::InvalidTypeAnnotation { context, reason },
        ) => Some(invalid_type_annotation(*context, reason.clone(), span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::IdentifierNamingConvention),
            DiagnosticPayload::IdentifierNamingConvention {
                name,
                expected_style,
            },
        ) => Some(identifier_naming_convention(*name, *expected_style, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName),
            DiagnosticPayload::ReservedBuiltinName { name },
        ) => Some(reserved_builtin_name(*name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidSignatureMember),
            DiagnosticPayload::InvalidSignatureMember { reason },
        ) => Some(invalid_signature_member(*reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if !matches!(
            (*expected, *found),
            (NameNamespace::Type, NameNamespace::Value)
                | (NameNamespace::Value, NameNamespace::Type)
        ) =>
        {
            Some(namespace_misuse(*name, *expected, *found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ValueUsedAsType),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if *expected == NameNamespace::Type && *found == NameNamespace::Value => {
            Some(namespace_misuse(*name, *expected, *found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::TypeUsedAsValue),
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        ) if *expected == NameNamespace::Value && *found == NameNamespace::Type => {
            Some(namespace_misuse(*name, *expected, *found, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            },
        ) => Some(unknown_value_name(*name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTypeName),
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Type,
            },
        ) => Some(unknown_type_name(*name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement),
            DiagnosticPayload::None,
        ) => Some(invalid_top_level_runtime_statement(span)),
        _ => None,
    }
}

diagnostic_family! {
    pilot INVALID_TYPE_ANNOTATION {
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
        view: InvalidTypeAnnotationView,
        render: render_invalid_type_annotation,
    }
}

diagnostic_family! {
    pilot UNKNOWN_NAME {
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
    pilot IDENTIFIER_NAMING_CONVENTION {
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
    pilot INVALID_TOP_LEVEL_RUNTIME_STATEMENT {
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
    pilot RESERVED_BUILTIN_NAME {
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
    pilot INVALID_SIGNATURE_MEMBER {
        internal: 3028,
        stable: "MOTH-RULE-0028",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid signature member",
        facts: [reason: UnitFact<InvalidSignatureMemberReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_signature_member,
        view: InvalidSignatureMemberView,
        render: render_invalid_signature_member,
    }
}

diagnostic_family! {
    identity INVALID_STRUCT_DEFAULT_VALUE {
        internal: 3030,
        stable: "MOTH-RULE-0030",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity MISSING_DECLARATION_INITIALIZER {
        internal: 3031,
        stable: "MOTH-RULE-0031",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    pilot VALUE_USED_AS_TYPE {
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
    pilot TYPE_USED_AS_VALUE {
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
    identity SHADOWED_NAME {
        internal: 3038,
        stable: "MOTH-RULE-0038",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity RESERVED_NAME_COLLISION {
        internal: 3039,
        stable: "MOTH-RULE-0039",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_DECLARATION {
        internal: 3043,
        stable: "MOTH-RULE-0043",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_ASSIGNMENT_TARGET {
        internal: 3044,
        stable: "MOTH-RULE-0044",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity CONST_RECORD_USED_AS_VALUE {
        internal: 3068,
        stable: "MOTH-RULE-0068",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DEFERRED_FEATURE {
        internal: 7001,
        stable: "MOTH-DEFERRED-0001",
        category: DiagnosticCategory::DeferredFeature,
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
