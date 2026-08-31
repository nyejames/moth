//! Regression tests for constant-expression folding helpers.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::value_production::ProducedValues;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::test_if_branch_metadata;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};

fn test_template_ir_store() -> Rc<RefCell<TemplateIrStore>> {
    Rc::new(RefCell::new(TemplateIrStore::new()))
}

fn assert_compile_time_error(
    error: &ConstantFoldError,
    expected_reason: CompileTimeEvaluationErrorReason,
    expected_operation: Option<&str>,
    string_table: &StringTable,
) {
    let diagnostic = match error {
        ConstantFoldError::Diagnostic(diagnostic) => diagnostic,
        ConstantFoldError::Infrastructure(error) => {
            panic!("expected compile-time diagnostic, found infrastructure error: {error:?}")
        }
    };

    match &diagnostic.payload {
        DiagnosticPayload::CompileTimeEvaluationError { reason, operation } => {
            assert_eq!(*reason, expected_reason);

            let operation_text = operation.map(|operation| string_table.resolve(operation));
            assert_eq!(operation_text, expected_operation);
        }
        payload => panic!("expected compile-time evaluation payload, found {payload:?}"),
    }
}

fn assert_invalid_cast_error(error: &ConstantFoldError, expected_reason: InvalidCastReason) {
    let diagnostic = match error {
        ConstantFoldError::Diagnostic(diagnostic) => diagnostic,
        ConstantFoldError::Infrastructure(error) => {
            panic!("expected invalid-cast diagnostic, found infrastructure error: {error:?}")
        }
    };

    match &diagnostic.payload {
        DiagnosticPayload::InvalidCast { reason, .. } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid-cast payload, found {payload:?}"),
    }
}

fn cast_expression(
    source: Expression,
    target: BuiltinCastTarget,
    target_type_id: TypeId,
    evidence: ResolvedCastEvidence,
    handling: CastHandling,
    requires_optional_wrap_after_cast: bool,
    type_environment: &mut TypeEnvironment,
) -> Expression {
    let source_type_id = source.type_id;
    let location = source.location.clone();
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        evidence,
        handling,
        location,
    };

    let result_type_id = if requires_optional_wrap_after_cast {
        type_environment.intern_option(target_type_id)
    } else {
        target_type_id
    };

    Expression::cast(cast, result_type_id, type_environment)
}

fn expect_folded_operator(result: Result<OperatorFoldOutcome, ConstantFoldError>) -> Expression {
    match result.expect("operator evaluation should succeed") {
        OperatorFoldOutcome::Folded(expression) => expression,
        OperatorFoldOutcome::NotConstant => panic!("operator should fold"),
        OperatorFoldOutcome::TextUnavailable { .. } => {
            panic!("operator text should be available")
        }
    }
}

fn expect_folded_stack(
    result: Result<ConstantFoldOutcome, ConstantFoldError>,
) -> Vec<ExpressionRpnItem> {
    match result.expect("constant folding should succeed") {
        ConstantFoldOutcome::Folded(stack) => stack,
        ConstantFoldOutcome::NotConstant(_) => panic!("expected a fully folded stack"),
        ConstantFoldOutcome::TextUnavailable { .. } => {
            panic!("expected all folded text to be available")
        }
    }
}

fn expect_not_constant_stack(
    result: Result<ConstantFoldOutcome, ConstantFoldError>,
) -> Vec<ExpressionRpnItem> {
    match result.expect("constant folding should succeed") {
        ConstantFoldOutcome::NotConstant(stack) => stack,
        ConstantFoldOutcome::Folded(_) => panic!("expected a runtime stack"),
        ConstantFoldOutcome::TextUnavailable { .. } => {
            panic!("expected a runtime-dependent stack")
        }
    }
}

