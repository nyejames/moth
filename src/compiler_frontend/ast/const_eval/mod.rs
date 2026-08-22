//! # AST Constant Evaluation
//!
//! WHAT: folds fully compile-time expression fragments during AST construction.
//! WHY: AST owns semantic compile-time folding so that HIR lowering only sees runtime
//! expressions or already-materialized constant values. Folding is runtime-parity only:
//! this module does not implement arbitrary precision, rational, Decimal, or BigInt const
//! folding.
//!
//! ## Algorithm
//!
//! The constant folder operates on expressions in Reverse Polish Notation (RPN) order:
//! 1. **Stack-Based Evaluation**: Processes operands and operators in RPN order
//! 2. **Immediate Folding**: Evaluates operations only when required operands are foldable literals
//! 3. **Runtime Preservation**: Keeps non-foldable expressions in runtime RPN form
//!
//! ## Supported Operations
//!
//! - **Arithmetic**: Addition, subtraction, multiplication, division for integers and floats
//! - **Boolean**: Logical AND, OR, NOT operations
//! - **Comparison**: Equality, inequality, relational comparisons
//! - **Type Coercion**: Automatic promotion between compatible numeric types

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
#[cfg(test)]
use crate::compiler_frontend::ast::expressions::expression::FallibleCarrierVariant;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator, type_id_hint_for_diagnostic_type,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::casts::{BuiltinCastLiteral, apply_builtin_cast_policy};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

#[derive(Debug)]
pub(crate) enum ConstantFoldError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for ConstantFoldError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ConstantFoldError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for ConstantFoldError {
    fn from(error: CompilerError) -> Self {
        ConstantFoldError::Infrastructure(Box::new(error))
    }
}

impl From<TemplateError> for ConstantFoldError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => ConstantFoldError::Diagnostic(diagnostic),
            TemplateError::Infrastructure(error) => ConstantFoldError::Infrastructure(error),
        }
    }
}

