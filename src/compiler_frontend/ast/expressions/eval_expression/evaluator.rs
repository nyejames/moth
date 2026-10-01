//! Expression evaluation and AST-side constant folding implementation.
//!
//! WHAT: resolves parsed infix expression fragments into typed AST expressions.
//! WHY: AST is the stage that owns operator typing, constant folding, and the decision about
//!      whether an expression can stay compile-time or must survive as runtime RPN.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldOutcome, constant_fold, fold_compile_time_expression,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::field_access::project_explicit_const_record_field;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidExpressionReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::NumberScale;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{
    AstCounter, FrontendCounter, increment_ast_counter, increment_frontend_counter,
};
use crate::compiler_frontend::numeric_text::parse::{
    literal_kind_initialises, materialize_fixed_scalar, materialize_float, materialize_int,
    materialize_number,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::eval_log;

use super::ordering;
use super::result_type::resolve_expression_result_type;
use super::typing_error::ExpressionTypingError;

/// Resolve a parsed expression fragment into a fully typed AST `Expression`.
///
/// WHAT: orders RPN, resolves literal materialization and operator typing in one pass, then folds
///       and validates the final result.
/// WHY: immediate peers select literal domains while the canonical operator policy remains the
///      sole source of result types.
pub fn evaluate_expression(
    context: &ScopeContext,
    nodes: Vec<ExpressionRpnItem>,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Expression, ExpressionTypingError> {
    let direct_destination =
        direct_numeric_destination(&nodes, type_interner.environment(), expected_type);
    let (mut ordered_nodes, span) = ordering::order_expression_nodes(nodes)?;

    // Fast path: a single R-value needs no operator resolution or RPN assembly.
    if ordered_nodes.len() == 1 {
        if matches!(
            &ordered_nodes[0],
            ExpressionRpnItem::PendingNumericLiteral { .. }
        ) {
            materialize_pending_literal(
                &mut ordered_nodes,
                0,
                direct_destination,
                context,
                string_table,
            )?;
        }

        let ExpressionRpnItem::Operand(expression) = &ordered_nodes[0] else {
            return Err(CompilerError::compiler_error(
                "Expression ordering produced a single operator without an operand.",
            )
            .into());
        };

        let only_expression = fold_compile_time_expression(
            expression,
            &context.template_ir_store,
            string_table,
            context.kind.is_constant_context(),
            context.numeric_profile,
        )?;

        validate_expression_result_type(
            expected_type,
            only_expression.type_id,
            ordered_nodes[0].source_span(),
            type_interner.environment_mut_for_derived_types(),
        )?;

        // References tighten the expected type so later fragments in the same statement
        // can resolve against the inferred target type.
        if let ExpressionKind::Reference(..) = only_expression.kind {
            *expected_type = ExpectedType::Known(only_expression.type_id);
        } else if matches!(
            expected_type,
            ExpectedType::Infer | ExpectedType::DirectLiteral(_)
        ) {
            *expected_type = ExpectedType::Known(only_expression.type_id);
        }

        return Ok(only_expression);
    }

    // General path: resolve operator types across the full RPN shape, then attempt folding.
    let resolved_type = resolve_expression_result_type(
        &mut ordered_nodes,
        span,
        direct_destination,
        context,
        string_table,
        type_interner.environment(),
        path_fork,
    )?;
    validate_expression_result_type(
        expected_type,
        resolved_type,
        span,
        type_interner.environment_mut_for_derived_types(),
    )?;

    if matches!(
        expected_type,
        ExpectedType::Infer | ExpectedType::DirectLiteral(_)
    ) {
        *expected_type = ExpectedType::Known(resolved_type);
    }

    // A multi-item expression has completed semantic typing. Resolve explicit # numeric
    // operands, including copied numeric places, only at this consumption boundary so standalone
    // copies and ordinary runtime bindings keep their place identity.
    substitute_explicit_numeric_constants(
        &mut ordered_nodes,
        context,
        path_fork,
        type_interner.environment(),
    );

    let stack_span = ordered_nodes.iter().find_map(|item| match item {
        ExpressionRpnItem::Operand(expression) => expression.span,
        ExpressionRpnItem::Operator { .. } => None,
        // Resolution replaces pending literals before folding.
        ExpressionRpnItem::PendingNumericLiteral { .. } => None,
    });
    // Runtime RPN needs an owned value mode for the final expression node.
    let value_mode = value_mode.as_owned();
    eval_log!("Attempting to Fold: ", Pretty ordered_nodes);
    increment_frontend_counter(FrontendCounter::ConstantFoldAttemptCount);
    let fold_outcome = constant_fold(
        ordered_nodes,
        string_table,
        context.numeric_profile,
        Some(context),
    )?;

    increment_frontend_counter(FrontendCounter::ConstantFoldSuccessCount);
    eval_log!("Stack after folding: ", Pretty fold_outcome);

    let stack = match fold_outcome {
        ConstantFoldOutcome::Folded(stack) | ConstantFoldOutcome::NotConstant(stack) => stack,
        ConstantFoldOutcome::TextUnavailable {
            items: _,
            diagnostic,
            ..
        } if context.kind.is_constant_context() => {
            return Err(ExpressionTypingError::Diagnostic(diagnostic));
        }
        ConstantFoldOutcome::TextUnavailable { items, .. } => items,
    };

    // Fully folded to a single compile-time value: hand the folded operand back by move.
    if stack.len() == 1 {
        let mut stack = stack;
        let Some(ExpressionRpnItem::Operand(expression)) = stack.pop() else {
            return Err(CompilerError::compiler_error(
                "Constant folding produced a non-operand item as the single result.",
            )
            .into());
        };
        return Ok(expression);
    }

    // Folding consumed every node but produced no result (e.g. empty input).
    if stack.is_empty() {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::UnresolvedStackShape,
            span,
        )
        .into());
    }

    // Partial fold: assemble the reduced stack into runtime RPN.
    // The only DataType spelling a successful expression evaluation builds: the partial-fold
    // result needs one for its runtime node. Fully folded expressions never reach here.
    increment_ast_counter(AstCounter::DiagnosticDataTypeMaterialisations);
    let diagnostic_type = diagnostic_type_spelling(resolved_type, type_interner.environment());

    Ok(runtime_expression_from_items(
        stack,
        diagnostic_type,
        resolved_type,
        value_mode,
        stack_span.or(span),
    )?)
}

