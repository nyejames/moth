//! Malformed-input parser regression tests.
//!
//! WHAT: asserts that common malformed inputs fail with stable typed diagnostic facts.
//! WHY: parser changes should not silently degrade recovery paths or produce vague errors.

use crate::compiler_frontend::compiler_errors::BoundaryFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidMultiBindCode, InvalidMultiBindView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidControlFlowStatementView, InvalidMatchPatternView, InvalidStatementPositionView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    InvalidFunctionSignatureCode, InvalidFunctionSignatureView, InvalidThisUsageCode,
    InvalidThisUsageView, InvalidTraitKeywordUsageView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidControlFlowStatementReason, InvalidMatchPatternReason, InvalidStatementPositionReason,
    InvalidTraitKeywordUsageReason,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, first_ast_error_view, parse_single_file_ast,
    parse_single_file_ast_diagnostic,
};

fn assert_failure(source: &str, code: &str, reason_key: &str) -> BoundaryFailure {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, code, Some(reason_key));
    failure
}

struct InvalidMatchPatternCase {
    name: &'static str,
    source: &'static str,
    expected_reason: InvalidMatchPatternReason,
}

struct MultiBindUnsupportedRhsCase {
    name: &'static str,
    source: &'static str,
}

struct ReservedTraitKeywordCase {
    name: &'static str,
    source: &'static str,
    expected_reason: InvalidTraitKeywordUsageReason,
}

