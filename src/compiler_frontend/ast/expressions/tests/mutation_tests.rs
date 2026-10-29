//! Mutation expression parsing and validation regression tests.
//!
//! WHAT: validates mutable assignment, field mutation, collection mutation, and place-expression
//!       requirements.
//! WHY: mutation rules are tightly coupled to borrow checking; parser-level tests ensure the
//!      frontend produces the right AST shapes for later analysis.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValuePayload;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::InvalidAssignmentTargetView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TypeMismatchView, UnsupportedOperatorTypesView,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCompoundAssignmentOperator, DiagnosticOperator, InvalidAssignmentTargetReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span,
    ast_first_error_rendered_message, parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn rejects_assignment_value_type_mismatch_with_specific_details() {
    assert_assignment_type_mismatch("value ~= 1\n~value = true\n");
}

#[test]
fn immutable_existing_writes_precede_missing_prefix_for_ready_and_pending_bindings() {
    for declaration in ["value I32 = 1", "value = 1"] {
        for prefix in ["", "~"] {
            for operator_text in ["=", "+="] {
                let source = format!("-- π\n{declaration}\n{prefix}value {operator_text} 2\n");
                let failure = parse_single_file_ast_diagnostic(&source);
                let diagnostic = assert_ast_diagnostic(
                    &failure,
                    "MOTH-RULE-0044",
                    Some("invalid_assignment_target.immutable_binding"),
                );
                let mut spans = ExtendedSpanBuilder::new();
                let operator = LocalSpan::exact(
                    source.rfind(operator_text).unwrap() as u32,
                    operator_text.len() as u32,
                    &mut spans,
                )
                .unwrap();
                let binding =
                    LocalSpan::exact(source.find("value").unwrap() as u32, 5, &mut spans).unwrap();
                assert_eq!(
                    ast_error_primary_span(diagnostic),
                    Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator)),
                    "{source}",
                );
                assert_eq!(
                    ast_error_label_spans(diagnostic),
                    vec![Some(SourceSpan::new(SourceId::COMPILATION_ROOT, binding))],
                    "{source}",
                );
            }
        }
    }
}

#[test]
fn existing_writes_require_a_prefix_for_direct_and_projected_targets() {
    for source in [
        "value ~= 1\nvalue = 2\n",
        "value ~= 1\nvalue += 2\n",
        "Point = |\n    x I32 = 0,\n|\npoint ~= Point()\npoint.x += 1\n",
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0044",
            Some("invalid_assignment_target.missing_write_prefix"),
        );
    }
}

#[test]
fn missing_write_prefix_reports_authored_target_and_operator() {
    for (operator, expected) in [
        ("=", None),
        ("+=", Some(DiagnosticCompoundAssignmentOperator::Add)),
        ("-=", Some(DiagnosticCompoundAssignmentOperator::Subtract)),
        ("*=", Some(DiagnosticCompoundAssignmentOperator::Multiply)),
        ("/=", Some(DiagnosticCompoundAssignmentOperator::Divide)),
        ("//=", Some(DiagnosticCompoundAssignmentOperator::IntDivide)),
        ("%=", Some(DiagnosticCompoundAssignmentOperator::Modulus)),
        ("^=", Some(DiagnosticCompoundAssignmentOperator::Exponent)),
    ] {
        let source = format!("value ~I32 = 1\nvalue {operator} 2\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0044",
            Some("invalid_assignment_target.missing_write_prefix"),
        );
        assert_eq!(
            diagnostic
                .family_view::<InvalidAssignmentTargetView>()
                .unwrap()
                .reason,
            InvalidAssignmentTargetReason::MissingWritePrefix { operator: expected },
        );
        assert_eq!(
            ast_first_error_rendered_message(&failure),
            format!("Updating `value` requires `~value {operator} ...`."),
        );
        let start = source.rfind("value").unwrap();
        let mut spans = ExtendedSpanBuilder::new();
        let span = LocalSpan::exact(start as u32, 5, &mut spans).unwrap();
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span))
        );
    }
}

