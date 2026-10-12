//! Mutation expression parsing and validation regression tests.
//!
//! WHAT: validates mutable assignment, field mutation, collection mutation, and place-expression
//!       requirements.
//! WHY: mutation rules are tightly coupled to borrow checking; parser-level tests ensure the
//!      frontend produces the right AST shapes for later analysis.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticOperator, DiagnosticPayload, InvalidAssignmentTargetReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn rejects_assignment_value_type_mismatch_with_specific_details() {
    assert_assignment_type_mismatch("value ~= 1\nvalue = true\n");
}

#[test]
fn immutable_assignment_retains_exact_operator_span() {
    let source = "-- π\nvalue = 100\nvalue = 200\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidAssignmentTarget {
            reason: InvalidAssignmentTargetReason::ImmutableBinding,
            ..
        }
    ));

    let operator_start = source
        .rfind('=')
        .expect("the reassignment operator should be present");
    let mut span_builder = ExtendedSpanBuilder::new();
    let operator_span = LocalSpan::exact(operator_start as u32, 1, &mut span_builder)
        .expect("the assignment operator should fit the local span table");
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, operator_span))
    );
    assert_eq!(diagnostic.labels.len(), 1);
}

#[test]
fn allows_whole_literal_into_decimal_binding_at_f64_context() {
    let (ast, path_fork, string_table) = parse_single_file_ast("total ~= 1.5\ntotal = 2\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
}

#[test]
fn allows_i32_divide_assign_with_checked_store_conversion() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\nvalue /= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };
    let ExpressionKind::Cast(cast) = &value.kind else {
        panic!("expected the F64 division result to convert at the I32 store");
    };

    assert_eq!(
        cast.source.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
    assert!(matches!(&cast.handling, CastHandling::StoreConversion));
    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I32)
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
    assert!(
        !matches!(value.kind, ExpressionKind::Cast(_)),
        "a same-type compound update must not carry a store conversion, got {:?}",
        value.kind
    );
    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U8)
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
    let ExpressionKind::Cast(cast) = &value.kind else {
        panic!("expected the mixed-domain U64 result to convert at the U8 store");
    };

    assert_eq!(
        cast.source.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U64)
    );
    assert!(matches!(&cast.handling, CastHandling::StoreConversion));
    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U8)
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
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I32)
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
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
}

#[test]
fn rejects_f64_integer_divide_assign_rhs() {
    let diagnostic = parse_single_file_ast_diagnostic("value ~F64 = 10\nvalue //= 4\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::IntDivide,
            ..
        }
    ));
}

#[test]
fn allows_i32_modulus_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 10\nvalue %= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I32)
    );
}

#[test]
fn allows_i32_exponent_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~I32 = 2\nvalue ^= 3\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I32)
    );
}

#[test]
fn allows_number_exponent_assign_with_i32_rhs() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Dec2 = 2.5\nvalue ^= 2\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { target, value } = &body[1].kind else {
        panic!("expected Dec exponent compound assignment");
    };

    assert_eq!(value.type_id, target.type_id);
    assert!(matches!(
        &value.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));
    let ExpressionKind::Runtime(rpn) = &value.kind else {
        panic!("mutable Dec power stays a runtime expression");
    };
    assert!(rpn.items.iter().any(|item| matches!(
        item,
        ExpressionRpnItem::Operand(operand)
            if operand.type_id == builtin_type_ids::fixed_scalar(FixedScalar::I32)
                && matches!(&operand.kind, ExpressionKind::FixedScalar(_))
    )));
    assert!(!rpn.items.iter().any(|item| matches!(
        item,
        ExpressionRpnItem::Operand(operand)
            if matches!(&operand.kind, ExpressionKind::Number(_))
    )));
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
    assert_eq!(value.type_id, target.type_id);
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
        let diagnostic = parse_single_file_ast_diagnostic(&source);

        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::UnsupportedOperatorTypes {
                operator: DiagnosticOperator::Exponent,
                ..
            }
        ));
    }
}

fn assert_assignment_type_mismatch(source: &str) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Assignment,
            ..
        }
    ));
}
