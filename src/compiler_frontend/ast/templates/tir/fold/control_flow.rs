//! Branch and loop reduction for prepared TIR folds.

use crate::compiler_frontend::ast::ast_nodes::RangeLoopSpec;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateBranchSelector, TemplateFoldBinding, TemplateLoopControlKind,
    TemplateLoopHeader, build_collection_iteration_bindings, build_range_iteration_bindings,
    const_collection_items,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext, condition_location_or_loop_location,
    fold_bool_condition_with_provenance, fold_conditional_loop_const_condition,
    selected_option_capture_payload_with_provenance,
};
use crate::compiler_frontend::ast::templates::tir::ids::TemplateIrNodeId;
use crate::compiler_frontend::ast::templates::tir::node::{
    TemplateIrBranch, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

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

pub(super) fn fold_tir_branch_chain_with_insertion(
    branches: &[TemplateIrBranch],
    fallback: Option<TemplateIrNodeId>,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    if insertion.is_aggregate() {
        return Err(CompilerError::compiler_error(
            "TIR fold: malformed aggregate wrapper subtree contains a branch chain.",
        )
        .into());
    }

    for branch in branches {
        let effective_expression =
            fold_input.effective_expression_for_site(branch.selector_site_id)?;

        let selected = match (&branch.selector, effective_expression) {
            (TemplateBranchSelector::Bool(condition), None) => {
                let (selected, condition_provenance) =
                    fold_bool_condition_with_provenance(condition, &branch.location, fold_context)?;
                output_state.provenance.merge(&condition_provenance);
                selected
            }
            (TemplateBranchSelector::Bool(_), Some(effective)) => {
                let (selected, condition_provenance) =
                    fold_bool_condition_with_provenance(effective, &branch.location, fold_context)?;
                output_state.provenance.merge(&condition_provenance);
                selected
            }
            (TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern }, None) => {
                let (payload, capture_provenance) =
                    selected_option_capture_payload_with_provenance(
                        scrutinee,
                        pattern,
                        fold_input.view.store(),
                        fold_context,
                    )?;
                output_state.provenance.merge(&capture_provenance);
                if let Some(payload) = payload {
                    return fold_tir_branch_with_insertion(
                        branch,
                        [payload],
                        output_state,
                        fold_context,
                        fold_input,
                        insertion,
                    );
                }

                false
            }
            (TemplateBranchSelector::OptionPresentCapture { pattern, .. }, Some(effective)) => {
                let (payload, capture_provenance) =
                    selected_option_capture_payload_with_provenance(
                        effective,
                        pattern,
                        fold_input.view.store(),
                        fold_context,
                    )?;
                output_state.provenance.merge(&capture_provenance);
                if let Some(payload) = payload {
                    return fold_tir_branch_with_insertion(
                        branch,
                        [payload],
                        output_state,
                        fold_context,
                        fold_input,
                        insertion,
                    );
                }

                false
            }
        };

        if selected {
            return fold_tir_node_into_buffer(
                branch.body,
                output_state,
                fold_context,
                fold_input,
                insertion,
            );
        }
    }

    let Some(fallback_id) = fallback else {
        return Ok(None);
    };

    fold_tir_node_into_buffer(
        fallback_id,
        output_state,
        fold_context,
        fold_input,
        insertion,
    )
}

fn fold_tir_branch_with_insertion<const N: usize>(
    branch: &TemplateIrBranch,
    bindings: [TemplateFoldBinding; N],
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    insertion: FoldInsertion<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    let previous_bindings_len = fold_context.push_bindings(bindings);
    let result = fold_tir_node_into_buffer(
        branch.body,
        output_state,
        fold_context,
        fold_input,
        insertion,
    );
    fold_context.restore_bindings(previous_bindings_len);

    result
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
    loop_location: &SourceLocation,
    insertion: FoldInsertion<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
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

            let condition_value =
                fold_conditional_loop_const_condition(condition_ref, loop_location)?;
            if !condition_value {
                return Ok(None);
            }

            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue,
                condition_location_or_loop_location(condition_ref, loop_location),
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
            let mut cursor = ConstRangeCursor::new(
                range_ref,
                fold_context.template_const_loop_iteration_limit,
                loop_location.clone(),
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
                let iteration_signal = fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;

                match iteration_signal {
                    Some(TemplateLoopControlKind::Break) => break,
                    Some(TemplateLoopControlKind::Continue) | None => {}
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

            let effective_iterable = fold_input.effective_expression_for_site(site_id)?;
            let iterable_ref = effective_iterable.unwrap_or(iterable.as_ref());
            output_state
                .provenance
                .merge(&iterable_ref.synthetic_interface_provenance);

            let items = const_collection_items(iterable_ref)?;
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
                        loop_location.clone(),
                    )
                    .into());
                }

                let iteration_bindings = build_collection_iteration_bindings(
                    bindings,
                    item,
                    index,
                    &iterable_ref.synthetic_interface_provenance,
                );
                let iteration_signal = fold_tir_loop_iteration(
                    body_id,
                    iteration_bindings,
                    fold_context,
                    &mut aggregate_state,
                    fold_input,
                    insertion,
                )?;

                match iteration_signal {
                    Some(TemplateLoopControlKind::Break) => break,
                    Some(TemplateLoopControlKind::Continue) | None => {}
                }
            }

            (aggregate_state, estimated_aggregate)
        }
    };

    output_state.provenance.merge(&aggregate_state.provenance);

    if !aggregate_state.emitted_output {
        return Ok(None);
    }

    let actual_aggregate_len = aggregate_state.output_buffer.len();
    record_tir_fold_output_estimate_miss(actual_aggregate_len, estimated_aggregate);
    let aggregate_id = fold_context
        .string_table
        .intern(&aggregate_state.output_buffer);
    record_tir_fold_output_intern(actual_aggregate_len);
    let aggregate_projection = aggregate_state.projection_pieces.take();

    let Some(wrapper_node_id) = aggregate_wrapper else {
        if output_state.projection_pieces.is_some() {
            output_state.append_pieces(aggregate_projection.as_deref())?;
        } else {
            output_state
                .output_buffer
                .push_str(fold_context.string_table.resolve(aggregate_id));
        }
        output_state.emitted_output = true;
        return Ok(None);
    };

    fold_tir_aggregate_wrapper(
        wrapper_node_id,
        aggregate_id,
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
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
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
        TemplateEmission::NoOutput => Ok(None),
        TemplateEmission::Output(output) => {
            aggregate_state
                .output_buffer
                .push_str(fold_context.string_table.resolve(output));
            aggregate_state.emitted_output = true;
            Ok(None)
        }
        TemplateEmission::Break(output) => {
            if let Some(output) = output {
                aggregate_state
                    .output_buffer
                    .push_str(fold_context.string_table.resolve(output));
                aggregate_state.emitted_output = true;
            }
            Ok(Some(TemplateLoopControlKind::Break))
        }
        TemplateEmission::Continue(output) => {
            if let Some(output) = output {
                aggregate_state
                    .output_buffer
                    .push_str(fold_context.string_table.resolve(output));
                aggregate_state.emitted_output = true;
            }
            Ok(Some(TemplateLoopControlKind::Continue))
        }
    }
}
