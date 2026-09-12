//! Mutation expression parsing and validation regression tests.
//!
//! WHAT: validates mutable assignment, field mutation, collection mutation, and place-expression
//!       requirements.
//! WHY: mutation rules are tightly coupled to borrow checking; parser-level tests ensure the
//!      frontend produces the right AST shapes for later analysis.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticOperator, DiagnosticPayload, InvalidAssignmentTargetReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

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
fn allows_int_to_float_assignment_via_contextual_coercion() {
    let (ast, path_fork, string_table) = parse_single_file_ast("total ~= 1.5\ntotal = 2\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(value.diagnostic_type, DataType::Float);
}

#[test]
fn rejects_int_divide_assign_when_regular_division_returns_float() {
    assert_assignment_type_mismatch("value ~Int = 10\nvalue /= 4\n");
}

#[test]
fn allows_int_integer_divide_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Int = 10\nvalue //= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(value.diagnostic_type, DataType::Int);
}

#[test]
fn allows_float_divide_assign_int_rhs() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Float = 10\nvalue /= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(value.diagnostic_type, DataType::Float);
}

#[test]
fn rejects_float_integer_divide_assign_rhs() {
    let diagnostic = parse_single_file_ast_diagnostic("value ~Float = 10\nvalue //= 4\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::IntDivide,
            ..
        }
    ));
}

#[test]
fn allows_int_modulus_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Int = 10\nvalue %= 4\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(value.diagnostic_type, DataType::Int);
}

#[test]
fn allows_int_exponent_assign() {
    let (ast, path_fork, string_table) = parse_single_file_ast("value ~Int = 2\nvalue ^= 3\n");
    let body = start_function_body(&ast, &path_fork, &string_table);

    let NodeKind::Assignment { value, .. } = &body[1].kind else {
        panic!("expected second statement to be an assignment");
    };

    assert_eq!(value.diagnostic_type, DataType::Int);
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