fn is_matching_materialized_numeric_value(
    expression: &Expression,
    expected_type_id: TypeId,
) -> bool {
    expression.type_id == expected_type_id
        && matches!(
            &expression.kind,
            ExpressionKind::Int(_)
                | ExpressionKind::Float(_)
                | ExpressionKind::FixedScalar(_)
                | ExpressionKind::Number(_)
        )
}

fn substitute_explicit_numeric_constants(
    nodes: &mut [ExpressionRpnItem],
    context: &ScopeContext,
    path_fork: &PathInternerFork,
    type_environment: &TypeEnvironment,
) {
    for item in nodes {
        let ExpressionRpnItem::Operand(expression) = item else {
            continue;
        };
        if NumericScalar::from_type_id(expression.type_id, type_environment).is_none() {
            continue;
        }

        let replacement = match &expression.kind {
            ExpressionKind::Reference(path) => context
                .with_explicit_constant_expression(path, path_fork, |value| {
                    // Avoid cloning unless both the canonical type and scalar value shape match.
                    is_matching_materialized_numeric_value(value, expression.type_id)
                        .then(|| value.clone())
                })
                .flatten(),
            ExpressionKind::FieldAccess { .. } => project_explicit_const_record_field(
                expression, context, path_fork,
            )
            .filter(|value| is_matching_materialized_numeric_value(value, expression.type_id)),
            ExpressionKind::Copy(place) => match &place.kind {
                PlaceExpressionKind::Local(path) => context
                    .with_explicit_constant_expression(path, path_fork, |value| {
                        is_matching_materialized_numeric_value(value, expression.type_id)
                            .then(|| value.clone())
                    })
                    .flatten(),
                PlaceExpressionKind::Field { .. } => project_explicit_const_record_field(
                    expression, context, path_fork,
                )
                .filter(|value| is_matching_materialized_numeric_value(value, expression.type_id)),
            },
            _ => None,
        };

        if let Some(mut replacement) = replacement {
            replacement.preserve_authored_metadata(expression);
            *expression = replacement;
        }
    }
}

