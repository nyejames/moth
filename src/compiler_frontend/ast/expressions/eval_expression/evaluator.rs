//! Resolves transient expression syntax into phase-owned semantic operation nodes.
//!
//! Literal and operator typing precede constant substitution and folding. Completed results
//! retain canonical operation facts and occurrence metadata, never an RPN payload.
//! Construction and returned roots carry owner-bound handles while RPN buffers and the
//! operation stack retain compact raw IDs attached only at constructor boundaries.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldOutcome, FoldRequirement, FoldServices, constant_fold, fold_compile_time_expression,
};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOp, OwnedExpr,
};
use crate::compiler_frontend::ast::field_access::project_explicit_const_record_field;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, TypeMismatchContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::numeric_text::parse::{materialize_fixed_scalar, materialize_number};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::grammar::NumericLiteralKind;
use moth_lexical::numeric::parse::literal_kind_initialises;

use super::ordering;
use super::result_type::resolve_expression_result_type;
use super::typing_error::ExpressionTypingError;

/// Resolve a parsed expression fragment into one fully typed, owner-bound expression handle.
///
/// WHAT: orders unresolved syntax, selects literal contexts and resolves operator types before
///       folding and final receiving validation.
/// WHY: Exact numeric receivers and local peers choose literal domains before arithmetic, while the
///      canonical operator policy remains the sole source of result types.
#[allow(
    clippy::too_many_arguments,
    reason = "expression evaluation keeps scope, RPN nodes, type interner, expression builder, const store, expected type, value mode, string table, and path fork as separate borrows at the shared operator-folding entry point"
)]
pub fn evaluate_expression(
    context: &ScopeContext,
    nodes: Vec<ExpressionRpnItem>,
    type_interner: &mut AstTypeInterner<'_>,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<OwnedExpr, ExpressionTypingError> {
    let (mut ordered_nodes, span) = ordering::order_expression_nodes(nodes, expressions)?;
    let literal_destination =
        numeric_literal_destination(type_interner.environment(), expected_type);

    // Fast path: a single R-value needs no operator resolution or RPN assembly.
    if ordered_nodes.len() == 1 {
        if matches!(
            &ordered_nodes[0],
            ExpressionRpnItem::PendingNumericLiteral { .. }
        ) {
            materialize_pending_literal(
                &mut ordered_nodes,
                0,
                literal_destination,
                expressions,
                const_values,
                string_table,
            )?;
        }

        let ExpressionRpnItem::Operand(expression) = &ordered_nodes[0] else {
            return Err(CompilerError::compiler_error(
                "Expression ordering produced a single operator without an operand.",
            )
            .into());
        };

        let requirement = if context.kind.is_constant_context() {
            FoldRequirement::Required
        } else {
            FoldRequirement::Opportunistic
        };
        let only_expression = match fold_compile_time_expression(
            *expression,
            expressions,
            const_values,
            FoldServices::new(
                string_table,
                None,
                Some(context.external_package_registry.as_ref()),
            ),
            requirement,
        )? {
            ConstantFoldOutcome::Folded(root) | ConstantFoldOutcome::NotConstant(root) => root,
            ConstantFoldOutcome::Refused { root, .. }
                if requirement == FoldRequirement::Opportunistic =>
            {
                root
            }
            ConstantFoldOutcome::Refused { reason, .. } => {
                return Err(reason.into_diagnostic().into());
            }
        };
        // A standalone producer retains its slots for multi-bind, return and handling owners.
        // Only scalar results participate in natural expected-type inference here.
        let Some(value_type_id) = expressions.view(only_expression)?.success_shape().single()
        else {
            return Ok(expressions.own(only_expression));
        };

        validate_expression_result_type(
            expected_type,
            value_type_id,
            ordered_nodes[0].source_span(expressions)?,
            type_interner.environment_mut_for_derived_types(),
        )?;

        // References tighten the expected type so later fragments in the same statement
        // can resolve against the inferred target type.
        if matches!(
            expressions.view(only_expression)?.operation_id(),
            ExpressionOp::Reference(_)
        ) || matches!(
            expected_type,
            ExpectedType::Infer | ExpectedType::NumericLiteral(_)
        ) {
            *expected_type = ExpectedType::Known(value_type_id);
        }

        return Ok(expressions.own(only_expression));
    }

    // General path: resolve operator types across the full RPN shape, then attempt folding.
    let resolved = resolve_expression_result_type(
        &mut ordered_nodes,
        span,
        literal_destination,
        expressions,
        const_values,
        string_table,
        type_interner.environment(),
        path_fork,
    )?;
    let resolved_type = resolved.type_id;
    let mut operation_failure_facts = resolved.failure_facts;
    validate_expression_result_type(
        expected_type,
        resolved_type,
        span,
        type_interner.environment_mut_for_derived_types(),
    )?;

    if matches!(
        expected_type,
        ExpectedType::Infer | ExpectedType::NumericLiteral(_)
    ) {
        *expected_type = ExpectedType::Known(resolved_type);
    }

    // Preserve standalone place identity: only a typed multi-item consumption substitutes
    // explicit numeric constants, after every authored operand domain has been selected.
    substitute_explicit_numeric_constants(
        &mut ordered_nodes,
        context,
        path_fork,
        type_interner.environment(),
        expressions,
        const_values,
    )?;
    let root = expression_from_resolved_items(
        ordered_nodes,
        expressions,
        const_values,
        string_table,
        Some(context),
        value_mode.as_owned(),
    )?;
    // The occurrence anchors at its first operand; operator facts keep the exact operator span.
    let root = if expressions.view_owned(root)?.span() != span {
        let mut input = expressions
            .view_owned(root)?
            .to_completed_input(expressions)?;
        input.span = span;
        expressions.commit_replacement(input)?
    } else {
        root
    };
    increment_frontend_counter(FrontendCounter::ConstantFoldAttemptCount);
    let requirement = if context.kind.is_constant_context() {
        FoldRequirement::Required
    } else {
        FoldRequirement::Opportunistic
    };
    let outcome = constant_fold(
        root.id(),
        expressions,
        const_values,
        FoldServices::new(
            string_table,
            Some(context),
            Some(context.external_package_registry.as_ref()),
        ),
        requirement,
    )?;
    let folded = match outcome {
        ConstantFoldOutcome::Folded(root) => {
            increment_frontend_counter(FrontendCounter::ConstantFoldSuccessCount);
            root
        }
        ConstantFoldOutcome::NotConstant(root) => root,
        ConstantFoldOutcome::Refused { root, .. }
            if requirement == FoldRequirement::Opportunistic =>
        {
            root
        }
        ConstantFoldOutcome::Refused { reason, .. } => {
            return Err(reason.into_diagnostic().into());
        }
    };

    // Surviving operation rows own their contributors. The outer occurrence adds only the
    // pending authored eligibility, even when folding removed the introducing operation.
    operation_failure_facts.summary = Default::default();
    let mut failure = expressions
        .view(folded)?
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    failure.merge_pending_from(&operation_failure_facts);
    if expressions.view(folded)?.failure_facts().is_none() && failure == Default::default() {
        return Ok(expressions.own(folded));
    }
    Ok(expressions.with_failure_facts(expressions.own(folded), failure)?)
}