#[test]
fn reports_missing_signature_colon() {
    let failure = assert_failure(
        "f|| -> I32\n;\n",
        "MOTH-RULE-0062",
        "invalid_function_signature.missing_colon_after_returns",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidFunctionSignatureView>()
        .expect("missing signature colon should retain typed facts");
    assert_eq!(
        facts.code,
        InvalidFunctionSignatureCode::MissingColonAfterReturns
    );
}

#[test]
fn reports_stray_comma_in_function_body() {
    let failure = assert_failure(
        "broken||:\n    value = 1\n    ,\n;\n",
        "MOTH-SYNTAX-0030",
        "invalid_statement_position.unexpected_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidStatementPositionView>()
        .expect("stray comma should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidStatementPositionReason::UnexpectedComma
    );
}

#[test]
fn reports_invalid_match_pattern_forms_as_permanent_rule_errors() {
    let cases = [
        InvalidMatchPatternCase {
            name: "scalar wildcard arm",
            source: "value = 1\nif value is:\n    _ => io.line([: [\"one\"]])\n;\n",
            expected_reason: InvalidMatchPatternReason::WildcardNotSupported,
        },
        InvalidMatchPatternCase {
            name: "choice payload wildcard capture",
            source: "Result :: Ok, Err | message String |;\n\nresult = Result::Err(\"bad\")\nif result is:\n    Err(_) => io.line([: [\"err\"]])\n;\n",
            expected_reason: InvalidMatchPatternReason::WildcardNotSupported,
        },
    ];

    for case in cases {
        let failure = assert_failure(
            case.source,
            "MOTH-RULE-0049",
            "invalid_match_pattern.wildcard_not_supported",
        );
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidMatchPatternView>()
            .expect("invalid match pattern should retain typed facts");
        assert_eq!(facts.reason, case.expected_reason, "{}", case.name);
        assert_eq!(facts.variant_name, None, "{}", case.name);
        assert_eq!(facts.scrutinee_name, None, "{}", case.name);
    }
}

#[test]
fn reports_unterminated_match_scope_at_end_of_file() {
    let failure = assert_failure(
        "value = 1\nif value is:\n    1 => io.line([: [\"one\"]])\n",
        "MOTH-RULE-0042",
        "invalid_control_flow_statement.unexpected_end_of_file_in_match",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidControlFlowStatementView>()
        .expect("unterminated match should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch
    );
}

#[test]
fn reports_multi_bind_malformed_comma_sequence() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, , b = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("malformed multi-bind should retain unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn reports_string_literal_as_invalid_multi_bind_target() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, \"b\" = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("a string multi-bind target retains unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn parses_multi_bind_targets_across_comma_continuation_newline() {
    parse_single_file_ast("pair || -> I32, I32:\n    return 1, 2\n;\n\na,\n    b = pair()\n");
}

#[test]
fn reports_multi_bind_trailing_comma_at_the_comma() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na,\n    = pair()\n",
        "MOTH-SYNTAX-0002",
        "invalid_multi_bind.missing_target_after_comma",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("trailing multi-bind comma should retain unexpected-token facts");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::MultiBind {
            code: InvalidMultiBindCode::MissingTargetAfterComma,
            target_name: None,
            ..
        }
    ));
}

#[test]
fn reports_multi_bind_mutable_target_without_explicit_type() {
    let failure = assert_failure(
        "pair || -> I32, I32:\n    return 1, 2\n;\n\na, b ~= pair()\n",
        "MOTH-RULE-0045",
        "invalid_multi_bind.mutable_target_needs_explicit_type",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidMultiBindView>()
        .expect("invalid multi-bind should retain typed facts");
    assert_eq!(
        facts.reason,
        InvalidMultiBindCode::MutableTargetNeedsExplicitType
    );
    assert!(facts.target_name.is_some());
}

#[test]
fn reports_multi_bind_with_unsupported_rhs_forms_rejected() {
    let cases = [
        MultiBindUnsupportedRhsCase {
            name: "variable",
            source: "value ~= 1\na, b = value\n",
        },
        MultiBindUnsupportedRhsCase {
            name: "literal",
            source: "a, b = 1\n",
        },
        MultiBindUnsupportedRhsCase {
            name: "field access",
            source: "Thing = |\n    x I32,\n    y I32,\n|\nthing ~= Thing(1, 2)\na, b = thing.x\n",
        },
    ];

    for case in cases {
        let failure = assert_failure(
            case.source,
            "MOTH-RULE-0045",
            "invalid_multi_bind.unsupported_rhs",
        );
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidMultiBindView>()
            .expect("invalid multi-bind should retain typed facts");
        assert_eq!(
            facts.reason,
            InvalidMultiBindCode::UnsupportedRhs,
            "{}",
            case.name
        );
        assert_eq!(facts.target_name, None, "{}", case.name);
    }
}

#[test]
fn reports_reserved_trait_keywords_outside_trait_syntax() {
    let cases = [
        ReservedTraitKeywordCase {
            name: "must binding",
            source: "must = 1\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "This statement",
            source: "f||:\n    This\n;\n",
            expected_reason: InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must expression",
            source: "value = must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "This expression",
            source: "value = This\n",
            expected_reason: InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must copy target",
            source: "value = copy must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must signature parameter",
            source: "sum|must I32| -> I32:\n    return 1\n;\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must postfix member",
            source: "Point = |\n    value I32 = 1,\n|\n\npoint ~= Point()\nvalue = point.must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
    ];

    for case in cases {
        let reason_key = match &case.expected_reason {
            InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax => {
                "invalid_trait_keyword_usage.must_outside_trait_syntax"
            }
            InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax => {
                "invalid_trait_keyword_usage.this_outside_trait_syntax"
            }
        };
        let failure = assert_failure(case.source, "MOTH-RULE-0076", reason_key);
        let diagnostic = first_ast_error_view(&failure);
        let facts = diagnostic
            .family_view::<InvalidTraitKeywordUsageView>()
            .expect("trait keyword diagnostic should retain typed facts");
        assert_eq!(facts.reason, case.expected_reason, "{}", case.name);
    }
}

#[test]
fn reports_trait_this_keyword_outside_trait_declaration_type_position() {
    let failure = assert_failure(
        "value This = 1\n",
        "MOTH-RULE-0040",
        "invalid_this_usage.outside_trait_declaration",
    );
    let diagnostic = first_ast_error_view(&failure);
    let facts = diagnostic
        .family_view::<InvalidThisUsageView>()
        .expect("invalid this usage should retain typed facts");
    assert_eq!(facts.code, InvalidThisUsageCode::OutsideTraitDeclaration);
}
