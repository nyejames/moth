//! Regression tests for constant-expression folding helpers.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::value_production::ProducedValues;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

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
        DiagnosticPayload::CompileTimeEvaluationError {
            reason, operation, ..
        } => {
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

/// Explicit cast facts for one test-built `ResolvedCastExpression`; the source type and span
/// come from `source`.
struct CastExpressionInput {
    source: Expression,
    target: BuiltinCastTarget,
    target_type_id: TypeId,
    evidence: ResolvedCastEvidence,
    fallibility: BuiltinCastFallibility,
    handling: CastHandling,
    requires_optional_wrap_after_cast: bool,
}

fn cast_expression(
    input: CastExpressionInput,
    type_environment: &mut TypeEnvironment,
) -> Expression {
    let CastExpressionInput {
        source,
        target,
        target_type_id,
        evidence,
        fallibility,
        handling,
        requires_optional_wrap_after_cast,
    } = input;
    let source_type_id = source.type_id;
    let span = source.span;
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id,
        target_type_id,
        target,
        requires_optional_wrap_after_cast,
        fallibility,
        evidence,
        handling,
        span,
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

fn fixed_integer_expression(scalar: FixedScalar, value: i128) -> Expression {
    let value = match scalar.class() {
        FixedScalarClass::SignedInteger => FixedScalarValue::signed(
            scalar,
            i64::try_from(value).expect("test signed fixed integer fits its carrier"),
        )
        .expect("test signed fixed integer fits its domain"),
        FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(
            scalar,
            u64::try_from(value).expect("test unsigned fixed integer fits its carrier"),
        )
        .expect("test unsigned fixed integer fits its domain"),
        FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => {
            panic!("fixed integer test helper requires an integer scalar")
        }
    };
    Expression::fixed_scalar(value, None, ValueMode::ImmutableOwned)
}

fn number_expression(
    normalized: &str,
    sign: NumericLiteralSign,
    scale: u16,
    type_id: TypeId,
) -> Expression {
    let scale = NumberScale::new(scale).expect("test Dec scale is within the supported range");
    let value = NumberValue::from_normalized(normalized, sign, scale)
        .expect("test Dec value is exact at its scale");
    Expression::number(value, type_id, None, ValueMode::ImmutableOwned)
}

fn fixed_float_value(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    let precision = match scalar {
        FixedScalar::F16 => BinaryFloatPrecision::Binary16,
        FixedScalar::F32 => BinaryFloatPrecision::Binary32,
        FixedScalar::F64 => BinaryFloatPrecision::Binary64,
        _ => panic!("fixed float test helper requires a binary-float scalar"),
    };
    FixedScalarValue::binary_float(scalar, precision.round(value))
        .expect("test float rounds to a finite fixed scalar")
}

fn fixed_float_expression(scalar: FixedScalar, value: f64) -> Expression {
    Expression::fixed_scalar(
        fixed_float_value(scalar, value),
        None,
        ValueMode::ImmutableOwned,
    )
}

fn expect_fixed_scalar(expression: Expression) -> FixedScalarValue {
    match expression.kind {
        ExpressionKind::FixedScalar(value) => value,
        other => panic!("expected a fixed scalar fold, got {other:?}"),
    }
}

fn expect_folded_boolean(
    lhs: Expression,
    rhs: Expression,
    operator: Operator,
    string_table: &mut StringTable,
) -> bool {
    let folded = expect_folded_operator(lhs.evaluate_operator(&rhs, &operator, string_table, None));
    match folded.kind {
        ExpressionKind::Bool(value) => value,
        other => panic!("expected a boolean comparison result, got {other:?}"),
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
        let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
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
    let _path_fork = PathInternerFork::empty();
    let lhs = Expression::structural_string(vec![ConstStringPiece::SiteRoot], Default::default());
    let rhs = Expression::string_slice(
        string_table.intern("plain"),
        Default::default(),
        ValueMode::ImmutableOwned,
    );

    let outcome = lhs
        .evaluate_operator(&rhs, &Operator::Equality, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
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
            span: None,
        },
    ];

    let outcome = constant_fold(nodes, &mut string_table, None)
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
    let mut path_fork = PathInternerFork::empty();
    let flag = path_fork
        .try_intern_portable_path("flag", &mut string_table)
        .expect("test path fits");
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
            span: None,
        },
        ExpressionRpnItem::Operand(Expression::reference(
            flag,
            DataType::Bool,
            None,
            ValueMode::ImmutableReference,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            span: None,
        },
    ];
    let authored_items = nodes.len();

    let outcome = constant_fold(nodes, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
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
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 2);
    let rhs = fixed_integer_expression(FixedScalar::I32, -1);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 2);
    let rhs = Expression::bool(true, Default::default(), ValueMode::ImmutableOwned);

    let result = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table, None)
        .expect("mismatched types should not error");

    assert!(matches!(result, OperatorFoldOutcome::NotConstant));
}

#[test]
fn evaluate_operator_divides_ints_to_float() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 5);
    let rhs = fixed_integer_expression(FixedScalar::I32, 2);

    let result = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Divide,
        &mut string_table,
        None,
    ));

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_f64() == Some(2.5)
    ));
    assert_eq!(
        result.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
    assert!(
        result.contains_regular_division,
        "folded regular division should preserve provenance"
    );
}

#[test]
fn evaluate_operator_integer_division_truncates_toward_zero() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, -5);
    let rhs = fixed_integer_expression(FixedScalar::I32, 2);

    let result = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::IntDivide,
        &mut string_table,
        None,
    ));

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(-2)
    ));
    assert_eq!(
        result.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I32)
    );
}

#[test]
fn evaluate_operator_rejects_divide_by_zero_for_both_division_operators() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 5);
    let zero = fixed_integer_expression(FixedScalar::I32, 0);

    let divide_error = lhs
        .evaluate_operator(&zero, &Operator::Divide, &mut string_table, None)
        .expect_err("regular division by zero should fail during fold");
    assert_compile_time_error(
        &divide_error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );

    let int_divide_error = lhs
        .evaluate_operator(&zero, &Operator::IntDivide, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, i128::from(i32::MAX));
    let rhs = fixed_integer_expression(FixedScalar::I32, 1);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table, None)
        .expect_err("integer add overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_i64_add_beyond_i32_range() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I64, i128::from(i32::MAX));
    let rhs = fixed_integer_expression(FixedScalar::I64, 1);

    let folded = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(2_147_483_648)
    ));
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I64)
    );
}