fn is_matching_materialized_numeric_value(
    expression: OwnedExpr,
    expected_type_id: TypeId,
    expressions: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<bool, CompilerError> {
    let view = expressions.view_owned(expression)?;
    Ok(view.success_shape().single() == Some(expected_type_id)
        && matches!(view.operation_id(), ExpressionOp::Constant(id)
            if matches!(const_values.payload(id), Some(ConstValuePayload::FixedScalar(_) | ConstValuePayload::Number(_)))))
}

fn substitute_explicit_numeric_constants(
    nodes: &mut [ExpressionRpnItem],
    context: &ScopeContext,
    path_fork: &PathInternerFork,
    type_environment: &TypeEnvironment,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<(), ExpressionTypingError> {
    for item in nodes {
        let ExpressionRpnItem::Operand(expression) = item else {
            continue;
        };
        let view = expressions.view(*expression)?;
        let type_id = view.value_type_id()?;
        if NumericScalar::from_type_id(type_id, type_environment).is_none() {
            continue;
        }
        let span = view.span();
        let mode = view.value_mode();
        let replacement = match view.operation_id() {
            ExpressionOp::Reference(path) => context
                .explicit_constant_value(&path, path_fork, const_values)
                .filter(|&value| {
                    const_values
                        .metadata(value)
                        .is_some_and(|metadata| metadata.type_id == type_id)
                        && matches!(
                            const_values.payload(value),
                            Some(ConstValuePayload::FixedScalar(_) | ConstValuePayload::Number(_))
                        )
                })
                .map(|value| expressions.constant(value, type_id, span, mode))
                .transpose()?,
            ExpressionOp::Copy(place) if expressions.place(place)?.projections().len() == 0 => {
                let path = expressions.place(place)?.metadata().root;
                context
                    .explicit_constant_value(&path, path_fork, const_values)
                    .filter(|&value| {
                        const_values
                            .metadata(value)
                            .is_some_and(|metadata| metadata.type_id == type_id)
                            && matches!(
                                const_values.payload(value),
                                Some(
                                    ConstValuePayload::FixedScalar(_)
                                        | ConstValuePayload::Number(_)
                                )
                            )
                    })
                    .map(|value| expressions.constant(value, type_id, span, mode))
                    .transpose()?
            }
            ExpressionOp::FieldAccess { .. } | ExpressionOp::Copy(_) => {
                project_explicit_const_record_field(
                    expressions.own(*expression),
                    context,
                    path_fork,
                    expressions,
                    const_values,
                )?
            }
            _ => None,
        };
        if let Some(replacement) = replacement
            && is_matching_materialized_numeric_value(
                replacement,
                type_id,
                expressions,
                const_values,
            )?
        {
            *expression = expressions
                .preserve_authored_metadata(replacement, expressions.own(*expression))?
                .id();
        }
    }
    Ok(())
}

/// Materialise one pending literal at its selected destination or natural value mode.
///
/// WHAT: replaces one deferred RPN item with a typed operand and returns its semantic type.
/// WHY: retained lexical facts allow receiving and peer contexts to choose a domain without
///      first producing a bounded integer or binary-float intermediate.
pub(super) fn materialize_pending_literal(
    nodes: &mut [ExpressionRpnItem],
    index: usize,
    destination: Option<PendingLiteralDestination>,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
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
            PendingLiteralDestination::FixedScalar(scalar) => {
                literal_kind_initialises(token.kind, *scalar)
            }
            PendingLiteralDestination::Number { .. } => true,
        });

        match destination {
            Some(PendingLiteralDestination::FixedScalar(scalar)) => {
                let value = materialize_fixed_scalar(token, token.sign, scalar, string_table)
                    .map_err(|reason| {
                        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, *span)
                    })?;
                expressions.fixed_scalar(const_values, value, *span, *value_mode)?
            }
            Some(PendingLiteralDestination::Number { scale, type_id }) => {
                let value = materialize_number(token, token.sign, scale, string_table).map_err(
                    |reason| {
                        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, *span)
                    },
                )?;
                expressions.number(const_values, value, type_id, *span, *value_mode)?
            }
            None => default_materialised_literal(
                token,
                *span,
                *value_mode,
                expressions,
                const_values,
                string_table,
            )?,
        }
    };
    let type_id = expressions.view_owned(expression)?.value_type_id()?;
    let Some(item) = nodes.get_mut(index) else {
        return Err(CompilerError::compiler_error(
            "Pending literal resolution could not retrieve its RPN item.",
        )
        .into());
    };
    *item = ExpressionRpnItem::Operand(expression.id());

    Ok(type_id)
}

