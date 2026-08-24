//! Expression evaluation and AST-side constant folding implementation.
//!
//! WHAT: resolves parsed infix expression fragments into typed AST expressions.
//! WHY: AST is the stage that owns operator typing, constant folding, and the decision about
//!      whether an expression can stay compile-time or must survive as runtime RPN.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{constant_fold, fold_compile_time_expression};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidExpressionReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{
    AstCounter, FrontendCounter, increment_ast_counter, increment_frontend_counter,
};
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
/// WHAT: applies shunting-yard ordering, operator type resolution, optional constant folding,
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
) -> Result<Expression, ExpressionTypingError> {
    let (rpn_items, location) = ordering::order_expression_nodes(nodes)?;

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
        )?;

        validate_expression_result_type(
            expected_type,
            only_expression.type_id,
            &rpn_items[0].source_location(),
            type_interner.environment_mut_for_derived_types(),
        )?;

        // References tighten the expected type so later fragments in the same statement
        // can resolve against the inferred target type.
        if let ExpressionKind::Reference(..) = only_expression.kind {
            *expected_type = ExpectedType::Known(only_expression.type_id);
        } else if matches!(expected_type, ExpectedType::Infer) {
            *expected_type = ExpectedType::Known(only_expression.type_id);
        }

        return Ok(only_expression);
    }

    // General path: resolve operator types across the full RPN shape, then attempt folding.
    let resolved_type = resolve_expression_result_type(
        &rpn_items,
        &location,
        string_table,
        type_interner.environment(),
    )?;

    validate_expression_result_type(
        expected_type,
        resolved_type,
        &location,
        type_interner.environment_mut_for_derived_types(),
    )?;

    if matches!(expected_type, ExpectedType::Infer) {
        *expected_type = ExpectedType::Known(resolved_type);
    }

    // Runtime RPN needs an owned value mode for the final expression node.
    let value_mode = value_mode.as_owned();
    eval_log!("Attempting to Fold: ", Pretty rpn_items);
    increment_frontend_counter(FrontendCounter::ConstantFoldAttemptCount);

    let stack = constant_fold(rpn_items, string_table)?;
    increment_frontend_counter(FrontendCounter::ConstantFoldSuccessCount);
    eval_log!("Stack after folding: ", Pretty stack);

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
            location,
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
        location.clone(),
    )?)
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
    location: SourceLocation,
) -> Result<Expression, CompilerError> {
    Ok(Expression::runtime_with_type_id(
        ExpressionRpn { items },
        diagnostic_type,
        type_id,
        location,
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
    location: &SourceLocation,
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
        location.clone(),
    )
    .into())
}