#[test]
fn missing_field_write_prefix_names_the_complete_place() {
    for (source, target, root) in [
        (
            "Point = |\n    x I32 = 0,\n|\npoint ~= Point()\npoint.x += 1\n",
            "point.x",
            "point",
        ),
        (
            "Point = |\n    x I32 = 0,\n|\nShape = |\n    point Point,\n|\n\
             café ~= Shape(Point())\ncafé.point.x += 1\n",
            "café.point.x",
            "café",
        ),
    ] {
        let failure = parse_single_file_ast_diagnostic(source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0044",
            Some("invalid_assignment_target.missing_write_prefix"),
        );
        assert_eq!(
            ast_first_error_rendered_message(&failure),
            format!("Updating `{target}` requires `~{target} += ...`."),
        );
        let mut spans = ExtendedSpanBuilder::new();
        let span = LocalSpan::exact(
            source.rfind(target).unwrap() as u32,
            root.len() as u32,
            &mut spans,
        )
        .unwrap();
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, span)),
        );
    }
}

#[test]
fn prefixed_unknown_and_nonplace_targets_are_not_declarations() {
    let failure = parse_single_file_ast_diagnostic("~unknown = 1\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.mutable_marker_requires_existing_place"),
    );
    let mut spans = ExtendedSpanBuilder::new();
    let marker = LocalSpan::exact(0, 1, &mut spans).unwrap();
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.mutable_marker_requires_existing_place"),
    );
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, marker)),
    );

    let failure = parse_single_file_ast_diagnostic("value ~= 1\n~(value) = 2\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.mutable_marker_requires_existing_place"),
    );
}

#[test]
fn prefixed_pending_binding_writes_use_the_mutation_owner() {
    let (ast, paths, strings) = parse_single_file_ast("value ~= 1\n~value += 2\n~value = 3\n");
    let body = start_function_body(&ast, &paths, &strings);
    assert!(matches!(&body[1].kind, NodeKind::Assignment { .. }));
    assert!(matches!(&body[2].kind, NodeKind::Assignment { .. }));
}

#[test]
fn field_write_diagnostics_prioritise_immutable_ready_and_pending_roots() {
    for (declaration, prefix) in [
        ("Point = |\n    x I32 = 0,\n|\npoint = Point()\n", ""),
        ("Point = |\n    x I32 = 0,\n|\npoint = Point()\n", "~"),
        ("Point type T = |x T|\npoint = Point(1)\n", ""),
        ("Point type T = |x T|\npoint = Point(1)\n", "~"),
    ] {
        for operator_text in ["=", "+="] {
            let source = format!("{declaration}{prefix}point.x {operator_text} 2\n");
            let failure = parse_single_file_ast_diagnostic(&source);
            let diagnostic = assert_ast_diagnostic(
                &failure,
                "MOTH-RULE-0044",
                Some("invalid_assignment_target.immutable_field_root"),
            );
            let mut spans = ExtendedSpanBuilder::new();
            let operator = LocalSpan::exact(
                source.rfind(operator_text).unwrap() as u32,
                operator_text.len() as u32,
                &mut spans,
            )
            .unwrap();
            let binding =
                LocalSpan::exact(source.find("point =").unwrap() as u32, 5, &mut spans).unwrap();
            assert_eq!(
                ast_error_primary_span(diagnostic),
                Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator)),
            );
            assert_eq!(
                ast_error_label_spans(diagnostic),
                vec![Some(SourceSpan::new(SourceId::COMPILATION_ROOT, binding))],
            );
        }
    }
    let failure =
        parse_single_file_ast_diagnostic("Point type T = |x T|\npoint ~= Point(1)\npoint.x = 2\n");
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.missing_write_prefix"),
    );
}

