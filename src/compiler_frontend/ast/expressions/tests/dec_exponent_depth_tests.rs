//! Structural Dec-exponent depth over an authored postfix shape.
//!
//! Only items inside the right operand of a `Dec` power carry exponent context. An expression
//! without a `Dec` power must report none, which is the path ordinary expressions take.

use super::evaluator::dec_power_exponent_depths;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, ResolvedNumericOperation, ResolvedOperation, ResolvedOperationKind,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

const I32_TYPE: TypeId = builtin_type_ids::fixed_scalar(FixedScalar::I32);

#[test]
fn dec_exponent_depths_mark_only_the_dec_power_operand() {
    // `base ^ (2 ^ 2)`: the inner power is I32 arithmetic inside the Dec power's exponent, so
    // every item of that exponent subtree carries exponent context.
    let dec_power_shape = vec![
        operand(0),
        operand(1),
        operand(2),
        fixed_power(),
        dec_power(),
    ];
    assert_eq!(
        dec_power_exponent_depths(&dec_power_shape),
        vec![0, 1, 1, 1, 0],
        "only the Dec power's right operand carries exponent context"
    );

    // The same shape without a Dec power records nothing, so ordinary expressions pay for no
    // depth buffer and every item reads depth zero.
    let ordinary_shape = vec![
        operand(0),
        operand(1),
        operand(2),
        fixed_power(),
        fixed_add(),
    ];
    assert_eq!(
        dec_power_exponent_depths(&ordinary_shape),
        Vec::<u32>::new(),
        "an I32 power and arithmetic record no Dec exponent context"
    );
}

fn operand(index: usize) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(ExprId::at(index))
}

fn fixed_add() -> ExpressionRpnItem {
    let domain = NumericScalar::Fixed(FixedScalar::I32);
    numeric_operator(Operator::Add, NumericOperator::Add, domain, Some(domain))
}

fn fixed_power() -> ExpressionRpnItem {
    let domain = NumericScalar::Fixed(FixedScalar::I32);
    numeric_operator(
        Operator::Exponent,
        NumericOperator::Power,
        domain,
        Some(domain),
    )
}

fn dec_power() -> ExpressionRpnItem {
    let domain = NumericScalar::Number(NumberScale::ZERO);
    numeric_operator(
        Operator::Exponent,
        NumericOperator::Power,
        domain,
        Some(NumericScalar::Fixed(FixedScalar::I32)),
    )
}

/// Builds the resolved facts the depth analysis reads: one binary operator and its numeric
/// domains.
fn numeric_operator(
    operator: Operator,
    numeric_operator: NumericOperator,
    domain: NumericScalar,
    right_domain: Option<NumericScalar>,
) -> ExpressionRpnItem {
    ExpressionRpnItem::ResolvedOperator(ResolvedOperation {
        operator,
        span: None,
        result_type: I32_TYPE,
        left_type: I32_TYPE,
        right_type: Some(I32_TYPE),
        kind: ResolvedOperationKind::Numeric(ResolvedNumericOperation {
            operator: numeric_operator,
            left_domain: domain,
            right_domain,
            domain,
            left_conversion: None,
            right_conversion: None,
            discharged: true,
        }),
        failure: None,
    })
}