#[test]
fn evaluate_operator_rejects_int64_add_overflow() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I64, i128::from(i64::MAX));
    let rhs = fixed_integer_expression(FixedScalar::I64, 1);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Add, &mut string_table, None)
        .expect_err("i64::MAX + 1 overflows the I64 boundary");
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, i128::from(i32::MIN));
    let rhs = fixed_integer_expression(FixedScalar::I32, 1);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Subtract, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, i128::from(i32::MAX));
    let rhs = fixed_integer_expression(FixedScalar::I32, 2);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Multiply, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 2);
    let rhs = fixed_integer_expression(FixedScalar::I32, 31);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table, None)
        .expect_err("integer exponent overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_trivial_base_exponent_despite_oversized_exponent() {
    // Only I64 can materialise an exponent that does not narrow to u32. Trivial bases still
    // fold to their exact result; |base| >= 2 keeps reporting IntegerOverflow.
    let oversized: i64 = i64::from(u32::MAX) + 1;

    for (base, exponent, expected) in [
        (0, oversized, 0),
        (1, oversized, 1),
        (-1, oversized, 1),
        (-1, oversized + 1, -1),
    ] {
        let mut string_table = StringTable::new();
        let lhs = fixed_integer_expression(FixedScalar::I64, i128::from(base));
        let rhs = fixed_integer_expression(FixedScalar::I64, i128::from(exponent));

        let outcome = lhs
            .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table, None)
            .unwrap_or_else(|_| panic!("{base}^{exponent} should fold"));
        let OperatorFoldOutcome::Folded(folded) = outcome else {
            panic!("{base}^{exponent} should fold to a constant");
        };
        assert!(
            matches!(folded.kind, ExpressionKind::FixedScalar(value) if value.as_i64() == Some(expected)),
            "{base}^{exponent} should fold to {expected}, got {:?}",
            folded.kind
        );
    }

    let mut string_table = StringTable::new();
    let lhs = fixed_integer_expression(FixedScalar::I64, 2);
    let rhs = fixed_integer_expression(FixedScalar::I64, i128::from(oversized));

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table, None)
        .expect_err("oversized exponent on base 2 should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_truncated_int_remainder_at_both_widths() {
    // Remainder keeps the dividend's sign, and `% -1` is zero even for the width's
    // signed minimum, whose paired quotient is the only overflowing case.
    for (scalar, minimum) in [
        (FixedScalar::I32, i128::from(i32::MIN)),
        (FixedScalar::I64, i128::from(i64::MIN)),
    ] {
        for (lhs, rhs, expected) in [(7, 3, 1), (-7, 3, -1), (7, -3, 1), (minimum, -1, 0)] {
            let mut string_table = StringTable::new();
            let lhs_expression = fixed_integer_expression(scalar, lhs);
            let rhs_expression = fixed_integer_expression(scalar, rhs);

            let outcome = lhs_expression
                .evaluate_operator(&rhs_expression, &Operator::Modulus, &mut string_table, None)
                .unwrap_or_else(|_| panic!("{lhs} % {rhs} should fold under {scalar:?}"));
            let OperatorFoldOutcome::Folded(folded) = outcome else {
                panic!("{lhs} % {rhs} should fold to a constant under {scalar:?}");
            };
            assert!(
                matches!(folded.kind, ExpressionKind::FixedScalar(value) if value.as_i64() == Some(expected)),
                "{lhs} % {rhs} should fold to {expected} under {scalar:?}, got {:?}",
                folded.kind
            );
        }
    }
}

#[test]
fn evaluate_operator_rejects_integer_division_overflow() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, i128::from(i32::MIN));
    let rhs = fixed_integer_expression(FixedScalar::I32, -1);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::IntDivide, &mut string_table, None)
        .expect_err("integer division overflow should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("//"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_signed_minimum_remainder_by_negative_one_to_zero() {
    // The quotient overflows, but the remainder is zero and must not inherit that check.
    for (scalar, minimum) in [
        (FixedScalar::I32, i128::from(i32::MIN)),
        (FixedScalar::I64, i128::from(i64::MIN)),
    ] {
        let mut string_table = StringTable::new();
        let lhs = fixed_integer_expression(scalar, minimum);
        let rhs = fixed_integer_expression(scalar, -1);

        let outcome = lhs
            .evaluate_operator(&rhs, &Operator::Modulus, &mut string_table, None)
            .unwrap_or_else(|_| panic!("{scalar:?} minimum % -1 should fold"));
        let OperatorFoldOutcome::Folded(folded) = outcome else {
            panic!("{scalar:?} minimum % -1 should fold to a constant");
        };
        assert!(
            matches!(folded.kind, ExpressionKind::FixedScalar(value) if value.as_i64() == Some(0)),
            "{scalar:?} minimum % -1 should be zero, got {:?}",
            folded.kind
        );
    }
}

#[test]
fn evaluate_operator_rejects_non_finite_float_exponent_result() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_float_expression(FixedScalar::F64, 1.0e308);
    let rhs = fixed_float_expression(FixedScalar::F64, 2.0);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Exponent, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_float_expression(FixedScalar::F64, 1.0e308);
    let rhs = fixed_float_expression(FixedScalar::F64, 1.0e308);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Multiply, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let nodes = vec![
        rvalue_item(fixed_integer_expression(
            FixedScalar::I32,
            i128::from(i32::MIN),
        )),
        operator_item(Operator::Negate),
    ];

    let error = constant_fold(nodes, &mut string_table, None)
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
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_float_expression(FixedScalar::F64, 1.0);
    let rhs = fixed_float_expression(FixedScalar::F64, 0.0);

    let error = lhs
        .evaluate_operator(&rhs, &Operator::Modulus, &mut string_table, None)
        .expect_err("float modulo by zero should fail during fold");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );
}

#[test]
fn evaluate_operator_folds_fixed_integer_results_in_the_operand_domain() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let cases = [
        (
            "equal U8 addition keeps U8",
            fixed_integer_expression(FixedScalar::U8, 100),
            fixed_integer_expression(FixedScalar::U8, 100),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("200 fits U8"),
        ),
        (
            "explicit U32 addition retains the wide sum",
            fixed_integer_expression(FixedScalar::U32, 255),
            fixed_integer_expression(FixedScalar::U32, 255),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U32, 510).expect("510 fits U32"),
        ),
        (
            "mixed signedness selects I16",
            fixed_integer_expression(FixedScalar::I8, -1),
            fixed_integer_expression(FixedScalar::U8, 255),
            Operator::Add,
            FixedScalarValue::signed(FixedScalar::I16, 254).expect("254 fits I16"),
        ),
        (
            "U64 maximum survives a no-op addition",
            fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX)),
            fixed_integer_expression(FixedScalar::U64, 0),
            Operator::Add,
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 maximum fits"),
        ),
        (
            "equal narrow integer division keeps I8",
            fixed_integer_expression(FixedScalar::I8, -128),
            fixed_integer_expression(FixedScalar::I8, 2),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I8, -64).expect("-64 fits I8"),
        ),
        (
            "explicit I32 integer division retains the former promoted quotient",
            fixed_integer_expression(FixedScalar::I32, -128),
            fixed_integer_expression(FixedScalar::I32, -1),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I32, 128).expect("128 fits I32"),
        ),
        (
            "signed minimum remainder by negative one is zero",
            fixed_integer_expression(FixedScalar::I64, i128::from(i64::MIN)),
            fixed_integer_expression(FixedScalar::I64, -1),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I64, 0).expect("zero fits I64"),
        ),
        (
            "negative integer division truncates toward zero",
            fixed_integer_expression(FixedScalar::I32, -7),
            fixed_integer_expression(FixedScalar::I32, 2),
            Operator::IntDivide,
            FixedScalarValue::signed(FixedScalar::I32, -3).expect("-3 fits I32"),
        ),
        (
            "remainder follows the dividend sign",
            fixed_integer_expression(FixedScalar::I32, -7),
            fixed_integer_expression(FixedScalar::I32, 2),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I32, -1).expect("-1 fits I32"),
        ),
        (
            "negative divisor keeps positive dividend remainder",
            fixed_integer_expression(FixedScalar::I32, 7),
            fixed_integer_expression(FixedScalar::I32, -2),
            Operator::Modulus,
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
        ),
        (
            "U8 exponentiation keeps U8",
            fixed_integer_expression(FixedScalar::U8, 2),
            fixed_integer_expression(FixedScalar::U8, 7),
            Operator::Exponent,
            FixedScalarValue::unsigned(FixedScalar::U8, 128).expect("2^7 fits U8"),
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        let folded =
            expect_folded_operator(lhs.evaluate_operator(&rhs, &operator, &mut string_table, None));
        assert_eq!(
            expect_fixed_scalar(folded),
            expected,
            "{case} must fold in the operand-derived domain"
        );
    }

    // These exact inputs folded through a wider carrier before D15 (`U8 + U8` into U32,
    // `I8::MIN // -1` into I32). They must now fail at the operand width, so a carrier-width
    // overflow check cannot accept them. The wider 510/128 calculations are retained above
    // with explicit U32/I32 operands.
    for (case, lhs, rhs, operator, operation) in [
        (
            "U8 255 + 255 overflows U8",
            fixed_integer_expression(FixedScalar::U8, 255),
            fixed_integer_expression(FixedScalar::U8, 255),
            Operator::Add,
            "+",
        ),
        (
            "I8 minimum divided by -1 overflows I8",
            fixed_integer_expression(FixedScalar::I8, -128),
            fixed_integer_expression(FixedScalar::I8, -1),
            Operator::IntDivide,
            "//",
        ),
    ] {
        let error = match lhs.evaluate_operator(&rhs, &operator, &mut string_table, None) {
            Err(error) => error,
            Ok(_) => panic!("{case} should fail during folding"),
        };
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
            Some(operation),
            &string_table,
        );
    }
}