#[test]
fn structural_string_requirement_has_stable_rule_identity() {
    let requirements = [
        (
            ConstStringRequirement::EqualityComparison,
            "string equality comparison",
        ),
        (ConstStringRequirement::CastOrParse, "string cast or parse"),
        (
            ConstStringRequirement::CompileTimeMapKey,
            "compile-time map key",
        ),
        (
            ConstStringRequirement::DuplicateKeyValidation,
            "duplicate map-key validation",
        ),
    ];

    for (requirement, operation_name) in requirements {
        let mut string_table = StringTable::new();
        let value =
            Expression::structural_string(vec![ConstStringPiece::SiteRoot], Default::default());
        let diagnostic = require_concrete_text(&value, requirement, &mut string_table)
            .expect_err("structural strings should require a final-text diagnostic");

        let identity = diagnostic.identity();
        assert_eq!(identity.code, "MOTH-RULE-0053");
        assert_eq!(
            identity.reason_key,
            Some("compile_time_evaluation_error.structural_string_requires_final_text")
        );
        match &diagnostic.payload {
            DiagnosticPayload::CompileTimeEvaluationError { operation, .. } => {
                assert_eq!(
                    operation.map(|id| string_table.resolve(id)),
                    Some(operation_name)
                );
            }
            payload => panic!("expected compile-time evaluation payload, found {payload:?}"),
        }
    }
}

#[test]
fn all_text_structural_string_requirement_concatenates_in_order() {
    // Test-only construction stands in for item 3's first structural text-piece producer.
    let mut string_table = StringTable::new();
    let first = string_table.intern("first/");
    let second = string_table.intern("second");
    let value = Expression::structural_string(
        vec![
            ConstStringPiece::Text(first),
            ConstStringPiece::Text(second),
        ],
        Default::default(),
    );

    let text = require_concrete_text(
        &value,
        ConstStringRequirement::EqualityComparison,
        &mut string_table,
    )
    .expect("all-text structural values have known final text")
    .expect("string values should return text");

    assert_eq!(string_table.resolve(text), "first/second");
}

#[test]
fn structural_string_equality_reports_text_unavailable_outcome() {
    let mut string_table = StringTable::new();
    let lhs = Expression::structural_string(vec![ConstStringPiece::SiteRoot], Default::default());
    let rhs = Expression::string_slice(
        string_table.intern("plain"),
        Default::default(),
        ValueMode::ImmutableOwned,
    );

    let outcome = lhs
        .evaluate_operator(&rhs, &Operator::Equality, &mut string_table)
        .expect("structural equality should be a typed fold refusal");

    let OperatorFoldOutcome::TextUnavailable { diagnostic } = outcome else {
        panic!("expected structural equality to report unavailable text");
    };
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("compile_time_evaluation_error.structural_string_requires_final_text")
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("string equality comparison")),
        "the refusal must name the operation that needed final text: {guidance:?}"
    );
}

#[test]
fn constant_fold_propagates_structural_string_text_unavailable_outcome() {
    let mut string_table = StringTable::new();
    let nodes = vec![
        ExpressionRpnItem::Operand(Expression::structural_string(
            vec![ConstStringPiece::SiteRoot],
            Default::default(),
        )),
        ExpressionRpnItem::Operand(Expression::string_slice(
            string_table.intern("plain"),
            Default::default(),
            ValueMode::ImmutableOwned,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::Equality,
            location: Default::default(),
        },
    ];

    let outcome = constant_fold(nodes, &mut string_table)
        .expect("structural equality should return a typed fold outcome");
    let ConstantFoldOutcome::TextUnavailable { diagnostic, .. } = outcome else {
        panic!("expected constant folding to preserve text-unavailable outcome");
    };
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("compile_time_evaluation_error.structural_string_requires_final_text")
    );
    let render_context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("string equality comparison")),
        "the operation name must survive the operator-to-stack hop: {guidance:?}"
    );
}

