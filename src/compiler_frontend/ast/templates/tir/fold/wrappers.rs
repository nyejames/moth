//! Wrapper-context application and virtual wrapper insertion for TIR folds.

use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopControlKind;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext, template_emission_from_output_and_signal,
};
use crate::compiler_frontend::ast::templates::tir::collect_tir_slot_schema;
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIr;
use crate::compiler_frontend::ast::templates::tir::overlays::{
    TirWrapperApplicationMode, TirWrapperContext,
};
use crate::compiler_frontend::ast::templates::tir::refs::TemplateWrapperReference;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

use super::estimate::{
    FoldEstimateMode, estimate_tir_node_output_bytes, record_tir_fold_output_estimate_miss,
    record_tir_fold_output_intern, reserve_tir_fold_output_buffer,
};
use super::reducer::{
    FoldInsertion, FoldOutputState, FoldTraversalInput, FoldedConstTemplatePiece,
    fold_tir_node_into_buffer, reject_slot_insert_template,
};

/// Applies the wrapper-context overlay for a child-template occurrence, if any.
///
/// WHAT: resolves the effective `TirWrapperContext` for `occurrence_id` and folds
///       any inherited wrapper templates around the already-folded child emission.
///       `$fresh` suppression is honored by treating a suppressed context as empty,
///       and no-output/signal emissions pass through unchanged so skipped branches
///       and zero-iteration loops do not receive wrappers.
/// WHY: wrapper-context overlays replace the structural mutation of
///      `conditional_child_wrapper_set`. Applying them at the child occurrence
///      boundary lets the same structural child template be shared under different
///      wrapper contexts without store mutation.
pub(super) fn apply_wrapper_context_overlay_to_child_emission(
    result: TemplateFoldResult,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
    context: Option<&TirWrapperContext>,
) -> Result<TemplateFoldResult, TemplateError> {
    let store = fold_input.view.store();
    let TemplateFoldResult {
        emission,
        provenance,
        projection_pieces,
    } = result;
    let Some(context) = context else {
        return Ok(TemplateFoldResult::with_projection(
            emission,
            provenance,
            projection_pieces,
        ));
    };

    // `$fresh` suppresses parent-applied wrappers at this occurrence. The
    // inherited wrapper set is omitted from the overlay when suppressed, but
    // honor the flag explicitly in case it coexists with a wrapper set ref.
    if context.skip_parent_child_wrappers {
        return Ok(TemplateFoldResult::with_projection(
            emission,
            provenance,
            projection_pieces,
        ));
    }

    let wrapper_set_ref = match context.inherited_wrapper_set {
        Some(wrapper_set_ref) => wrapper_set_ref,
        None => {
            return Ok(TemplateFoldResult::with_projection(
                emission,
                provenance,
                projection_pieces,
            ));
        }
    };

    let wrapper_set = store.get_wrapper_set(wrapper_set_ref).ok_or_else(|| {
        CompilerError::compiler_error(
            "TIR fold: inherited wrapper set referenced by overlay is missing.",
        )
    })?;

    fold_conditional_child_wrappers_around_emission(
        &wrapper_set.wrappers,
        emission,
        provenance,
        projection_pieces,
        context.application_mode,
        fold_context,
        fold_input,
    )
}

/// Appends a child-template result to the caller's output buffer.
pub(super) fn append_template_result_to_buffer(
    result: TemplateFoldResult,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    let TemplateFoldResult {
        emission,
        projection_pieces,
        ..
    } = result;

    match emission {
        TemplateEmission::NoOutput => Ok(None),
        TemplateEmission::Output(output) => {
            if output_state.projection_pieces.is_some() {
                output_state.append_pieces(projection_pieces.as_deref())?;
            } else {
                output_state
                    .output_buffer
                    .push_str(fold_context.string_table.resolve(output));
            }
            output_state.emitted_output = true;
            Ok(None)
        }
        TemplateEmission::Break(output) => {
            if let Some(output) = output {
                if output_state.projection_pieces.is_some() {
                    output_state.append_pieces(projection_pieces.as_deref())?;
                } else {
                    output_state
                        .output_buffer
                        .push_str(fold_context.string_table.resolve(output));
                }
                output_state.emitted_output = true;
            }
            Ok(Some(TemplateLoopControlKind::Break))
        }
        TemplateEmission::Continue(output) => {
            if let Some(output) = output {
                if output_state.projection_pieces.is_some() {
                    output_state.append_pieces(projection_pieces.as_deref())?;
                } else {
                    output_state
                        .output_buffer
                        .push_str(fold_context.string_table.resolve(output));
                }
                output_state.emitted_output = true;
            }
            Ok(Some(TemplateLoopControlKind::Continue))
        }
    }
}