/// Select a receiving literal destination after groups have been ordered.
///
/// Every eligible exact numeric destination reaches raw literal leaves, including leaves
/// beside named operands, before peer context and defaults. Already typed operands always
/// retain their identities, and a destination never retags a completed result.
fn numeric_literal_destination(
    type_environment: &TypeEnvironment,
    expected_type: &ExpectedType,
) -> Option<PendingLiteralDestination> {
    let expected_type_id = expected_type.literal_destination_type_id()?;
    let destination_type_id = type_environment
        .option_inner_type(expected_type_id)
        .unwrap_or(expected_type_id);
    pending_literal_destination_for_type_id(destination_type_id, type_environment)
}

/// A semantic numeric or `Byte` destination eligible for pending-literal materialisation.
#[derive(Clone, Copy)]
pub(crate) enum PendingLiteralDestination {
    /// One exact fixed-width destination, including the non-arithmetic `Byte` octet.
    FixedScalar(FixedScalar),
    /// One exact-decimal destination carrying its already-resolved local identity.
    Number { scale: NumberScale, type_id: TypeId },
}

/// Classify one direct semantic type as an eligible pending-literal destination.
pub(super) fn pending_literal_destination_for_type_id(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<PendingLiteralDestination> {
    if let Some(scale) = type_environment.number_scale(type_id) {
        return Some(PendingLiteralDestination::Number { scale, type_id });
    }
    type_environment
        .fixed_scalar(type_id)
        .map(PendingLiteralDestination::FixedScalar)
}

/// Default literal materialisation for an unconstrained numeric literal.
///
/// WHAT: whole literals materialise as I32 and decimal/exponent literals as F64, each with the
///       retained token's effective sign applied by the shared destination materialiser.
/// WHY: the language default is an exact width, never a size-selected width.
///      Defaults apply only after every permitted use has contributed.
fn default_materialised_literal(
    token: &NumericLiteralToken,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<OwnedExpr, ExpressionTypingError> {
    if token.kind == NumericLiteralKind::WholeNumber {
        let value = materialize_fixed_scalar(token, token.sign, FixedScalar::I32, string_table)
            .map_err(|reason| {
                CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
            })?;

        return Ok(expressions.fixed_scalar(const_values, value, span, value_mode)?);
    }

    // The retained token carries one effective sign, so the shared binary-float materialiser
    // rounds and negates in one destination-precision step: no positive intermediate exists and
    // signed zero is preserved exactly.
    let value = materialize_fixed_scalar(token, token.sign, FixedScalar::F64, string_table)
        .map_err(|reason| {
            CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
        })?;

    Ok(expressions.fixed_scalar(const_values, value, span, value_mode)?)
}

/// Compiler-bug error for pending expression syntax that escaped evaluation.
///
/// Group markers disappear during ordering and pending literals during typing. Every later
/// consumer handles their presence as a broken compiler invariant, not a source diagnostic.
pub(crate) fn pending_expression_item_bug(function: &'static str) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Pending expression item reached {function}; evaluate_expression must resolve it first."
    ))
}