#[test]
fn text_unavailable_refusal_keeps_the_items_that_follow_it() {
    let mut string_table = StringTable::new();
    let flag = InternedPath::from_single_str("flag", &mut string_table);
    let nodes = vec![
        ExpressionRpnItem::Operand(Expression::structural_string(
            vec![ConstStringPiece::SiteRoot],
            Default::default(),
        )),
        ExpressionRpnItem::Operand(Expression::string_slice(
            string_table.intern("plain"),
            Default::default(),
            ValueMode::ImmutableOwned,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::Equality,
            location: Default::default(),
        },
        ExpressionRpnItem::Operand(Expression::reference(
            flag,
            DataType::Bool,
            SourceLocation::default(),
            ValueMode::ImmutableReference,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            location: Default::default(),
        },
    ];
    let authored_items = nodes.len();

    let outcome = constant_fold(nodes, &mut string_table)
        .expect("structural equality should return a typed fold outcome");
    let ConstantFoldOutcome::TextUnavailable { items, .. } = outcome else {
        panic!("expected constant folding to report unavailable text");
    };

    // Returning early on the refusal would drop `flag and` and silently miscompile
    // `(@/ == "plain") and flag` into a comparison alone.
    assert_eq!(
        items.len(),
        authored_items,
        "every authored item must reach runtime lowering: {items:?}"
    );
    assert!(
        matches!(
            items.last(),
            Some(ExpressionRpnItem::Operator {
                operator: Operator::And,
                ..
            })
        ),
        "the trailing operator must survive the refusal: {items:?}"
    );
}

#[test]
fn evaluate_operator_rejects_string_concatenation() {
    let mut string_table = StringTable::new();
    let lhs = Expression::string_slice(
        string_table.intern("moth"),
        Default::default(),
        ValueMode::ImmutableOwned,
    );
    let rhs = Expression::string_slice(
        string_table.intern("ball"),
        Default::default(),
        ValueMode::ImmutableOwned,
    );

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table)
        .expect_err("string concatenation should not fold at compile time");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::InvalidOperatorForType,
        Some("+"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_negative_integer_exponent() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(-1, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table)
        .expect_err("negative integer exponent should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_returns_not_constant_for_mismatched_constant_types() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::bool(true, Default::default(), ValueMode::ImmutableOwned);

    let result = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table)
        .expect("mismatched types should not error");

    assert!(matches!(result, OperatorFoldOutcome::NotConstant));
}

#[test]
fn evaluate_operator_divides_ints_to_float() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(5, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);

    let result =
        expect_folded_operator(lhs.evaluate_operator(&rhs, &Operator::Divide, &mut string_table));

    assert!(matches!(
        result.kind,
        ExpressionKind::Float(value) if (value - 2.5).abs() < f64::EPSILON
    ));
    assert_eq!(result.diagnostic_type, DataType::Float);
    assert!(
        result.contains_regular_division,
        "folded regular division should preserve provenance"
    );
}

#[test]
fn evaluate_operator_integer_division_truncates_toward_zero() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(-5, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);

    let result = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::IntDivide,
        &mut string_table,
    ));

    assert!(matches!(result.kind, ExpressionKind::Int(-2)));
    assert_eq!(result.diagnostic_type, DataType::Int);
}