#[test]
fn evaluate_operator_folds_fixed_integer_division_to_f64() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let cases = [
        (
            "U8 division",
            fixed_integer_expression(FixedScalar::U8, 7),
            fixed_integer_expression(FixedScalar::U8, 2),
            fixed_float_value(FixedScalar::F64, 3.5),
        ),
        (
            "I64 maximum division rounds each operand to binary64",
            fixed_integer_expression(FixedScalar::I64, i128::from(i64::MAX)),
            fixed_integer_expression(FixedScalar::I64, 1),
            fixed_float_value(FixedScalar::F64, 9_223_372_036_854_775_808.0),
        ),
    ];

    for (case, lhs, rhs, expected) in cases {
        let folded = expect_folded_operator(lhs.evaluate_operator(
            &rhs,
            &Operator::Divide,
            &mut string_table,
            None,
        ));
        assert_eq!(expect_fixed_scalar(folded), expected, "{case}");
    }
}

#[test]
fn evaluate_operator_handles_huge_fixed_integer_exponents_without_host_overflow() {
    let _path_fork = PathInternerFork::empty();
    let successes = [
        (
            "every integer raised to zero is one",
            fixed_integer_expression(FixedScalar::U64, 2),
            fixed_integer_expression(FixedScalar::U64, 0),
            FixedScalarValue::unsigned(FixedScalar::U64, 1).expect("1 fits U64"),
        ),
        (
            "one raised to a huge exponent stays one",
            fixed_integer_expression(FixedScalar::U64, 1),
            fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX)),
            FixedScalarValue::unsigned(FixedScalar::U64, 1).expect("1 fits U64"),
        ),
    ];

    for (case, lhs, rhs, expected) in successes {
        let mut string_table = StringTable::new();
        let folded = expect_folded_operator(lhs.evaluate_operator(
            &rhs,
            &Operator::Exponent,
            &mut string_table,
            None,
        ));
        assert_eq!(expect_fixed_scalar(folded), expected, "{case}");
    }

    let mut string_table = StringTable::new();
    let error = fixed_integer_expression(FixedScalar::U64, 2)
        .evaluate_operator(
            &fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX)),
            &Operator::Exponent,
            &mut string_table,
            None,
        )
        .expect_err("a huge non-trivial power should overflow without a host-sized loop");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );
}

#[test]
fn evaluate_operator_reports_fixed_integer_overflow_and_invalid_exponents() {
    let _path_fork = PathInternerFork::empty();
    let cases = [
        (
            "U32 multiplication overflow",
            fixed_integer_expression(FixedScalar::U32, i128::from(u32::MAX)),
            fixed_integer_expression(FixedScalar::U32, 2),
            Operator::Multiply,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "I64 minimum quotient overflow",
            fixed_integer_expression(FixedScalar::I64, i128::from(i64::MIN)),
            fixed_integer_expression(FixedScalar::I64, -1),
            Operator::IntDivide,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "I32 minimum quotient overflows its own domain",
            fixed_integer_expression(FixedScalar::I32, i128::from(i32::MIN)),
            fixed_integer_expression(FixedScalar::I32, -1),
            Operator::IntDivide,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "U32 exponent overflow",
            fixed_integer_expression(FixedScalar::U8, 2),
            fixed_integer_expression(FixedScalar::U8, 32),
            Operator::Exponent,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
        ),
        (
            "negative fixed integer exponent",
            fixed_integer_expression(FixedScalar::I32, 2),
            fixed_integer_expression(FixedScalar::I32, -1),
            Operator::Exponent,
            CompileTimeEvaluationErrorReason::InvalidExponent,
        ),
    ];

    for (case, lhs, rhs, operator, reason) in cases {
        let mut string_table = StringTable::new();
        let error = match lhs.evaluate_operator(&rhs, &operator, &mut string_table, None) {
            Err(error) => error,
            Ok(_) => panic!("{case} should fail during folding"),
        };
        assert_compile_time_error(&error, reason, Some(operator.to_str()), &string_table);
    }
}

#[test]
fn evaluate_operator_reports_fixed_integer_and_float_zero_divisors() {
    let _path_fork = PathInternerFork::empty();
    let cases = [
        (
            "fixed integer division",
            fixed_integer_expression(FixedScalar::I32, 5),
            fixed_integer_expression(FixedScalar::I32, 0),
            Operator::IntDivide,
        ),
        (
            "fixed integer remainder",
            fixed_integer_expression(FixedScalar::U8, 5),
            fixed_integer_expression(FixedScalar::U8, 0),
            Operator::Modulus,
        ),
        (
            "fixed integer regular division",
            fixed_integer_expression(FixedScalar::U8, 5),
            fixed_integer_expression(FixedScalar::U8, 0),
            Operator::Divide,
        ),
        (
            "fixed float regular division",
            fixed_float_expression(FixedScalar::F32, 5.0),
            fixed_float_expression(FixedScalar::F32, 0.0),
            Operator::Divide,
        ),
        (
            "fixed float remainder",
            fixed_float_expression(FixedScalar::F16, 5.0),
            fixed_float_expression(FixedScalar::F16, -0.0),
            Operator::Modulus,
        ),
    ];

    for (case, lhs, rhs, operator) in cases {
        let mut string_table = StringTable::new();
        let error = match lhs.evaluate_operator(&rhs, &operator, &mut string_table, None) {
            Err(error) => error,
            Ok(_) => panic!("{case} should fail during folding"),
        };
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::DivideByZero,
            None,
            &string_table,
        );
    }
}

#[test]
fn evaluate_operator_rounds_fixed_float_results_once_in_the_operand_domain() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let half = fixed_float_value(FixedScalar::F16, 0.1);
    let half_product = fixed_float_value(
        FixedScalar::F16,
        half.as_f64().expect("F16 has a float value")
            * half.as_f64().expect("F16 has a float value"),
    );
    let f32_value = fixed_float_value(FixedScalar::F32, 0.1);
    let f64_value = fixed_float_value(FixedScalar::F64, 0.2);
    let cases = [
        (
            "F16 product rounds once to F16",
            Expression::fixed_scalar(half, None, ValueMode::ImmutableOwned),
            Expression::fixed_scalar(half, None, ValueMode::ImmutableOwned),
            Operator::Multiply,
            half_product,
        ),
        (
            "F32 and F64 promote to F64",
            Expression::fixed_scalar(f32_value, None, ValueMode::ImmutableOwned),
            Expression::fixed_scalar(f64_value, None, ValueMode::ImmutableOwned),
            Operator::Add,
            fixed_float_value(
                FixedScalar::F64,
                f32_value.as_f64().expect("F32 has a float value")
                    + f64_value.as_f64().expect("F64 has a float value"),
            ),
        ),
        (
            "F16 and F16 keep F16",
            fixed_float_expression(FixedScalar::F16, 0.5),
            fixed_float_expression(FixedScalar::F16, 0.5),
            Operator::Add,
            fixed_float_value(FixedScalar::F16, 1.0),
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        let folded =
            expect_folded_operator(lhs.evaluate_operator(&rhs, &operator, &mut string_table, None));
        assert_eq!(
            expect_fixed_scalar(folded),
            expected,
            "{case} must retain the semantic precision"
        );
    }

    // B20: F16 completes every operation before the next one starts. The ulp at 2048 is 2, so
    // `2048 + 1` is a binary16 midpoint whose ties-to-even result is 2048, and the following
    // subtraction then cancels the chain to zero. A carrier that postponed the F16 rounding
    // until after the chain would keep 2049 and end at 1 instead.
    let intermediate = expect_folded_stack(constant_fold(
        vec![
            rvalue_item(fixed_float_expression(FixedScalar::F16, 2048.0)),
            rvalue_item(fixed_float_expression(FixedScalar::F16, 1.0)),
            operator_item(Operator::Add),
        ],
        &mut string_table,
        None,
    ));
    let [ExpressionRpnItem::Operand(intermediate)] = intermediate.as_slice() else {
        panic!("F16 `2048 + 1` should fold to one operand");
    };
    let intermediate_value = match &intermediate.kind {
        ExpressionKind::FixedScalar(value) => *value,
        other => panic!("F16 `2048 + 1` should fold to a fixed scalar, got {other:?}"),
    };
    assert_eq!(intermediate_value.scalar(), FixedScalar::F16);
    assert_eq!(
        intermediate_value
            .as_f64()
            .expect("F16 has a float value")
            .to_bits(),
        2048.0f64.to_bits(),
        "F16 `2048 + 1` must round to 2048 before the next operation"
    );

    let chained = expect_folded_stack(constant_fold(
        vec![
            rvalue_item(fixed_float_expression(FixedScalar::F16, 2048.0)),
            rvalue_item(fixed_float_expression(FixedScalar::F16, 1.0)),
            operator_item(Operator::Add),
            rvalue_item(fixed_float_expression(FixedScalar::F16, 2048.0)),
            operator_item(Operator::Subtract),
        ],
        &mut string_table,
        None,
    ));
    let [ExpressionRpnItem::Operand(chained)] = chained.as_slice() else {
        panic!("chained F16 fold should reduce to one operand");
    };
    let chained_value = match &chained.kind {
        ExpressionKind::FixedScalar(value) => *value,
        other => panic!("chained F16 fold should fold to a fixed scalar, got {other:?}"),
    };
    assert_eq!(chained_value.scalar(), FixedScalar::F16);
    assert_eq!(
        chained_value
            .as_f64()
            .expect("F16 has a float value")
            .to_bits(),
        0.0f64.to_bits(),
        "chained F16 `(2048 + 1) - 2048` must cancel to zero"
    );
}