#[test]
fn receiver_field_write_prioritises_immutability_and_keeps_mutable_prefix_requirement() {
    for prefix in ["", "~"] {
        let source = format!(
            "Point = |\n    x I32 = 0,\n|\nreset |this Point|:\n    {prefix}this.x += 1\n;\n"
        );
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0044",
            Some("invalid_assignment_target.immutable_field_root"),
        );
        let mut spans = ExtendedSpanBuilder::new();
        let operator = LocalSpan::exact(source.find("+=").unwrap() as u32, 2, &mut spans).unwrap();
        let binding = LocalSpan::exact(source.find("this").unwrap() as u32, 4, &mut spans).unwrap();
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator)),
        );
        assert_eq!(
            ast_error_label_spans(diagnostic),
            vec![Some(SourceSpan::new(SourceId::COMPILATION_ROOT, binding))],
        );
    }
    let failure = parse_single_file_ast_diagnostic(
        "Point = |\n    x I32 = 0,\n|\nreset |this ~Point|:\n    this.x = 1\n;\n",
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.missing_write_prefix"),
    );
}

#[test]
fn missing_prefix_preserves_the_compound_operator_fact() {
    let failure = parse_single_file_ast_diagnostic("value ~= 1\nvalue += 2\n");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.missing_write_prefix"),
    );
    let facts = diagnostic
        .family_view::<InvalidAssignmentTargetView>()
        .unwrap();
    assert_eq!(
        facts.reason,
        InvalidAssignmentTargetReason::MissingWritePrefix {
            operator: Some(DiagnosticCompoundAssignmentOperator::Add),
        },
    );
}

