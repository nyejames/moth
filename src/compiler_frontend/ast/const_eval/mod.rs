//! # AST Constant Evaluation
//!
//! WHAT: folds fully compile-time expression fragments during AST construction.
//! WHY: AST owns semantic compile-time folding so that HIR lowering only sees runtime
//! expressions or already-materialized constant values. Folding is runtime-parity only and
//! performs exact Dec arithmetic through the shared NumberValue owner.
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
//! - **Arithmetic**: Checked Int, Uint, Float and fixed-width numeric arithmetic, plus exact Dec
//!   arithmetic, with domain-specific rounding
//! - **Boolean**: Logical AND, OR, NOT operations
//! - **Comparison**: Exact integer and Dec ordering, numeric float comparisons, and equality
//! - **Promotion**: Shared operator-domain rules keep compile-time results aligned with typed
//!   operations

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::eval_expression::pending_expression_item_bug;
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
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticPayload, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::{NumberArithmeticError, NumberValue};
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, comparison_supported, negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_power;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::NumericProfile;

#[derive(Debug)]
pub(crate) enum ConstantFoldError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstStringRequirement {
    EqualityComparison,
    CastOrParse,
    CompileTimeMapKey,
    DuplicateKeyValidation,
}

#[derive(Debug)]
pub(crate) enum ConstantFoldOutcome {
    Folded(Vec<ExpressionRpnItem>),
    NotConstant(Vec<ExpressionRpnItem>),
    TextUnavailable {
        items: Vec<ExpressionRpnItem>,
        diagnostic: CompilerDiagnostic,
    },
}

/// WHY: `Folded` carries an AST `Expression`, which is large next to the two refusal variants.
/// Boxing it would allocate on the hot fold path that every constant expression takes, so this
/// follows the existing `NodeKind` and `ExpressionRpnItem` convention of allowing the disparity.
#[allow(
    clippy::large_enum_variant,
    reason = "Folded holds an AST Expression on the hot fold path; boxing would allocate per fold"
)]
#[derive(Debug)]
pub(crate) enum OperatorFoldOutcome {
    Folded(Expression),
    NotConstant,
    TextUnavailable { diagnostic: CompilerDiagnostic },
}

impl ConstStringRequirement {
    fn operation_name(self) -> &'static str {
        match self {
            Self::EqualityComparison => "string equality comparison",
            Self::CastOrParse => "string cast or parse",
            Self::CompileTimeMapKey => "compile-time map key",
            Self::DuplicateKeyValidation => "duplicate map-key validation",
        }
    }
}

/// Apply the one policy for operations that need concrete characters from a folded string.
///
/// WHAT: returns the interned text for plain strings and all-text structural values, while
///       refusing structural values that still contain a resource or site-root URL piece.
/// WHY: callers must not independently inspect `ExpressionKind` to decide whether a folded string
///      can participate in a character-dependent operation.
pub(crate) fn require_concrete_text(
    value: &Expression,
    requirement: ConstStringRequirement,
    string_table: &mut StringTable,
) -> Result<Option<StringId>, CompilerDiagnostic> {
    match &value.kind {
        ExpressionKind::StringSlice(string) => Ok(Some(*string)),
        ExpressionKind::StructuralString { pieces } => {
            let mut text = String::new();
            for piece in pieces {
                match piece {
                    ConstStringPiece::Text(string) => {
                        text.push_str(string_table.resolve(*string));
                    }
                    ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                        let operation =
                            string_table.get_or_intern(requirement.operation_name().to_owned());
                        return Err(CompilerDiagnostic::compile_time_evaluation_error(
                            CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
                            Some(operation),
                            value.span,
                            None,
                        ));
                    }
                }
            }
            Ok(Some(string_table.get_or_intern(text)))
        }
        _ => Ok(None),
    }
}

