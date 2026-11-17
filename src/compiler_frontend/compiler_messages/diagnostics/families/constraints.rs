//! Family row for source-constraint verification diagnostics.
//!
//! WHAT: the stable code, typed facts, labels, view and constructor for a failed `$safe_math`
//!       or `$infallible` boundary.
//! WHY: verification is the one stage that owns the guarantee verdict. Keeping this family beside
//!       its producer lets the reason enum, label roles and view stay in one group.

use super::super::schema::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptStringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    LabelDraftMessage, LabelMessageCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::render_invalid_source_constraint;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticSeverity, InvalidSourceConstraintReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl InvalidSourceConstraintReason {
        FastOperation = 1,
        FastRangeGuard = 2,
        FastWriteBack = 3,
        UnknownFastEvidence = 4,
        UnavailableCalleeFastEvidence = 5,
        FastEvidenceBeyondWitnessBound = 6,
        FastEvidenceRecursiveCycle = 25,
        TrapOverflow = 7,
        TrapDivision = 8,
        TrapDivideByZero = 27,
        TrapExponent = 9,
        TrapFloatNonFinite = 10,
        TrapFloatDivision = 11,
        TrapRangeGuard = 12,
        TrapWriteBack = 13,
        TrapAssertion = 14,
        TrapFloatValidation = 15,
        TrapConversion = 16,
        UnknownTrapEvidence = 18,
        UnavailableCalleeTrapEvidence = 19,
        TrapEvidenceBeyondWitnessBound = 20,
        TrapEvidenceRecursiveCycle = 26,
        EscapingError = 21,
    }
}

diagnostic_family! {
    INVALID_SOURCE_CONSTRAINT {
        internal: 3091,
        stable: "MOTH-RULE-0091",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Source constraint not satisfied",
        facts: [
            reason: UnitFact<InvalidSourceConstraintReason>,
            callee: OptStringFact,
        ],
        labels: [
            LabelMessageCode::CONSTRAINT_DIRECTIVE,
            LabelMessageCode::CONSTRAINT_EVIDENCE,
            LabelMessageCode::CONSTRAINT_CALL_HOP,
        ],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidSourceConstraintView,
        render: render_invalid_source_constraint,
    }
}

/// Builds one failed source-constraint diagnostic.
///
/// WHAT: the reason, optional unavailable callee name, directive span, offending operation span
///       and the bounded call-hop spans in witness order.
/// WHY: verification proves the reason and locations; rendering owns only the prose. Hop labels
///       keep one note per rendered call site without a second data channel.
pub(crate) fn invalid_source_constraint(
    reason: InvalidSourceConstraintReason,
    callee: Option<StringId>,
    directive_span: Option<SourceSpan>,
    evidence_span: Option<SourceSpan>,
    hop_spans: &[Option<SourceSpan>],
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&INVALID_SOURCE_CONSTRAINT, directive_span, |cursor| {
        cursor.push::<UnitFact<InvalidSourceConstraintReason>>(&reason)?;
        cursor.push::<OptStringFact>(&callee)?;
        Ok(())
    })
    .with_primary_label(LabelDraftMessage::ConstraintDirective);

    let draft = hop_spans.iter().fold(draft, |draft, span| {
        draft.with_label(*span, Some(LabelDraftMessage::ConstraintCallHop))
    });

    if evidence_span.is_some() {
        draft.with_label(evidence_span, Some(LabelDraftMessage::ConstraintEvidence))
    } else {
        draft
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] =
    &[INVALID_SOURCE_CONSTRAINT];