#[test]
fn evaluate_operator_rounds_fixed_f32_arithmetic_to_f32_bits() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let lhs = fixed_float_expression(FixedScalar::F32, 0.1);
    let rhs = fixed_float_expression(FixedScalar::F32, 0.2);
    let expected_bits = f64::from(0.1f32 + 0.2f32).to_bits();

    let folded = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    let actual = expect_fixed_scalar(folded);
    assert_eq!(actual.scalar(), FixedScalar::F32);
    assert_eq!(
        actual
            .as_f64()
            .expect("F32 result has a float value")
            .to_bits(),
        expected_bits,
        "F32 addition must match independently computed F32 rounding"
    );
}

#[test]
fn evaluate_operator_preserves_signed_zero_result_bits() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let lhs = fixed_float_expression(FixedScalar::F32, -0.0);
    let rhs = fixed_float_expression(FixedScalar::F32, 1.0);

    let folded = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Multiply,
        &mut string_table,
        None,
    ));
    let actual = expect_fixed_scalar(folded);
    assert_eq!(actual.scalar(), FixedScalar::F32);
    assert_eq!(
        actual
            .as_f64()
            .expect("F32 result has a float value")
            .to_bits(),
        f64::from(-0.0f32).to_bits(),
        "negative zero must retain its sign bit through fixed-float folding"
    );
}

#[test]
fn evaluate_operator_rejects_fixed_float_overflow() {
    let _path_fork = PathInternerFork::empty();
    let cases = [
        (
            "F32 multiplication overflow",
            fixed_float_expression(FixedScalar::F32, 3.0e38),
            fixed_float_expression(FixedScalar::F32, 10.0),
            Operator::Multiply,
            "*",
        ),
        (
            "F16 maximum plus itself overflows finite F16",
            fixed_float_expression(FixedScalar::F16, 65504.0),
            fixed_float_expression(FixedScalar::F16, 65504.0),
            Operator::Add,
            "+",
        ),
    ];

    for (case, lhs, rhs, operator, operation) in cases {
        let mut string_table = StringTable::new();
        let error = match lhs.evaluate_operator(&rhs, &operator, &mut string_table, None) {
            Err(error) => error,
            Ok(_) => panic!("{case} should fail during folding"),
        };
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::FloatOverflow,
            Some(operation),
            &string_table,
        );
    }
}

#[test]
fn constant_fold_uses_fixed_integer_and_float_negation_domains() {
    let _path_fork = PathInternerFork::empty();
    let successes = [
        (
            "I8 negates in I8",
            fixed_integer_expression(FixedScalar::I8, 127),
            FixedScalarValue::signed(FixedScalar::I8, -127).expect("-127 fits I8"),
        ),
        (
            "F16 negates in F16",
            fixed_float_expression(FixedScalar::F16, 0.5),
            fixed_float_value(FixedScalar::F16, -0.5),
        ),
    ];

    for (case, operand, expected) in successes {
        let mut string_table = StringTable::new();
        let folded = expect_folded_stack(constant_fold(
            vec![rvalue_item(operand), operator_item(Operator::Negate)],
            &mut string_table,
            None,
        ));
        let [ExpressionRpnItem::Operand(folded)] = folded.as_slice() else {
            panic!("{case} should produce one folded operand");
        };
        let actual = match &folded.kind {
            ExpressionKind::FixedScalar(value) => *value,
            other => panic!("{case} should return a fixed scalar, got {other:?}"),
        };
        assert_eq!(actual, expected, "{case}");
    }

    // Negating a signed minimum fails in the operand's own type, including the narrow I8.
    for (case, operand) in [
        (
            "negating I64 minimum should overflow I64",
            fixed_integer_expression(FixedScalar::I64, i128::from(i64::MIN)),
        ),
        (
            "negating I8 minimum should overflow I8",
            fixed_integer_expression(FixedScalar::I8, -128),
        ),
    ] {
        let mut string_table = StringTable::new();
        let error = constant_fold(
            vec![rvalue_item(operand), operator_item(Operator::Negate)],
            &mut string_table,
            None,
        )
        .expect_err(case);
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::IntegerOverflow,
            Some("-"),
            &string_table,
        );
    }
}

#[test]
fn evaluate_operator_compares_fixed_scalars_without_losing_value_identity() {
    let _path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let cases = [
        (
            "negative signed value orders below U64 maximum",
            fixed_integer_expression(FixedScalar::I64, -1),
            fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX)),
            Operator::LessThan,
            true,
        ),
        (
            "U64 maximum is not I64 maximum",
            fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX)),
            fixed_integer_expression(FixedScalar::I64, i128::from(i64::MAX)),
            Operator::Equality,
            false,
        ),
        (
            "adjacent integers above f64 exact precision remain ordered",
            fixed_integer_expression(FixedScalar::U64, 9_007_199_254_740_993),
            fixed_integer_expression(FixedScalar::I64, 9_007_199_254_740_992),
            Operator::GreaterThan,
            true,
        ),
        (
            "positive and negative float zero compare equal",
            fixed_float_expression(FixedScalar::F16, 0.0),
            fixed_float_expression(FixedScalar::F64, -0.0),
            Operator::Equality,
            true,
        ),
        (
            "Byte ordering is unsigned",
            Expression::fixed_scalar(
                FixedScalarValue::unsigned(FixedScalar::Byte, 200).expect("200 fits Byte"),
                None,
                ValueMode::ImmutableOwned,
            ),
            Expression::fixed_scalar(
                FixedScalarValue::unsigned(FixedScalar::Byte, 100).expect("100 fits Byte"),
                None,
                ValueMode::ImmutableOwned,
            ),
            Operator::GreaterThan,
            true,
        ),
    ];

    for (case, lhs, rhs, operator, expected) in cases {
        assert_eq!(
            expect_folded_boolean(lhs, rhs, operator, &mut string_table),
            expected,
            "{case}"
        );
    }
}

#[test]
fn evaluate_operator_rejects_mixed_fixed_integer_and_float_operands() {
    // D15: fixed integers and binary floats require an explicit conversion to mix, so the
    // operand pair reports the invalid-operator diagnostic instead of folding.
    let cases = [
        (
            fixed_integer_expression(FixedScalar::I32, 2),
            fixed_float_expression(FixedScalar::F64, 1.5),
            Operator::Add,
        ),
        (
            fixed_integer_expression(FixedScalar::I32, 5),
            fixed_float_expression(FixedScalar::F64, 2.0),
            Operator::Divide,
        ),
        (
            fixed_integer_expression(FixedScalar::I32, 16_777_217),
            fixed_float_expression(FixedScalar::F32, 0.5),
            Operator::Add,
        ),
    ];

    for (lhs, rhs, operator) in cases {
        let mut string_table = StringTable::new();
        let error = lhs
            .evaluate_operator(&rhs, &operator, &mut string_table, None)
            .expect_err("mixed fixed integer and binary float operands cannot fold");
        assert_compile_time_error(
            &error,
            CompileTimeEvaluationErrorReason::InvalidOperatorForType,
            Some(operator.to_str()),
            &string_table,
        );
    }
}