/// Applies conditional child wrappers to an already-folded emission using
/// a virtual wrapper fold that does not push synthetic nodes into the store.
///
/// WHAT: folds each inherited wrapper template around the already-folded child
///       output string, injecting the child output at the slot that the fill
///       content would route to (or appending it after slot-less wrapper
///       content). No-output and empty-signal cases pass through unchanged so
///       skipped branches or zero-iteration loops do not receive wrappers.
///
/// WHY: the folded child remains an owned output value while wrapper structure
///      is read from the immutable TIR store. Slot injection therefore needs no
///      temporary TIR nodes or second wrapper representation.
#[allow(clippy::too_many_arguments)]
pub(super) fn fold_conditional_child_wrappers_around_emission(
    wrapper_references: &[TemplateWrapperReference],
    emission: TemplateEmission,
    provenance: SyntheticInterfaceProvenance,
    projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
    application_mode: TirWrapperApplicationMode,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let (output, signal_kind) = match emission {
        TemplateEmission::NoOutput => {
            if matches!(application_mode, TirWrapperApplicationMode::IfChildEmits)
                || wrapper_references.is_empty()
            {
                return Ok(TemplateFoldResult::with_projection(
                    TemplateEmission::NoOutput,
                    provenance,
                    projection_pieces,
                ));
            }

            (fold_context.string_table.intern(""), None)
        }
        TemplateEmission::Output(output) => (output, None),
        TemplateEmission::Break(Some(output)) => (output, Some(TemplateLoopControlKind::Break)),
        TemplateEmission::Continue(Some(output)) => {
            (output, Some(TemplateLoopControlKind::Continue))
        }
        TemplateEmission::Break(None) => {
            if matches!(application_mode, TirWrapperApplicationMode::IfChildEmits)
                || wrapper_references.is_empty()
            {
                return Ok(TemplateFoldResult::with_projection(
                    TemplateEmission::Break(None),
                    provenance,
                    projection_pieces,
                ));
            }

            (
                fold_context.string_table.intern(""),
                Some(TemplateLoopControlKind::Break),
            )
        }
        TemplateEmission::Continue(None) => {
            if matches!(application_mode, TirWrapperApplicationMode::IfChildEmits)
                || wrapper_references.is_empty()
            {
                return Ok(TemplateFoldResult::with_projection(
                    TemplateEmission::Continue(None),
                    provenance,
                    projection_pieces,
                ));
            }

            (
                fold_context.string_table.intern(""),
                Some(TemplateLoopControlKind::Continue),
            )
        }
    };

    if wrapper_references.is_empty() {
        return Ok(TemplateFoldResult::with_projection(
            template_emission_from_output_and_signal(output, signal_kind),
            provenance,
            projection_pieces,
        ));
    }

    add_ast_counter(
        AstCounter::TemplateWrapperApplications,
        wrapper_references.len(),
    );

    // Iterate wrappers forward (innermost-first), folding each around the current
    // child output. The output of one wrapper becomes the input to the next, so
    // forward consumption of the innermost-to-outermost store order yields the
    // outermost wrapper as the final layer, matching the structural wrap path.
    let mut current_output = output;
    let mut current_provenance = provenance;
    let mut current_projection = projection_pieces;
    for wrapper_reference in wrapper_references.iter() {
        let wrapper_result = fold_tir_wrapper_around_child_output(
            wrapper_reference,
            current_output,
            current_provenance,
            current_projection.as_deref(),
            fold_context,
            fold_input,
        )?;
        let TemplateFoldResult {
            emission,
            provenance,
            projection_pieces,
        } = wrapper_result;
        current_output = match emission {
            TemplateEmission::Output(output)
            | TemplateEmission::Break(Some(output))
            | TemplateEmission::Continue(Some(output)) => output,
            TemplateEmission::NoOutput
            | TemplateEmission::Break(None)
            | TemplateEmission::Continue(None) => {
                return Ok(TemplateFoldResult::with_projection(
                    emission,
                    provenance,
                    projection_pieces,
                ));
            }
        };
        current_provenance = provenance;
        current_projection = projection_pieces;
    }

    Ok(TemplateFoldResult::with_projection(
        template_emission_from_output_and_signal(current_output, signal_kind),
        current_provenance,
        current_projection,
    ))
}