/// Materialise a pending literal using its immediate numeric peer or natural value mode.
///
/// WHAT: replaces one deferred RPN item with a typed operand and returns its semantic type.
/// WHY: direct literal parsing must preserve lexical facts until either a peer or the receiving
///      boundary chooses the materialisation domain.
pub(super) fn materialize_pending_literal(
    nodes: &mut [ExpressionRpnItem],
    index: usize,
    destination: Option<PendingLiteralDestination>,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<TypeId, ExpressionTypingError> {
    let expression = {
        let Some(item) = nodes.get(index) else {
            return Err(CompilerError::compiler_error(
                "Pending literal resolution could not retrieve its RPN item.",
            )
            .into());
        };
        let ExpressionRpnItem::PendingNumericLiteral {
            token,
            span,
            value_mode,
        } = item
        else {
            return Err(CompilerError::compiler_error(
                "Pending literal resolution indexed a non-literal RPN item.",
            )
            .into());
        };

        let destination = destination.filter(|destination| match destination {
            PendingLiteralDestination::Int => token.kind == NumericLiteralKind::WholeNumber,
            PendingLiteralDestination::Float => true,
            PendingLiteralDestination::FixedScalar(scalar) => {
                literal_kind_initialises(token.kind, *scalar)
            }
            PendingLiteralDestination::Number { .. } => true,
        });

        match destination {
            Some(PendingLiteralDestination::Int) => {
                let value = materialize_int(
                    token,
                    token.sign,
                    context.numeric_profile.int_width,
                    string_table,
                )
                .map_err(|reason| {
                    CompilerDiagnostic::invalid_number_literal(token.source_text, reason, *span)
                })?;
                Expression::int(value, *span, value_mode.clone())
            }
            Some(PendingLiteralDestination::Float) => {
                let value =
                    materialize_float(token, context.numeric_profile.float_precision, string_table)
                        .map_err(|reason| {
                            CompilerDiagnostic::invalid_number_literal(
                                token.source_text,
                                reason,
                                *span,
                            )
                        })?;
                Expression::float(value, *span, value_mode.clone())
            }
            Some(PendingLiteralDestination::FixedScalar(scalar)) => {
                let value = materialize_fixed_scalar(token, token.sign, scalar, string_table)
                    .map_err(|reason| {
                        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, *span)
                    })?;
                Expression::fixed_scalar(value, *span, value_mode.clone())
            }
            Some(PendingLiteralDestination::Number { scale, type_id }) => {
                let value = materialize_number(token, token.sign, scale, string_table).map_err(
                    |reason| {
                        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, *span)
                    },
                )?;
                Expression::number(value, type_id, *span, value_mode.clone())
            }
            None => default_materialised_literal(
                token,
                *span,
                value_mode.clone(),
                context,
                string_table,
            )?,
        }
    };
    let type_id = expression.type_id;
    let Some(item) = nodes.get_mut(index) else {
        return Err(CompilerError::compiler_error(
            "Pending literal resolution could not retrieve its RPN item.",
        )
        .into());
    };
    *item = ExpressionRpnItem::Operand(expression);

    Ok(type_id)
}

/// The direct destination when the whole fragment is one literal with a numeric expectation.
///
/// WHAT: returns the numeric destination only when the parsed nodes are exactly one pending
///       literal and the expected type (or its option inner type) is numeric or `Byte`.
/// WHY: "direct" means the literal is the entire receiving expression, so an operator result
///      such as `1 + 1` never retags through its receiving annotation.
fn direct_numeric_destination(
    nodes: &[ExpressionRpnItem],
    type_environment: &TypeEnvironment,
    expected_type: &ExpectedType,
) -> Option<PendingLiteralDestination> {
    let [ExpressionRpnItem::PendingNumericLiteral { .. }] = nodes else {
        return None;
    };

    let expected_type_id = expected_type.literal_destination_type_id()?;
    let destination_type_id = type_environment
        .option_inner_type(expected_type_id)
        .unwrap_or(expected_type_id);
    pending_literal_destination_for_type_id(destination_type_id, type_environment)
}

/// A semantic numeric or `Byte` destination eligible for pending-literal materialisation.
#[derive(Clone, Copy)]
pub(super) enum PendingLiteralDestination {
    Int,
    Float,
    FixedScalar(FixedScalar),
    /// One exact-decimal destination carrying its already-resolved local identity.
    Number {
        scale: NumberScale,
        type_id: TypeId,
    },
}

