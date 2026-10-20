//! Family rows for generics, traits and API diagnostics.
//!
//! WHAT: ten rows are full for generics, trait and API diagnostics; eleven remain identity rows.
//! WHY: the table declares each family's stable code and category, so the registry covers this group.
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptStringFact, StringFact, TypeFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_duplicate_public_export, render_generic_bound_private_surface_leak,
    render_invalid_trait_incompatibility, render_invalid_trait_keyword_usage,
    render_private_type_in_exported_api, render_title_only, render_trait_name_used_as_type,
    render_unsupported_trait_feature,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, PublicGenericOwnerKind,
    RuleDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl InvalidTraitIncompatibilityReason {
        SelfIncompatible = 1,
        UnknownTrait = 2,
        DuplicateRelation = 3,
        PrivateTraitSurfaceLeak = 4,
    }
}

unit_code! {
    impl InvalidTraitKeywordUsageReason {
        MustOutsideTraitSyntax = 1,
        ThisOutsideTraitSyntax = 2,
    }
}

unit_code! {
    impl PublicGenericOwnerKind {
        Function = 1,
        ReceiverMethod = 2,
        Struct = 3,
        Choice = 4,
    }
}

diagnostic_family! {
    identity INVALID_GENERIC_APPLICATION {
        internal: 1015,
        stable: "MOTH-SYNTAX-0015",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_GENERIC_PARAMETER {
        internal: 1020,
        stable: "MOTH-SYNTAX-0020",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_PARAMETER_CONTRACT {
        internal: 1038,
        stable: "MOTH-SYNTAX-0038",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_THIS_USAGE {
        internal: 3040,
        stable: "MOTH-RULE-0040",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_RECEIVER_DECLARATION {
        internal: 3041,
        stable: "MOTH-RULE-0041",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_GENERIC_INSTANTIATION {
        internal: 3057,
        stable: "MOTH-RULE-0057",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_FUNCTION_SIGNATURE {
        internal: 3062,
        stable: "MOTH-RULE-0062",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity UNKNOWN_TRAIT {
        internal: 3069,
        stable: "MOTH-RULE-0069",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DUPLICATE_TRAIT_REQUIREMENT {
        internal: 3070,
        stable: "MOTH-RULE-0070",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity TRAIT_PRIVATE_SURFACE_LEAK {
        internal: 3071,
        stable: "MOTH-RULE-0071",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    pilot UNSUPPORTED_TRAIT_FEATURE {
        internal: 3072,
        stable: "MOTH-RULE-0072",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported trait feature",
        facts: [trait_name: StringFact, feature: StringFact],
        labels: [],
        extra: None,
        ctor: generated unsupported_trait_feature,
        view: UnsupportedTraitFeatureView,
        render: render_unsupported_trait_feature,
    }
}

diagnostic_family! {
    identity INVALID_TRAIT_CONFORMANCE {
        internal: 3073,
        stable: "MOTH-RULE-0073",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    pilot GENERIC_BOUND_PRIVATE_SURFACE_LEAK {
        internal: 3074,
        stable: "MOTH-RULE-0074",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Private trait exposed by generic bound",
        facts: [
            owner_kind: UnitFact<PublicGenericOwnerKind>,
            owner_name: StringFact,
            trait_name: StringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated generic_bound_private_surface_leak,
        view: GenericBoundPrivateSurfaceLeakView,
        render: render_generic_bound_private_surface_leak,
    }
}

diagnostic_family! {
    pilot TRAIT_NAME_USED_AS_TYPE {
        internal: 3075,
        stable: "MOTH-RULE-0075",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Trait name used as value type",
        facts: [trait_name: StringFact],
        labels: [],
        extra: None,
        ctor: generated trait_name_used_as_type,
        view: TraitNameUsedAsTypeView,
        render: render_trait_name_used_as_type,
    }
}

diagnostic_family! {
    pilot INVALID_TRAIT_KEYWORD_USAGE {
        internal: 3076,
        stable: "MOTH-RULE-0076",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid trait keyword usage",
        facts: [reason: UnitFact<InvalidTraitKeywordUsageReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_trait_keyword_usage,
        view: InvalidTraitKeywordUsageView,
        render: render_invalid_trait_keyword_usage,
    }
}

diagnostic_family! {
    pilot EXPORT_OUTSIDE_MODULE_ROOT {
        internal: 3077,
        stable: "MOTH-RULE-0077",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "`export:` is only valid in a module root file",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated export_outside_module_root,
        view: ExportOutsideModuleRootView,
        render: render_title_only,
    }
}
/// Maps legacy diagnostics in this group onto their compact constructors.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ExportOutsideModuleRoot),
            DiagnosticPayload::None,
        ) => Some(export_outside_module_root(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedTraitFeature),
            DiagnosticPayload::UnsupportedTraitFeature {
                trait_name,
                feature,
            },
        ) => Some(unsupported_trait_feature(*trait_name, *feature, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitNameUsedAsType),
            DiagnosticPayload::TraitNameUsedAsType { trait_name },
        ) => Some(trait_name_used_as_type(*trait_name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::GenericBoundPrivateSurfaceLeak),
            DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
                owner_kind,
                owner_name,
                trait_name,
            },
        ) => Some(generic_bound_private_surface_leak(
            *owner_kind,
            *owner_name,
            *trait_name,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitKeywordUsage),
            DiagnosticPayload::InvalidTraitKeywordUsage { reason },
        ) => Some(invalid_trait_keyword_usage(*reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitIncompatibility),
            DiagnosticPayload::InvalidTraitIncompatibility {
                subject_name,
                incompatible_trait_name,
                reason,
            },
        ) => Some(invalid_trait_incompatibility(
            *subject_name,
            *incompatible_trait_name,
            *reason,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget),
            DiagnosticPayload::None,
        ) => Some(invalid_export_target(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicatePublicExport),
            DiagnosticPayload::DuplicatePublicExport { name },
        ) => {
            // The outer adapter copies this existing message-less label once.
            Some(duplicate_public_export(*name, None, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::PrivateTypeInExportedApi),
            DiagnosticPayload::PrivateTypeInExportedApi {
                exported_name,
                private_type,
            },
        ) => Some(private_type_in_exported_api(
            *exported_name,
            *private_type,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateExportBlock),
            DiagnosticPayload::None,
        ) => Some(duplicate_export_block(span)),
        _ => None,
    }
}

diagnostic_family! {
    pilot INVALID_EXPORT_TARGET {
        internal: 3080,
        stable: "MOTH-RULE-0080",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "`export:` contains an invalid module API item",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated invalid_export_target,
        view: InvalidExportTargetView,
        render: render_title_only,
    }
}

diagnostic_family! {
    pilot DUPLICATE_PUBLIC_EXPORT {
        internal: 3081,
        stable: "MOTH-RULE-0081",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate public export",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: DuplicatePublicExportView,
        render: render_duplicate_public_export,
    }
}

/// Stores the duplicate name and its message-less prior-declaration label.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn duplicate_public_export(
    name: StringId,
    previous_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&DUPLICATE_PUBLIC_EXPORT, span, |cursor| {
        cursor.push::<StringFact>(&name)?;
        Ok(())
    });
    match previous_span {
        Some(previous_span) => draft.with_label(Some(previous_span), None),
        None => draft,
    }
}

diagnostic_family! {
    pilot PRIVATE_TYPE_IN_EXPORTED_API {
        internal: 3082,
        stable: "MOTH-RULE-0082",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Private type exposed by exported API",
        facts: [exported_name: StringFact, private_type: TypeFact],
        labels: [],
        extra: None,
        ctor: generated private_type_in_exported_api,
        view: PrivateTypeInExportedApiView,
        render: render_private_type_in_exported_api,
    }
}

diagnostic_family! {
    pilot INVALID_TRAIT_INCOMPATIBILITY {
        internal: 3084,
        stable: "MOTH-RULE-0084",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid trait incompatibility",
        facts: [
            subject_name: StringFact,
            incompatible_trait_name: OptStringFact,
            reason: UnitFact<InvalidTraitIncompatibilityReason>,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_trait_incompatibility,
        view: InvalidTraitIncompatibilityView,
        render: render_invalid_trait_incompatibility,
    }
}

diagnostic_family! {
    pilot DUPLICATE_EXPORT_BLOCK {
        internal: 3085,
        stable: "MOTH-RULE-0085",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate export block",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated duplicate_export_block,
        view: DuplicateExportBlockView,
        render: render_title_only,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_GENERIC_APPLICATION,
    INVALID_GENERIC_PARAMETER,
    INVALID_PARAMETER_CONTRACT,
    INVALID_THIS_USAGE,
    INVALID_RECEIVER_DECLARATION,
    INVALID_GENERIC_INSTANTIATION,
    INVALID_FUNCTION_SIGNATURE,
    UNKNOWN_TRAIT,
    DUPLICATE_TRAIT_REQUIREMENT,
    TRAIT_PRIVATE_SURFACE_LEAK,
    UNSUPPORTED_TRAIT_FEATURE,
    INVALID_TRAIT_CONFORMANCE,
    GENERIC_BOUND_PRIVATE_SURFACE_LEAK,
    TRAIT_NAME_USED_AS_TYPE,
    INVALID_TRAIT_KEYWORD_USAGE,
    EXPORT_OUTSIDE_MODULE_ROOT,
    INVALID_EXPORT_TARGET,
    DUPLICATE_PUBLIC_EXPORT,
    PRIVATE_TYPE_IN_EXPORTED_API,
    INVALID_TRAIT_INCOMPATIBILITY,
    DUPLICATE_EXPORT_BLOCK,
];