/// Complete resolved transient postfix syntax as fixed-edge semantic operations.
///
/// WHAT: folds each operand and operator as soon as it is complete, so every constant
/// sub-expression is committed as a `Constant` child of its runtime parents.
/// WHY: folding only the completed root would leave constant operands under a runtime operator.
/// Operands are folded when pushed, so an operator only folds when all of its direct children are
/// already constant; runtime children are never walked again.
pub(crate) fn expression_from_resolved_items(
    items: Vec<ExpressionRpnItem>,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    scope: Option<&ScopeContext>,
    value_mode: ValueMode,
) -> Result<OwnedExpr, ExpressionTypingError> {
    let mut stack: Vec<ExprId> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ExpressionRpnItem::Operand(id) => {
                expressions.view(id)?;
                let folded =
                    fold_completed_node(id, expressions, const_values, string_table, scope)?;
                stack.push(folded);
            }
            ExpressionRpnItem::ResolvedOperator(facts) => {
                let right = if facts.operator.required_values() == 2 {
                    Some(stack.pop().ok_or_else(|| {
                        CompilerError::compiler_error("Resolved operation lost its right operand.")
                    })?)
                } else {
                    None
                };
                let left = stack.pop().ok_or_else(|| {
                    CompilerError::compiler_error("Resolved operation lost its left operand.")
                })?;
                let right_constant = match right {
                    Some(right) => is_constant_node(expressions, right)?,
                    None => true,
                };
                let operands_constant = is_constant_node(expressions, left)? && right_constant;
                let node = expressions.resolved_operation(
                    expressions.own(left),
                    right.map(|id| expressions.own(id)),
                    facts,
                    value_mode,
                )?;
                let node = if operands_constant {
                    fold_completed_node(node.id(), expressions, const_values, string_table, scope)?
                } else {
                    node.id()
                };
                stack.push(node);
            }
            _ => return Err(pending_expression_item_bug("resolved operation construction").into()),
        }
    }
    let [root] = stack.as_slice() else {
        return Err(CompilerError::compiler_error(
            "Resolved expression construction did not produce one root.",
        )
        .into());
    };
    Ok(expressions.own(*root))
}