/// Folds a single wrapper template around an already-folded child output string
/// without pushing synthetic nodes into the store.
///
/// WHAT: folds the wrapper template's root, injecting the child output at the
///       slot that the fill content would route to. For slot-less wrappers the
///       child output is appended after the wrapper content. The wrapper's own
///       `conditional_child_wrapper_set` is not applied, matching the structural
///       composed/prepended template which always carried `None`.
///
/// WHY: slot-bearing wrappers inject at their loose-fill target, while slot-less
///      wrappers append the child after their own content.
fn fold_tir_wrapper_around_child_output(
    wrapper_reference: &TemplateWrapperReference,
    child_output: StringId,
    child_provenance: SyntheticInterfaceProvenance,
    child_projection: Option<&[FoldedConstTemplatePiece]>,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let wrapper_store = fold_input.view.store();
    let wrapper_template = wrapper_store
        .get_template(wrapper_reference.root)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR wrapper fold: wrapper template {} not found in the module store.",
                wrapper_reference.root
            ))
        })?;

    let parent_view = fold_input.view;
    let wrapper_view = parent_view.wrapper(*wrapper_reference)?;
    let wrapper_fold_input = fold_input.with_view(&wrapper_view);

    fold_tir_wrapper_with_input(
        wrapper_reference.root,
        wrapper_template,
        child_output,
        child_provenance,
        child_projection,
        fold_context,
        &wrapper_fold_input,
    )
}