/// Perform conservative constant folding on an expression in RPN order.
///
/// Takes expression-owned RPN items and evaluates all constant sub-expressions at compile time.
/// Returns a simplified expression stack with constant operations pre-computed.
/// Runtime-dependent operands and operators remain in RPN order.
///
/// ## Algorithm
///
/// 1. **Process RPN Stack**: Iterate through nodes in RPN order
/// 2. **Accumulate Operands**: Push operands onto evaluation stack
/// 3. **Evaluate Operators**: When encountering operators, attempt to fold with constant operands
/// 4. **Preserve Runtime**: Non-foldable operations are preserved for runtime evaluation
///
/// ## Error Handling
///
/// Returns [`ConstantFoldError`] so compile-time source failures stay as
/// [`CompilerDiagnostic`] values while malformed internal RPN state remains an
/// infrastructure [`CompilerError`]. This keeps constant-folding diagnostics on the
/// AST-owned user-diagnostic path without hiding compiler invariants.
pub fn constant_fold(
    output_stack: &[ExpressionRpnItem],
    string_table: &mut StringTable,
) -> Result<Vec<ExpressionRpnItem>, ConstantFoldError> {
    // Fold individual constant sub-expressions while leaving runtime-dependent operands and
    // operators in place. This keeps RPN ordering while still reporting statically known
    // numeric failures that happen to sit inside a larger runtime expression.
    add_ast_counter(AstCounter::ExpressionFoldItems, output_stack.len());

    let mut stack: Vec<ExpressionRpnItem> = Vec::with_capacity(output_stack.len());

    for item in output_stack {
        match item {
            ExpressionRpnItem::Operator { operator, location } => {
                let required_values = operator.required_values();
                // Validate stack arity before popping operands so malformed RPN is reported as an
                // internal compiler invariant failure instead of panicking.
                if stack.len() < required_values {
                    return Err(CompilerError::new(
                        format!(
                            "Not enough items on the stack for the {} operator when folding an expression. Starting Stack: {:?}. Stack being folded: {:?}",
                            operator.to_str(),
                            output_stack,
                            stack
                        ),
                        location.to_owned(),
                        ErrorType::Compiler,
                    )
                    .into());
                }

                if matches!(operator, Operator::Not | Operator::Negate) {
                    let operand = stack.pop().expect(
                        "unary operator should have one operand after the stack-length guard",
                    );

                    if let Some(folded) =
                        fold_unary_operator(operator, &operand, string_table, location)?
                    {
                        stack.push(folded);
                    } else {
                        // Keep unary operators as runtime RPN when the operand cannot fold.
                        stack.push(operand);
                        stack.push(item.to_owned());
                    }

                    continue;
                }

                let rhs = stack
                    .pop()
                    .expect("binary operator should have a right operand after the length guard");
                let lhs = stack
                    .pop()
                    .expect("binary operator should have a left operand after the length guard");

                let (lhs_expr, rhs_expr) = match (&lhs, &rhs) {
                    (
                        ExpressionRpnItem::Operand(lhs_expr),
                        ExpressionRpnItem::Operand(rhs_expr),
                    ) if lhs_expr.kind.is_foldable() && rhs_expr.kind.is_foldable() => {
                        (lhs_expr, rhs_expr)
                    }
                    _ => {
                        // Preserve runtime RPN when either side is not foldable.
                        stack.push(lhs);
                        stack.push(rhs);
                        stack.push(item.to_owned());
                        continue;
                    }
                };

                if let Some(result) =
                    lhs_expr.evaluate_operator(rhs_expr, operator, string_table)?
                {
                    stack.push(ExpressionRpnItem::Operand(result));
                } else {
                    // Keep the original operation for runtime lowering when AST cannot fold it.
                    stack.push(lhs);
                    stack.push(rhs);
                    stack.push(item.to_owned());
                    continue;
                }
            }

            operand @ ExpressionRpnItem::Operand(_) => {
                increment_ast_counter(AstCounter::ExpressionOperandClones);
                stack.push(operand.to_owned());
            }
        }
    }

    Ok(stack)
}

fn fold_unary_operator(
    op: &Operator,
    operand: &ExpressionRpnItem,
    string_table: &mut StringTable,
    operator_location: &SourceLocation,
) -> Result<Option<ExpressionRpnItem>, ConstantFoldError> {
    let ExpressionRpnItem::Operand(expression) = operand else {
        return Ok(None);
    };

    let folded_expression = match (op, &expression.kind) {
        (Operator::Not, ExpressionKind::Bool(value)) => Expression::bool(
            !value,
            expression.location.clone(),
            expression.value_mode.to_owned(),
        ),

        (Operator::Negate, ExpressionKind::Int(value)) => {
            let Some(negated) = value.checked_neg() else {
                integer_overflow_error(op, string_table, operator_location)?;
                return Ok(None);
            };
            Expression::int(
                negated,
                expression.location.clone(),
                expression.value_mode.to_owned(),
            )
        }

        (Operator::Negate, ExpressionKind::Float(value)) => Expression::float(
            match checked_float_result(-value, op, string_table, operator_location)? {
                ExpressionKind::Float(value) => value,
                _ => return Ok(None),
            },
            expression.location.clone(),
            expression.value_mode.to_owned(),
        ),

        _ => return Ok(None),
    };

    let folded_expression = folded_expression
        .with_synthetic_interface_provenance(expression.synthetic_interface_provenance.clone());

    Ok(Some(ExpressionRpnItem::Operand(folded_expression)))
}