#[test]
fn evaluate_operator_rejects_divide_by_zero_for_both_division_operators() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(5, Default::default(), ValueMode::ImmutableOwned);
    let zero = Expression::int(0, Default::default(), ValueMode::ImmutableOwned);

    let divide_error = lhs
        .evaluate_operator(&zero, &Operator::Divide, &mut string_table)
        .expect_err("regular division by zero should fail during fold");
    assert_compile_time_error(
        &divide_error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );

    let int_divide_error = lhs
        .evaluate_operator(&zero, &Operator::IntDivide, &mut string_table)
        .expect_err("integer division by zero should fail during fold");
    assert_compile_time_error(
        &int_divide_error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_add_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(i32::MAX, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(1, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table)
        .expect_err("integer add overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_subtract_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(i32::MIN, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(1, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Subtract, &mut string_table)
        .expect_err("integer subtract overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_multiply_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(i32::MAX, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Multiply, &mut string_table)
        .expect_err("integer multiply overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("*"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_exponent_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(31, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table)
        .expect_err("integer exponent overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_division_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(i32::MIN, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(-1, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::IntDivide, &mut string_table)
        .expect_err("integer division overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("//"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_integer_modulus_overflow() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(i32::MIN, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::int(-1, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Modulus, &mut string_table)
        .expect_err("integer modulus overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("%"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_non_finite_float_exponent_result() {
    let mut string_table = StringTable::new();
    let lhs = Expression::float(1.0e308, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::float(2.0, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table)
        .expect_err("non-finite float exponent result should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_non_finite_float_multiply_result() {
    let mut string_table = StringTable::new();
    let lhs = Expression::float(1.0e308, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::float(1.0e308, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Multiply, &mut string_table)
        .expect_err("non-finite float multiply result should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some("*"),
        &string_table,
    );
}

#[test]
fn constant_fold_rejects_integer_unary_negation_overflow() {
    let mut string_table = StringTable::new();
    let nodes = vec![
        rvalue_item(Expression::int(
            i32::MIN,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Negate),
    ];

    let error = constant_fold(nodes, &mut string_table)
        .expect_err("unary negation of i32::MIN should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_rejects_float_modulo_by_zero() {
    let mut string_table = StringTable::new();
    let lhs = Expression::float(1.0, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::float(0.0, Default::default(), ValueMode::ImmutableOwned);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Modulus, &mut string_table)
        .expect_err("float modulo by zero should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_mixed_int_float_addition() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(2, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::float(1.5, Default::default(), ValueMode::ImmutableOwned);

    let result =
        expect_folded_operator(lhs.evaluate_operator(&rhs, &Operator::Add, &mut string_table));

    assert!(matches!(
        result.kind,
        ExpressionKind::Float(value) if (value - 3.5).abs() < f64::EPSILON
    ));
    assert_eq!(result.diagnostic_type, DataType::Float);
}

#[test]
fn evaluate_operator_folds_mixed_int_float_division() {
    let mut string_table = StringTable::new();
    let lhs = Expression::int(5, Default::default(), ValueMode::ImmutableOwned);
    let rhs = Expression::float(2.0, Default::default(), ValueMode::ImmutableOwned);

    let result =
        expect_folded_operator(lhs.evaluate_operator(&rhs, &Operator::Divide, &mut string_table));

    assert!(matches!(
        result.kind,
        ExpressionKind::Float(value) if (value - 2.5).abs() < f64::EPSILON
    ));
    assert_eq!(result.diagnostic_type, DataType::Float);
}

#[test]
fn constant_fold_reports_static_failure_inside_runtime_expression() {
    let mut string_table = StringTable::new();
    let runtime_var = Expression::reference(
        InternedPath::from_single_str("runtime_var", &mut string_table),
        DataType::Int,
        SourceLocation::default(),
        ValueMode::ImmutableReference,
    );
    let one = Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned);
    let zero = Expression::int(0, SourceLocation::default(), ValueMode::ImmutableOwned);

    let nodes = vec![
        rvalue_item(runtime_var),
        rvalue_item(one),
        rvalue_item(zero),
        operator_item(Operator::Divide),
        operator_item(Operator::Add),
    ];

    let error = constant_fold(nodes, &mut string_table)
        .expect_err("divide by zero inside a runtime expression should still be diagnosed");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );
}

#[test]
fn constant_fold_partially_folds_runtime_expression() {
    let mut string_table = StringTable::new();
    let runtime_var = Expression::reference(
        InternedPath::from_single_str("runtime_var", &mut string_table),
        DataType::Int,
        SourceLocation::default(),
        ValueMode::ImmutableReference,
    );
    let two = Expression::int(2, SourceLocation::default(), ValueMode::ImmutableOwned);
    let three = Expression::int(3, SourceLocation::default(), ValueMode::ImmutableOwned);

    let nodes = vec![
        rvalue_item(runtime_var),
        rvalue_item(two),
        rvalue_item(three),
        operator_item(Operator::Add),
        operator_item(Operator::Multiply),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table));

    assert_eq!(folded.len(), 3);
    assert!(matches!(
        &folded[0],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Reference(..),
            ..
        })
    ));
    assert!(matches!(
        &folded[1],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Int(5),
            ..
        })
    ));
    assert!(matches!(
        &folded[2],
        ExpressionRpnItem::Operator {
            operator: Operator::Multiply,
            ..
        }
    ));
}

#[test]
fn fold_int_cast_rejects_out_of_range_float_with_dedicated_code() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

    let source = BuiltinCastLiteral::Float(9_223_372_036_854_775_808.0);
    let error = apply_builtin_cast_policy(BuiltinCastPolicyId::FloatToInt, &source)
        .expect_err("out-of-range float to int cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn fold_int_cast_rejects_non_finite_float_with_dedicated_code() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

    let source = BuiltinCastLiteral::Float(f64::INFINITY);
    let error = apply_builtin_cast_policy(BuiltinCastPolicyId::FloatToInt, &source)
        .expect_err("non-finite float to int cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntInvalidValue);
}

#[test]
fn fold_int_cast_truncates_toward_zero() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };

    let source = BuiltinCastLiteral::Float(1.9);
    let result = apply_builtin_cast_policy(BuiltinCastPolicyId::FloatToInt, &source)
        .expect("float to int cast should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(1));

    let source = BuiltinCastLiteral::Float(-1.9);
    let result = apply_builtin_cast_policy(BuiltinCastPolicyId::FloatToInt, &source)
        .expect("negative float to int cast should fold");
    assert_eq!(result, BuiltinCastLiteral::Int(-1));
}

#[test]
fn fold_float_cast_rejects_non_finite_string_value() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

    let huge = format!("{}.0", "9".repeat(400));
    let source = BuiltinCastLiteral::String(huge);
    let error = apply_builtin_cast_policy(BuiltinCastPolicyId::StringToFloat, &source)
        .expect_err("non-finite float string cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseOutOfRange);
}

#[test]
fn fold_string_to_int_cast_uses_string_policy_row() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("42".to_string());
    let source = Expression::string_slice(text, Default::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToInt,
        },
        CastHandling::Propagate,
        false,
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("valid string to int cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(folded.kind, ExpressionKind::Int(42)));
}

#[test]
fn fold_string_to_float_cast_uses_string_policy_row() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("3.5e2".to_string());
    let source = Expression::string_slice(text, Default::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().float;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::Float,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToFloat,
        },
        CastHandling::Propagate,
        false,
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("valid string to float cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(folded.kind, ExpressionKind::Float(value) if value == 350.0));
}

fn rvalue_item(expression: Expression) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

fn operator_item(operator: Operator) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator {
        operator,
        location: SourceLocation::default(),
    }
}

#[test]
fn constant_fold_folds_comparison_then_boolean_chain() {
    let mut string_table = StringTable::new();
    let nodes = vec![
        rvalue_item(Expression::int(
            1,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        rvalue_item(Expression::int(
            2,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::LessThan),
        rvalue_item(Expression::bool(
            true,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::And),
    ];

    let folded = expect_folded_stack(constant_fold(nodes, &mut string_table));
    assert_eq!(folded.len(), 1);
    assert!(matches!(
        folded[0],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Bool(true),
            ..
        })
    ));
}

#[test]
fn constant_fold_keeps_unary_not_when_operand_is_not_bool_literal() {
    let mut string_table = StringTable::new();
    let nodes = vec![
        rvalue_item(Expression::int(
            1,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Not),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table));
    assert_eq!(folded.len(), 2);
    assert!(matches!(
        folded[0],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Int(1),
            ..
        })
    ));
    assert!(matches!(
        folded[1],
        ExpressionRpnItem::Operator {
            operator: Operator::Not,
            ..
        }
    ));
}

#[test]
fn constant_fold_preserves_runtime_operands_in_partial_fold() {
    let mut string_table = StringTable::new();
    let flag_name = InternedPath::from_single_str("flag", &mut string_table);
    let nodes = vec![
        rvalue_item(Expression::reference(
            flag_name,
            DataType::Bool,
            SourceLocation::default(),
            ValueMode::ImmutableReference,
        )),
        rvalue_item(Expression::bool(
            true,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::And),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table));

    assert_eq!(folded.len(), 3);
    assert!(matches!(
        folded[0],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Reference(_),
            ..
        })
    ));
    assert!(matches!(
        folded[1],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::Bool(true),
            ..
        })
    ));
    assert!(matches!(
        folded[2],
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            ..
        }
    ));
}