/// Fold one completed node in place; an opportunistic refusal keeps the node.
///
/// WHAT: `constant_fold` is the single folding owner. A refusal preserves the runtime expression
/// here; a required receiver reports the refusal's owned diagnostic.
fn fold_completed_node(
    node: ExprId,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    scope: Option<&ScopeContext>,
) -> Result<ExprId, ExpressionTypingError> {
    let registry = scope.map(|scope| scope.external_package_registry.as_ref());
    let requirement = if scope.is_some_and(|scope| scope.kind.is_constant_context()) {
        FoldRequirement::Required
    } else {
        FoldRequirement::Opportunistic
    };
    let outcome = constant_fold(
        node,
        expressions,
        const_values,
        FoldServices::new(string_table, scope, registry),
        requirement,
    )?;
    match outcome {
        ConstantFoldOutcome::Folded(folded) | ConstantFoldOutcome::NotConstant(folded) => {
            Ok(folded)
        }
        ConstantFoldOutcome::Refused { root, .. }
            if requirement == FoldRequirement::Opportunistic =>
        {
            Ok(root)
        }
        ConstantFoldOutcome::Refused { reason, .. } => Err(reason.into_diagnostic().into()),
    }
}

/// Whether a completed node is already a folded `Constant`.
fn is_constant_node(
    expressions: &mut AstExpressionBuilder,
    id: ExprId,
) -> Result<bool, ExpressionTypingError> {
    Ok(matches!(
        expressions.view(id)?.operation_id(),
        ExpressionOp::Constant(_)
    ))
}

/// Validate that an expression's resolved semantic type is compatible with the
/// contextual expectation.
///
/// WHAT: compares canonical `TypeId`s through the `TypeEnvironment`.
/// WHY: compatibility is defined over canonical `TypeId` pairs and queried through the
///      `TypeEnvironment`.
fn validate_expression_result_type(
    expected_type: &mut ExpectedType,
    actual_type_id: TypeId,
    span: Option<SourceSpan>,
    type_environment: &mut TypeEnvironment,
) -> Result<(), ExpressionTypingError> {
    let Some(expected_type_id) = expected_type.known_type_id() else {
        return Ok(());
    };

    if is_type_compatible(expected_type_id, actual_type_id, type_environment) {
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