/// Folds a typed expression that has a dedicated AST const-eval path.
///
/// `template_ir_store` is the shared module-local store from the caller's `ScopeContext`.
/// Catch-handler templates retain their exact TIR reference so const classification reads
/// their effective view instead of reconstructing template structure.
pub fn fold_compile_time_expression(
    expression: &Expression,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
    constant_context: bool,
) -> Result<Expression, ConstantFoldError> {
    match &expression.kind {
        ExpressionKind::Cast(cast) => {
            let folded_source = fold_compile_time_expression(
                &cast.source,
                template_ir_store,
                string_table,
                constant_context,
            )?;
            fold_resolved_cast(
                expression,
                cast,
                &folded_source,
                template_ir_store,
                string_table,
                constant_context,
                None,
            )
        }
        ExpressionKind::HandledFallibleExpression {
            value, handling, ..
        } => {
            let folded_value = fold_compile_time_expression(
                value,
                template_ir_store,
                string_table,
                constant_context,
            )?;

            match &folded_value.kind {
                #[cfg(test)]
                ExpressionKind::FallibleCarrierConstruct {
                    variant: FallibleCarrierVariant::Success,
                    value,
                } => Ok(value.as_ref().to_owned()),
                #[cfg(test)]
                ExpressionKind::FallibleCarrierConstruct {
                    variant: FallibleCarrierVariant::Error,
                    ..
                } => Ok(Expression::handled_result_with_type_id(
                    folded_value,
                    handling.to_owned(),
                    expression.type_id,
                    expression.diagnostic_type.to_owned(),
                    expression.location.clone(),
                )),
                _ => Ok(Expression::handled_result_with_type_id(
                    folded_value,
                    handling.to_owned(),
                    expression.type_id,
                    expression.diagnostic_type.to_owned(),
                    expression.location.clone(),
                )),
            }
        }
        ExpressionKind::ValueBlock { block } => match block.as_ref() {
            ValueBlock::Catch(value_catch) => {
                let FallibleHandling::Handler { body, .. } = &value_catch.handler else {
                    return Ok(expression.to_owned());
                };

                let ExpressionKind::Cast(cast) = &value_catch.handled_value.kind else {
                    return Ok(expression.to_owned());
                };

                let folded_source = fold_compile_time_expression(
                    &cast.source,
                    template_ir_store,
                    string_table,
                    constant_context,
                )?;

                fold_resolved_cast(
                    expression,
                    cast,
                    &folded_source,
                    template_ir_store,
                    string_table,
                    constant_context,
                    Some(body),
                )
            }

            ValueBlock::If(_) | ValueBlock::Match(_) => Ok(expression.to_owned()),
        },
        _ => Ok(expression.to_owned()),
    }
}

