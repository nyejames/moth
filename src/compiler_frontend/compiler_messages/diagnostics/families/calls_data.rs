//! Family rows for call, data and multi-bind diagnostics.
//!
//! WHAT: the invalid-multi-bind pilot, in full, with the one-word code for each multi-bind reason.
//! The other 12 rows are identity rows for call and data families.
//! WHY: the reason is stored as one word and its stable key follows from that code, so the arity counts and the
//! target name travel in their own fact slots.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    CountFact, OptStringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::render_invalid_multi_bind;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    InvalidMultiBindReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    pub(crate) enum InvalidMultiBindCode {
        ThisTargetReserved = 1,
        ExpectedTargetName = 2,
        MissingTargetAfterComma = 3,
        MissingAssignmentOperator = 4,
        InvalidTokenAfterTarget = 5,
        MissingRightHandExpression = 6,
        MultipleRightHandExpressions = 7,
        MutableTargetNeedsExplicitType = 8,
        CompileTimeMarkerOnTarget = 9,
        DuplicateTarget = 10,
        UnsupportedRhs = 11,
        ExistingTargetMutableMarker = 12,
        ExistingTargetImmutable = 13,
        ArityMismatch = 14,
        RhsNotMultiValue = 15,
    }
}

impl InvalidMultiBindCode {
    /// The code for a legacy reason. The arity counts are stored separately, so `ArityMismatch` carries no data here.
    pub(crate) fn of(reason: &InvalidMultiBindReason) -> Self {
        match reason {
            InvalidMultiBindReason::ThisTargetReserved => Self::ThisTargetReserved,
            InvalidMultiBindReason::ExpectedTargetName => Self::ExpectedTargetName,
            InvalidMultiBindReason::MissingTargetAfterComma => Self::MissingTargetAfterComma,
            InvalidMultiBindReason::MissingAssignmentOperator => Self::MissingAssignmentOperator,
            InvalidMultiBindReason::InvalidTokenAfterTarget => Self::InvalidTokenAfterTarget,
            InvalidMultiBindReason::MissingRightHandExpression => Self::MissingRightHandExpression,
            InvalidMultiBindReason::MultipleRightHandExpressions => {
                Self::MultipleRightHandExpressions
            }
            InvalidMultiBindReason::MutableTargetNeedsExplicitType => {
                Self::MutableTargetNeedsExplicitType
            }
            InvalidMultiBindReason::CompileTimeMarkerOnTarget => Self::CompileTimeMarkerOnTarget,
            InvalidMultiBindReason::DuplicateTarget => Self::DuplicateTarget,
            InvalidMultiBindReason::UnsupportedRhs => Self::UnsupportedRhs,
            InvalidMultiBindReason::ExistingTargetMutableMarker => {
                Self::ExistingTargetMutableMarker
            }
            InvalidMultiBindReason::ExistingTargetImmutable => Self::ExistingTargetImmutable,
            InvalidMultiBindReason::ArityMismatch { .. } => Self::ArityMismatch,
            InvalidMultiBindReason::RhsNotMultiValue => Self::RhsNotMultiValue,
        }
    }

    /// The legacy reason for this code. The arity counts fill `ArityMismatch`, the only reason that reads them.
    pub(crate) fn to_reason(self, expected: usize, found: usize) -> InvalidMultiBindReason {
        match self {
            Self::ThisTargetReserved => InvalidMultiBindReason::ThisTargetReserved,
            Self::ExpectedTargetName => InvalidMultiBindReason::ExpectedTargetName,
            Self::MissingTargetAfterComma => InvalidMultiBindReason::MissingTargetAfterComma,
            Self::MissingAssignmentOperator => InvalidMultiBindReason::MissingAssignmentOperator,
            Self::InvalidTokenAfterTarget => InvalidMultiBindReason::InvalidTokenAfterTarget,
            Self::MissingRightHandExpression => InvalidMultiBindReason::MissingRightHandExpression,
            Self::MultipleRightHandExpressions => {
                InvalidMultiBindReason::MultipleRightHandExpressions
            }
            Self::MutableTargetNeedsExplicitType => {
                InvalidMultiBindReason::MutableTargetNeedsExplicitType
            }
            Self::CompileTimeMarkerOnTarget => InvalidMultiBindReason::CompileTimeMarkerOnTarget,
            Self::DuplicateTarget => InvalidMultiBindReason::DuplicateTarget,
            Self::UnsupportedRhs => InvalidMultiBindReason::UnsupportedRhs,
            Self::ExistingTargetMutableMarker => {
                InvalidMultiBindReason::ExistingTargetMutableMarker
            }
            Self::ExistingTargetImmutable => InvalidMultiBindReason::ExistingTargetImmutable,
            Self::ArityMismatch => InvalidMultiBindReason::ArityMismatch { expected, found },
            Self::RhsNotMultiValue => InvalidMultiBindReason::RhsNotMultiValue,
        }
    }
}