#[test]
fn evaluate_operator_folds_number_arithmetic_once_and_retains_canonical_type() {
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let scale = NumberScale::new(1).expect("test scale is valid");
    let number_type_id = type_environment.intern_number(scale);
    let mut string_table = StringTable::new();

    let lhs = number_expression("1.5", NumericLiteralSign::Positive, 1, number_type_id);
    let rhs = number_expression("0.1", NumericLiteralSign::Positive, 1, number_type_id);
    let product = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Multiply,
        &mut string_table,
        None,
    ));
    assert_eq!(product.type_id, number_type_id);
    assert_eq!(product.diagnostic_type, DataType::Number(scale));
    assert!(matches!(
        &product.kind,
        ExpressionKind::Number(value) if value.coefficient().to_string() == "2"
    ));

    let whole = fixed_integer_expression(FixedScalar::U64, 2);
    let fraction = number_expression("0.5", NumericLiteralSign::Positive, 1, number_type_id);
    let mixed_sum = expect_folded_operator(whole.evaluate_operator(
        &fraction,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert_eq!(mixed_sum.type_id, number_type_id);
    assert_eq!(mixed_sum.diagnostic_type, DataType::Number(scale));
    assert!(matches!(
        &mixed_sum.kind,
        ExpressionKind::Number(value) if value.coefficient().to_string() == "25"
    ));

    let base = number_expression("1.5", NumericLiteralSign::Positive, 1, number_type_id);
    let exponent = fixed_integer_expression(FixedScalar::I32, 3);
    let power = expect_folded_operator(base.evaluate_operator(
        &exponent,
        &Operator::Exponent,
        &mut string_table,
        None,
    ));
    assert_eq!(power.type_id, number_type_id);
    assert!(matches!(
        &power.kind,
        ExpressionKind::Number(value) if value.coefficient().to_string() == "34"
    ));

    let negated = expect_folded_stack(constant_fold(
        vec![
            rvalue_item(number_expression(
                "1.2",
                NumericLiteralSign::Positive,
                1,
                number_type_id,
            )),
            operator_item(Operator::Negate),
        ],
        &mut string_table,
        None,
    ));
    let [ExpressionRpnItem::Operand(negated)] = negated.as_slice() else {
        panic!("Dec negation should fold to one value");
    };
    assert_eq!(negated.type_id, number_type_id);
    assert!(matches!(
        &negated.kind,
        ExpressionKind::Number(value) if value.coefficient().to_string() == "-12"
    ));
}

#[test]
fn number_folding_reports_checked_division_and_exponent_failures() {
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let scale = NumberScale::new(1).expect("test scale is valid");
    let number_type_id = type_environment.intern_number(scale);
    let mut string_table = StringTable::new();

    let lhs = number_expression("1.0", NumericLiteralSign::Positive, 1, number_type_id);
    let zero = number_expression("0.0", NumericLiteralSign::Positive, 1, number_type_id);
    let error = lhs
        .evaluate_operator(&zero, &Operator::Divide, &mut string_table, None)
        .expect_err("a known Dec zero divisor must be a compile-time failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );

    let error = lhs
        .evaluate_operator(&zero, &Operator::Modulus, &mut string_table, None)
        .expect_err("a known Dec remainder-by-zero must be a compile-time failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );

    let base = number_expression("1.5", NumericLiteralSign::Positive, 1, number_type_id);
    let negative_exponent = fixed_integer_expression(FixedScalar::I32, -1);
    let error = base
        .evaluate_operator(
            &negative_exponent,
            &Operator::Exponent,
            &mut string_table,
            None,
        )
        .expect_err("a negative Dec exponent must use the checked exponent failure");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some("^"),
        &string_table,
    );
}

#[test]
fn mixed_number_integer_comparisons_preserve_values_beyond_float_precision() {
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let number_type_id = type_environment.intern_number(NumberScale::ZERO);
    let mut string_table = StringTable::new();

    let number = number_expression(
        "9007199254740992",
        NumericLiteralSign::Positive,
        0,
        number_type_id,
    );
    let integer = fixed_integer_expression(FixedScalar::I64, 9_007_199_254_740_993);
    assert!(expect_folded_boolean(
        number,
        integer,
        Operator::LessThan,
        &mut string_table
    ));

    let number = number_expression(
        "9007199254740992",
        NumericLiteralSign::Positive,
        0,
        number_type_id,
    );
    let integer = fixed_integer_expression(FixedScalar::I64, 9_007_199_254_740_993);
    assert!(expect_folded_boolean(
        integer,
        number,
        Operator::GreaterThan,
        &mut string_table
    ));

    let number = number_expression(
        "18446744073709551616",
        NumericLiteralSign::Positive,
        0,
        number_type_id,
    );
    let maximum_u64 = fixed_integer_expression(FixedScalar::U64, i128::from(u64::MAX));
    assert!(expect_folded_boolean(
        number,
        maximum_u64,
        Operator::GreaterThan,
        &mut string_table
    ));
}

#[test]
fn evaluate_operator_rounds_float_add_at_the_operand_domain_precision() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();

    // F32 operands fold at F32 precision: the sum of the widened carriers rounds once.
    let f32_lhs = fixed_float_expression(FixedScalar::F32, f64::from(0.1f32));
    let f32_rhs = fixed_float_expression(FixedScalar::F32, f64::from(0.2f32));
    let folded = expect_folded_operator(f32_lhs.evaluate_operator(
        &f32_rhs,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_f64() == Some(f64::from(0.1f32 + 0.2f32))
    ));
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F32)
    );

    // The same carrier values under the F64 domain keep the exact f64 sum.
    let f64_lhs = fixed_float_expression(FixedScalar::F64, f64::from(0.1f32));
    let f64_rhs = fixed_float_expression(FixedScalar::F64, f64::from(0.2f32));
    let folded = expect_folded_operator(f64_lhs.evaluate_operator(
        &f64_rhs,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_f64() == Some(f64::from(0.1f32) + f64::from(0.2f32))
    ));

    // Digits materialised at F64 fold to the plain f64 sum.
    let f64_lhs = fixed_float_expression(FixedScalar::F64, 0.1_f64);
    let f64_rhs = fixed_float_expression(FixedScalar::F64, 0.2_f64);
    let folded = expect_folded_operator(f64_lhs.evaluate_operator(
        &f64_rhs,
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_f64() == Some(0.1 + 0.2)
    ));
}

#[test]
fn evaluate_operator_folds_int_division_quotient_in_f64() {
    // Integer real division computes in F64: both operands convert exactly, and the
    // quotient rounds once at binary64 precision.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let lhs = fixed_integer_expression(FixedScalar::I32, 16_777_217);
    let rhs = fixed_integer_expression(FixedScalar::I32, 10);
    let folded = expect_folded_operator(lhs.evaluate_operator(
        &rhs,
        &Operator::Divide,
        &mut string_table,
        None,
    ));
    assert!(
        matches!(folded.kind, ExpressionKind::FixedScalar(value) if value.as_f64() == Some(1_677_721.7)),
        "I32 / I32 must fold to the exact f64 quotient, got {:?}",
        folded.kind
    );
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
}

#[test]
fn constant_fold_reports_static_failure_at_authored_operator_inside_runtime_expression() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let runtime_var = Expression::reference(
        path_fork
            .try_intern_portable_path("runtime_var", &mut string_table)
            .expect("test path fits"),
        DataType::FixedScalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );
    let one = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
        Some(marked_span(100)),
        ValueMode::ImmutableOwned,
    );
    let zero = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 0).expect("0 fits I32"),
        Some(marked_span(140)),
        ValueMode::ImmutableOwned,
    );
    let operator_span = marked_span(230);

    let nodes = vec![
        rvalue_item(runtime_var),
        rvalue_item(one),
        rvalue_item(zero),
        ExpressionRpnItem::Operator {
            operator: Operator::Divide,
            span: Some(operator_span),
        },
        operator_item(Operator::Add),
    ];

    let error = constant_fold(nodes, &mut string_table, None)
        .expect_err("divide by zero inside a runtime expression should still be diagnosed");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );
    let ConstantFoldError::Diagnostic(diagnostic) = error else {
        panic!("divide by zero should remain a source diagnostic");
    };
    assert_eq!(diagnostic.primary_span, Some(operator_span));
}