impl From<CompilerDiagnostic> for ConstantFoldError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ConstantFoldError::Diagnostic(diagnostic)
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
/// Takes expression-owned RPN items and evaluates all constant sub-expressions during AST
/// construction. The explicit outcome distinguishes a runtime-dependent expression from a
/// folded string operation whose characters are unavailable until URL contexts are assigned.
///
/// ## Algorithm
///
/// 1. **Process RPN Stack**: Iterate through nodes in RPN order
/// 2. **Accumulate Operands**: Push operands onto evaluation stack
/// 3. **Evaluate Operators**: Attempt to fold operators with constant operands
/// 4. **Preserve Runtime**: Return [`ConstantFoldOutcome::NotConstant`] when folding cannot decide
/// 5. **Carry Structure**: Return [`ConstantFoldOutcome::TextUnavailable`] when a constant string
///    needs final URL-dependent text
///
/// ## Error Handling
///
/// Returns [`ConstantFoldError`] so compile-time source failures stay as
/// [`CompilerDiagnostic`] values while malformed internal RPN state remains an
/// infrastructure [`CompilerError`]. A text-unavailable outcome is not an error for speculative
/// callers because runtime lowering resolves the structural pieces later.
/// `scope_context`, when available, resolves retained source-`#Config` provenance for numeric
/// failure labels; speculative callers without that owner pass `None`.
pub fn constant_fold(
    output_stack: Vec<ExpressionRpnItem>,
    string_table: &mut StringTable,
    numeric_profile: NumericProfile,
    scope_context: Option<&ScopeContext>,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    #[cfg(feature = "benchmark_counters")]
    let input_item_count = {
        increment_frontend_counter(FrontendCounter::CensusConstantFoldEntries);
        output_stack.len()
    };
    // Fold individual constant sub-expressions while leaving runtime-dependent operands and
    // operators in place. This keeps RPN ordering while still reporting statically known
    // numeric failures that happen to sit inside a larger runtime expression.
    add_ast_counter(AstCounter::ExpressionFoldItems, output_stack.len());

    let mut stack: Vec<ExpressionRpnItem> = Vec::with_capacity(output_stack.len());

    // A text-unavailable refusal must not stop the fold: the remaining items still belong to the
    // runtime expression, and returning early would silently truncate legal code such as
    // `(@/ == "x") and flag`. The first refusal is carried to the end alongside the complete stack.
    let mut text_unavailable: Option<CompilerDiagnostic> = None;

    // The input vector is consumed: every item either moves onto the fold stack unchanged or is
    // replaced by the value it folded to. Nothing here copies an `Expression`.
    for item in output_stack {
        // Pending syntax never survives evaluation; constant folding consumes only completed
        // RPN, so an escaped pending item is a broken compiler invariant on the existing
        // infrastructure lane rather than a foldable or preservable operand.
        if matches!(
            item,
            ExpressionRpnItem::PendingNumericLiteral { .. }
                | ExpressionRpnItem::PendingGroup { .. }
        ) {
            return Err(pending_expression_item_bug("constant folding").into());
        }
        // Operands move straight onto the stack; only operators need inspection.
        let ExpressionRpnItem::Operator { operator, span, .. } = &item else {
            stack.push(item);
            continue;
        };

        let required_values = operator.required_values();
        // Validate stack arity before popping operands so malformed RPN is reported as an
        // internal compiler invariant failure instead of panicking.
        if stack.len() < required_values {
            return Err(CompilerError::new(
                format!(
                    "Not enough items on the stack for the {} operator when folding an expression. Stack being folded: {:?}",
                    operator.to_str(),
                    stack
                ),
                *span,
                ErrorType::Compiler,
            )
            .into());
        }

        if matches!(operator, Operator::Not | Operator::Negate) {
            let operand = stack
                .pop()
                .expect("unary operator should have one operand after the stack-length guard");

            let folded =
                match fold_unary_operator(operator, &operand, string_table, *span, numeric_profile)
                {
                    Ok(folded) => folded,
                    Err(ConstantFoldError::Diagnostic(mut diagnostic)) => {
                        if let ExpressionRpnItem::Operand(expression) = &operand {
                            attach_source_config_origin_labels(
                                &mut diagnostic,
                                &[&expression.synthetic_interface_provenance],
                                string_table,
                                scope_context,
                            )?;
                        }
                        return Err(ConstantFoldError::Diagnostic(diagnostic));
                    }
                    Err(error) => return Err(error),
                };

            if let Some(folded) = folded {
                stack.push(folded);
            } else {
                // Keep unary operators as runtime RPN when the operand cannot fold.
                stack.push(operand);
                stack.push(item);
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
            (ExpressionRpnItem::Operand(lhs_expr), ExpressionRpnItem::Operand(rhs_expr))
                if lhs_expr.kind.is_foldable() && rhs_expr.kind.is_foldable() =>
            {
                (lhs_expr, rhs_expr)
            }
            _ => {
                // Preserve runtime RPN when either side is not foldable.
                stack.push(lhs);
                stack.push(rhs);
                stack.push(item);
                continue;
            }
        };

        let outcome = match lhs_expr.evaluate_operator(
            rhs_expr,
            operator,
            string_table,
            numeric_profile,
            *span,
        ) {
            Ok(outcome) => outcome,
            Err(ConstantFoldError::Diagnostic(mut diagnostic)) => {
                attach_source_config_origin_labels(
                    &mut diagnostic,
                    &[
                        &lhs_expr.synthetic_interface_provenance,
                        &rhs_expr.synthetic_interface_provenance,
                    ],
                    string_table,
                    scope_context,
                )?;
                return Err(ConstantFoldError::Diagnostic(diagnostic));
            }
            Err(error) => return Err(error),
        };

        match outcome {
            OperatorFoldOutcome::Folded(result) => {
                stack.push(ExpressionRpnItem::Operand(result));
            }

            OperatorFoldOutcome::NotConstant => {
                // Keep the original operation for runtime lowering when AST cannot fold it.
                stack.push(lhs);
                stack.push(rhs);
                stack.push(item);
                continue;
            }

            OperatorFoldOutcome::TextUnavailable { diagnostic } => {
                // Runtime lowering still receives the original operation. Const-required
                // callers inspect this typed refusal and preserve its precise diagnostic.
                stack.push(lhs);
                stack.push(rhs);
                stack.push(item);
                text_unavailable.get_or_insert(diagnostic);
                continue;
            }
        }
    }

    // Reduction is moved/discarded RPN items, not retained bytes or clone traffic.
    #[cfg(feature = "benchmark_counters")]
    crate::compiler_frontend::instrumentation::add_frontend_counter(
        FrontendCounter::CensusFoldReducedRpnItems,
        input_item_count.saturating_sub(stack.len()),
    );
    // Unlike historical ConstantFoldSuccessCount, these distinguish every
    // successful outcome at the folder owner, including speculative callers.

    if let Some(diagnostic) = text_unavailable {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusFoldOutcomeTextUnavailable);
        Ok(ConstantFoldOutcome::TextUnavailable {
            items: stack,
            diagnostic,
        })
    } else if stack.len() == 1 && matches!(&stack[0], ExpressionRpnItem::Operand(_)) {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusFoldOutcomeFolded);
        Ok(ConstantFoldOutcome::Folded(stack))
    } else {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusFoldOutcomeRuntime);
        Ok(ConstantFoldOutcome::NotConstant(stack))
    }
}

