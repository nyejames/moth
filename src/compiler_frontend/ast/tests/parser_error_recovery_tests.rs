//! Malformed-input parser regression tests.
//!
//! WHAT: asserts that common malformed inputs fail with stable typed diagnostic facts.
//! WHY: parser changes should not silently degrade recovery paths or produce vague errors.

use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, InvalidControlFlowStatementReason,
    InvalidFunctionSignatureReason, InvalidMatchPatternReason, InvalidMultiBindReason,
    InvalidStatementPositionReason, InvalidThisUsageReason, InvalidTraitKeywordUsageReason,
    InvalidTypeAnnotationReason, RuleDiagnosticKind, SyntaxDiagnosticKind, TypeAnnotationContext,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

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
    let diagnostic = parse_single_file_ast_diagnostic("f|| -> Int\n;\n");

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFunctionSignature)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidFunctionSignature {
            reason: InvalidFunctionSignatureReason::MissingColonAfterReturns
        }
    );
}

#[test]
fn reports_stray_comma_in_function_body() {
    let diagnostic = parse_single_file_ast_diagnostic("broken||:\n    value = 1\n    ,\n;\n");

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStatementPosition)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStatementPosition {
            reason: InvalidStatementPositionReason::UnexpectedComma
        }
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
        let diagnostic = parse_single_file_ast_diagnostic(case.source);

        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMatchPattern),
            "{}",
            case.name
        );
        assert_eq!(
            diagnostic.payload,
            DiagnosticPayload::InvalidMatchPattern {
                reason: case.expected_reason,
                variant_name: None,
                scrutinee_name: None,
            },
            "{}",
            case.name
        );
        assert!(
            diagnostic.primary_location.start_pos.char_column > 0,
            "{}",
            case.name
        );
    }
}

#[test]
fn rejects_bare_labeled_blocks_with_declaration_guidance() {
    let diagnostic = parse_single_file_ast_diagnostic("label:\n    io.line([: [\"x\"]])\n;\n");

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTypeAnnotation)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTypeAnnotation {
            context: TypeAnnotationContext::DeclarationTarget,
            reason: InvalidTypeAnnotationReason::UnexpectedColon,
        }
    );
    assert!(diagnostic.primary_location.start_pos.char_column > 0);
}

#[test]
fn reports_unterminated_match_scope_at_end_of_file() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "value = 1\nif value is:\n    1 => io.line([: [\"one\"]])\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidControlFlowStatement)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidControlFlowStatement {
            reason: InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch
        }
    );
}

#[test]
fn reports_multi_bind_malformed_comma_sequence() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "pair || -> Int, Int:\n    return 1, 2\n;\n\na, , b = pair()\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMultiBind {
            reason: InvalidMultiBindReason::MissingTargetAfterComma,
            target_name: None,
        }
    );
}

#[test]
fn parses_multi_bind_targets_across_comma_continuation_newline() {
    parse_single_file_ast("pair || -> Int, Int:\n    return 1, 2\n;\n\na,\n    b = pair()\n");
}

#[test]
fn reports_multi_bind_trailing_comma_at_the_comma() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "pair || -> Int, Int:\n    return 1, 2\n;\n\na,\n    = pair()\n",
    );

    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidMultiBind {
            reason: InvalidMultiBindReason::MissingTargetAfterComma,
            target_name: None,
        }
    );
    assert_eq!(diagnostic.primary_location.start_pos.line_number, 4);
    assert_eq!(diagnostic.primary_location.start_pos.char_column, 2);
}

#[test]
fn reports_multi_bind_mutable_target_without_explicit_type() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "pair || -> Int, Int:\n    return 1, 2\n;\n\na, b ~= pair()\n",
    );

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind)
    );
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidMultiBind {
                reason: InvalidMultiBindReason::MutableTargetNeedsExplicitType,
                target_name: Some(_),
            }
        ),
        "{:?}",
        diagnostic.payload
    );
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
            source: "Thing = |\n    x Int,\n    y Int,\n|\nthing ~= Thing(1, 2)\na, b = thing.x\n",
        },
    ];

    for case in cases {
        let diagnostic = parse_single_file_ast_diagnostic(case.source);

        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
            "{}",
            case.name
        );
        assert_eq!(
            diagnostic.payload,
            DiagnosticPayload::InvalidMultiBind {
                reason: InvalidMultiBindReason::UnsupportedRhs,
                target_name: None,
            },
            "{}",
            case.name
        );
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
            source: "sum|must Int| -> Int:\n    return 1\n;\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
        ReservedTraitKeywordCase {
            name: "must postfix member",
            source: "Point = |\n    value Int = 1,\n|\n\npoint ~= Point()\nvalue = point.must\n",
            expected_reason: InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
        },
    ];

    for case in cases {
        let diagnostic = parse_single_file_ast_diagnostic(case.source);

        assert_eq!(
            diagnostic.payload,
            DiagnosticPayload::InvalidTraitKeywordUsage {
                reason: case.expected_reason,
            },
            "{}",
            case.name
        );
        assert!(
            diagnostic.primary_location.start_pos.char_column > 0,
            "{}",
            case.name
        );
    }
}

#[test]
fn reports_trait_this_keyword_outside_trait_declaration_type_position() {
    let diagnostic = parse_single_file_ast_diagnostic("value This = 1\n");

    assert_eq!(
        diagnostic.payload,
        DiagnosticPayload::InvalidThisUsage {
            reason: InvalidThisUsageReason::OutsideTraitDeclaration
        }
    );
}