diagnostic_family! {
    pilot INVALID_MULTI_BIND {
        internal: 3045,
        stable: "MOTH-RULE-0045",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid multi-bind",
        facts: [
            reason: UnitFact<InvalidMultiBindCode>,
            target_name: OptStringFact,
            expected: CountFact,
            found: CountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidMultiBindView,
        render: render_invalid_multi_bind,
    }
}

/// The invalid-multi-bind draft for a rule-kind diagnostic. The arity counts are zero unless the reason is
/// `ArityMismatch`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_multi_bind(
    reason: InvalidMultiBindReason,
    target_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (expected, found) = match reason {
        InvalidMultiBindReason::ArityMismatch { expected, found } => (expected, found),
        _ => (0, 0),
    };
    let code = InvalidMultiBindCode::of(&reason);
    DiagnosticDraft::with_facts(&INVALID_MULTI_BIND, span, |cursor| {
        cursor.push::<UnitFact<InvalidMultiBindCode>>(&code)?;
        cursor.push::<OptStringFact>(&target_name)?;
        cursor.push::<CountFact>(&expected)?;
        cursor.push::<CountFact>(&found)?;
        Ok(())
    })
}

/// Maps the legacy invalid-multi-bind diagnostic onto its compact constructor. Declines other payloads.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name,
            },
        ) => Some(invalid_multi_bind(
            *reason,
            *target_name,
            diagnostic.primary_span,
        )),
        _ => None,
    }
}

diagnostic_family! {
    identity INVALID_COLLECTION_TYPE {
        internal: 1016,
        stable: "MOTH-SYNTAX-0016",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_MAP_LITERAL {
        internal: 1033,
        stable: "MOTH-SYNTAX-0033",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_MAP_TYPE {
        internal: 1039,
        stable: "MOTH-SYNTAX-0016-MAP",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_BUILTIN_CALL {
        internal: 3046,
        stable: "MOTH-RULE-0046",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_RECEIVER_CALL {
        internal: 3047,
        stable: "MOTH-RULE-0047",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_FIELD_ACCESS {
        internal: 3048,
        stable: "MOTH-RULE-0048",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_CALL_SHAPE {
        internal: 3054,
        stable: "MOTH-RULE-0054",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_RETURN_SHAPE {
        internal: 3055,
        stable: "MOTH-RULE-0055",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_COPY_TARGET {
        internal: 3056,
        stable: "MOTH-RULE-0056",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_RANGE_OPERAND {
        internal: 3059,
        stable: "MOTH-RULE-0059",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_CAST {
        internal: 3083,
        stable: "MOTH-RULE-0083",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_VALUE_RECEIVER {
        internal: 3087,
        stable: "MOTH-RULE-0087",
        category: DiagnosticCategory::Rule,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_MULTI_BIND,
    INVALID_COLLECTION_TYPE,
    INVALID_MAP_LITERAL,
    INVALID_MAP_TYPE,
    INVALID_BUILTIN_CALL,
    INVALID_RECEIVER_CALL,
    INVALID_FIELD_ACCESS,
    INVALID_CALL_SHAPE,
    INVALID_RETURN_SHAPE,
    INVALID_COPY_TARGET,
    INVALID_RANGE_OPERAND,
    INVALID_CAST,
    INVALID_VALUE_RECEIVER,
];
