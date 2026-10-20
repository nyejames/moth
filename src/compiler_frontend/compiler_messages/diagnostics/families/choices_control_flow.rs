//! Family rows for choice and control-flow diagnostics.
//!
//! WHAT: the unreachable-match-arm and non-exhaustive-match pilots, in full, with the code for each match reason.
//! The other 9 rows are identity rows for choice and control-flow families.
//! WHY: both pilots describe how a match is written, so they share one group table with the identity rows.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{UnitFact, unit_code};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_non_exhaustive_match, render_unreachable_match_arm,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    NonExhaustiveMatchReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl NonExhaustiveMatchReason {
        MissingElseArm = 1,
        MissingVariants = 2,
        GuardedArmsRequireElse = 3,
        MissingOptionPatterns = 4,
    }
}

diagnostic_family! {
    pilot UNREACHABLE_MATCH_ARM {
        internal: 3022,
        stable: "MOTH-RULE-0022",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Warning,
        title: "Unreachable match arm",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated unreachable_match_arm,
        view: UnreachableMatchArmView,
        render: render_unreachable_match_arm,
    }
}

diagnostic_family! {
    pilot NON_EXHAUSTIVE_MATCH {
        internal: 3050,
        stable: "MOTH-RULE-0050",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Non-exhaustive match",
        facts: [reason: UnitFact<NonExhaustiveMatchReason>],
        labels: [],
        extra: StringIds,
        ctor: manual,
        view: NonExhaustiveMatchView,
        render: render_non_exhaustive_match,
    }
}

/// The non-exhaustive-match draft. The missing variant names are stored as the extra string IDs.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn non_exhaustive_match(
    reason: NonExhaustiveMatchReason,
    missing_variants: Vec<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&NON_EXHAUSTIVE_MATCH, span, |cursor| {
        cursor.push::<UnitFact<NonExhaustiveMatchReason>>(&reason)
    })
    .with_string_ids(missing_variants)
}

/// Maps the legacy match diagnostics onto their compact constructors. Declines other payloads.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnreachableMatchArm),
            DiagnosticPayload::UnreachableMatchArm,
        ) => Some(unreachable_match_arm(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::NonExhaustiveMatch),
            DiagnosticPayload::NonExhaustiveMatch {
                reason,
                missing_variants,
            },
        ) => Some(non_exhaustive_match(
            *reason,
            missing_variants.clone(),
            span,
        )),
        _ => None,
    }
}

diagnostic_family! {
    identity INVALID_STANDALONE_STATEMENT {
        internal: 1025,
        stable: "MOTH-SYNTAX-0025",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_MATCH_ARM {
        internal: 1028,
        stable: "MOTH-SYNTAX-0028",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_LOOP_HEADER {
        internal: 1029,
        stable: "MOTH-SYNTAX-0029",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_STATEMENT_POSITION {
        internal: 1030,
        stable: "MOTH-SYNTAX-0030",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_CHOICE_VARIANT {
        internal: 3029,
        stable: "MOTH-RULE-0029",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_CONTROL_FLOW_STATEMENT {
        internal: 3042,
        stable: "MOTH-RULE-0042",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_MATCH_PATTERN {
        internal: 3049,
        stable: "MOTH-RULE-0049",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_FALLIBLE_HANDLING {
        internal: 3051,
        stable: "MOTH-RULE-0051",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity COMPILE_TIME_EVALUATION_ERROR {
        internal: 3053,
        stable: "MOTH-RULE-0053",
        category: DiagnosticCategory::Rule,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    UNREACHABLE_MATCH_ARM,
    NON_EXHAUSTIVE_MATCH,
    INVALID_STANDALONE_STATEMENT,
    INVALID_MATCH_ARM,
    INVALID_LOOP_HEADER,
    INVALID_STATEMENT_POSITION,
    INVALID_CHOICE_VARIANT,
    INVALID_CONTROL_FLOW_STATEMENT,
    INVALID_MATCH_PATTERN,
    INVALID_FALLIBLE_HANDLING,
    COMPILE_TIME_EVALUATION_ERROR,
];
