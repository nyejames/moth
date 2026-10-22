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
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    TypeMismatchView, UnsupportedOperatorTypesView,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticOperator, TypeMismatchContext};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span, parse_single_file_ast,
    parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn rejects_assignment_value_type_mismatch_with_specific_details() {
    assert_assignment_type_mismatch("value ~= 1\nvalue = true\n");
}

#[test]
fn immutable_assignment_retains_exact_operator_span() {
    let source = "-- π\nvalue = 100\nvalue = 200\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0044",
        Some("invalid_assignment_target.immutable_binding"),
    );

    let operator_start = source
        .rfind('=')
        .expect("the reassignment operator should be present");
    let mut span_builder = ExtendedSpanBuilder::new();
    let operator_span = LocalSpan::exact(operator_start as u32, 1, &mut span_builder)
        .expect("the assignment operator should fit the local span table");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator_span))
    );
    assert_eq!(ast_error_label_spans(diagnostic).len(), 1);
}

#[test]
fn allows_whole_literal_into_decimal_binding_at_f64_context() {
    let (ast, path_fork, string_table) = parse_single_file_ast("total ~= 1.5\ntotal = 2\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\nvalue /= 4\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~U8 = 1\nvalue += 1\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected compound assignment");
    };
    // D15: equal `U8` operands keep `U8`, so the same-type update performs one checked
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
        parse_single_file_ast("wide U64 = 1\nvalue ~U8 = 1\nvalue += wide\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\nvalue //= 4\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~F64 = 10\nvalue /= 4\n");
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
    let failure = parse_single_file_ast_diagnostic("value ~F64 = 10\nvalue //= 4\n");
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0003", None);
    let facts = diagnostic
        .family_view::<UnsupportedOperatorTypesView>()
        .expect("unsupported-operator diagnostic should retain typed facts");
    assert_eq!(facts.operator, DiagnosticOperator::IntDivide);
}

#[test]
fn allows_i32_modulus_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\nvalue %= 4\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 2\nvalue ^= 3\n");
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
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Dec2 = 2.5\nvalue ^= 2\n");
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
        parse_single_file_ast("value ~I32 = 2\nvalue = (1 + 2)\nmarker I32 = value\n");
    let body = start_function_body(&ast, &path_fork, &string_table);
    assert!(
        matches!(&body[2].kind, NodeKind::VariableDeclaration(_)),
        "a trailing grouped RHS must leave the next statement intact"
    );

    let (ast, path_fork, string_table) =
        parse_single_file_ast("value ~Dec4 = 2\nvalue ^= (1 + 2)\nmarker I32 = 1\n");
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
            format!("value ~Dec2 = 2.5\nright {right_type} = {right_value}\nvalue ^= right\n");
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