fn attach_source_config_origin_labels(
    diagnostic: &mut CompilerDiagnostic,
    provenances: &[&SyntheticInterfaceProvenance],
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> Result<(), ConstantFoldError> {
    if !matches!(
        &diagnostic.payload,
        DiagnosticPayload::CompileTimeEvaluationError {
            numeric_profile: Some(_),
            ..
        }
    ) {
        return Ok(());
    }

    let mut input_names = provenances
        .iter()
        .flat_map(|provenance| provenance.source_config_member_names())
        .collect::<Vec<_>>();
    input_names.sort_unstable();
    input_names.dedup();
    if input_names.is_empty() {
        return Ok(());
    }

    let Some(scope_context) = scope_context else {
        return Err(CompilerError::compiler_error(
            "A numeric compile-time diagnostic retained source #Config provenance without its owning scope resolver.",
        )
        .into());
    };

    for input_name in input_names {
        let Some((origin, Some(label_span))) =
            scope_context.source_config_diagnostic_origin(input_name)
        else {
            return Err(CompilerError::compiler_error(format!(
                "A numeric compile-time diagnostic retained source #Config input '{input_name}' without a matching resolver origin and source anchor."
            ))
            .into());
        };
        diagnostic.labels.push(DiagnosticLabel::secondary(
            Some(label_span),
            Some(DiagnosticLabelMessage::ConfigInputOrigin {
                input_name: string_table.intern(input_name),
                origin,
            }),
        ));
    }

    Ok(())
}

fn fold_unary_operator(
    op: &Operator,
    operand: &ExpressionRpnItem,
    string_table: &mut StringTable,
    operator_span: Option<SourceSpan>,
    numeric_profile: NumericProfile,
) -> Result<Option<ExpressionRpnItem>, ConstantFoldError> {
    let ExpressionRpnItem::Operand(expression) = operand else {
        return Ok(None);
    };

    let mut failure_context = NumericFoldFailure {
        operator: op,
        span: operator_span,
        numeric_profile,
        string_table,
    };

    let folded_expression = match (&expression.kind, op) {
        (ExpressionKind::Bool(value), Operator::Not) => {
            Expression::bool(!value, expression.span, expression.value_mode.to_owned())
        }
        (_, Operator::Negate) => {
            let Some(operand_domain) = numeric_scalar_for_expression(expression) else {
                return Ok(None);
            };
            let Some(result_domain) = negation_domain(operand_domain) else {
                return Ok(None);
            };

            if let NumericScalar::Number(_) = result_domain {
                let ExpressionKind::Number(value) = &expression.kind else {
                    return Err(CompilerError::compiler_error(
                        "Dec negation received a non-Dec operand.",
                    )
                    .into());
                };

                Expression::number(
                    value.negated(),
                    expression.type_id,
                    expression.span,
                    expression.value_mode.to_owned(),
                )
            } else if result_domain.is_integer() {
                let value = integer_value_from_expression(expression)
                    .expect("integer negation domain only accepts integer-valued expressions");
                let Some(value) = value.checked_neg() else {
                    return integer_overflow_error(&mut failure_context);
                };
                let Some((minimum, maximum)) = result_domain.integer_range(numeric_profile) else {
                    return Err(CompilerError::compiler_error(
                        "Integer negation received a non-integer result domain.",
                    )
                    .into());
                };
                if !(minimum..=maximum).contains(&value) {
                    return integer_overflow_error(&mut failure_context);
                }

                integer_result_expression(
                    result_domain,
                    value,
                    expression.span,
                    expression.value_mode.to_owned(),
                )
            } else if let Some(precision) = result_domain.binary_float_precision(numeric_profile) {
                let value = float_value_from_expression(expression)
                    .expect("float negation domain only accepts binary-float expressions");
                let value = checked_float_result(-value, precision, &mut failure_context)?;
                float_result_expression(
                    result_domain,
                    value,
                    expression.span,
                    expression.value_mode.to_owned(),
                )
            } else {
                return Err(CompilerError::compiler_error(
                    "Numeric negation received a result domain without scalar arithmetic.",
                )
                .into());
            }
        }
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
    numeric_profile: NumericProfile,
) -> Result<Expression, ConstantFoldError> {
    // Dedicated expression classifier includes the evaluator's single-value path.
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusFoldCompileTimeExpressionEntries);
    match &expression.kind {
        ExpressionKind::Cast(cast) => {
            let folded_source = fold_compile_time_expression(
                &cast.source,
                template_ir_store,
                string_table,
                constant_context,
                numeric_profile,
            )?;
            fold_resolved_cast(
                expression,
                cast,
                &folded_source,
                template_ir_store,
                string_table,
                constant_context,
                numeric_profile,
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
                numeric_profile,
            )?;

            let mut folded_expression = match &folded_value.kind {
                #[cfg(test)]
                ExpressionKind::FallibleCarrierConstruct {
                    variant: FallibleCarrierVariant::Success,
                    value,
                } => value.as_ref().to_owned(),
                #[cfg(test)]
                ExpressionKind::FallibleCarrierConstruct {
                    variant: FallibleCarrierVariant::Error,
                    ..
                } => Expression::handled_result_with_type_id(
                    folded_value,
                    handling.to_owned(),
                    expression.type_id,
                    expression.diagnostic_type.to_owned(),
                    expression.span,
                ),
                _ => Expression::handled_result_with_type_id(
                    folded_value,
                    handling.to_owned(),
                    expression.type_id,
                    expression.diagnostic_type.to_owned(),
                    expression.span,
                ),
            };
            if matches!(
                folded_expression.kind,
                ExpressionKind::HandledFallibleExpression { .. }
            ) {
                // Pending receiver calls own their producers on this wrapper, not its carrier
                // operand. Rebuilding the folded value must preserve that selected identity,
                // catch disposition and authored propagation span.
                folded_expression.failure_facts = expression.failure_facts.clone();
                if let ExpressionKind::HandledFallibleExpression {
                    propagation_span, ..
                } = &mut folded_expression.kind
                {
                    *propagation_span = expression.propagation_span();
                }
            }
            Ok(folded_expression)
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
                    numeric_profile,
                )?;

                fold_resolved_cast(
                    expression,
                    cast,
                    &folded_source,
                    template_ir_store,
                    string_table,
                    constant_context,
                    numeric_profile,
                    Some(body),
                )
            }

            ValueBlock::If(_) | ValueBlock::LexicalScope(_) | ValueBlock::Match(_) => {
                Ok(expression.to_owned())
            }
        },
        _ => Ok(expression.to_owned()),
    }
}