/// Build a source location that is distinguishable from every other one in a test.
fn marked_location(line: i32, string_table: &mut StringTable) -> SourceLocation {
    SourceLocation::new(
        InternedPath::from_single_str("provenance_probe", string_table),
        CharPosition {
            line_number: line,
            char_column: line * 10,
        },
        CharPosition {
            line_number: line,
            char_column: line * 10 + 4,
        },
    )
}

#[test]
fn partial_fold_moves_non_foldable_operands_back_without_rebuilding_them() {
    // Folding consumes its input, so a moved-back operand could silently become a
    // reconstruction. Distinct locations and value modes on every input make that visible:
    // a rebuilt operand would carry defaults, not the values asserted below.
    let mut string_table = StringTable::new();
    let flag_name = InternedPath::from_single_str("flag", &mut string_table);
    let flag_location = marked_location(7, &mut string_table);
    let literal_location = marked_location(11, &mut string_table);
    let operator_location = marked_location(23, &mut string_table);

    let nodes = vec![
        rvalue_item(Expression::reference(
            flag_name.clone(),
            DataType::Bool,
            flag_location.clone(),
            ValueMode::MutableReference,
        )),
        rvalue_item(Expression::bool(
            true,
            literal_location.clone(),
            ValueMode::ImmutableOwned,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            location: operator_location.clone(),
        },
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table));

    assert_eq!(folded.len(), 3);

    let ExpressionRpnItem::Operand(runtime_operand) = &folded[0] else {
        panic!("the runtime reference should stay an operand");
    };
    assert_eq!(runtime_operand.location, flag_location);
    assert_eq!(runtime_operand.value_mode, ValueMode::MutableReference);

    let ExpressionRpnItem::Operand(literal_operand) = &folded[1] else {
        panic!("the literal should stay an operand");
    };
    assert_eq!(literal_operand.location, literal_location);
    assert_eq!(literal_operand.value_mode, ValueMode::ImmutableOwned);

    let ExpressionRpnItem::Operator { operator, location } = &folded[2] else {
        panic!("the unfoldable operator should be preserved");
    };
    assert_eq!(*operator, Operator::And);
    assert_eq!(*location, operator_location);
}