#[test]
fn constant_fold_partially_folds_runtime_expression() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let runtime_var = Expression::reference(
        path_fork
            .try_intern_portable_path("runtime_var", &mut string_table)
            .expect("test path fits"),
        DataType::FixedScalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );
    let two = fixed_integer_expression(FixedScalar::I32, 2);
    let three = fixed_integer_expression(FixedScalar::I32, 3);

    let nodes = vec![
        rvalue_item(runtime_var),
        rvalue_item(two),
        rvalue_item(three),
        operator_item(Operator::Add),
        operator_item(Operator::Multiply),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table, None));

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
            kind: ExpressionKind::FixedScalar(value),
            ..
        }) if value.as_i64() == Some(5)
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

    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, 9_223_372_036_854_775_808.0)
            .expect("2^63 is a finite F64"),
    );
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::NumericConversion {
            source: NumericScalar::Fixed(FixedScalar::F64),
            target: NumericScalar::Fixed(FixedScalar::I64),
        },
        &source,
    )
    .expect_err("out-of-range float to I64 cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatCastToIntOutOfRange);
}

#[test]
fn fold_int_cast_truncates_toward_zero() {
    use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
    use crate::compiler_frontend::builtins::casts::{
        BuiltinCastLiteral, apply_builtin_cast_policy,
    };

    let policy = BuiltinCastPolicyId::NumericConversion {
        source: NumericScalar::Fixed(FixedScalar::F64),
        target: NumericScalar::Fixed(FixedScalar::I32),
    };
    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, 1.9).expect("1.9 is a finite F64"),
    );
    let result = apply_builtin_cast_policy(policy, &source).expect("float to int cast should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32")
        )
    );

    let source = BuiltinCastLiteral::Fixed(
        FixedScalarValue::binary_float(FixedScalar::F64, -1.9).expect("-1.9 is a finite F64"),
    );
    let result =
        apply_builtin_cast_policy(policy, &source).expect("negative float to int cast should fold");
    assert_eq!(
        result,
        BuiltinCastLiteral::Fixed(
            FixedScalarValue::signed(FixedScalar::I32, -1).expect("-1 fits I32")
        )
    );
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
    let error = apply_builtin_cast_policy(
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::F64)),
        &source,
    )
    .expect_err("non-finite float string cast should fail");
    assert_eq!(error.code, BuiltinErrorCode::FloatParseOutOfRange);
}

#[test]
fn fold_string_to_int_cast_uses_string_policy_row() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("42".to_string());
    let source = Expression::string_slice(text, Default::default(), ValueMode::ImmutableOwned);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::Fixed(FixedScalar::I32),
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect("valid string to int cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(42)
    ));
}

#[test]
fn fold_string_to_float_cast_uses_string_policy_row() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("3.5e2".to_string());
    let source = Expression::string_slice(text, Default::default(), ValueMode::ImmutableOwned);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::F64);

    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::F64));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::Fixed(FixedScalar::F64),
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect("valid string to float cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_f64() == Some(350.0)
    ));
}

fn rvalue_item(expression: Expression) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

fn operator_item(operator: Operator) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator {
        operator,
        span: None,
    }
}

#[test]
fn constant_fold_reports_escaped_pending_items_as_infrastructure() {
    let mut string_table = StringTable::new();
    let literal_text = string_table.intern("1");
    let pending_literal = ExpressionRpnItem::PendingNumericLiteral {
        token: crate::compiler_frontend::numeric_text::token::NumericLiteralToken::new(
            NumericLiteralSign::Positive,
            literal_text,
            literal_text,
            moth_lexical::numeric::grammar::NumericLiteralKind::WholeNumber,
            1,
            0,
            0,
            moth_lexical::numeric::grammar::NumericExponentSign::None,
        ),
        span: None,
        value_mode: ValueMode::ImmutableOwned,
    };
    let pending_group = ExpressionRpnItem::PendingGroup {
        nodes: Vec::new(),
        span: None,
    };
    let mut path_fork = PathInternerFork::empty();
    let pending_binding = pending_binding_item(&mut string_table, &mut path_fork);
    let pending_cast = pending_cast_item();
    let pending_call = pending_call_item(&mut string_table, &mut path_fork);

    for pending in [
        pending_literal,
        pending_group,
        pending_binding,
        pending_cast,
        pending_call,
    ] {
        // A foldable peer and operator around the pending item would fold or preserve it if the
        // guard were removed, so only the infrastructure lane satisfies this assertion.
        let nodes = vec![
            rvalue_item(fixed_integer_expression(FixedScalar::I32, 2)),
            pending,
            operator_item(Operator::Add),
        ];
        match constant_fold(nodes, &mut string_table, None) {
            Err(ConstantFoldError::Infrastructure(error)) => assert!(
                error
                    .msg
                    .contains("evaluate_expression must resolve it first"),
                "unexpected invariant message: {error:?}"
            ),
            other => panic!("escaped pending syntax must be an infrastructure error: {other:?}"),
        }
    }
}

