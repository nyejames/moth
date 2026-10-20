//! Family rows for name and declaration diagnostics.
//!
//! WHAT: the duplicate-declaration, unknown-value-name and unknown-type-name pilots, in full.
//! The other 16 rows are identity rows for families in this group.
//! WHY: the group keeps the name and declaration families together, each with one code and category.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::StringFact;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticLabelMessage, DiagnosticSeverity,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

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
    }
}

/// The duplicate-declaration draft. The primary site is the duplicate. The earlier declaration becomes a label
/// only when its span is known.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "P4.6 producer switch: duplicate declarations")
)]
pub(crate) fn duplicate_declaration(
    name: StringId,
    first_span: Option<SourceSpan>,
    duplicate_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&DUPLICATE_DECLARATION, duplicate_span, |cursor| {
        cursor.push::<StringFact>(&name)
    });
    match first_span {
        Some(_) => draft.with_label(first_span, DiagnosticLabelMessage::PreviousDeclaration),
        None => draft,
    }
}

diagnostic_family! {
    identity INVALID_TYPE_ANNOTATION {
        internal: 1014,
        stable: "MOTH-SYNTAX-0014",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNKNOWN_NAME {
        internal: 3001,
        stable: "MOTH-RULE-0001",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity IDENTIFIER_NAMING_CONVENTION {
        internal: 3021,
        stable: "MOTH-RULE-0021",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_TOP_LEVEL_RUNTIME_STATEMENT {
        internal: 3023,
        stable: "MOTH-RULE-0023",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity RESERVED_BUILTIN_NAME {
        internal: 3027,
        stable: "MOTH-RULE-0027",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_SIGNATURE_MEMBER {
        internal: 3028,
        stable: "MOTH-RULE-0028",
        category: DiagnosticCategory::Rule,
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
    identity VALUE_USED_AS_TYPE {
        internal: 3036,
        stable: "MOTH-RULE-0036",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity TYPE_USED_AS_VALUE {
        internal: 3037,
        stable: "MOTH-RULE-0037",
        category: DiagnosticCategory::Rule,
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

pub(crate) const FAMILIES: &[FamilyRow] = &[
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