#[test]
fn partial_fold_keeps_the_folded_half_and_the_moved_half_distinct() {
    // A fold that reduces only part of the expression must move the untouched operands back in
    // their original order while the folded operand takes its own provenance from the fold.
    let mut string_table = StringTable::new();
    let counter_name = InternedPath::from_single_str("counter", &mut string_table);
    let counter_location = marked_location(3, &mut string_table);
    let left_literal_location = marked_location(5, &mut string_table);

    let nodes = vec![
        rvalue_item(Expression::reference(
            counter_name,
            DataType::Int,
            counter_location.clone(),
            ValueMode::ImmutableReference,
        )),
        rvalue_item(Expression::int(
            2,
            left_literal_location.clone(),
            ValueMode::ImmutableOwned,
        )),
        rvalue_item(Expression::int(
            3,
            marked_location(6, &mut string_table),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Add),
        operator_item(Operator::Multiply),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table));

    assert_eq!(folded.len(), 3);

    let ExpressionRpnItem::Operand(moved) = &folded[0] else {
        panic!("the runtime reference should stay an operand");
    };
    assert_eq!(moved.location, counter_location);

    let ExpressionRpnItem::Operand(computed) = &folded[1] else {
        panic!("the constant half should fold to one operand");
    };
    assert!(matches!(computed.kind, ExpressionKind::Int(5)));
    // The folded operand inherits the left operand's anchor, so the reduction stays
    // attributable to authored source rather than to a synthesized position.
    assert_eq!(computed.location, left_literal_location);
}