/// Folds a resolved cast expression when its source has folded to a supported builtin literal.
///
/// WHAT: builtin evidence is evaluated here; user-defined or generic-bound
///      evidence is rejected in const-required contexts because the compiler
///      cannot execute user code or validate an unresolved generic bound at
///      compile time.
/// WHY: keeping this logic in the AST const-eval owner means HIR lowering only sees
///      runtime casts that could not be folded away.
#[allow(
    clippy::too_many_arguments,
    reason = "cast folding keeps the original and folded expressions, the resolved cast, the TIR store, mutable string state, const context, boundary numeric profile and recovery body as separate inputs"
)]
fn fold_resolved_cast(
    original_expression: &Expression,
    cast: &ResolvedCastExpression,
    folded_source: &Expression,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
    constant_context: bool,
    numeric_profile: NumericProfile,
    recovery_handler_body: Option<&[AstNode]>,
) -> Result<Expression, ConstantFoldError> {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusCastFoldAttempts);
    match &cast.evidence {
        ResolvedCastEvidence::Builtin { policy } => {
            if !policy.is_const_foldable() {
                if constant_context {
                    return Err(CompilerDiagnostic::invalid_cast(
                        InvalidCastReason::BuiltinEvidenceNotConstFoldable,
                        Some(cast.source_type_id),
                        Some(cast.target_type_id),
                        original_expression.span,
                    )
                    .into());
                }

                return Ok(original_expression.to_owned());
            }

            let source_literal =
                match builtin_cast_literal_from_expression(folded_source, string_table) {
                    Ok(Some(literal)) => literal,
                    Ok(None) => return Ok(original_expression.to_owned()),
                    Err(_diagnostic) if !constant_context => {
                        // Runtime casts remain legal because the selected URL context materialises
                        // structural pieces before execution.
                        return Ok(original_expression.to_owned());
                    }
                    Err(diagnostic) => return Err(ConstantFoldError::Diagnostic(diagnostic)),
                };

            match apply_builtin_cast_policy(*policy, &source_literal, numeric_profile) {
                Ok(folded_literal) => {
                    let Some(mut folded_expression) = builtin_cast_expression_from_literal(
                        &folded_literal,
                        cast.target_type_id,
                        original_expression.span,
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
                            original_expression.span,
                            template_ir_store,
                            string_table,
                            numeric_profile,
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
                        original_expression.span,
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
                    original_expression.span,
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
                    original_expression.span,
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
#[allow(
    clippy::too_many_arguments,
    reason = "recovery folding keeps the handler body, target/result types, wrap flag, span, TIR store, mutable string state and boundary numeric profile as separate inputs"
)]
fn fold_cast_recovery_handler(
    handler_body: &[AstNode],
    target_type_id: TypeId,
    requires_optional_wrap_after_cast: bool,
    result_type_id: TypeId,
    diagnostic_span: Option<SourceSpan>,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &mut StringTable,
    numeric_profile: NumericProfile,
) -> Result<Option<Expression>, ConstantFoldError> {
    let Some(handler_expression) = extract_single_produced_value(handler_body) else {
        return Err(CompilerDiagnostic::invalid_cast(
            InvalidCastReason::CatchHandlerNotConstFoldable,
            None,
            Some(target_type_id),
            diagnostic_span,
        )
        .into());
    };

    let folded_handler = fold_compile_time_expression(
        handler_expression,
        template_ir_store,
        string_table,
        true,
        numeric_profile,
    )?;

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
            folded_handler.span,
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
            | NodeKind::LexicalScope { .. }
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
/// WHAT: extracts literal scalar values while routing string values through the single
///       concrete-text requirement owner.
/// WHY: explicit casts share the builtin policy table, but structural strings cannot be
///      materialized until URL contexts are assigned.
fn builtin_cast_literal_from_expression(
    value: &Expression,
    string_table: &mut StringTable,
) -> Result<Option<BuiltinCastLiteral>, CompilerDiagnostic> {
    match &value.kind {
        ExpressionKind::Bool(value) => Ok(Some(BuiltinCastLiteral::Bool(*value))),
        ExpressionKind::Int(int) => Ok(Some(BuiltinCastLiteral::Int(*int))),
        ExpressionKind::Uint(uint) => Ok(Some(BuiltinCastLiteral::Uint(*uint))),
        ExpressionKind::Float(float) => Ok(Some(BuiltinCastLiteral::Float(*float))),
        ExpressionKind::FixedScalar(fixed) => Ok(Some(BuiltinCastLiteral::Fixed(*fixed))),
        ExpressionKind::Number(value) => Ok(Some(BuiltinCastLiteral::Number(value.clone()))),
        ExpressionKind::StringSlice(_) | ExpressionKind::StructuralString { .. } => {
            let Some(string) =
                require_concrete_text(value, ConstStringRequirement::CastOrParse, string_table)?
            else {
                return Ok(None);
            };
            Ok(Some(BuiltinCastLiteral::String(
                string_table.resolve(string).to_owned(),
            )))
        }
        ExpressionKind::Char(value) => Ok(Some(BuiltinCastLiteral::Char(*value))),
        _ => Ok(None),
    }
}

/// Builds an `Expression` literal from a `BuiltinCastLiteral`.
fn builtin_cast_expression_from_literal(
    literal: &BuiltinCastLiteral,
    type_id: TypeId,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Option<Expression> {
    match literal {
        BuiltinCastLiteral::Bool(value) => {
            Some(Expression::bool(*value, span, ValueMode::ImmutableOwned))
        }
        BuiltinCastLiteral::Int(value) => {
            Some(Expression::int(*value, span, ValueMode::ImmutableOwned))
        }
        BuiltinCastLiteral::Uint(value) => {
            Some(Expression::uint(*value, span, ValueMode::ImmutableOwned))
        }
        BuiltinCastLiteral::Float(value) => {
            Some(Expression::float(*value, span, ValueMode::ImmutableOwned))
        }
        BuiltinCastLiteral::String(value) => Some(Expression::string_slice(
            string_table.get_or_intern(value.to_owned()),
            span,
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Char(value) => {
            Some(Expression::char(*value, span, ValueMode::ImmutableOwned))
        }
        BuiltinCastLiteral::Fixed(value) => Some(Expression::fixed_scalar(
            *value,
            span,
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Number(value) => Some(Expression::number(
            value.clone(),
            type_id,
            span,
            ValueMode::ImmutableOwned,
        )),
        BuiltinCastLiteral::Error { .. } => None,
    }
}

fn compile_time_evaluation_diagnostic(
    reason: CompileTimeEvaluationErrorReason,
    operation: Option<String>,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
    numeric_profile: Option<NumericProfile>,
) -> ConstantFoldError {
    let operation = operation.map(|operation| string_table.get_or_intern(operation));

    CompilerDiagnostic::compile_time_evaluation_error(reason, operation, span, numeric_profile)
        .into()
}

fn integer_overflow_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
        Some(ctx.numeric_profile),
    ))
}

fn float_non_finite_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
        Some(ctx.numeric_profile),
    ))
}

/// Shared failure context for one numeric fold attempt.
///
/// WHAT: carries the authored operator, its span, the selected numeric profile and the shared
///       string table so both helpers funnel typed numeric diagnostics through one value.
/// WHY: keeps the fold-function argument lists short while failures retain the same span,
///       profile and operation identity that the standalone parameters provided.
struct NumericFoldFailure<'table, 'operator> {
    operator: &'operator Operator,
    span: Option<SourceSpan>,
    numeric_profile: NumericProfile,
    string_table: &'table mut StringTable,
}

fn checked_float_result(
    value: f64,
    precision: BinaryFloatPrecision,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<f64, ConstantFoldError> {
    // Every numeric float domain rounds its semantic result once before the finite-value check.
    let rounded = precision.round(value);
    if rounded.is_finite() {
        Ok(rounded)
    } else {
        float_non_finite_error(ctx)
    }
}

/// Folds one integer operation exactly in `i128` and checks the result against the operation
/// domain's inclusive `range`, so `Int` and every fixed integer share one set of rules.
fn checked_int_binary_result(
    lhs: i128,
    rhs: i128,
    (minimum, maximum): (i128, i128),
    numeric_operator: NumericOperator,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<i128, ConstantFoldError> {
    let checked = match numeric_operator {
        NumericOperator::Add => lhs.checked_add(rhs),
        NumericOperator::Subtract => lhs.checked_sub(rhs),
        NumericOperator::Multiply => lhs.checked_mul(rhs),
        NumericOperator::IntegerDivide => {
            if rhs == 0 {
                return divide_by_zero_error(ctx);
            }

            lhs.checked_div(rhs)
        }
        NumericOperator::Remainder => {
            if rhs == 0 {
                return divide_by_zero_error(ctx);
            }

            // Remainder by -1 is zero even where the paired quotient overflows.
            if rhs == -1 {
                Some(0)
            } else {
                lhs.checked_rem(rhs)
            }
        }
        NumericOperator::Power => {
            if rhs < 0 {
                return exponent_operand_context_error(ctx);
            }

            // Trivial bases have exact answers even when the exponent cannot fit u32. Other
            // powers use checked exponentiation, whose work grows logarithmically with exponent.
            if lhs == 0 {
                Some(if rhs == 0 { 1 } else { 0 })
            } else if lhs == 1 {
                Some(1)
            } else if lhs == -1 {
                Some(if rhs % 2 == 0 { 1 } else { -1 })
            } else {
                let Ok(exponent) = u32::try_from(rhs) else {
                    return integer_overflow_error(ctx);
                };
                lhs.checked_pow(exponent)
            }
        }
        NumericOperator::Divide | NumericOperator::Negate => {
            return Err(CompilerError::compiler_error(format!(
                "Integer folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };

    let Some(value) = checked else {
        return integer_overflow_error(ctx);
    };
    if !(minimum..=maximum).contains(&value) {
        return integer_overflow_error(ctx);
    }

    Ok(value)
}

fn divide_by_zero_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        ctx.string_table,
        ctx.span,
        Some(ctx.numeric_profile),
    ))
}

fn exponent_operand_context_error<T>(
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
        Some(ctx.numeric_profile),
    ))
}

fn integer_division_operand_error<T>(
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
    numeric_profile: NumericProfile,
) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::IntegerDivisionOnlyIntInt,
        None,
        string_table,
        span,
        Some(numeric_profile),
    ))
}

fn invalid_operator_for_compile_time_type<T>(
    op: &Operator,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
    numeric_profile: Option<NumericProfile>,
) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::InvalidOperatorForType,
        Some(op.to_str().to_string()),
        string_table,
        span,
        numeric_profile,
    ))
}