/// Folds one resolved wrapper template around an already-folded child output.
///
/// WHAT: applies the wrapper's effective slot routing and preserves injected
///      child precedence at the loose-fill target.
/// WHY: the same wrapper identity is shared across entry paths, so the
///      output walk must not discard its exact view.
#[allow(clippy::too_many_arguments)]
fn fold_tir_wrapper_with_input(
    wrapper_template_id: TemplateIrId,
    wrapper_template: &TemplateIr,
    child_output: StringId,
    child_provenance: SyntheticInterfaceProvenance,
    child_projection: Option<&[FoldedConstTemplatePiece]>,
    fold_context: &mut TirFoldContext<'_>,
    wrapper_fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<TemplateFoldResult, TemplateError> {
    let wrapper_store = wrapper_fold_input.view.store();
    reject_slot_insert_template(&wrapper_template.kind)?;

    if wrapper_template.runtime_slot_plan.is_some() {
        return Err(CompilerError::compiler_error(
            "TIR wrapper fold: a runtime slot plan reached the fold reducer without a foldable preparation proof.",
        )
        .into());
    }

    let child_output_len = fold_context.string_table.resolve(child_output).len();
    let estimated_bytes = wrapper_template.summary.estimated_output_bytes + child_output_len;
    let mut output_state = FoldOutputState::with_provenance(
        reserve_tir_fold_output_buffer(estimated_bytes),
        child_provenance,
    );
    if wrapper_fold_input.projection_enabled {
        output_state.enable_projection();
    }

    let schema = collect_tir_slot_schema(wrapper_store, wrapper_template_id)?;

    if !schema.has_any_slots() {
        // Slot-less wrappers keep their content before the inherited child.
        fold_tir_node_into_buffer(
            wrapper_template.root,
            &mut output_state,
            fold_context,
            wrapper_fold_input,
            FoldInsertion::None,
        )?;

        if wrapper_fold_input.projection_enabled {
            output_state.append_pieces(child_projection)?;
        } else {
            output_state
                .output_buffer
                .push_str(fold_context.string_table.resolve(child_output));
        }
        output_state.emitted_output = true;
    } else {
        // Slot-bearing wrappers inject at the loose-fill target first. Named-
        // only wrappers have no target, so their resolved slots are folded and
        // the child is appended after the wrapper content.
        let fill_target_key = schema.loose_fill_target_key();
        let insertion =
            fill_target_key
                .as_ref()
                .map_or(FoldInsertion::None, |key| FoldInsertion::Slot {
                    key,
                    output: child_output,
                    projection: child_projection,
                });
        fold_tir_node_into_buffer(
            wrapper_template.root,
            &mut output_state,
            fold_context,
            wrapper_fold_input,
            insertion,
        )?;

        if fill_target_key.is_none() {
            if wrapper_fold_input.projection_enabled {
                output_state.append_pieces(child_projection)?;
            } else {
                output_state
                    .output_buffer
                    .push_str(fold_context.string_table.resolve(child_output));
            }
            output_state.emitted_output = true;
        }
    }

    let actual_len = output_state.output_buffer.len();
    record_tir_fold_output_estimate_miss(actual_len, estimated_bytes);
    let output_id = fold_context
        .string_table
        .intern(&output_state.output_buffer);
    record_tir_fold_output_intern(actual_len);

    Ok(TemplateFoldResult::with_projection(
        TemplateEmission::Output(output_id),
        output_state.provenance,
        output_state.projection_pieces,
    ))
}

/// Folds a TIR aggregate wrapper subtree, replacing the `AggregateOutput` marker
/// with the already-folded aggregate string.
///
/// WHAT: walks the TIR subtree that the converter built from the AST aggregate
///       render plan, replacing the `AggregateOutput` marker with the already-folded
///       aggregate string.
/// WHY: aggregate output is a normal TIR marker, so the shared reducer can fold
///      the wrapper without a separate render-plan representation.
pub(super) fn fold_tir_aggregate_wrapper(
    wrapper_node_id: TemplateIrNodeId,
    aggregate_output: StringId,
    aggregate_projection: Option<&[FoldedConstTemplatePiece]>,
    output_state: &mut FoldOutputState,
    fold_context: &mut TirFoldContext<'_>,
    fold_input: &FoldTraversalInput<'_, '_>,
) -> Result<Option<TemplateLoopControlKind>, TemplateError> {
    let store = fold_input.view.store();
    let aggregate_output_len = fold_context.string_table.resolve(aggregate_output).len();
    let estimated_bytes = estimate_tir_node_output_bytes(
        store,
        wrapper_node_id,
        fold_context.string_table,
        FoldEstimateMode::Aggregate {
            output_len: aggregate_output_len,
        },
    )?;
    let mut wrapper_state = FoldOutputState::with_capacity(estimated_bytes);
    if fold_input.projection_enabled {
        wrapper_state.enable_projection();
    }

    let signal = fold_tir_node_into_buffer(
        wrapper_node_id,
        &mut wrapper_state,
        fold_context,
        fold_input,
        FoldInsertion::Aggregate {
            output: aggregate_output,
            projection: aggregate_projection,
        },
    )?;

    if signal.is_some() {
        return Err(CompilerError::compiler_error(
            "Loop-control signal reached aggregate wrapper folding; aggregate wrappers should not contain loop control.",
        )
        .into());
    }

    output_state.provenance.merge(&wrapper_state.provenance);

    if !wrapper_state.emitted_output {
        return Ok(None);
    }

    let actual_len = wrapper_state.output_buffer.len();
    record_tir_fold_output_estimate_miss(actual_len, estimated_bytes);
    let wrapper_id = fold_context
        .string_table
        .intern(&wrapper_state.output_buffer);
    record_tir_fold_output_intern(actual_len);
    let wrapper_projection = wrapper_state.projection_pieces.take();

    if output_state.projection_pieces.is_some() {
        output_state.append_pieces(wrapper_projection.as_deref())?;
    } else {
        output_state
            .output_buffer
            .push_str(fold_context.string_table.resolve(wrapper_id));
    }
    output_state.emitted_output = true;

    Ok(None)
}