/// Classify one direct semantic type as an eligible pending-literal destination.
pub(super) fn pending_literal_destination_for_type_id(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<PendingLiteralDestination> {
    let builtins = type_environment.builtins();
    if type_id == builtins.int {
        return Some(PendingLiteralDestination::Int);
    }
    if type_id == builtins.float {
        return Some(PendingLiteralDestination::Float);
    }

    if let Some(scale) = type_environment.number_scale(type_id) {
        return Some(PendingLiteralDestination::Number { scale, type_id });
    }
    type_environment
        .fixed_scalar(type_id)
        .map(PendingLiteralDestination::FixedScalar)
}

/// Today's default literal materialisation, unchanged without an eligible destination.
///
/// WHAT: whole literals materialise at the boundary `Int` width and decimal/exponent literals
///       at the boundary `Float` precision, with the exact-negation behaviour the parser
///       previously applied to parser-owned minus.
/// WHY: default values, diagnostics, reasons and spans must stay identical now that the
///      materialisation site moved from the literal parser into the evaluator.
fn default_materialised_literal(
    token: &NumericLiteralToken,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionTypingError> {
    if token.kind == NumericLiteralKind::WholeNumber {
        let value = materialize_int(
            token,
            token.sign,
            context.numeric_profile.int_width,
            string_table,
        )
        .map_err(|reason| {
            CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
        })?;

        return Ok(Expression::int(value, span, value_mode));
    }

    // The folded sign is part of the retained token: a negative decimal/exponent token
    // materialises its positive magnitude at the boundary precision, then negates exactly.
    let positive = NumericLiteralToken {
        sign: NumericLiteralSign::Positive,
        ..token.clone()
    };
    let mut value = materialize_float(
        &positive,
        context.numeric_profile.float_precision,
        string_table,
    )
    .map_err(|reason| {
        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
    })?;

    if token.sign == NumericLiteralSign::Negative {
        value = -value;
    }

    Ok(Expression::float(value, span, value_mode))
}

/// Compiler-bug error for a pending literal that escaped `evaluate_expression`.
///
/// WHAT: one shared constructor for the defensive arms in ordering, result typing and every
///       later RPN consumer that must never observe a deferred literal.
/// WHY: pending literals live only between parsing and evaluation; reaching any later stage
///      is a broken invariant, not a source diagnostic.
pub(crate) fn pending_numeric_literal_bug(function: &'static str) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Pending numeric literal reached {function}; evaluate_expression must resolve it first."
    ))
}

/// Assemble a runtime RPN `Expression` from an ordered expression-owned stack.
///
/// WHAT: wraps the narrowed RPN stack in `Expression::runtime_with_type_id`.
/// WHY: `evaluate_expression` is the boundary where broad parser nodes are
///      replaced by expression-owned runtime payloads.
fn runtime_expression_from_items(
    items: Vec<ExpressionRpnItem>,
    diagnostic_type: DataType,
    type_id: TypeId,
    value_mode: ValueMode,
    span: Option<SourceSpan>,
) -> Result<Expression, CompilerError> {
    Ok(Expression::runtime_with_type_id(
        ExpressionRpn { items },
        diagnostic_type,
        type_id,
        span,
        value_mode,
    ))
}

/// Validate that an expression's resolved semantic type is compatible with the
/// contextual expectation.
///
/// WHAT: compares canonical `TypeId`s through the `TypeEnvironment`.
/// WHY: semantic type decisions must use `TypeId` equality, not parse-level
///      `DataType` shape matching.
fn validate_expression_result_type(
    expected_type: &mut ExpectedType,
    actual_type_id: TypeId,
    span: Option<SourceSpan>,
    type_environment: &mut TypeEnvironment,
) -> Result<(), ExpressionTypingError> {
    let Some(expected_type_id) = expected_type.known_type_id() else {
        return Ok(());
    };

    if is_declaration_compatible(expected_type_id, actual_type_id, type_environment) {
        return Ok(());
    }

    Err(CompilerDiagnostic::type_mismatch(
        expected_type_id,
        actual_type_id,
        TypeMismatchContext::General,
        span,
    )
    .into())
}