fn numeric_scalar_for_expression(expression: &Expression) -> Option<NumericScalar> {
    match &expression.kind {
        ExpressionKind::Int(_) => Some(NumericScalar::Int),
        ExpressionKind::Uint(_) => Some(NumericScalar::Uint),
        ExpressionKind::Float(_) => Some(NumericScalar::Float),
        ExpressionKind::FixedScalar(value) if value.scalar() != FixedScalar::Byte => {
            Some(NumericScalar::Fixed(value.scalar()))
        }
        ExpressionKind::Number(value) => Some(NumericScalar::Number(value.scale())),
        _ => None,
    }
}

fn integer_value_from_expression(expression: &Expression) -> Option<i128> {
    match &expression.kind {
        ExpressionKind::Int(value) => Some(i128::from(*value)),
        // The `u64` payload widens exactly into `i128`, so values above `i64::MAX`
        // keep their full unsigned value through checked folding and comparison.
        ExpressionKind::Uint(value) => Some(i128::from(*value)),
        ExpressionKind::FixedScalar(value) => match value.scalar().class() {
            FixedScalarClass::SignedInteger => value.as_i64().map(i128::from),
            FixedScalarClass::UnsignedInteger => value.as_u64().map(i128::from),
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
        },
        _ => None,
    }
}

fn float_value_from_expression(expression: &Expression) -> Option<f64> {
    match &expression.kind {
        ExpressionKind::Float(value) => Some(*value),
        ExpressionKind::FixedScalar(value) => value.as_f64(),
        _ => None,
    }
}

fn float_operand_in_domain(
    expression: &Expression,
    precision: BinaryFloatPrecision,
) -> Option<f64> {
    if let Some(integer) = integer_value_from_expression(expression) {
        return Some(precision.round_integer(integer));
    }

    float_value_from_expression(expression)
}

