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
    ExpressionRpn, ExpressionRpnItem,
};
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
use crate::compiler_frontend::instrumentation::{
    AstCounter, FrontendCounter, increment_ast_counter, increment_frontend_counter,
};
use crate::compiler_frontend::numeric_text::parse::{
    literal_kind_initialises, materialize_fixed_scalar, materialize_float, materialize_int,
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
/// WHAT: resolves deferred numeric literals against the caller's expected type, then applies
///       shunting-yard ordering, operator type resolution, optional constant folding,
///       and final type validation against the caller's expectation.
/// WHY: this is the single entry point where AST decides whether an expression collapses to a
///      compile-time value or must be preserved as runtime RPN for HIR lowering.
pub fn evaluate_expression(
    context: &ScopeContext,
    nodes: Vec<ExpressionRpnItem>,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Expression, ExpressionTypingError> {
    let nodes = resolve_pending_numeric_literals(
        nodes,
        context,
        type_interner.environment(),
        expected_type,
        string_table,
    )?;

    let (rpn_items, span) = ordering::order_expression_nodes(nodes)?;

    // Fast path: a single R-value needs no operator resolution or RPN assembly.
    if rpn_items.len() == 1 {
        let ExpressionRpnItem::Operand(expression) = &rpn_items[0] else {
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
            rpn_items[0].source_span(),
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
        &rpn_items,
        span,
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

    let stack_span = rpn_items.iter().find_map(|item| match item {
        ExpressionRpnItem::Operand(expression) => expression.span,
        ExpressionRpnItem::Operator { .. } => None,
        // Resolution removes pending literals before this stage.
        ExpressionRpnItem::PendingNumericLiteral { .. } => None,
    });
    // Runtime RPN needs an owned value mode for the final expression node.
    let value_mode = value_mode.as_owned();
    eval_log!("Attempting to Fold: ", Pretty rpn_items);
    increment_frontend_counter(FrontendCounter::ConstantFoldAttemptCount);
    let fold_outcome = constant_fold(rpn_items, string_table, context.numeric_profile)?;
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

/// The one destination-aware literal boundary. Phase 3 peer typing extends this helper.
///
/// WHAT: replaces every deferred numeric literal with a materialised operand. A fragment that
///       is exactly one pending literal with a fixed-scalar (or option-of-fixed) expectation
///       materialises directly in that scalar with no `Int` intermediate; every other pending
///       literal keeps today's default `Int`/`Float` materialisation byte for byte.
/// WHY: direct typed receivers such as `#limit U64 = 18_000_000_000` must see the destination
///      before the literal's range is checked, while operators, casts and peer positions keep
///      their natural types. After this helper no pending item exists downstream.
fn resolve_pending_numeric_literals(
    nodes: Vec<ExpressionRpnItem>,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    expected_type: &ExpectedType,
    string_table: &mut StringTable,
) -> Result<Vec<ExpressionRpnItem>, ExpressionTypingError> {
    if !nodes
        .iter()
        .any(|node| matches!(node, ExpressionRpnItem::PendingNumericLiteral { .. }))
    {
        return Ok(nodes);
    }

    let direct_scalar = direct_fixed_scalar_destination(&nodes, type_environment, expected_type);

    let mut resolved = Vec::with_capacity(nodes.len());
    for node in nodes {
        let ExpressionRpnItem::PendingNumericLiteral {
            token,
            span,
            value_mode,
        } = node
        else {
            resolved.push(node);
            continue;
        };

        if let Some(scalar) = direct_scalar
            && literal_kind_initialises(token.kind, scalar)
        {
            let value = materialize_fixed_scalar(&token, token.sign, scalar, string_table)
                .map_err(|reason| {
                    CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
                })?;

            resolved.push(ExpressionRpnItem::Operand(Expression::fixed_scalar(
                value, span, value_mode,
            )));
            continue;
        }

        resolved.push(ExpressionRpnItem::Operand(default_materialised_literal(
            &token,
            span,
            value_mode,
            context,
            string_table,
        )?));
    }

    Ok(resolved)
}

/// The direct destination when the whole fragment is one literal with a fixed expectation.
///
/// WHAT: returns the fixed scalar only when the parsed nodes are exactly one pending literal
///       and the expected type (or its option inner type) is a fixed scalar.
/// WHY: "direct" means the literal is the entire receiving expression, so an operator result
///      such as `1 + 1` never retags through its receiving annotation.
fn direct_fixed_scalar_destination(
    nodes: &[ExpressionRpnItem],
    type_environment: &TypeEnvironment,
    expected_type: &ExpectedType,
) -> Option<FixedScalar> {
    let [ExpressionRpnItem::PendingNumericLiteral { .. }] = nodes else {
        return None;
    };

    let expected_type_id = expected_type.literal_destination_type_id()?;
    type_environment.fixed_scalar_of(expected_type_id)
}

/// Today's default literal materialisation, unchanged for non-fixed destinations.
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
