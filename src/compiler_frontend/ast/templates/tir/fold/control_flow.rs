//! Branch and loop reduction for prepared TIR folds.

use crate::compiler_frontend::ast::ast_nodes::RangeLoopSpec;
use crate::compiler_frontend::ast::const_eval::{
    BindingFold, ConstantFoldError, ConstantFoldOutcome, ConstantFoldRefusalReason,
    FoldRequirement, FoldServices, constant_fold, fold_expression_with_bindings,
};
use crate::compiler_frontend::ast::const_values::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateBranchSelector, TemplateFoldBinding, TemplateLoopHeader,
    build_collection_iteration_bindings, build_range_iteration_bindings, const_collection_items,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    FoldedCondition, TemplateEmission, TemplateFoldResult, TirFoldContext,
    fold_condition_with_provenance, resolve_fold_bindings_in_expression,
    selected_option_capture_payload_with_provenance,
};
use crate::compiler_frontend::ast::templates::tir::ids::{ExpressionSiteId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::TemplateLoopHeaderExpressionSites;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
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

fn expression_provenance(expression: ExpressionView<'_>) -> SyntheticInterfaceProvenance {
    expression
        .occurrence_facts()
        .map_or_else(SyntheticInterfaceProvenance::empty, |facts| {
            facts.provenance.clone()
        })
}

fn range_operand_provenance(
    range: &RangeLoopSpec,
    fold_context: &TirFoldContext<'_>,
) -> Result<SyntheticInterfaceProvenance, TemplateError> {
    let mut provenances = Vec::new();
    provenances.push(expression_provenance(
        fold_context.expression_builder.view(range.start)?,
    ));
    provenances.push(expression_provenance(
        fold_context.expression_builder.view(range.end)?,
    ));
    if let Some(step) = range.step {
        provenances.push(expression_provenance(
            fold_context.expression_builder.view(step)?,
        ));
    }
    Ok(SyntheticInterfaceProvenance::union_all(&provenances))
}

/// One range header's resolved bounds and the refusal that kept them dynamic.
///
/// WHAT: `spec` replaces the authored bounds when resolution changed at least one operand, while
///       `refusal` reports an operation that refused to produce a constant operand.
struct ResolvedRangeOperands {
    spec: Option<RangeLoopSpec>,
    refusal: Option<ConstantFoldRefusalReason>,
}

fn resolve_range_constant_operands(
    range: &RangeLoopSpec,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<ResolvedRangeOperands, TemplateError> {
    let start = resolve_range_constant_operand(range.start, fold_context)?;
    let end = resolve_range_constant_operand(range.end, fold_context)?;
    let step = range
        .step
        .map(|step| resolve_range_constant_operand(step, fold_context))
        .transpose()?;

    let refusal = start
        .refusal
        .clone()
        .or_else(|| end.refusal.clone())
        .or_else(|| step.as_ref().and_then(|resolved| resolved.refusal.clone()));
    if refusal.is_some() {
        return Ok(ResolvedRangeOperands {
            spec: None,
            refusal,
        });
    }

    let start = start.bound;
    let end = end.bound;
    let step = step.and_then(|resolved| resolved.bound);
    let start_unchanged = start.is_none_or(|id| id == range.start);
    let end_unchanged = end.is_none_or(|id| id == range.end);
    let step_unchanged = match (range.step, step) {
        (None, _) => true,
        (Some(before), Some(after)) => before == after,
        (Some(_), None) => true,
    };
    if start_unchanged && end_unchanged && step_unchanged {
        return Ok(ResolvedRangeOperands {
            spec: None,
            refusal: None,
        });
    }

    let mut resolved = range.clone();
    if let Some(start) = start {
        resolved.start = start;
    }
    if let Some(end) = end {
        resolved.end = end;
    }
    if let Some(step) = step {
        resolved.step = Some(step);
    }
    Ok(ResolvedRangeOperands {
        spec: Some(resolved),
        refusal: None,
    })
}

/// One resolved range operand.
struct ResolvedRangeOperand {
    /// A replacement bound, or `None` when the operand is already the folded form it needs.
    bound: Option<ExprId>,
    refusal: Option<ConstantFoldRefusalReason>,
}

fn resolve_range_constant_operand(
    expression: ExprId,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<ResolvedRangeOperand, TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(expression, fold_context)?;
    if let Some(reason) = resolved.refusal {
        return Ok(ResolvedRangeOperand {
            bound: None,
            refusal: Some(reason),
        });
    }
    let resolved = resolved.expression;
    if let Some(substituted) = resolve_source_constant_references(resolved, fold_context)? {
        return Ok(ResolvedRangeOperand {
            bound: Some(substituted),
            refusal: None,
        });
    }
    // A registered Core call in a bound folds through the shared engine like any other operand,
    // so a refusal keeps the whole loop dynamic instead of failing as non-constant bounds.
    match constant_fold(
        resolved,
        fold_context.expression_builder,
        fold_context.const_values,
        FoldServices::new(
            &mut *fold_context.string_table,
            fold_context.source_scope,
            fold_context.external_package_registry,
        ),
        FoldRequirement::Opportunistic,
    ) {
        Ok(ConstantFoldOutcome::Folded(expression)) => Ok(ResolvedRangeOperand {
            bound: Some(expression),
            refusal: None,
        }),
        Ok(ConstantFoldOutcome::Refused { reason, .. }) => Ok(ResolvedRangeOperand {
            bound: None,
            refusal: Some(reason),
        }),
        Ok(ConstantFoldOutcome::NotConstant(_)) => Ok(ResolvedRangeOperand {
            bound: None,
            refusal: None,
        }),
        Err(ConstantFoldError::Diagnostic(diagnostic))
            if !diagnostic.is_compiler_capacity_exceeded() =>
        {
            Ok(ResolvedRangeOperand {
                bound: None,
                refusal: None,
            })
        }
        Err(error) => Err(error.into()),
    }
}

/// WHAT: folds module-constant references inside a range bound or step through
/// the atomic store, preserving the authored span and value mode.
/// WHY: loop bounds may name module constants; the const cursor only accepts
/// folded numeric roots, so source constants must resolve before cursor entry.
fn resolve_source_constant_references(
    expression: ExprId,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<ExprId>, TemplateError> {
    if !source_constant_tree_is_numeric(expression, fold_context)? {
        return Ok(None);
    }
    let mut paths = Vec::new();
    collect_source_constant_paths(expression, fold_context, &mut paths)?;
    if paths.is_empty() {
        return Ok(None);
    }
    paths.sort();
    paths.dedup();
    let (root_span, root_mode) = {
        let view = fold_context.expression_builder.view(expression)?;
        (view.span(), view.value_mode())
    };
    let mut substitutions = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(value) = fold_context.source_scope.and_then(|scope| {
            scope.resolved_module_constant_value(&path, fold_context.const_values)
        }) else {
            return Ok(None);
        };
        let Some(metadata) = fold_context.const_values.metadata(value) else {
            return Err(CompilerError::compiler_error(
                "TIR fold: module constant is missing const metadata.",
            )
            .into());
        };
        let replacement = fold_context.expression_builder.constant(
            value,
            metadata.type_id,
            root_span,
            root_mode,
        )?;
        // WHY: a substituted module constant reference carries the constant's synthetic provenance,
        //      as the baseline cloned declaration expression did. Template folding reads it from the
        //      occurrence, so it must survive substitution.
        let provenance = metadata.synthetic_interface_provenance.clone();
        let replacement = fold_context
            .expression_builder
            .with_synthetic_interface_provenance(replacement, provenance)?;
        substitutions.push((path, replacement.id()));
    }
    let substituted = fold_expression_with_bindings(
        expression,
        fold_context.expression_builder,
        fold_context.const_values,
        fold_context.string_table,
        BindingFold::strict(
            fold_context.source_scope,
            fold_context.external_package_registry,
        ),
        |path| {
            substitutions
                .iter()
                .find(|(candidate, _)| *candidate == path)
                .map(|(_, replacement)| *replacement)
        },
    )?
    .expression;
    let folded = if matches!(
        fold_context
            .expression_builder
            .view(substituted)?
            .operation(),
        ExpressionOperationView::Constant(_)
    ) {
        substituted
    } else {
        match constant_fold(
            substituted,
            fold_context.expression_builder,
            fold_context.const_values,
            FoldServices::new(
                &mut *fold_context.string_table,
                fold_context.source_scope,
                fold_context.external_package_registry,
            ),
            FoldRequirement::Opportunistic,
        )? {
            ConstantFoldOutcome::Folded(id) => id,
            ConstantFoldOutcome::NotConstant(_) | ConstantFoldOutcome::Refused { .. } => {
                return Ok(None);
            }
        }
    };
    let folded = fold_context.expression_builder.own(folded);
    let expression = fold_context.expression_builder.own(expression);
    Ok(Some(
        fold_context
            .expression_builder
            .preserve_authored_metadata(folded, expression)?
            .id(),
    ))
}

/// WHAT: gates source-constant folding to operator trees whose leaves are all
/// numeric constants or resolvable module-constant references.
/// WHY: the old recursive initializer fold only accepted numeric recipes; a
/// shared gate here keeps that refusal shape without rebuilding dead RPN.
fn source_constant_tree_is_numeric(
    expression: ExprId,
    fold_context: &TirFoldContext<'_>,
) -> Result<bool, TemplateError> {
    enum Gate {
        Constant(ConstValueId),
        Reference(PathId),
        Unary(ExprId),
        Binary(ExprId, ExprId),
        Other,
    }
    let gate = {
        let view = fold_context.expression_builder.view(expression)?;
        match view.operation() {
            ExpressionOperationView::Constant(id) => Gate::Constant(id),
            ExpressionOperationView::Reference(path) => Gate::Reference(path),
            ExpressionOperationView::Coerce { value, .. }
            | ExpressionOperationView::NumericUnary { operand: value, .. }
            | ExpressionOperationView::LogicalNot { operand: value, .. } => Gate::Unary(value.id()),
            ExpressionOperationView::NumericBinary { left, right, .. }
            | ExpressionOperationView::LogicalBinary { left, right, .. }
            | ExpressionOperationView::Compare { left, right, .. } => {
                Gate::Binary(left.id(), right.id())
            }
            _ => Gate::Other,
        }
    };
    match gate {
        Gate::Constant(id) => Ok(constant_tree_is_numeric(id, fold_context.const_values)),
        Gate::Reference(path) => Ok(fold_context
            .source_scope
            .and_then(|scope| {
                scope.resolved_module_constant_value(&path, fold_context.const_values)
            })
            .is_some_and(|value| constant_tree_is_numeric(value, fold_context.const_values))),
        Gate::Unary(child) => source_constant_tree_is_numeric(child, fold_context),
        Gate::Binary(left, right) => Ok(source_constant_tree_is_numeric(left, fold_context)?
            && source_constant_tree_is_numeric(right, fold_context)?),
        Gate::Other => Ok(false),
    }
}

fn constant_tree_is_numeric(id: ConstValueId, constants: &ConstValueStore) -> bool {
    match constants.payload(id) {
        Some(ConstValuePayload::Number(_)) | Some(ConstValuePayload::FixedScalar(_)) => true,
        Some(ConstValuePayload::Coerced(inner)) => constant_tree_is_numeric(*inner, constants),
        _ => false,
    }
}

fn collect_source_constant_paths(
    expression: ExprId,
    fold_context: &TirFoldContext<'_>,
    paths: &mut Vec<PathId>,
) -> Result<(), TemplateError> {
    let view = fold_context.expression_builder.view(expression)?;
    if let ExpressionOperationView::Reference(path) = view.operation() {
        paths.push(path);
        return Ok(());
    }
    let mut children = Vec::new();
    view.visit_children(|child| children.push(child.id()));
    for child in children {
        collect_source_constant_paths(child, fold_context, paths)?;
    }
    Ok(())
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
        .effective_expression_for_site(&*fold_context.expression_builder, selector_site_id)?
        .unwrap_or_else(|| selector.condition_expression());

    match selector {
        TemplateBranchSelector::Bool(_) => {
            let condition = fold_condition_with_provenance(
                effective_expression,
                node_span,
                InvalidTemplateStructureReason::TemplateIfConditionNotConst,
                fold_context,
            )?;
            let (selected, condition_provenance) = match condition {
                FoldedCondition::Selected { value, provenance } => (value, provenance),
                FoldedCondition::Refused { reason, provenance } => {
                    output_state.provenance.merge(&provenance);
                    output_state.refusal = Some(reason);
                    return Ok(());
                }
            };
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
#[allow(
    clippy::too_many_arguments,
    reason = "loop folding keeps authored header and effective header sites, body and aggregate-wrapper identities, mutable output and fold owners, traversal input, loop span and insertion routing separate"
)]
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

            let effective_condition = fold_input
                .effective_expression_for_site(&*fold_context.expression_builder, site_id)?;
            let condition_ref = effective_condition.unwrap_or(*condition);
            output_state.provenance.merge(&expression_provenance(
                fold_context.expression_builder.view(condition_ref)?,
            ));

            let condition = fold_condition_with_provenance(
                condition_ref,
                loop_span,
                InvalidTemplateStructureReason::TemplateLoopConditionNotConst,
                fold_context,
            )?;
            let condition_value = match condition {
                FoldedCondition::Selected { value, provenance } => {
                    output_state.provenance.merge(&provenance);
                    value
                }
                FoldedCondition::Refused { reason, provenance } => {
                    output_state.provenance.merge(&provenance);
                    output_state.refusal = Some(reason);
                    return Ok(());
                }
            };
            if !condition_value {
                return Ok(());
            }

            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
                fold_context
                    .expression_builder
                    .view(condition_ref)?
                    .span()
                    .or(loop_span),
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

            let effective_start = fold_input
                .effective_expression_for_site(&*fold_context.expression_builder, start_site)?;
            let effective_end = fold_input
                .effective_expression_for_site(&*fold_context.expression_builder, end_site)?;
            let effective_step = step_site
                .map(|site_id| {
                    fold_input
                        .effective_expression_for_site(&*fold_context.expression_builder, site_id)
                })
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
                    range.start = expression;
                }
                if let Some(expression) = effective_end {
                    range.end = expression;
                }
                if let Some(expression) = effective_step {
                    range.step = Some(expression);
                }
                effective_range = range;
                &effective_range
            } else {
                range.as_ref()
            };
            let resolved_operands = resolve_range_constant_operands(range_ref, fold_context)?;
            if let Some(reason) = resolved_operands.refusal {
                output_state.refusal = Some(reason);
                return Ok(());
            }
            let range_ref = resolved_operands.spec.as_ref().unwrap_or(range_ref);
            let mut cursor = ConstRangeCursor::new(
                range_ref,
                &*fold_context.expression_builder,
                &*fold_context.const_values,
                fold_context.template_const_loop_iteration_limit,
                loop_span,
            )?;
            let range_provenance = range_operand_provenance(range_ref, fold_context)?;
            output_state.provenance.merge(&range_provenance);

            while let Some(counter) = cursor.next_counter()? {
                add_ast_counter(AstCounter::TemplateFoldLoopIterations, 1);
                let iteration_bindings = build_range_iteration_bindings(
                    bindings,
                    counter,
                    cursor.iteration_count() - 1,
                    &range_provenance,
                    fold_context.expression_builder,
                    fold_context.const_values,
                )?;
                fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;
                if aggregate_state.refusal.is_some() {
                    break;
                }
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

            let effective_iterable = fold_input
                .effective_expression_for_site(&*fold_context.expression_builder, site_id)?;
            let iterable_ref = effective_iterable.unwrap_or(*iterable);
            let resolved_iterable =
                resolve_fold_bindings_in_expression(iterable_ref, fold_context)?.expression;
            output_state.provenance.merge(&expression_provenance(
                fold_context.expression_builder.view(iterable_ref)?,
            ));
            let resolved_iterable_provenance =
                expression_provenance(fold_context.expression_builder.view(resolved_iterable)?);
            output_state.provenance.merge(&resolved_iterable_provenance);

            let items = const_collection_items(
                resolved_iterable,
                fold_context.expression_builder,
                &*fold_context.const_values,
            )?;
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
                let resolved_item =
                    resolve_fold_bindings_in_expression(*item, fold_context)?.expression;
                let resolved_item = if resolved_item != *item {
                    let item_provenance =
                        expression_provenance(fold_context.expression_builder.view(*item)?);
                    let combined =
                        expression_provenance(fold_context.expression_builder.view(resolved_item)?)
                            .union(&item_provenance);
                    fold_context
                        .expression_builder
                        .with_synthetic_interface_provenance(
                            fold_context.expression_builder.own(resolved_item),
                            combined,
                        )?
                        .id()
                } else {
                    resolved_item
                };
                output_state.provenance.merge(&expression_provenance(
                    fold_context.expression_builder.view(resolved_item)?,
                ));
                let iteration_bindings = build_collection_iteration_bindings(
                    bindings,
                    resolved_item,
                    index,
                    &resolved_iterable_provenance,
                    fold_context.expression_builder,
                    fold_context.const_values,
                )?;
                fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;
                if aggregate_state.refusal.is_some() {
                    break;
                }
            }

            (aggregate_state, estimated_aggregate)
        }
    };

    // Visited bodies can contribute provenance even when they emit no output.
    output_state.provenance.merge(&aggregate_state.provenance);
    if let Some(reason) = aggregate_state.refusal.take() {
        output_state.refusal = Some(reason);
        return Ok(());
    }

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
        refusal,
    } = emission;
    if let Some(reason) = refusal {
        aggregate_state.refusal = Some(reason);
        return Ok(());
    }

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