/// Folds a resolved explicit `ExpressionKind::Cast` when its source has folded to
/// a supported builtin literal.
///
/// WHAT: builtin evidence is evaluated here; user-defined or generic-bound
///      evidence is rejected in const-required contexts because the compiler
///      cannot execute user code or validate an unresolved generic bound at
///      compile time.
/// WHY: keeping this logic in the AST const-eval owner means HIR lowering only sees
///      runtime casts that could not be folded away.
fn fold_resolved_cast(
    original_expression: &Expression,
    cast: &ResolvedCastExpression,
    folded_source: &Expression,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
    constant_context: bool,
    recovery_handler_body: Option<&[AstNode]>,
) -> Result<Expression, ConstantFoldError> {
    match &cast.evidence {
        ResolvedCastEvidence::Builtin { policy } => {
            if !policy.is_const_foldable() {
                if constant_context {
                    return Err(CompilerDiagnostic::invalid_cast(
                        InvalidCastReason::BuiltinEvidenceNotConstFoldable,
                        Some(cast.source_type_id),
                        Some(cast.target_type_id),
                        original_expression.location.clone(),
                    )
                    .into());
                }

                return Ok(original_expression.to_owned());
            }

            let source_literal =
                match builtin_cast_literal_from_expression(folded_source, string_table) {
                    Some(literal) => literal,
                    None => return Ok(original_expression.to_owned()),
                };

            match apply_builtin_cast_policy(*policy, &source_literal) {
                Ok(folded_literal) => {
                    let Some(mut folded_expression) = builtin_cast_expression_from_literal(
                        &folded_literal,
                        &folded_source.location,
                        string_table,
                    ) else {
                        return Ok(original_expression.to_owned());
                    };

                    if cast.requires_optional_wrap_after_cast {
                        folded_expression =
                            Expression::coerced(folded_expression, original_expression.type_id);
                    }

                    folded_expression = folded_expression.with_synthetic_interface_provenance(
                        folded_source.synthetic_interface_provenance.clone(),
                    );

                    Ok(folded_expression)
                }
                Err(_) if !constant_context => Ok(original_expression.to_owned()),
                Err(_) => {
                    // A const-required fallible cast with a local recovery handler should
                    // fold to the handler's produced value when the source folded but the
                    // builtin policy failed. If the handler itself cannot fold, report that
                    // as a separate diagnostic so the user knows the recovery path is the
                    // remaining obstacle.
                    if let Some(handler_body) = recovery_handler_body
                        && let Some(folded_handler) = fold_cast_recovery_handler(
                            handler_body,
                            cast.target_type_id,
                            cast.requires_optional_wrap_after_cast,
                            original_expression.type_id,
                            &original_expression.location,
                            template_ir_store,
                            string_table,
                        )?
                    {
                        let recovery_provenance = folded_source
                            .synthetic_interface_provenance
                            .union(&folded_handler.synthetic_interface_provenance);
                        let folded_recovery =
                            folded_handler.with_synthetic_interface_provenance(recovery_provenance);
                        return Ok(folded_recovery);
                    }

                    Err(CompilerDiagnostic::invalid_cast(
                        InvalidCastReason::BuiltinCastFailedInConst,
                        Some(cast.source_type_id),
                        Some(cast.target_type_id),
                        original_expression.location.clone(),
                    )
                    .into())
                }
            }
        }

        ResolvedCastEvidence::UserDefined { .. } => {
            if constant_context {
                return Err(CompilerDiagnostic::invalid_cast(
                    InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
                    Some(cast.source_type_id),
                    Some(cast.target_type_id),
                    original_expression.location.clone(),
                )
                .into());
            }

            Ok(original_expression.to_owned())
        }

        ResolvedCastEvidence::GenericBound { .. } => {
            if constant_context {
                return Err(CompilerDiagnostic::invalid_cast(
                    InvalidCastReason::GenericBoundEvidenceNotConstFoldable,
                    Some(cast.source_type_id),
                    Some(cast.target_type_id),
                    original_expression.location.clone(),
                )
                .into());
            }

            Ok(original_expression.to_owned())
        }
    }
}

/// Folds a `cast ... catch:` handler body to its produced value in a const-required context.
///
/// WHAT: when a builtin cast failed at compile time, the handler body is the only remaining
///      source for the result. This helper extracts the single produced value, folds it, and
///      returns it if it collapsed to a compile-time value. If the handler cannot be folded, it
///      reports a dedicated diagnostic so the failure is attributed to the recovery path, not the
///      cast.
/// WHY: keeping this small and local to the AST const-eval owner means HIR lowering does not need to
///      interpret general catch handler bodies at compile time.
fn fold_cast_recovery_handler(
    handler_body: &[AstNode],
    target_type_id: TypeId,
    requires_optional_wrap_after_cast: bool,
    result_type_id: TypeId,
    diagnostic_location: &SourceLocation,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
) -> Result<Option<Expression>, ConstantFoldError> {
    let Some(handler_expression) = extract_single_produced_value(handler_body) else {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::CatchHandlerNotConstFoldable,
            None,
            Some(target_type_id),
            diagnostic_location.to_owned(),
        )
        .into());
    };

    let folded_handler =
        fold_compile_time_expression(handler_expression, template_ir_store, string_table, true)?;

    let handler_is_compile_time_constant = folded_handler
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, template_ir_store)
        })?
        .is_compile_time_value();
    if !handler_is_compile_time_constant {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::CatchHandlerNotConstFoldable,
            None,
            Some(target_type_id),
            folded_handler.location,
        )
        .into());
    }

    let mut result = folded_handler;
    if requires_optional_wrap_after_cast {
        result = Expression::coerced(result, result_type_id);
    }

    Ok(Some(result))
}

