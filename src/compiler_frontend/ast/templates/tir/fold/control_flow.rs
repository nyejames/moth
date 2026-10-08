//! Branch and loop reduction for prepared TIR folds.

use crate::compiler_frontend::ast::ast_nodes::RangeLoopSpec;
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldError, ConstantFoldOutcome, constant_fold,
};
use crate::compiler_frontend::ast::expressions::eval_expression::pending_expression_item_bug;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateBranchSelector, TemplateFoldBinding, TemplateLoopHeader,
    build_collection_iteration_bindings, build_range_iteration_bindings, const_collection_items,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    FoldResolvedExpression, TemplateEmission, TemplateFoldResult, TirFoldContext,
    fold_bool_condition_with_provenance, fold_conditional_loop_const_condition,
    resolve_fold_bindings_in_expression, selected_option_capture_payload_with_provenance,
};
use crate::compiler_frontend::ast::templates::tir::ids::{ExpressionSiteId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::TemplateLoopHeaderExpressionSites;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

use super::estimate::{
    FoldEstimateMode, estimate_loop_aggregate_bytes, estimate_tir_node_output_bytes,
    estimated_range_iteration_count, record_tir_fold_output_estimate_miss,
    record_tir_fold_output_intern,
};
use super::reducer::{
    FoldInsertion, FoldOutputState, FoldTraversalInput, fold_tir_node, fold_tir_node_into_buffer,
};
use super::wrappers::fold_tir_aggregate_wrapper;

fn range_provenance_expressions(
    range: &RangeLoopSpec,
) -> impl Iterator<Item = &SyntheticInterfaceProvenance> {
    std::iter::once(&range.start.synthetic_interface_provenance)
        .chain(std::iter::once(&range.end.synthetic_interface_provenance))
        .chain(
            range
                .step
                .as_ref()
                .map(|step| &step.synthetic_interface_provenance),
        )
}

fn resolve_range_constant_operands(
    range: &RangeLoopSpec,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<RangeLoopSpec>, TemplateError> {
    let start = resolve_range_constant_operand(&range.start, fold_context)?;
    let end = resolve_range_constant_operand(&range.end, fold_context)?;
    let step = range
        .step
        .as_ref()
        .map(|step| resolve_range_constant_operand(step, fold_context))
        .transpose()?;

    if start.is_none() && end.is_none() && !step.as_ref().is_some_and(|step| step.is_some()) {
        return Ok(None);
    }

    let mut resolved = range.clone();
    if let Some(start) = start {
        resolved.start = start;
    }
    if let Some(end) = end {
        resolved.end = end;
    }
    if let Some(Some(step)) = step {
        resolved.step = Some(step);
    }
    Ok(Some(resolved))
}

fn resolve_range_constant_operand(
    expression: &Expression,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<Expression>, TemplateError> {
    match resolve_fold_bindings_in_expression(expression, fold_context)? {
        FoldResolvedExpression::Borrowed(expression) => {
            resolve_source_constant_references(expression, fold_context)
        }
        FoldResolvedExpression::Owned(expression) => {
            let expression = *expression;
            Ok(Some(
                resolve_source_constant_references(&expression, fold_context)?
                    .unwrap_or(expression),
            ))
        }
    }
}

fn has_resolved_source_constant_reference(
    expression: &Expression,
    fold_context: &TirFoldContext<'_>,
) -> bool {
    match &expression.kind {
        ExpressionKind::Reference(path) => fold_context
            .source_scope
            .and_then(|scope| scope.resolved_module_constant_expression(path))
            .is_some(),
        ExpressionKind::Coerced { value, .. } => {
            has_resolved_source_constant_reference(value, fold_context)
        }
        ExpressionKind::Runtime(rpn) => rpn.items.iter().any(|item| match item {
            ExpressionRpnItem::Operand(operand) => {
                has_resolved_source_constant_reference(operand, fold_context)
            }
            ExpressionRpnItem::Operator { .. } => false,
            // Pending syntax never survives evaluation. This predicate cannot return the
            // canonical error, so it flags the broken invariant in debug builds and reports
            // conservatively here; every fallible fold path below rejects the pending item
            // with `pending_expression_item_bug` once it is actually visited.
            ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingGroup { .. } => {
                debug_assert!(
                    false,
                    "pending expression syntax reached TIR source-constant predicate"
                );
                true
            }
        }),
        _ => false,
    }
}

fn resolve_source_constant_references(
    expression: &Expression,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<Expression>, TemplateError> {
    if !has_resolved_source_constant_reference(expression, fold_context) {
        return Ok(None);
    }
    let ExpressionKind::Reference(path) = &expression.kind else {
        return fold_source_constant_initializer(expression, fold_context);
    };

    let Some(initializer) = fold_context
        .source_scope
        .and_then(|scope| scope.resolved_module_constant_expression(path))
        .cloned()
    else {
        return Ok(None);
    };
    let Some(mut resolved) = fold_source_constant_initializer(&initializer, fold_context)? else {
        return Ok(None);
    };
    resolved.span = expression.span;
    resolved.synthetic_interface_provenance = resolved
        .synthetic_interface_provenance
        .union(&expression.synthetic_interface_provenance);
    Ok(Some(resolved))
}

fn fold_source_constant_initializer(
    expression: &Expression,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<Expression>, TemplateError> {
    match &expression.kind {
        ExpressionKind::Int(_)
        | ExpressionKind::Uint(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::FixedScalar(_)
        | ExpressionKind::Number(_) => Ok(Some(expression.clone())),
        ExpressionKind::Reference(_) => {
            resolve_source_constant_references(expression, fold_context)
        }
        ExpressionKind::Coerced { value, to_type } => {
            let Some(value) = fold_source_constant_initializer(value, fold_context)? else {
                return Ok(None);
            };
            Ok(Some(Expression {
                kind: ExpressionKind::Coerced {
                    value: Box::new(value),
                    to_type: *to_type,
                },
                ..expression.clone()
            }))
        }
        ExpressionKind::Runtime(rpn) => {
            let mut items = Vec::with_capacity(rpn.items.len());
            for item in &rpn.items {
                match item {
                    ExpressionRpnItem::Operand(operand) => {
                        let Some(operand) =
                            fold_source_constant_initializer(operand, fold_context)?
                        else {
                            return Ok(None);
                        };
                        items.push(ExpressionRpnItem::Operand(operand));
                    }
                    ExpressionRpnItem::Operator { .. } => items.push(item.clone()),
                    // Pending syntax never survives evaluation; its presence here is a broken
                    // compiler invariant, not a foldable position.
                    ExpressionRpnItem::PendingNumericLiteral { .. }
                    | ExpressionRpnItem::PendingGroup { .. } => {
                        return Err(
                            pending_expression_item_bug("TIR source-constant folding").into()
                        );
                    }
                }
            }
            fold_source_constant_rpn(expression, items, fold_context)
        }
        _ => Ok(None),
    }
}

fn fold_source_constant_rpn(
    expression: &Expression,
    items: Vec<ExpressionRpnItem>,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<Expression>, TemplateError> {
    let mut folded = match constant_fold(
        items,
        fold_context.string_table,
        fold_context.numeric_profile,
        fold_context.source_scope,
    ) {
        Ok(ConstantFoldOutcome::Folded(items)) => {
            if items.len() != 1 {
                return Ok(None);
            }
            let Some(ExpressionRpnItem::Operand(expression)) = items.into_iter().next() else {
                return Ok(None);
            };
            expression
        }
        Ok(ConstantFoldOutcome::NotConstant(_))
        | Ok(ConstantFoldOutcome::TextUnavailable { .. }) => return Ok(None),
        Err(ConstantFoldError::Diagnostic(diagnostic)) => return Err(diagnostic.into()),
        Err(ConstantFoldError::Infrastructure(error)) => return Err((*error).into()),
    };
    folded.span = expression.span;
    folded.synthetic_interface_provenance = folded
        .synthetic_interface_provenance
        .union(&expression.synthetic_interface_provenance);
    Ok(Some(folded))
}

#[allow(
    clippy::too_many_arguments,
    reason = "The selector, site, body, span and fold owners are independent inputs at this TIR boundary."
)]
pub(super) fn fold_tir_conditional_with_insertion(
    selector: &TemplateBranchSelector,
    selector_site_id: ExpressionSiteId,
    body_id: TemplateIrNodeId,
    node_span: Option<SourceSpan>,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<(), TemplateError> {
    if insertion.is_aggregate() {
        return Err(CompilerError::compiler_error(
            "TIR fold: malformed aggregate wrapper subtree contains a conditional.",
        )
        .into());
    }

    let effective_expression = fold_input
        .effective_expression_for_site(selector_site_id)?
        .unwrap_or_else(|| selector.condition_expression());

    match selector {
        TemplateBranchSelector::Bool(_) => {
            let (selected, condition_provenance) =
                fold_bool_condition_with_provenance(effective_expression, node_span, fold_context)?;
            output_state.provenance.merge(&condition_provenance);

            if selected {
                fold_tir_node_into_buffer(
                    body_id,
                    output_state,
                    fold_context,
                    fold_input,
                    insertion,
                )
            } else {
                Ok(())
            }
        }
        TemplateBranchSelector::OptionPresentCapture { pattern, .. } => {
            let (payload, capture_provenance) = selected_option_capture_payload_with_provenance(
                effective_expression,
                pattern,
                fold_input.view.store(),
                fold_context,
            )?;
            output_state.provenance.merge(&capture_provenance);
            let Some(payload) = payload else {
                return Ok(());
            };

            let previous_bindings_len = fold_context.push_bindings([payload]);
            let result = fold_tir_node_into_buffer(
                body_id,
                output_state,
                fold_context,
                fold_input,
                insertion,
            );
            fold_context.restore_bindings(previous_bindings_len);
            result
        }
    }
}

/// Folds a TIR loop node, including its aggregate wrapper.
#[allow(clippy::too_many_arguments)]
pub(super) fn fold_tir_loop(
    header: &TemplateLoopHeader,
    header_sites: TemplateLoopHeaderExpressionSites,
    body_id: TemplateIrNodeId,
    aggregate_wrapper: Option<TemplateIrNodeId>,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    loop_span: Option<crate::compiler_frontend::source::SourceSpan>,
    insertion: FoldInsertion<'_>,
) -> Result<(), TemplateError> {
    let store = fold_input.view.store();
    let body_estimate = estimate_tir_node_output_bytes(
        store,
        body_id,
        fold_context.string_table,
        FoldEstimateMode::Structural,
    )?;

    let (mut aggregate_state, estimated_aggregate) = match header {
        TemplateLoopHeader::Conditional { condition } => {
            let site_id = match header_sites {
                TemplateLoopHeaderExpressionSites::Conditional { condition } => condition,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR fold: loop header/header_sites shape mismatch (Conditional).",
                    )
                    .into());
                }
            };

            let effective_condition = fold_input.effective_expression_for_site(site_id)?;
            let condition_ref = effective_condition.unwrap_or(condition.as_ref());
            output_state
                .provenance
                .merge(&condition_ref.synthetic_interface_provenance);

            let condition_value = fold_conditional_loop_const_condition(condition_ref, loop_span)?;
            if !condition_value {
                return Ok(());
            }

            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
                condition_ref.span.or(loop_span),
            )
            .into());
        }

        TemplateLoopHeader::Range { bindings, range } => {
            let (start_site, end_site, step_site) = match header_sites {
                TemplateLoopHeaderExpressionSites::Range { start, end, step } => (start, end, step),
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR fold: loop header/header_sites shape mismatch (Range).",
                    )
                    .into());
                }
            };

            let effective_start = fold_input.effective_expression_for_site(start_site)?;
            let effective_end = fold_input.effective_expression_for_site(end_site)?;
            let effective_step = step_site
                .map(|site_id| fold_input.effective_expression_for_site(site_id))
                .transpose()?
                .flatten();
            let has_override =
                effective_start.is_some() || effective_end.is_some() || effective_step.is_some();

            let estimated_iterations =
                estimated_range_iteration_count(fold_context.template_const_loop_iteration_limit);
            let estimated_aggregate =
                estimate_loop_aggregate_bytes(body_estimate, estimated_iterations);
            let mut aggregate_state = FoldOutputState::with_capacity(estimated_aggregate);
            if fold_input.projection_enabled {
                aggregate_state.enable_projection();
            }

            let effective_range;
            let range_ref: &RangeLoopSpec = if has_override {
                let mut range = range.as_ref().clone();
                if let Some(expression) = effective_start {
                    range.start = expression.clone();
                }
                if let Some(expression) = effective_end {
                    range.end = expression.clone();
                }
                if let Some(expression) = effective_step {
                    range.step = Some(expression.clone());
                }
                effective_range = range;
                &effective_range
            } else {
                range.as_ref()
            };
            let source_resolved_range = resolve_range_constant_operands(range_ref, fold_context)?;
            let range_ref = source_resolved_range.as_ref().unwrap_or(range_ref);
            let mut cursor = ConstRangeCursor::new(
                range_ref,
                fold_context.template_const_loop_iteration_limit,
                loop_span,
                fold_context.numeric_profile.float_precision,
            )?;
            let range_provenance =
                SyntheticInterfaceProvenance::union_all(range_provenance_expressions(range_ref));
            output_state.provenance.merge(&range_provenance);

            while let Some(counter) = cursor.next_counter()? {
                add_ast_counter(AstCounter::TemplateFoldLoopIterations, 1);
                let iteration_bindings = build_range_iteration_bindings(
                    bindings,
                    counter,
                    cursor.iteration_count() - 1,
                    &range_provenance,
                );
                fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;
            }

            (aggregate_state, estimated_aggregate)
        }

        TemplateLoopHeader::Collection { bindings, iterable } => {
            let site_id = match header_sites {
                TemplateLoopHeaderExpressionSites::Collection { iterable } => iterable,
                _ => {
                    return Err(CompilerError::compiler_error(
                        "TIR fold: loop header/header_sites shape mismatch (Collection).",
                    )
                    .into());
                }
            };

            let effective_iterable = fold_input.effective_expression_for_site(site_id)?;
            let iterable_ref = effective_iterable.unwrap_or(iterable.as_ref());
            let resolved_iterable =
                resolve_fold_bindings_in_expression(iterable_ref, fold_context)?;
            let resolved_ref: &Expression = match &resolved_iterable {
                FoldResolvedExpression::Borrowed(expression) => expression,
                FoldResolvedExpression::Owned(expression) => expression,
            };
            output_state
                .provenance
                .merge(&iterable_ref.synthetic_interface_provenance);
            output_state
                .provenance
                .merge(&resolved_ref.synthetic_interface_provenance);

            let items = const_collection_items(resolved_ref)?;
            let estimated_iterations = std::cmp::min(
                items.len(),
                fold_context.template_const_loop_iteration_limit,
            );
            let estimated_aggregate =
                estimate_loop_aggregate_bytes(body_estimate, estimated_iterations);
            let mut aggregate_state = FoldOutputState::with_capacity(estimated_aggregate);
            if fold_input.projection_enabled {
                aggregate_state.enable_projection();
            }

            for (index, item) in items.iter().enumerate() {
                add_ast_counter(AstCounter::TemplateFoldLoopIterations, 1);
                if index >= fold_context.template_const_loop_iteration_limit {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                            limit: fold_context.template_const_loop_iteration_limit,
                        },
                        loop_span,
                    )
                    .into());
                }
                let mut resolved_item = resolve_fold_bindings_in_expression(item, fold_context)?;
                if let FoldResolvedExpression::Owned(expression) = &mut resolved_item {
                    expression
                        .synthetic_interface_provenance
                        .merge(&item.synthetic_interface_provenance);
                }
                let resolved_item_ref: &Expression = match &resolved_item {
                    FoldResolvedExpression::Borrowed(expression) => expression,
                    FoldResolvedExpression::Owned(expression) => expression,
                };
                output_state
                    .provenance
                    .merge(&resolved_item_ref.synthetic_interface_provenance);
                let iteration_bindings = build_collection_iteration_bindings(
                    bindings,
                    resolved_item_ref,
                    index,
                    &resolved_ref.synthetic_interface_provenance,
                );
                fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;
            }

            (aggregate_state, estimated_aggregate)
        }
    };

    // Visited bodies can contribute provenance even when they emit no output.
    output_state.provenance.merge(&aggregate_state.provenance);

    if !aggregate_state.emitted_output {
        return Ok(());
    }

    let actual_aggregate_len = aggregate_state.output_buffer.len();
    record_tir_fold_output_estimate_miss(actual_aggregate_len, estimated_aggregate);
    let aggregate_projection = aggregate_state.projection_pieces.take();
    let aggregate_output = aggregate_state.into_const_string_value(fold_context.string_table);
    record_tir_fold_output_intern(actual_aggregate_len);

    let Some(wrapper_node_id) = aggregate_wrapper else {
        if output_state.projection_pieces.is_some() {
            output_state.append_pieces(aggregate_projection.as_deref())?;
        }
        output_state.append_emission_value(&aggregate_output, fold_context.string_table);
        output_state.emitted_output = true;
        return Ok(());
    };

    fold_tir_aggregate_wrapper(
        wrapper_node_id,
        &aggregate_output,
        aggregate_projection.as_deref(),
        output_state,
        fold_context,
        fold_input,
    )
}

#[allow(clippy::too_many_arguments)]
fn fold_tir_loop_iteration(
    body_id: TemplateIrNodeId,
    iteration_bindings: Vec<TemplateFoldBinding>,
    fold_context: &mut TirFoldContext<'_>,
    aggregate_state: &mut FoldOutputState,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<(), TemplateError> {
    let previous_bindings_len = fold_context.push_bindings(iteration_bindings);
    let folded_result = fold_tir_node(body_id, fold_context, fold_input, insertion);
    fold_context.restore_bindings(previous_bindings_len);

    let emission = folded_result?;
    let TemplateFoldResult {
        emission,
        provenance,
        projection_pieces,
    } = emission;

    aggregate_state.provenance.merge(&provenance);
    if aggregate_state.projection_pieces.is_some() {
        aggregate_state.append_pieces(projection_pieces.as_deref())?;
    }

    match emission {
        TemplateEmission::NoOutput => Ok(()),
        TemplateEmission::Output(output) => {
            aggregate_state.append_emission_value(&output, fold_context.string_table);
            aggregate_state.emitted_output = true;
            Ok(())
        }
    }
}
