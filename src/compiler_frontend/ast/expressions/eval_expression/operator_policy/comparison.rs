//! Comparison operator typing policy.
//!
//! WHAT: decides the result type of comparison operators and rejects invalid operand combinations.
//! WHY: structural equality rules for choices, scalar ordering, and mixed numeric comparisons
//! must be enforced consistently before backend lowering.

use super::super::result_type::ExpressionResultType;
use super::diagnostics::invalid_comparison_types;
use super::shared::is_mixed_int_float;
use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, IncompatibleChoiceComparisonReason,
};
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

pub(super) fn is_comparison_operator(op: &Operator) -> bool {
    matches!(
        op,
        Operator::Equality
            | Operator::NotEqual
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual
    )
}

pub(super) fn resolve_comparison_operator_type(
    lhs: &ExpressionResultType,
    rhs: &ExpressionResultType,
    op: &Operator,
    location: &SourceLocation,
    type_environment: &TypeEnvironment,
) -> Result<ExpressionResultType, ExpressionTypingError> {
    let builtins = type_environment.builtins();

    // ------------------------
    //  Option equality short-circuit
    // ------------------------
    if matches!(op, Operator::Equality | Operator::NotEqual)
        && expression_pair_has_option_context(lhs, rhs, type_environment)
    {
        return resolve_option_equality_type(lhs, rhs, op, location, type_environment);
    }

    // ------------------------
    //  Same-type comparisons
    // ------------------------
    if lhs.type_id == rhs.type_id {
        let bool_result = || ExpressionResultType::from_type_id(builtins.bool, type_environment);

        // Numeric scalars support full ordering and equality.
        // Decimal is intentionally inactive in the Alpha surface and is not treated
        // as a comparable numeric type.
        let same_numeric_scalar = lhs.type_id == builtins.int || lhs.type_id == builtins.float;

        if same_numeric_scalar {
            return match op {
                Operator::Equality
                | Operator::NotEqual
                | Operator::GreaterThan
                | Operator::GreaterThanOrEqual
                | Operator::LessThan
                | Operator::LessThanOrEqual => Ok(bool_result()),
                _ => invalid_comparison_types(lhs, rhs, op, location),
            };
        }

        // Booleans only support equality checks.
        if lhs.type_id == builtins.bool {
            return match op {
                Operator::Equality | Operator::NotEqual => Ok(bool_result()),
                _ => invalid_comparison_types(lhs, rhs, op, location),
            };
        }

        // Strings support equality only.
        if lhs.type_id == builtins.string {
            return match op {
                Operator::Equality | Operator::NotEqual => Ok(bool_result()),
                _ => invalid_comparison_types(lhs, rhs, op, location),
            };
        }

        // Characters support full ordering.
        if lhs.type_id == builtins.char {
            return match op {
                Operator::Equality
                | Operator::NotEqual
                | Operator::LessThan
                | Operator::LessThanOrEqual
                | Operator::GreaterThan
                | Operator::GreaterThanOrEqual => Ok(bool_result()),
                _ => invalid_comparison_types(lhs, rhs, op, location),
            };
        }

        // Choice types support structural equality when every payload field supports it.
        if type_environment.variants_for(lhs.type_id).is_some() {
            return match op {
                Operator::Equality | Operator::NotEqual => {
                    validate_choice_equality_support(
                        lhs.type_id,
                        rhs.type_id,
                        location,
                        type_environment,
                    )?;
                    Ok(bool_result())
                }
                _ => invalid_comparison_types(lhs, rhs, op, location),
            };
        }

        // Same type but not a comparable category.
        return invalid_comparison_types(lhs, rhs, op, location);
    }

    // ------------------------
    //  Mixed-type comparisons
    // ------------------------

    // Int and Float can be compared directly.
    if is_mixed_int_float(lhs, rhs, type_environment) {
        return Ok(ExpressionResultType::from_type_id(
            builtins.bool,
            type_environment,
        ));
    }

    // Two choice values of different nominal types are never comparable.
    let lhs_is_choice = type_environment.variants_for(lhs.type_id).is_some();
    let rhs_is_choice = type_environment.variants_for(rhs.type_id).is_some();

    if lhs_is_choice && rhs_is_choice {
        return Err(CompilerDiagnostic::incompatible_choice_comparison(
            IncompatibleChoiceComparisonReason::DifferentChoiceTypes,
            lhs.type_id,
            rhs.type_id,
            location.clone(),
        )
        .into());
    }

    // A choice value can only be compared with another value of the same choice type.
    let exactly_one_is_choice = lhs_is_choice != rhs_is_choice;
    if exactly_one_is_choice {
        return Err(CompilerDiagnostic::incompatible_choice_comparison(
            IncompatibleChoiceComparisonReason::ChoiceWithNonChoice,
            lhs.type_id,
            rhs.type_id,
            location.clone(),
        )
        .into());
    }

    invalid_comparison_types(lhs, rhs, op, location)
}