#[test]
fn full_fold_returns_the_folded_operand_with_its_source_anchor() {
    // The single-result path hands the folded operand back by move. Its anchor must still be
    // the authored one, not a default produced by rebuilding the value.
    let mut string_table = StringTable::new();
    let left_location = marked_location(13, &mut string_table);

    let nodes = vec![
        rvalue_item(Expression::int(
            20,
            left_location.clone(),
            ValueMode::ImmutableOwned,
        )),
        rvalue_item(Expression::int(
            22,
            marked_location(14, &mut string_table),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Add),
    ];

    let folded = expect_folded_stack(constant_fold(nodes, &mut string_table));

    assert_eq!(folded.len(), 1);
    let ExpressionRpnItem::Operand(result) = &folded[0] else {
        panic!("a fully folded expression should be one operand");
    };
    assert!(matches!(result.kind, ExpressionKind::Int(42)));
    assert_eq!(result.location, left_location);
}

#[test]
fn fold_cast_infallible_int_to_string_folds_to_string_literal() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().string;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::String,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::IntToString,
        },
        CastHandling::Infallible,
        false,
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("infallible builtin cast should fold");

    assert_eq!(folded.type_id, target_type_id);

    let ExpressionKind::StringSlice(interned) = folded.kind else {
        panic!("expected folded Int -> String cast to produce a string slice");
    };

    assert_eq!(string_table.resolve(interned), "42");
}

#[test]
fn fold_structural_string_cast_reports_text_unavailable_rule() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let source =
        Expression::structural_string(vec![ConstStringPiece::SiteRoot], Default::default());
    let target_type_id = type_environment.builtins().int;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToInt,
        },
        CastHandling::Propagate,
        false,
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("structural string cast should require final text");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        Some("string cast or parse"),
        &string_table,
    );
}

#[test]
fn fold_cast_optional_wrap_coerces_value_to_optional() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::int(7, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().string;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::String,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::IntToString,
        },
        CastHandling::Infallible,
        true,
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("optional-wrapped infallible cast should fold");

    assert_eq!(
        folded.type_id,
        type_environment.intern_option(target_type_id)
    );

    let ExpressionKind::Coerced { value, .. } = folded.kind else {
        panic!("expected optional-wrapped cast to produce a Coerced expression");
    };

    let ExpressionKind::StringSlice(interned) = value.kind else {
        panic!("expected coerced inner value to be a string slice");
    };

    assert_eq!(string_table.resolve(interned), "7");
}

#[test]
fn fold_cast_fallible_string_to_int_success_folds_to_int() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("123".to_string());
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToInt,
        },
        CastHandling::Propagate,
        false,
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("successful fallible builtin cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(folded.kind, ExpressionKind::Int(123)));
}

#[test]
fn fold_cast_fallible_string_to_int_failure_reports_builtin_cast_failed_in_const() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("not a number".to_string());
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToInt,
        },
        CastHandling::Propagate,
        false,
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("failed fallible builtin cast should report a const diagnostic");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn fold_cast_user_defined_evidence_rejected_in_const_context() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().string;
    let method_path = InternedPath::from_single_str("to_string", &mut string_table);

    let cast = cast_expression(
        source,
        BuiltinCastTarget::String,
        target_type_id,
        ResolvedCastEvidence::UserDefined {
            evidence_id: TraitEvidenceId(0),
            method_path,
        },
        CastHandling::Infallible,
        false,
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("user-defined evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
    );
}

#[test]
fn fold_cast_generic_bound_evidence_rejected_in_const_context() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().string;

    let cast = cast_expression(
        source,
        BuiltinCastTarget::String,
        target_type_id,
        ResolvedCastEvidence::GenericBound {
            trait_id: TraitId(0),
            parameter_id: GenericParameterId(0),
        },
        CastHandling::Infallible,
        false,
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("generic-bound evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::GenericBoundEvidenceNotConstFoldable,
    );
}

fn catch_handler_body(value: Expression) -> Vec<AstNode> {
    let location = value.location.clone();

    vec![AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions: vec![value],
            location: location.clone(),
        }),
        location,
        scope: InternedPath::new(),
    }]
}