fn integer_result_expression(
    domain: NumericScalar,
    value: i128,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> Expression {
    match domain {
        NumericScalar::Int => {
            // The profile-specific domain range check immediately precedes construction.
            let value = i64::try_from(value).expect("Int result fits its i64 expression carrier");
            Expression::int(value, span, value_mode)
        }
        NumericScalar::Uint => {
            // The profile-specific unsigned range check immediately precedes construction.
            let value = u64::try_from(value).expect("Uint result fits its u64 expression carrier");
            Expression::uint(value, span, value_mode)
        }
        NumericScalar::Fixed(scalar) => match scalar.class() {
            FixedScalarClass::SignedInteger => {
                // The fixed-domain range check immediately precedes construction.
                let value = i64::try_from(value).expect("signed fixed result fits its i64 carrier");
                let value = FixedScalarValue::signed(scalar, value)
                    .expect("checked result fits the signed fixed domain");
                Expression::fixed_scalar(value, span, value_mode)
            }
            FixedScalarClass::UnsignedInteger => {
                // The fixed-domain range check immediately precedes construction.
                let value =
                    u64::try_from(value).expect("unsigned fixed result fits its u64 carrier");
                let value = FixedScalarValue::unsigned(scalar, value)
                    .expect("checked result fits the unsigned fixed domain");
                Expression::fixed_scalar(value, span, value_mode)
            }
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => {
                unreachable!("integer result domain must be an integer scalar")
            }
        },
        NumericScalar::Float => unreachable!("integer result domain cannot be Float"),
        NumericScalar::Number(_) => unreachable!("Number results use the exact Number constructor"),
    }
}

fn float_result_expression(
    domain: NumericScalar,
    value: f64,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> Expression {
    match domain {
        NumericScalar::Float => Expression::float(value, span, value_mode),
        NumericScalar::Fixed(scalar) if scalar.class() == FixedScalarClass::BinaryFloat => {
            let value = FixedScalarValue::binary_float(scalar, value)
                .expect("rounded finite result fits the fixed float domain");
            Expression::fixed_scalar(value, span, value_mode)
        }
        NumericScalar::Int
        | NumericScalar::Uint
        | NumericScalar::Fixed(_)
        | NumericScalar::Number(_) => {
            unreachable!("float result domain must be a binary float scalar")
        }
    }
}

fn number_operand_in_domain(
    expression: &Expression,
    scale: NumberScale,
) -> Option<Cow<'_, NumberValue>> {
    match &expression.kind {
        ExpressionKind::Number(value) if value.scale() == scale => Some(Cow::Borrowed(value)),
        // `Uint` joins the exact mixed-integer rule with its full unsigned value,
        // without an `Int` intermediary, including values above `i64::MAX`.
        ExpressionKind::Int(_) | ExpressionKind::Uint(_) | ExpressionKind::FixedScalar(_) => {
            integer_value_from_expression(expression)
                .map(|integer| Cow::Owned(NumberValue::from_integer(integer, scale)))
        }
        _ => None,
    }
}

fn fold_number_binary(
    lhs: &Expression,
    rhs: &Expression,
    numeric_operator: NumericOperator,
    scale: NumberScale,
    value_mode: ValueMode,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<Expression, ConstantFoldError> {
    let result_type_id = match numeric_scalar_for_expression(lhs) {
        Some(NumericScalar::Number(left_scale)) if left_scale == scale => lhs.type_id,
        _ => match numeric_scalar_for_expression(rhs) {
            Some(NumericScalar::Number(right_scale)) if right_scale == scale => rhs.type_id,
            _ => {
                return Err(CompilerError::compiler_error(
                    "Dec folding could not retain a Dec operand's canonical TypeId.",
                )
                .into());
            }
        },
    };

    let result = if numeric_operator == NumericOperator::Power {
        let ExpressionKind::Number(base) = &lhs.kind else {
            return Err(CompilerError::compiler_error(
                "Dec power folding received a non-Dec base.",
            )
            .into());
        };
        let ExpressionKind::Int(exponent) = &rhs.kind else {
            return Err(CompilerError::compiler_error(
                "Dec power folding received a non-Int exponent.",
            )
            .into());
        };
        base.checked_power(*exponent)
    } else {
        let Some(left_value) = number_operand_in_domain(lhs, scale) else {
            return Err(CompilerError::compiler_error(
                "Dec folding could not materialize the left operand in the result scale.",
            )
            .into());
        };
        let Some(right_value) = number_operand_in_domain(rhs, scale) else {
            return Err(CompilerError::compiler_error(
                "Dec folding could not materialize the right operand in the result scale.",
            )
            .into());
        };

        left_value
            .as_ref()
            .checked_binary(numeric_operator, right_value.as_ref())
    };

    let value = match result {
        Ok(value) => value,
        Err(NumberArithmeticError::DivideByZero) => {
            return divide_by_zero_error(ctx);
        }
        Err(NumberArithmeticError::InvalidExponent) => {
            return exponent_operand_context_error(ctx);
        }
        Err(NumberArithmeticError::InvalidOperation) => {
            return Err(CompilerError::compiler_error(format!(
                "Dec folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };

    Ok(Expression::number(
        value,
        result_type_id,
        lhs.span,
        value_mode,
    ))
}

fn fold_numeric_binary(
    lhs: &Expression,
    rhs: &Expression,
    numeric_operator: NumericOperator,
    domain: NumericScalar,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<Expression, ConstantFoldError> {
    let value_mode = numeric_value_mode(lhs, rhs);
    if let NumericScalar::Number(scale) = domain {
        return fold_number_binary(lhs, rhs, numeric_operator, scale, value_mode, ctx);
    }

    if domain.is_integer() {
        let left_value = integer_value_from_expression(lhs)
            .expect("integer operation domains only accept integer-valued expressions");
        let right_value = integer_value_from_expression(rhs)
            .expect("integer operation domains only accept integer-valued expressions");
        let Some(range) = domain.integer_range(ctx.numeric_profile) else {
            return Err(CompilerError::compiler_error(
                "Integer folding received a non-integer result domain.",
            )
            .into());
        };
        let value =
            checked_int_binary_result(left_value, right_value, range, numeric_operator, ctx)?;
        return Ok(integer_result_expression(
            domain, value, lhs.span, value_mode,
        ));
    }

    let Some(precision) = domain.binary_float_precision(ctx.numeric_profile) else {
        return Err(CompilerError::compiler_error(
            "Numeric folding received a domain without integer or binary-float arithmetic.",
        )
        .into());
    };
    let left_value = float_operand_in_domain(lhs, precision)
        .expect("binary-float operation domains only accept numeric expressions");
    let right_value = float_operand_in_domain(rhs, precision)
        .expect("binary-float operation domains only accept numeric expressions");

    let value = match numeric_operator {
        NumericOperator::Add => left_value + right_value,
        NumericOperator::Subtract => left_value - right_value,
        NumericOperator::Multiply => left_value * right_value,
        NumericOperator::Divide => {
            if right_value == 0.0 {
                return divide_by_zero_error(ctx);
            }
            left_value / right_value
        }
        NumericOperator::Remainder => {
            if right_value == 0.0 {
                return divide_by_zero_error(ctx);
            }
            left_value % right_value
        }
        NumericOperator::Power => numeric_power::pow(left_value, right_value),
        NumericOperator::IntegerDivide | NumericOperator::Negate => {
            return Err(CompilerError::compiler_error(format!(
                "Binary-float folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };
    let value = checked_float_result(value, precision, ctx)?;
    Ok(float_result_expression(domain, value, lhs.span, value_mode))
}

fn comparison_result(op: &Operator, ordering: std::cmp::Ordering) -> Option<bool> {
    match op {
        Operator::Equality => Some(ordering.is_eq()),
        Operator::NotEqual => Some(!ordering.is_eq()),
        Operator::GreaterThan => Some(ordering.is_gt()),
        Operator::GreaterThanOrEqual => Some(!ordering.is_lt()),
        Operator::LessThan => Some(ordering.is_lt()),
        Operator::LessThanOrEqual => Some(!ordering.is_gt()),
        _ => None,
    }
}

fn fold_numeric_comparison(
    lhs: &Expression,
    rhs: &Expression,
    op: &Operator,
    numeric_profile: NumericProfile,
) -> Option<bool> {
    if let (ExpressionKind::FixedScalar(left), ExpressionKind::FixedScalar(right)) =
        (&lhs.kind, &rhs.kind)
        && left.scalar() == FixedScalar::Byte
        && right.scalar() == FixedScalar::Byte
    {
        let left = left.as_u64()?;
        let right = right.as_u64()?;
        return comparison_result(op, left.cmp(&right));
    }

    let left_domain = numeric_scalar_for_expression(lhs)?;
    let right_domain = numeric_scalar_for_expression(rhs)?;
    if !comparison_supported(left_domain, right_domain) {
        return None;
    }

    let ordering = match (left_domain, right_domain) {
        (NumericScalar::Int, NumericScalar::Int)
        | (NumericScalar::Uint, NumericScalar::Uint)
        | (NumericScalar::Uint, NumericScalar::Int)
        | (NumericScalar::Int, NumericScalar::Uint) => {
            // Both values widen exactly into `i128`, so a negative `Int` compares
            // less than every `Uint` and values above `i64::MAX` stay exact.
            integer_value_from_expression(lhs)?.cmp(&integer_value_from_expression(rhs)?)
        }
        (NumericScalar::Fixed(left), NumericScalar::Fixed(right))
            if matches!(
                left.class(),
                FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
            ) && matches!(
                right.class(),
                FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
            ) =>
        {
            integer_value_from_expression(lhs)?.cmp(&integer_value_from_expression(rhs)?)
        }
        (NumericScalar::Float, NumericScalar::Float)
        | (NumericScalar::Fixed(_), NumericScalar::Fixed(_)) => {
            float_value_from_expression(lhs)?.partial_cmp(&float_value_from_expression(rhs)?)?
        }
        (NumericScalar::Int, NumericScalar::Float)
        | (NumericScalar::Uint, NumericScalar::Float) => {
            let integer = integer_value_from_expression(lhs)?;
            let precision: BinaryFloatPrecision = numeric_profile.float_precision.into();
            // The unsigned value converts directly at the profile precision, inheriting
            // the same Float rounding as `Int`/`Float` comparisons.
            precision
                .round_integer(integer)
                .partial_cmp(&float_value_from_expression(rhs)?)?
        }
        (NumericScalar::Float, NumericScalar::Int)
        | (NumericScalar::Float, NumericScalar::Uint) => {
            let integer = integer_value_from_expression(rhs)?;
            let precision: BinaryFloatPrecision = numeric_profile.float_precision.into();
            float_value_from_expression(lhs)?.partial_cmp(&precision.round_integer(integer))?
        }
        (NumericScalar::Number(_), NumericScalar::Number(_)) => {
            let ExpressionKind::Number(left) = &lhs.kind else {
                return None;
            };
            let ExpressionKind::Number(right) = &rhs.kind else {
                return None;
            };
            left.coefficient().cmp(right.coefficient())
        }
        (NumericScalar::Number(scale), integer_domain) if integer_domain.is_integer() => {
            let ExpressionKind::Number(left) = &lhs.kind else {
                return None;
            };
            let integer = integer_value_from_expression(rhs)?;
            let right = NumberValue::from_integer(integer, scale);
            left.coefficient().cmp(right.coefficient())
        }
        (integer_domain, NumericScalar::Number(scale)) if integer_domain.is_integer() => {
            let ExpressionKind::Number(right) = &rhs.kind else {
                return None;
            };
            let integer = integer_value_from_expression(lhs)?;
            let left = NumberValue::from_integer(integer, scale);
            left.coefficient().cmp(right.coefficient())
        }
        _ => return None,
    };

    comparison_result(op, ordering)
}

fn numeric_value_mode(lhs: &Expression, rhs: &Expression) -> ValueMode {
    if lhs.value_mode.is_mutable() || rhs.value_mode.is_mutable() {
        ValueMode::MutableOwned
    } else {
        ValueMode::ImmutableOwned
    }
}

fn folded_operator_expression(
    mut expression: Expression,
    lhs: &Expression,
    rhs: &Expression,
    op: &Operator,
) -> Expression {
    expression.contains_regular_division = lhs.contains_regular_division
        || rhs.contains_regular_division
        || matches!(op, Operator::Divide);
    expression.synthetic_interface_provenance = lhs
        .synthetic_interface_provenance
        .union(&rhs.synthetic_interface_provenance);
    expression
}

impl Expression {
    /// Fold an already-typed binary operation in its shared numeric result domain.
    ///
    /// WHAT: numeric arithmetic uses the same promotion table as AST operator typing. Comparisons
    ///       keep their exact integer or numeric-value rules, while non-numeric constant operators
    ///       retain their existing AST-local policies.
    /// WHY: compile-time evaluation must produce the same domain and failure boundary as runtime
    ///      lowering, without reducing fixed scalars through the Int/Float carrier variants.
    pub(crate) fn evaluate_operator(
        &self,
        rhs: &Expression,
        op: &Operator,
        string_table: &mut StringTable,
        numeric_profile: NumericProfile,
        operator_span: Option<SourceSpan>,
    ) -> Result<OperatorFoldOutcome, ConstantFoldError> {
        if let (Some(numeric_operator), Some(left_domain), Some(right_domain)) = (
            op.numeric_operator(),
            numeric_scalar_for_expression(self),
            numeric_scalar_for_expression(rhs),
        ) && !numeric_operator.is_unary()
            && let Some(domain) =
                binary_operation_domain(numeric_operator, left_domain, right_domain)
        {
            let mut failure_context = NumericFoldFailure {
                operator: op,
                span: operator_span,
                numeric_profile,
                string_table,
            };
            let expression =
                fold_numeric_binary(self, rhs, numeric_operator, domain, &mut failure_context)?;
            return Ok(OperatorFoldOutcome::Folded(folded_operator_expression(
                expression, self, rhs, op,
            )));
        }

        let kind = if let Some(value) = fold_numeric_comparison(self, rhs, op, numeric_profile) {
            ExpressionKind::Bool(value)
        } else {
            match (&self.kind, &rhs.kind) {
                (ExpressionKind::Int(left), ExpressionKind::Int(right))
                    if matches!(op, Operator::Range) =>
                {
                    ExpressionKind::Range(
                        Box::new(Expression::int(*left, self.span, ValueMode::ImmutableOwned)),
                        Box::new(Expression::int(*right, rhs.span, ValueMode::ImmutableOwned)),
                    )
                }
                (ExpressionKind::Int(_), ExpressionKind::Float(_))
                | (ExpressionKind::Float(_), ExpressionKind::Int(_))
                    if matches!(op, Operator::IntDivide) =>
                {
                    integer_division_operand_error(string_table, operator_span, numeric_profile)?
                }
                (ExpressionKind::Float(_), ExpressionKind::Float(_))
                    if matches!(op, Operator::IntDivide) =>
                {
                    invalid_operator_for_compile_time_type(
                        op,
                        string_table,
                        operator_span,
                        Some(numeric_profile),
                    )?
                }
                (ExpressionKind::Int(_), ExpressionKind::Int(_))
                | (ExpressionKind::Int(_), ExpressionKind::Float(_))
                | (ExpressionKind::Float(_), ExpressionKind::Int(_))
                | (ExpressionKind::Float(_), ExpressionKind::Float(_)) => {
                    invalid_operator_for_compile_time_type(
                        op,
                        string_table,
                        operator_span,
                        Some(numeric_profile),
                    )?
                }
                (ExpressionKind::Bool(left), ExpressionKind::Bool(right)) => match op {
                    Operator::And => ExpressionKind::Bool(*left && *right),
                    Operator::Or => ExpressionKind::Bool(*left || *right),
                    Operator::Equality => ExpressionKind::Bool(left == right),
                    Operator::NotEqual => ExpressionKind::Bool(left != right),
                    _ => invalid_operator_for_compile_time_type(
                        op,
                        string_table,
                        operator_span,
                        None,
                    )?,
                },
                (
                    ExpressionKind::StringSlice(_) | ExpressionKind::StructuralString { .. },
                    ExpressionKind::StringSlice(_) | ExpressionKind::StructuralString { .. },
                ) => {
                    if !matches!(op, Operator::Equality | Operator::NotEqual) {
                        invalid_operator_for_compile_time_type(
                            op,
                            string_table,
                            operator_span,
                            None,
                        )?;
                    }

                    let requirement = ConstStringRequirement::EqualityComparison;
                    let lhs_text = match require_concrete_text(self, requirement, string_table) {
                        Ok(Some(text)) => text,
                        Ok(None) => return Ok(OperatorFoldOutcome::NotConstant),
                        Err(diagnostic) => {
                            return Ok(OperatorFoldOutcome::TextUnavailable { diagnostic });
                        }
                    };
                    let rhs_text = match require_concrete_text(rhs, requirement, string_table) {
                        Ok(Some(text)) => text,
                        Ok(None) => return Ok(OperatorFoldOutcome::NotConstant),
                        Err(diagnostic) => {
                            return Ok(OperatorFoldOutcome::TextUnavailable { diagnostic });
                        }
                    };

                    match op {
                        Operator::Equality => ExpressionKind::Bool(lhs_text == rhs_text),
                        Operator::NotEqual => ExpressionKind::Bool(lhs_text != rhs_text),
                        _ => unreachable!("string operator was checked above"),
                    }
                }
                _ => return Ok(OperatorFoldOutcome::NotConstant),
            }
        };

        let value_mode = numeric_value_mode(self, rhs);
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
            self.span,
            type_id_hint_for_diagnostic_type(&result_type),
            result_type,
            value_mode,
        )
        .with_regular_division_provenance(contains_regular_division);
        result_expression.synthetic_interface_provenance = folded_provenance;
        Ok(OperatorFoldOutcome::Folded(result_expression))
    }
}

#[cfg(test)]
#[path = "tests/constant_folding_tests.rs"]
mod tests;