/// One construction-only pending binding item for escaped-pending guard coverage.
///
/// WHAT: mints a real `NumericBindingId` through the construction owner instead of fabricating
///       an ID, so the guard test exercises the exact carrier body parsing builds.
fn pending_binding_item(
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ExpressionRpnItem {
    use crate::compiler_frontend::ast::module_ast::scope_context::ScopeArena;
    use crate::compiler_frontend::ast::numeric_construction::{
        NumericConstructionState, NumericFamily, NumericRecipe, PendingBindingRow,
    };

    let name = string_table.intern("pending_binding");
    let path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits")
        .to_owned();
    let mut state = NumericConstructionState::new();
    let class = state.create_class().expect("test class allocation fits");
    let frame = ScopeArena::new().alloc_root_frame_with_capacity(0);
    let binding = state
        .register_binding(PendingBindingRow {
            name,
            path,
            span: None,
            binding_span: None,
            value_mode: ValueMode::ImmutableOwned,
            mutable: false,
            class,
            recipe: Some(NumericRecipe::Raw {
                items: Vec::new(),
                family: NumericFamily::Whole,
            }),
            origin: Default::default(),
            frame,
        })
        .expect("test binding allocation fits");

    ExpressionRpnItem::PendingNumericBinding {
        binding,
        span: None,
        value_mode: ValueMode::ImmutableOwned,
        is_copy: false,
    }
}

/// One construction-only pending cast item for escaped-pending guard coverage.
fn pending_cast_item() -> ExpressionRpnItem {
    ExpressionRpnItem::PendingNumericCast {
        target_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        target: BuiltinCastTarget::Fixed(FixedScalar::I32),
        requires_optional_wrap_after_cast: false,
        handling: CastHandling::Implicit,
        source: Vec::new(),
        span: None,
    }
}

/// One construction-only pending call item for escaped-pending guard coverage.
fn pending_call_item(
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ExpressionRpnItem {
    use crate::compiler_frontend::ast::numeric_construction::{
        DeferredCall, DeferredCallHandling, DeferredSourceCall,
    };
    use crate::compiler_frontend::ast::statements::functions::FunctionSignature;

    let name = string_table.intern("pending_call");
    let function_path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits")
        .to_owned();
    let call = DeferredCall::Source(Box::new(DeferredSourceCall {
        function_path,
        signature: FunctionSignature {
            parameters: Vec::new(),
            returns: Vec::new(),
        },
        arguments: Vec::new(),
        call_span: None,
        authored_handling: DeferredCallHandling::Unhandled,
    }));

    ExpressionRpnItem::PendingNumericCall {
        call: Box::new(call),
        span: None,
    }
}

#[test]
fn constant_fold_folds_comparison_then_boolean_chain() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let nodes = vec![
        rvalue_item(fixed_integer_expression(FixedScalar::I32, 1)),
        rvalue_item(fixed_integer_expression(FixedScalar::I32, 2)),
        operator_item(Operator::LessThan),
        rvalue_item(Expression::bool(true, None, ValueMode::ImmutableOwned)),
        operator_item(Operator::And),
    ];

    let folded = expect_folded_stack(constant_fold(nodes, &mut string_table, None));
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
    let _path_fork = PathInternerFork::empty();
    let nodes = vec![
        rvalue_item(fixed_integer_expression(FixedScalar::I32, 1)),
        operator_item(Operator::Not),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table, None));
    assert_eq!(folded.len(), 2);
    assert!(matches!(
        folded[0],
        ExpressionRpnItem::Operand(Expression {
            kind: ExpressionKind::FixedScalar(value),
            ..
        }) if value.as_i64() == Some(1)
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
    let mut path_fork = PathInternerFork::empty();
    let flag_name = path_fork
        .try_intern_portable_path("flag", &mut string_table)
        .expect("test path fits");
    let nodes = vec![
        rvalue_item(Expression::reference(
            flag_name,
            DataType::Bool,
            None,
            ValueMode::ImmutableReference,
        )),
        rvalue_item(Expression::bool(true, None, ValueMode::ImmutableOwned)),
        operator_item(Operator::And),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table, None));

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

/// Build a source span that is distinguishable from every other one in a test.
fn marked_span(start: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::from_index(7),
        LocalSpan::exact(start, 4, &mut span_builder).expect("provenance span should fit"),
    )
}

#[test]
fn partial_fold_moves_non_foldable_operands_back_without_rebuilding_them() {
    // Folding consumes its input, so a moved-back operand could silently become a
    // reconstruction. Distinct spans and value modes on every input make that visible:
    // a rebuilt operand would carry defaults, not the values asserted below.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let flag_name = path_fork
        .try_intern_portable_path("flag", &mut string_table)
        .expect("test path fits");
    let flag_span = marked_span(70);
    let literal_span = marked_span(110);
    let operator_span = marked_span(230);

    let nodes = vec![
        rvalue_item(Expression::reference(
            flag_name,
            DataType::Bool,
            Some(flag_span),
            ValueMode::MutableReference,
        )),
        rvalue_item(Expression::bool(
            true,
            Some(literal_span),
            ValueMode::ImmutableOwned,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            span: Some(operator_span),
        },
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table, None));

    assert_eq!(folded.len(), 3);

    let ExpressionRpnItem::Operand(runtime_operand) = &folded[0] else {
        panic!("the runtime reference should stay an operand");
    };
    assert_eq!(runtime_operand.span, Some(flag_span));
    assert_eq!(runtime_operand.value_mode, ValueMode::MutableReference);

    let ExpressionRpnItem::Operand(literal_operand) = &folded[1] else {
        panic!("the literal should stay an operand");
    };
    assert_eq!(literal_operand.span, Some(literal_span));
    assert_eq!(literal_operand.value_mode, ValueMode::ImmutableOwned);

    let ExpressionRpnItem::Operator { operator, span } = &folded[2] else {
        panic!("the unfoldable operator should be preserved");
    };
    assert_eq!(*operator, Operator::And);
    assert_eq!(*span, Some(operator_span));
}

#[test]
fn partial_fold_keeps_the_folded_half_and_the_moved_half_distinct() {
    // A fold that reduces only part of the expression must move the untouched operands back in
    // their original order while the folded operand takes its own provenance from the fold.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let counter_name = path_fork
        .try_intern_portable_path("counter", &mut string_table)
        .expect("test path fits");
    let counter_span = marked_span(30);
    let left_literal_span = marked_span(50);

    let nodes = vec![
        rvalue_item(Expression::reference(
            counter_name,
            DataType::FixedScalar(FixedScalar::I32),
            Some(counter_span),
            ValueMode::ImmutableReference,
        )),
        rvalue_item(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 fits I32"),
            Some(left_literal_span),
            ValueMode::ImmutableOwned,
        )),
        rvalue_item(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 3).expect("3 fits I32"),
            Some(marked_span(60)),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Add),
        operator_item(Operator::Multiply),
    ];

    let folded = expect_not_constant_stack(constant_fold(nodes, &mut string_table, None));

    assert_eq!(folded.len(), 3);

    let ExpressionRpnItem::Operand(moved) = &folded[0] else {
        panic!("the runtime reference should stay an operand");
    };
    assert_eq!(moved.span, Some(counter_span));

    let ExpressionRpnItem::Operand(computed) = &folded[1] else {
        panic!("the constant half should fold to one operand");
    };
    assert!(matches!(
        computed.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(5)
    ));
    // The folded operand inherits the left operand's anchor, so the reduction stays
    // attributable to authored source rather than to a synthesized position.
    assert_eq!(computed.span, Some(left_literal_span));
}

#[test]
fn full_fold_returns_the_folded_operand_with_its_source_anchor() {
    // The single-result path hands the folded operand back by move. Its anchor must still be
    // the authored one, not a default produced by rebuilding the value.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let left_span = marked_span(130);

    let nodes = vec![
        rvalue_item(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 20).expect("20 fits I32"),
            Some(left_span),
            ValueMode::ImmutableOwned,
        )),
        rvalue_item(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 22).expect("22 fits I32"),
            Some(marked_span(140)),
            ValueMode::ImmutableOwned,
        )),
        operator_item(Operator::Add),
    ];

    let folded = expect_folded_stack(constant_fold(nodes, &mut string_table, None));

    assert_eq!(folded.len(), 1);
    let ExpressionRpnItem::Operand(result) = &folded[0] else {
        panic!("a fully folded expression should be one operand");
    };
    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(42)
    ));
    assert_eq!(result.span, Some(left_span));
}

#[test]
fn fold_cast_infallible_int_to_string_folds_to_string_literal() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 42).expect("42 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let target_type_id = type_environment.builtins().string;

    let policy = BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::String,
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Infallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect("infallible builtin cast should fold");

    assert_eq!(folded.type_id, target_type_id);

    let ExpressionKind::StringSlice(interned) = folded.kind else {
        panic!("expected folded I32 -> String cast to produce a string slice");
    };

    assert_eq!(string_table.resolve(interned), "42");
}

#[test]
fn fold_structural_string_cast_reports_text_unavailable_rule() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source =
        Expression::structural_string(vec![ConstStringPiece::SiteRoot], Default::default());
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::Fixed(FixedScalar::I32),
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &mut string_table, true)
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
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let target_type_id = type_environment.builtins().string;

    let policy = BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::String,
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Infallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: true,
        },
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
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
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("123".to_string());
    let source = Expression::string_slice(text, None, ValueMode::ImmutableOwned);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::Fixed(FixedScalar::I32),
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect("successful fallible builtin cast should fold");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(123)
    ));
}

#[test]
fn fold_cast_fallible_string_to_int_failure_reports_builtin_cast_failed_in_const() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("not a number".to_string());
    let source = Expression::string_slice(text, None, ValueMode::ImmutableOwned);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let policy = BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32));
    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::Fixed(FixedScalar::I32),
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect_err("failed fallible builtin cast should report a const diagnostic");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn fold_cast_user_defined_evidence_rejected_in_const_context() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 42).expect("42 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let target_type_id = type_environment.builtins().string;
    let method_path = path_fork
        .try_intern_portable_path("to_string", &mut string_table)
        .expect("test path fits");

    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::String,
            target_type_id,
            evidence: ResolvedCastEvidence::UserDefined {
                evidence_id: TraitEvidenceId(0),
                method_path,
            },
            fallibility: BuiltinCastFallibility::Infallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect_err("user-defined evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
    );
}