/// Extracts a direct single-value `then` expression from a value-producing body.
///
/// WHAT: catch handlers for cast recovery must produce exactly one value in a shape the constant
///      folder can evaluate without interpreting statements or control-flow conditions.
/// WHY: nested `if` or `match` handlers require real const statement evaluation to choose the
///      executed branch. Until that owner exists, the safe frontend behavior is to reject those
///      handlers in const-required casts instead of guessing at the first branch.
fn extract_single_produced_value(body: &[AstNode]) -> Option<&Expression> {
    for node in body {
        match &node.kind {
            NodeKind::ThenValue(produced_values) if produced_values.expressions.len() == 1 => {
                return Some(&produced_values.expressions[0]);
            }

            NodeKind::If(..)
            | NodeKind::Match { .. }
            | NodeKind::ScopedBlock { .. }
            | NodeKind::RangeLoop { .. }
            | NodeKind::CollectionLoop { .. }
            | NodeKind::WhileLoop(..)
            | NodeKind::Return(_)
            | NodeKind::ReturnError(_) => return None,

            _ => {}
        }
    }

    None
}

/// Converts an AST `Expression` into a `BuiltinCastLiteral` for policy lookup.
///
/// WHAT: extracts the literal scalar value from supported `ExpressionKind`
///      variants so the policy owner can answer in policy space.
/// WHY: explicit casts and direct policy tests share the same policy table, so this
/// narrow converter is reused for any folded scalar source that the builtin
/// evidence catalogue accepts.
fn builtin_cast_literal_from_expression(
    value: &Expression,
    string_table: &StringTable,
) -> Option<BuiltinCastLiteral> {
    match &value.kind {
        ExpressionKind::Bool(value) => Some(BuiltinCastLiteral::Bool(*value)),
        ExpressionKind::Int(int) => Some(BuiltinCastLiteral::Int(*int)),
        ExpressionKind::Float(float) => Some(BuiltinCastLiteral::Float(*float)),
        ExpressionKind::StringSlice(string) => Some(BuiltinCastLiteral::String(
            string_table.resolve(*string).to_owned(),
        )),
        ExpressionKind::Char(value) => Some(BuiltinCastLiteral::Char(*value)),
        _ => None,
    }
}

/// Builds an `Expression` literal from a `BuiltinCastLiteral`.
fn builtin_cast_expression_from_literal(
    literal: &BuiltinCastLiteral,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Option<Expression> {
    match literal {
        BuiltinCastLiteral::Bool(value) => Some(Expression::bool(
            *value,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Int(value) => Some(Expression::int(
            *value,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Float(value) => Some(Expression::float(
            *value,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::String(value) => Some(Expression::string_slice(
            string_table.get_or_intern(value.to_owned()),
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Char(value) => Some(Expression::char(
            *value,
            location.clone(),
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Error { .. } => None,
    }
}

fn compile_time_evaluation_diagnostic(
    reason: CompileTimeEvaluationErrorReason,
    operation: Option<String>,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> ConstantFoldError {
    let operation = operation.map(|operation| string_table.get_or_intern(operation));

    CompilerDiagnostic::compile_time_evaluation_error(reason, operation, location.to_owned()).into()
}

fn integer_overflow_error(
    op: &Operator,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some(op.to_str().to_string()),
        string_table,
        location,
    ))
}

fn float_non_finite_error(
    op: &Operator,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some(op.to_str().to_string()),
        string_table,
        location,
    ))
}

fn checked_float_result(
    value: f64,
    op: &Operator,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    if value.is_finite() {
        Ok(ExpressionKind::Float(value))
    } else {
        float_non_finite_error(op, string_table, location)
    }
}

fn checked_int_binary_result(
    lhs: i32,
    rhs: i32,
    op: &Operator,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    let checked = match op {
        Operator::Add => lhs.checked_add(rhs),
        Operator::Subtract => lhs.checked_sub(rhs),
        Operator::Multiply => lhs.checked_mul(rhs),
        Operator::IntDivide => lhs.checked_div(rhs),
        Operator::Modulus => lhs.checked_rem(rhs),
        // Non-negative exponents are guaranteed by the caller; `rhs` is already validated as i32.
        Operator::Exponent => lhs.checked_pow(rhs as u32),
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "Checked integer folding does not support '{}'",
                op.to_str()
            ))
            .into());
        }
    };

    match checked {
        Some(value) => Ok(ExpressionKind::Int(value)),
        None => integer_overflow_error(op, string_table, location),
    }
}

fn divide_by_zero_error(
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        string_table,
        location,
    ))
}

fn integer_division_operand_error(
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::IntegerDivisionOnlyIntInt,
        None,
        string_table,
        location,
    ))
}

