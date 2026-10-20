//! Pilot parity: each pilot row agrees with the legacy kind and reason keys it replaces.
//!
//! WHAT: compares every pilot row with the `DiagnosticKind` that still serves it, and the
//! multi-bind and non-exhaustive reason keys with their literal legacy strings.
//! WHY: producers switch to these rows in P4.6, so both paths must agree before then.

use crate::compiler_frontend::compiler_messages::diagnostic_payload::reason_keys::StableReasonKey;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    INVALID_MULTI_BIND, InvalidMultiBindCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    NON_EXHAUSTIVE_MATCH, UNREACHABLE_MATCH_ARM,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    MISSING_IMPORT_TARGET, UNUSED_IMPORT,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    DUPLICATE_DECLARATION, UNKNOWN_TYPE_NAME, UNKNOWN_VALUE_NAME,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    COMPILER_CAPACITY_EXCEEDED, EXPECTED_TOKEN,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TYPE_MISMATCH, USE_OF_UNINITIALIZED_LOCAL,
};
use crate::compiler_frontend::compiler_messages::diagnostics::registry::family_for_code;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::{
    BorrowDiagnosticKind, DiagnosticCategory, DiagnosticDescriptor, DiagnosticKind,
    DiagnosticSeverity, ImportDiagnosticKind, InvalidMultiBindReason, NonExhaustiveMatchReason,
    RuleDiagnosticKind, SyntaxDiagnosticKind, TypeDiagnosticKind,
};

/// One pilot row paired with the legacy kind and the descriptor fields it must reproduce.
struct PilotCase {
    row: &'static FamilyRow,
    kind: DiagnosticKind,
    category: DiagnosticCategory,
    stable_code: &'static str,
    title: &'static str,
    severity: DiagnosticSeverity,
}

const PILOTS: &[PilotCase] = &[
    PilotCase {
        row: &EXPECTED_TOKEN,
        kind: DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
        category: DiagnosticCategory::Syntax,
        stable_code: "MOTH-SYNTAX-0001",
        title: "Expected token",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &COMPILER_CAPACITY_EXCEEDED,
        kind: DiagnosticKind::Syntax(SyntaxDiagnosticKind::CompilerCapacityExceeded),
        category: DiagnosticCategory::Syntax,
        stable_code: "MOTH-SYNTAX-0037",
        title: "Compiler capacity exceeded",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &TYPE_MISMATCH,
        kind: DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
        category: DiagnosticCategory::Type,
        stable_code: "MOTH-TYPE-0001",
        title: "Type mismatch",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &DUPLICATE_DECLARATION,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0002",
        title: "Duplicate declaration",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &UNREACHABLE_MATCH_ARM,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::UnreachableMatchArm),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0022",
        title: "Unreachable match arm",
        severity: DiagnosticSeverity::Warning,
    },
    PilotCase {
        row: &UNKNOWN_VALUE_NAME,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0034",
        title: "Unknown value name",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &UNKNOWN_TYPE_NAME,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTypeName),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0035",
        title: "Unknown type name",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &INVALID_MULTI_BIND,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0045",
        title: "Invalid multi-bind",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &NON_EXHAUSTIVE_MATCH,
        kind: DiagnosticKind::Rule(RuleDiagnosticKind::NonExhaustiveMatch),
        category: DiagnosticCategory::Rule,
        stable_code: "MOTH-RULE-0050",
        title: "Non-exhaustive match",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &UNUSED_IMPORT,
        kind: DiagnosticKind::Import(ImportDiagnosticKind::UnusedImport),
        category: DiagnosticCategory::Import,
        stable_code: "MOTH-IMPORT-0002",
        title: "Unused dependency binding",
        severity: DiagnosticSeverity::Warning,
    },
    PilotCase {
        row: &MISSING_IMPORT_TARGET,
        kind: DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
        category: DiagnosticCategory::Import,
        stable_code: "MOTH-IMPORT-0005",
        title: "Missing dependency target",
        severity: DiagnosticSeverity::Error,
    },
    PilotCase {
        row: &USE_OF_UNINITIALIZED_LOCAL,
        kind: DiagnosticKind::Borrow(BorrowDiagnosticKind::UseOfUninitializedLocal),
        category: DiagnosticCategory::Borrow,
        stable_code: "MOTH-BORROW-0009",
        title: "Use of uninitialized local",
        severity: DiagnosticSeverity::Error,
    },
];

#[test]
fn pilot_rows_agree_with_their_legacy_kinds() {
    assert_eq!(PILOTS.len(), 12, "one case per pilot family");
    for case in PILOTS {
        let descriptor = DiagnosticDescriptor::new(case.stable_code, case.title, case.severity);
        assert_eq!(
            case.row.pilot_descriptor(),
            descriptor,
            "{} row descriptor",
            case.stable_code
        );
        assert_eq!(
            case.kind.descriptor(),
            descriptor,
            "{} kind descriptor",
            case.stable_code
        );
        assert_eq!(
            case.kind.category(),
            case.category,
            "{} category",
            case.stable_code
        );
        assert_eq!(
            case.kind.code(),
            case.stable_code,
            "{} kind code",
            case.stable_code
        );
        assert_eq!(
            case.kind.default_severity(),
            case.severity,
            "{} severity",
            case.stable_code
        );
        assert_eq!(
            case.row.stable_code, case.stable_code,
            "{} row code",
            case.stable_code
        );
        assert_eq!(
            case.row.category, case.category,
            "{} row category",
            case.stable_code
        );
        assert_eq!(
            family_for_code(case.row.code).map(|row| row.stable_code),
            Some(case.stable_code),
            "{} registry lookup",
            case.stable_code
        );
    }
}

#[test]
fn multi_bind_reason_keys_match_the_legacy_strings() {
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
    for (reason, legacy_key) in &cases {
        assert_eq!(reason.stable_reason_key(), *legacy_key);
        assert_eq!(
            InvalidMultiBindCode::of(reason).stable_reason_key(),
            *legacy_key
        );
    }
}

#[test]
fn non_exhaustive_reason_keys_match_the_legacy_strings() {
    assert_eq!(
        NonExhaustiveMatchReason::MissingElseArm.stable_reason_key(),
        "non_exhaustive_match.missing_else_arm"
    );
    assert_eq!(
        NonExhaustiveMatchReason::MissingVariants.stable_reason_key(),
        "non_exhaustive_match.missing_variants"
    );
    assert_eq!(
        NonExhaustiveMatchReason::GuardedArmsRequireElse.stable_reason_key(),
        "non_exhaustive_match.guarded_arms_require_else"
    );
    assert_eq!(
        NonExhaustiveMatchReason::MissingOptionPatterns.stable_reason_key(),
        "non_exhaustive_match.missing_option_patterns"
    );
}