#[test]
fn fold_cast_generic_bound_evidence_rejected_in_const_context() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 42).expect("42 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let target_type_id = type_environment.builtins().string;

    let cast = cast_expression(
        CastExpressionInput {
            source,
            target: BuiltinCastTarget::String,
            target_type_id,
            evidence: ResolvedCastEvidence::GenericBound {
                trait_id: TraitId(0),
                parameter_id: GenericParameterId(0),
            },
            fallibility: BuiltinCastFallibility::Infallible,
            handling: CastHandling::Implicit,
            requires_optional_wrap_after_cast: false,
        },
        &mut type_environment,
    );

    let error = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect_err("generic-bound evidence should not fold in a const context");

    assert_invalid_cast_error(
        &error,
        InvalidCastReason::GenericBoundEvidenceNotConstFoldable,
    );
}

fn catch_handler_body(value: Expression) -> Vec<AstNode> {
    let span = value.span;

    vec![AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions: vec![value],
            span,
        }),
        span,
        scope: PathId::ROOT,
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
        CastExpressionInput {
            source,
            target,
            target_type_id,
            evidence: ResolvedCastEvidence::Builtin { policy },
            fallibility: BuiltinCastFallibility::Fallible,
            handling: CastHandling::Recover,
            requires_optional_wrap_after_cast: false,
        },
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

fn fold_invalid_string_cast_with_catch(
    constant_context: bool,
) -> Result<Expression, ConstantFoldError> {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let text = string_table.get_or_intern("nope".to_string());
    let source = Expression::string_slice(text, None, ValueMode::ImmutableOwned);
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        target_type_id,
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32)),
        catch_handler_body(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 0).expect("0 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )),
        &mut type_environment,
    );

    fold_compile_time_expression(&cast, &mut string_table, constant_context)
}

#[test]
fn known_invalid_cast_under_catch_is_diagnostic_in_constant_context() {
    let error = fold_invalid_string_cast_with_catch(true)
        .expect_err("a known invalid cast must not use its catch fallback");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn known_invalid_cast_under_catch_is_diagnostic_in_runtime_context() {
    let error = fold_invalid_string_cast_with_catch(false)
        .expect_err("a known invalid cast must not fall back to runtime catch delivery");

    assert_invalid_cast_error(&error, InvalidCastReason::BuiltinCastFailedInConst);
}

#[test]
fn runtime_parameter_cast_under_catch_remains_runtime() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let source_path = path_fork
        .try_intern_portable_path("source", &mut string_table)
        .expect("test path fits");
    let source = Expression::reference(
        source_path,
        DataType::StringSlice,
        None,
        ValueMode::ImmutableReference,
    );
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        target_type_id,
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32)),
        catch_handler_body(Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 0).expect("0 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )),
        &mut type_environment,
    );
    let folded = fold_compile_time_expression(&cast, &mut string_table, false)
        .expect("an unknown parameter value must remain a runtime cast");

    assert!(matches!(folded.kind, ExpressionKind::ValueBlock { .. }));
}

#[test]
fn fold_cast_fallible_builtin_success_with_catch_ignores_handler() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
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
    let source = Expression::string_slice(text, None, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(
            source_member.clone(),
        ));
    let target_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let handler_value = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 999).expect("999 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    )
    .with_synthetic_interface_provenance(SyntheticInterfaceProvenance::single(handler_member));

    let cast = fallible_builtin_cast_with_catch(
        source,
        BuiltinCastTarget::Fixed(FixedScalar::I32),
        target_type_id,
        BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32)),
        catch_handler_body(handler_value),
        &mut type_environment,
    );

    let folded = fold_compile_time_expression(&cast, &mut string_table, true)
        .expect("successful builtin cast should fold to success value even with catch handler");

    assert_eq!(folded.type_id, target_type_id);
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(123)
    ));
    assert_eq!(
        folded.synthetic_interface_provenance.members(),
        &[source_member]
    );
}

#[test]
fn evaluate_operator_folds_u32_arithmetic_and_rejects_overflow() {
    // U32 leaves fold in their own domain; overflow, underflow and
    // zero divisors report the stable integer failure codes at compile time.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let uint = |value: u64| fixed_integer_expression(FixedScalar::U32, i128::from(value));

    let folded = expect_folded_operator(uint(1).evaluate_operator(
        &uint(2),
        &Operator::Add,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_u64() == Some(3)
    ));
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U32)
    );

    let error = uint(u32::MAX as u64)
        .evaluate_operator(&uint(1), &Operator::Add, &mut string_table, None)
        .expect_err("u32::MAX + 1 exceeds the U32 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &string_table,
    );

    // `0 ^ 0` folds to one and overflowing powers report the boundary.
    let folded = expect_folded_operator(uint(0).evaluate_operator(
        &uint(0),
        &Operator::Exponent,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_u64() == Some(1)
    ));

    let error = uint(2)
        .evaluate_operator(&uint(32), &Operator::Exponent, &mut string_table, None)
        .expect_err("2 ^ 32 exceeds the U32 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );

    let error = uint(0)
        .evaluate_operator(&uint(1), &Operator::Subtract, &mut string_table, None)
        .expect_err("0 - 1 underflows U32");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("-"),
        &string_table,
    );

    let error = uint(7)
        .evaluate_operator(&uint(0), &Operator::IntDivide, &mut string_table, None)
        .expect_err("U32 // 0 must fail at compile time");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        &string_table,
    );

    // U32 `/` folds in F64 directly, never through an integer carrier.
    let folded = expect_folded_operator(uint(7).evaluate_operator(
        &uint(2),
        &Operator::Divide,
        &mut string_table,
        None,
    ));
    assert_eq!(expect_fixed_scalar(folded).scalar(), FixedScalar::F64);

    let folded = expect_folded_operator(uint(7).evaluate_operator(
        &uint(2),
        &Operator::Divide,
        &mut string_table,
        None,
    ));
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
}

#[test]
fn evaluate_operator_folds_u32_comparisons_and_dec_mixed_pairs() {
    // U32 compares exactly with U32 and I32, and Dec mixes with
    // U32 through the shared exact rule used by `k2 #Dec2 = cu + 0.5`.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let uint = |value: u64| fixed_integer_expression(FixedScalar::U32, i128::from(value));
    let int = |value: i64| fixed_integer_expression(FixedScalar::I32, i128::from(value));

    assert!(expect_folded_boolean(
        uint(3),
        uint(5),
        Operator::LessThan,
        &mut string_table,
    ));
    assert!(!expect_folded_boolean(
        uint(3),
        int(-1),
        Operator::LessThan,
        &mut string_table,
    ));
    assert!(expect_folded_boolean(
        uint(0),
        int(-1),
        Operator::GreaterThan,
        &mut string_table,
    ));

    let mut type_environment = TypeEnvironment::new();
    let dec_type =
        type_environment.intern_number(NumberScale::new(2).expect("test Dec scale is valid"));
    let dec = number_expression("5.00", NumericLiteralSign::Positive, 2, dec_type);
    assert!(expect_folded_boolean(
        uint(3),
        dec,
        Operator::LessThan,
        &mut string_table,
    ));
}

#[test]
fn evaluate_operator_folds_u64_at_full_bounds_and_rejects_overflow() {
    // The U64 domain spans the whole u64 range, so values above i64::MAX fold exactly
    // and u64::MAX + 1, 2 ^ 64 and underflowing products report the stable integer failures.
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let uint = |value: u64| fixed_integer_expression(FixedScalar::U64, i128::from(value));

    let folded = expect_folded_operator(uint(u64::MAX).evaluate_operator(
        &uint(0),
        &Operator::Subtract,
        &mut string_table,
        None,
    ));
    assert!(matches!(
        folded.kind,
        ExpressionKind::FixedScalar(value) if value.as_u64() == Some(u64::MAX)
    ));
    assert_eq!(
        folded.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U64)
    );

    let error = uint(u64::MAX)
        .evaluate_operator(&uint(1), &Operator::Add, &mut string_table, None)
        .expect_err("u64::MAX + 1 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("+"),
        &string_table,
    );

    let error = uint(2)
        .evaluate_operator(&uint(64), &Operator::Exponent, &mut string_table, None)
        .expect_err("2 ^ 64 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("^"),
        &string_table,
    );

    let error = uint(9_223_372_036_854_775_808)
        .evaluate_operator(&uint(2), &Operator::Multiply, &mut string_table, None)
        .expect_err("2^63 * 2 exceeds the U64 boundary");
    assert_compile_time_error(
        &error,
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some("*"),
        &string_table,
    );
}