/// Validates that every payload field in a choice type supports runtime equality.
///
/// Returns `Ok(())` when the choice can participate in equality comparisons,
/// or an error pointing to the first unsupported field.
fn validate_choice_equality_support(
    lhs_type_id: TypeId,
    rhs_type_id: TypeId,
    location: &SourceLocation,
    type_environment: &TypeEnvironment,
) -> Result<(), ExpressionTypingError> {
    let Some(variants) = type_environment.variants_for(lhs_type_id) else {
        return Ok(());
    };

    for variant in variants {
        if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload {
            for field in fields {
                if !type_environment.supports_runtime_equality(field.type_id) {
                    let Some(field_name_id) = field.name.name() else {
                        return Err(CompilerError::compiler_error(
                            "Field definition has empty path in choice payload equality check",
                        )
                        .into());
                    };

                    return Err(CompilerDiagnostic::incompatible_choice_comparison(
                        IncompatibleChoiceComparisonReason::PayloadEqualityNotSupported {
                            field_name: field_name_id,
                            field_type: field.type_id,
                        },
                        lhs_type_id,
                        rhs_type_id,
                        location.clone(),
                    )
                    .into());
                }
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum OptionComparisonOperand {
    Option { inner: TypeId },
    NoneLiteral,
    Other,
}

fn classify_option_comparison_operand(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> OptionComparisonOperand {
    let Some(inner) = type_environment.option_inner_type(type_id) else {
        return OptionComparisonOperand::Other;
    };

    if inner == type_environment.builtins().none {
        return OptionComparisonOperand::NoneLiteral;
    }

    OptionComparisonOperand::Option { inner }
}

fn expression_pair_has_option_context(
    lhs: &ExpressionResultType,
    rhs: &ExpressionResultType,
    type_environment: &TypeEnvironment,
) -> bool {
    matches!(
        classify_option_comparison_operand(lhs.type_id, type_environment),
        OptionComparisonOperand::Option { .. }
    ) || matches!(
        classify_option_comparison_operand(rhs.type_id, type_environment),
        OptionComparisonOperand::Option { .. }
    )
}

fn resolve_option_equality_type(
    lhs: &ExpressionResultType,
    rhs: &ExpressionResultType,
    op: &Operator,
    location: &SourceLocation,
    type_environment: &TypeEnvironment,
) -> Result<ExpressionResultType, ExpressionTypingError> {
    let lhs_kind = classify_option_comparison_operand(lhs.type_id, type_environment);
    let rhs_kind = classify_option_comparison_operand(rhs.type_id, type_environment);

    let comparison_supported = match (lhs_kind, rhs_kind) {
        // Option vs NoneLiteral is always supported.
        (OptionComparisonOperand::Option { .. }, OptionComparisonOperand::NoneLiteral)
        | (OptionComparisonOperand::NoneLiteral, OptionComparisonOperand::Option { .. }) => true,

        // Two Options are comparable only when their inner types are identical and support equality.
        (
            OptionComparisonOperand::Option { inner: left_inner },
            OptionComparisonOperand::Option { inner: right_inner },
        ) => left_inner == right_inner && type_environment.supports_runtime_equality(left_inner),

        // Option vs a compatible non-option type is supported when the option inner type supports equality.
        (OptionComparisonOperand::Option { inner }, OptionComparisonOperand::Other) => {
            is_type_compatible(lhs.type_id, rhs.type_id, type_environment)
                && type_environment.supports_runtime_equality(inner)
        }

        (OptionComparisonOperand::Other, OptionComparisonOperand::Option { inner }) => {
            is_type_compatible(rhs.type_id, lhs.type_id, type_environment)
                && type_environment.supports_runtime_equality(inner)
        }

        _ => false,
    };

    if comparison_supported {
        return Ok(ExpressionResultType::from_type_id(
            type_environment.builtins().bool,
            type_environment,
        ));
    }

    invalid_comparison_types(lhs, rhs, op, location)
}