fn invalid_operator_for_compile_time_type(
    op: &Operator,
    string_table: &mut StringTable,
    location: &SourceLocation,
) -> Result<ExpressionKind, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::InvalidOperatorForType,
        Some(op.to_str().to_string()),
        string_table,
        location,
    ))
}

impl Expression {
    // Evaluates a binary operation between two expressions based on the operator
    // This helps with constant folding by handling type-specific operations
    pub(crate) fn evaluate_operator(
        &self,
        rhs: &Expression,
        op: &Operator,
        string_table: &mut StringTable,
    ) -> Result<Option<Expression>, ConstantFoldError> {
        let kind: ExpressionKind = match (&self.kind, &rhs.kind) {
            // Float operations: Moth `Float` is finite f64. Require finite results and
            // report divide/modulo-by-zero explicitly instead of relying on NaN/Inf classification.
            (ExpressionKind::Float(lhs_val), ExpressionKind::Float(rhs_val)) => match op {
                Operator::Add => {
                    checked_float_result(lhs_val + rhs_val, op, string_table, &self.location)?
                }
                Operator::Subtract => {
                    checked_float_result(lhs_val - rhs_val, op, string_table, &self.location)?
                }
                Operator::Multiply => {
                    checked_float_result(lhs_val * rhs_val, op, string_table, &self.location)?
                }
                Operator::Divide => {
                    if *rhs_val == 0.0 {
                        divide_by_zero_error(string_table, &self.location)?
                    } else {
                        checked_float_result(lhs_val / rhs_val, op, string_table, &self.location)?
                    }
                }
                Operator::Modulus => {
                    if *rhs_val == 0.0 {
                        divide_by_zero_error(string_table, &self.location)?
                    } else {
                        checked_float_result(lhs_val % rhs_val, op, string_table, &self.location)?
                    }
                }
                Operator::Exponent => {
                    checked_float_result(lhs_val.powf(*rhs_val), op, string_table, &self.location)?
                }

                // Logical operations with float operands
                Operator::Equality => ExpressionKind::Bool(lhs_val == rhs_val),
                Operator::NotEqual => ExpressionKind::Bool(lhs_val != rhs_val),
                Operator::GreaterThan => ExpressionKind::Bool(lhs_val > rhs_val),
                Operator::GreaterThanOrEqual => ExpressionKind::Bool(lhs_val >= rhs_val),
                Operator::LessThan => ExpressionKind::Bool(lhs_val < rhs_val),
                Operator::LessThanOrEqual => ExpressionKind::Bool(lhs_val <= rhs_val),

                // Other operations are not applicable to floats.
                _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
            },

            // Integer operations use checked i32 arithmetic so compile-time folding stays
            // equivalent to the Alpha runtime `Int` contract.
            (ExpressionKind::Int(lhs_val), ExpressionKind::Int(rhs_val)) => {
                match op {
                    Operator::Add | Operator::Subtract | Operator::Multiply => {
                        checked_int_binary_result(
                            *lhs_val,
                            *rhs_val,
                            op,
                            string_table,
                            &self.location,
                        )?
                    }
                    Operator::Divide => {
                        if *rhs_val == 0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_float_result(
                                f64::from(*lhs_val) / f64::from(*rhs_val),
                                op,
                                string_table,
                                &self.location,
                            )?
                        }
                    }
                    Operator::IntDivide => {
                        if *rhs_val == 0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_int_binary_result(
                                *lhs_val,
                                *rhs_val,
                                op,
                                string_table,
                                &self.location,
                            )?
                        }
                    }
                    Operator::Modulus => {
                        if *rhs_val == 0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_int_binary_result(
                                *lhs_val,
                                *rhs_val,
                                op,
                                string_table,
                                &self.location,
                            )?
                        }
                    }
                    Operator::Exponent => {
                        if *rhs_val < 0 {
                            return Err(compile_time_evaluation_diagnostic(
                                CompileTimeEvaluationErrorReason::InvalidExponent,
                                Some(op.to_str().to_string()),
                                string_table,
                                &self.location,
                            ));
                        }
                        checked_int_binary_result(
                            *lhs_val,
                            *rhs_val,
                            op,
                            string_table,
                            &self.location,
                        )?
                    }

                    // Logical operations with integer operands
                    Operator::Equality => ExpressionKind::Bool(lhs_val == rhs_val),
                    Operator::NotEqual => ExpressionKind::Bool(lhs_val != rhs_val),
                    Operator::GreaterThan => ExpressionKind::Bool(lhs_val > rhs_val),
                    Operator::GreaterThanOrEqual => ExpressionKind::Bool(lhs_val >= rhs_val),
                    Operator::LessThan => ExpressionKind::Bool(lhs_val < rhs_val),
                    Operator::LessThanOrEqual => ExpressionKind::Bool(lhs_val <= rhs_val),

                    Operator::Range => ExpressionKind::Range(
                        Box::new(Expression::int(
                            *lhs_val,
                            self.location.to_owned(),
                            ValueMode::ImmutableOwned,
                        )),
                        Box::new(Expression::int(
                            *rhs_val,
                            self.location.to_owned(),
                            ValueMode::ImmutableOwned,
                        )),
                    ),

                    _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
                }
            }

            // Mixed Int/Float operations promote the i32 operand to Float, then require a finite
            // f64 result.
            (ExpressionKind::Int(lhs_val), ExpressionKind::Float(rhs_val)) => {
                let lhs = f64::from(*lhs_val);
                match op {
                    Operator::Add => {
                        checked_float_result(lhs + rhs_val, op, string_table, &self.location)?
                    }
                    Operator::Subtract => {
                        checked_float_result(lhs - rhs_val, op, string_table, &self.location)?
                    }
                    Operator::Multiply => {
                        checked_float_result(lhs * rhs_val, op, string_table, &self.location)?
                    }
                    Operator::Divide => {
                        if *rhs_val == 0.0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_float_result(lhs / rhs_val, op, string_table, &self.location)?
                        }
                    }
                    Operator::Modulus => {
                        if *rhs_val == 0.0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_float_result(lhs % rhs_val, op, string_table, &self.location)?
                        }
                    }
                    Operator::Exponent => {
                        checked_float_result(lhs.powf(*rhs_val), op, string_table, &self.location)?
                    }
                    Operator::Equality => ExpressionKind::Bool(lhs == *rhs_val),
                    Operator::NotEqual => ExpressionKind::Bool(lhs != *rhs_val),
                    Operator::GreaterThan => ExpressionKind::Bool(lhs > *rhs_val),
                    Operator::GreaterThanOrEqual => ExpressionKind::Bool(lhs >= *rhs_val),
                    Operator::LessThan => ExpressionKind::Bool(lhs < *rhs_val),
                    Operator::LessThanOrEqual => ExpressionKind::Bool(lhs <= *rhs_val),
                    Operator::IntDivide => {
                        integer_division_operand_error(string_table, &self.location)?
                    }
                    _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
                }
            }

            (ExpressionKind::Float(lhs_val), ExpressionKind::Int(rhs_val)) => {
                let rhs = f64::from(*rhs_val);
                match op {
                    Operator::Add => {
                        checked_float_result(lhs_val + rhs, op, string_table, &self.location)?
                    }
                    Operator::Subtract => {
                        checked_float_result(lhs_val - rhs, op, string_table, &self.location)?
                    }
                    Operator::Multiply => {
                        checked_float_result(lhs_val * rhs, op, string_table, &self.location)?
                    }
                    Operator::Divide => {
                        if *rhs_val == 0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_float_result(lhs_val / rhs, op, string_table, &self.location)?
                        }
                    }
                    Operator::Modulus => {
                        if *rhs_val == 0 {
                            divide_by_zero_error(string_table, &self.location)?
                        } else {
                            checked_float_result(lhs_val % rhs, op, string_table, &self.location)?
                        }
                    }
                    Operator::Exponent => {
                        checked_float_result(lhs_val.powf(rhs), op, string_table, &self.location)?
                    }
                    Operator::Equality => ExpressionKind::Bool(*lhs_val == rhs),
                    Operator::NotEqual => ExpressionKind::Bool(*lhs_val != rhs),
                    Operator::GreaterThan => ExpressionKind::Bool(*lhs_val > rhs),
                    Operator::GreaterThanOrEqual => ExpressionKind::Bool(*lhs_val >= rhs),
                    Operator::LessThan => ExpressionKind::Bool(*lhs_val < rhs),
                    Operator::LessThanOrEqual => ExpressionKind::Bool(*lhs_val <= rhs),
                    Operator::IntDivide => {
                        integer_division_operand_error(string_table, &self.location)?
                    }
                    _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
                }
            }

            // Boolean operations
            (ExpressionKind::Bool(lhs_val), ExpressionKind::Bool(rhs_val)) => match op {
                Operator::And => ExpressionKind::Bool(*lhs_val && *rhs_val),
                Operator::Or => ExpressionKind::Bool(*lhs_val || *rhs_val),
                Operator::Equality => ExpressionKind::Bool(lhs_val == rhs_val),
                Operator::NotEqual => ExpressionKind::Bool(lhs_val != rhs_val),

                _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
            },

            // String operations
            (ExpressionKind::StringSlice(lhs_val), ExpressionKind::StringSlice(rhs_val)) => {
                match op {
                    Operator::Equality => ExpressionKind::Bool(lhs_val == rhs_val),
                    Operator::NotEqual => ExpressionKind::Bool(lhs_val != rhs_val),
                    _ => invalid_operator_for_compile_time_type(op, string_table, &self.location)?,
                }
            }
            // Any other combination of types
            _ => return Ok(None),
        };

        let value_mode = if self.value_mode.is_mutable() || rhs.value_mode.is_mutable() {
            ValueMode::MutableOwned
        } else {
            ValueMode::ImmutableOwned
        };
        let contains_regular_division = self.contains_regular_division
            || rhs.contains_regular_division
            || matches!(op, Operator::Divide);

        let result_type = match &kind {
            ExpressionKind::Int(_) => DataType::Int,
            ExpressionKind::Float(_) => DataType::Float,
            ExpressionKind::Bool(_) => DataType::Bool,
            ExpressionKind::StringSlice(_) => DataType::StringSlice,
            ExpressionKind::Range(_, _) => DataType::Range,
            ExpressionKind::Char(_) => DataType::Char,
            _ => self.diagnostic_type.to_owned(),
        };

        let folded_provenance = self
            .synthetic_interface_provenance
            .union(&rhs.synthetic_interface_provenance);

        let mut result_expression = Expression::new(
            kind,
            self.location.to_owned(),
            type_id_hint_for_diagnostic_type(&result_type),
            result_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division);
        result_expression.synthetic_interface_provenance = folded_provenance;

        Ok(Some(result_expression))
    }
}

#[cfg(test)]
#[path = "tests/constant_folding_tests.rs"]
mod tests;