fn fallible_builtin_cast_with_catch(
    source: Expression,
    target: BuiltinCastTarget,
    target_type_id: TypeId,
    policy: BuiltinCastPolicyId,
    handler_body: Vec<AstNode>,
    type_environment: &mut TypeEnvironment,
) -> Expression {
    let cast = cast_expression(
        source,
        target,
        target_type_id,
        ResolvedCastEvidence::Builtin { policy },
        CastHandling::Recover,
        false,
        type_environment,
    );

    wrap_catch_expression(
        cast,
        FallibleHandling::Handler {
            error: None,
            body: handler_body,
        },
        vec![target_type_id],
    )
}

#[test]
fn fold_cast_fallible_builtin_failure_with_catch_folds_to_handler_value() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("nope".to_string());
    let source_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "source",
    );
    let handler_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::Builder,
        "render",
        "fallback",
    );
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
                source_member.clone(),
            ));
    let target_type_id = type_environment.builtins().int;
    let handler_value = Expression::int(0, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::from_members(vec![
            handler_member.clone(),
            handler_member.clone(),
        ]));

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        BuiltinCastPolicyId::StringToInt,
        catch_handler_body(handler_value),
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("failed builtin cast with foldable catch handler should fold to handler value");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(folded.kind, ExpressionKind::Int(0)));
    assert_eq!(
        folded.synthetic_interface_provenance.members(),
        &[source_member, handler_member]
    );
}

#[test]
fn fold_cast_fallible_builtin_success_with_catch_ignores_handler() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("123".to_string());
    let source_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "source",
    );
    let handler_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::Builder,
        "render",
        "fallback",
    );
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned)
            .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
                source_member.clone(),
            ));
    let target_type_id = type_environment.builtins().int;
    let handler_value = Expression::int(999, SourceLocation::default(), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(handler_member));

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        BuiltinCastPolicyId::StringToInt,
        catch_handler_body(handler_value),
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect("successful builtin cast should fold to success value even with catch handler");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(folded.kind, ExpressionKind::Int(123)));
    assert_eq!(
        folded.synthetic_interface_provenance.members(),
        &[source_member]
    );
}

#[test]
fn fold_cast_fallible_builtin_failure_with_non_foldable_catch_rejects_handler() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("nope".to_string());
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;

    let handler_value = Expression::reference(
        InternedPath::from_single_str("runtime_value", &mut string_table),
        DataType::Int,
        SourceLocation::default(),
        ValueMode::ImmutableReference,
    );

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        BuiltinCastPolicyId::StringToInt,
        catch_handler_body(handler_value),
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("non-foldable catch handler should be rejected in const context");

    assert_invalid_cast_error(&error, InvalidCastReason::CatchHandlerNotConstFoldable);
}

#[test]
fn fold_cast_fallible_builtin_failure_with_empty_catch_rejects_handler() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("nope".to_string());
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        BuiltinCastPolicyId::StringToInt,
        Vec::new(),
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("empty catch handler should be rejected in const context");

    assert_invalid_cast_error(&error, InvalidCastReason::CatchHandlerNotConstFoldable);
}

#[test]
fn fold_cast_fallible_builtin_failure_with_branching_catch_rejects_handler() {
    let mut string_table = StringTable::new();
    let template_ir_store = test_template_ir_store();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("nope".to_string());
    let source =
        Expression::string_slice(text, SourceLocation::default(), ValueMode::ImmutableOwned);
    let target_type_id = type_environment.builtins().int;
    let location = SourceLocation::default();

    let then_body = catch_handler_body(Expression::int(
        1,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    ));
    let else_body = catch_handler_body(Expression::int(
        2,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    ));
    let branching_handler = vec![AstNode {
        kind: NodeKind::If(
            Expression::bool(false, location.clone(), ValueMode::ImmutableOwned),
            then_body,
            Some(else_body),
            test_if_branch_metadata(true),
        ),
        location,
        scope: InternedPath::new(),
    }];

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Int,
        target_type_id,
        BuiltinCastPolicyId::StringToInt,
        branching_handler,
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &template_ir_store, &mut string_table, true)
        .expect_err("branching catch handler needs real const statement evaluation");

    assert_invalid_cast_error(&error, InvalidCastReason::CatchHandlerNotConstFoldable);
}