#[test]
fn allows_whole_literal_into_decimal_binding_at_f64_context() {
    let (ast, path_fork, string_table) = parse_single_file_ast("total ~= 1.5\n~total = 2\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        ast.expressions
            .view(*value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn allows_i32_divide_assign_with_checked_store_conversion() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\n~value /= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };
    let value = ast.expressions.view(*value).unwrap();
    let ExpressionOperationView::Cast { source, facts } = value.operation() else {
        panic!("expected the F64 division result to convert at the I32 store");
    };
    assert_eq!(
        source.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(matches!(facts.handling, CastHandling::StoreConversion));
    assert_eq!(
        value.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn allows_same_type_u8_add_assign_without_store_conversion() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~U8 = 1\n~value += 1\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected compound assignment");
    };
    // Equal `U8` operands keep `U8`, so the same-type update performs one checked
    // operation and inserts no artificial promotion/narrowing conversion.
    let value = ast.expressions.view(*value).unwrap();
    assert!(
        !matches!(value.operation(), ExpressionOperationView::Cast { .. }),
        "a same-type compound update must not carry a store conversion"
    );
    assert_eq!(
        value.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
}

#[test]
fn allows_mixed_domain_u8_add_assign_with_checked_store_conversion() {
    let (ast, path_fork, string_table) =
        parse_single_file_ast("wide U64 = 1\nvalue ~U8 = 1\n~value += wide\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[2].kind else {
        panic!("expected compound assignment");
    };
    let value = ast.expressions.view(*value).unwrap();
    let ExpressionOperationView::Cast { source, facts } = value.operation() else {
        panic!("expected the mixed-domain U64 result to convert at the U8 store");
    };
    assert_eq!(
        source.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::U64)
    );
    assert!(matches!(facts.handling, CastHandling::StoreConversion));
    assert_eq!(
        value.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::U8)
    );
}

#[test]
fn allows_i32_integer_divide_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\n~value //= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        ast.expressions
            .view(*value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn allows_f64_divide_assign_whole_literal_rhs() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~F64 = 10\n~value /= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        ast.expressions
            .view(*value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
}

#[test]
fn rejects_f64_integer_divide_assign_rhs() {
    let failure = parse_single_file_ast_diagnostic("value ~F64 = 10\n~value //= 4\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0003", None);
    let facts = diagnostic
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("unsupported-operator diagnostic should retain typed facts");
    assert_eq!(facts.operator, DiagnosticOperator::IntDivide);
}

#[test]
fn allows_i32_modulus_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\n~value %= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        ast.expressions
            .view(*value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn allows_i32_exponent_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 2\n~value ^= 3\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    let value = ast.expressions.view(*value).unwrap();
    let ExpressionOperationView::NumericBinary { facts, .. } = value.operation() else {
        panic!("integer exponent assignment should be a numeric binary operation");
    };
    assert_eq!(facts.operator, Operator::Exponent);
    assert_eq!(
        value.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn allows_number_exponent_assign_with_i32_rhs() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Dec2 = 2.5\n~value ^= 2\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { target, value } = &body[1].kind else {
        panic!("expected Dec exponent compound assignment");
    };

    let value = ast.expressions.view(*value).unwrap();
    assert_eq!(
        value.value_type_id().unwrap(),
        ast.expressions.place(*target).unwrap().metadata().type_id
    );
    let ExpressionOperationView::NumericBinary { left, right, facts } = value.operation() else {
        panic!("mutable Dec power stays a resolved numeric operation");
    };
    assert_eq!(facts.operator, Operator::Exponent);
    assert_eq!(
        left.value_type_id().unwrap(),
        value.value_type_id().unwrap()
    );
    assert_eq!(
        right.value_type_id().unwrap(),
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    let ExpressionOperationView::Constant(constant) = right.operation() else {
        panic!("the exponent should be a folded fixed-width constant");
    };
    let ConstValuePayload::FixedScalar(exponent) = ast.const_values.payload(constant).unwrap()
    else {
        panic!("the exponent should be a folded fixed-width constant");
    };
    assert_eq!(exponent.scalar(), FixedScalar::I32);
    assert_eq!(facts.result_type, value.value_type_id().unwrap());
}

#[test]
fn grouped_rhs_ends_at_newline_for_plain_and_compound_assignment() {
    // A trailing parenthesised group must not swallow the statement newline:
    // the following statement parses on its own line.
    let (ast, path_fork, string_table) =
        parse_single_file_ast("value ~I32 = 2\n~value = (1 + 2)\nmarker I32 = value\n");
    let body = start_function_body(&ast, &path_fork, &string_table);
    assert!(
        matches!(&body[2].kind, NodeKind::VariableDeclaration(_)),
        "a trailing grouped RHS must leave the next statement intact"
    );

    let (ast, path_fork, string_table) =
        parse_single_file_ast("value ~Dec4 = 2\n~value ^= (1 + 2)\nmarker I32 = 1\n");
    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::Assignment { target, value } = &body[1].kind else {
        panic!("expected grouped Dec exponent compound assignment");
    };
    assert_eq!(
        ast.expressions
            .view(*value)
            .unwrap()
            .value_type_id()
            .unwrap(),
        ast.expressions.place(*target).unwrap().metadata().type_id
    );
    assert!(
        matches!(&body[2].kind, NodeKind::VariableDeclaration(_)),
        "a trailing grouped compound RHS must leave the next statement intact"
    );
}

#[test]
fn rejects_number_exponent_assign_with_non_int_rhs() {
    for (right_type, right_value) in [("Dec2", "2.5"), ("I64", "2"), ("F64", "2.5"), ("Byte", "2")]
    {
        let source =
            format!("value ~Dec2 = 2.5\nright {right_type} = {right_value}\n~value ^= right\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0003", None);
        let facts = diagnostic
            .family_view::<UnsupportedOperatorTypesView>()
            .expect("unsupported-operator diagnostic should retain typed facts");
        assert_eq!(facts.operator, DiagnosticOperator::Exponent);
    }
}

fn assert_assignment_type_mismatch(source: &str) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("assignment mismatch should retain typed facts");
    assert_eq!(facts.context, TypeMismatchContext::Assignment);
}
