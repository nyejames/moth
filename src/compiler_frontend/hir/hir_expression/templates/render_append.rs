//! Runtime-template append helpers.
//!
//! WHAT: appends AST-owned runtime-template nodes into a string accumulator and performs final
//! string coercion for dynamic chunks.
//! WHY: template control flow and aggregate wrapping share the same HIR concatenation semantics,
//! and runtime slot source/site plans use that path after AST routing and validation.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::expression_store::{
    HirConstructionFailure, HirStringPieceRange,
};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::aggregate::RuntimeTemplateAggregateAppend;
use super::append_context::RuntimeTemplateAppendContext;

/// Classifies append construction; runtime output flags still track actual selected-path output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuntimeTemplateEmission {
    NoOutput,
    Output,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn initialize_runtime_template_accumulator(
        &mut self,
        span_ref: &Option<SourceSpan>,
    ) -> Result<LocalId, HirConstructionFailure> {
        let string_ty = builtin_type_ids::STRING;
        let accumulator = self.allocate_temp_local(string_ty, None)?;
        let region = self.current_region_or_error(span_ref)?;
        let empty_string = self.make_expression(
            span_ref,
            HirExpressionKind::StringLiteral(String::new()),
            string_ty,
            ValueKind::Const,
            region,
        )?;

        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(accumulator),
                value: empty_string,
            },
            span_ref,
        )?;

        Ok(accumulator)
    }

    fn append_aggregate_local_to_accumulator(
        &mut self,
        aggregate: LocalId,
        accumulator: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let region = self.current_region_or_error(span_ref)?;
        let aggregate_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(aggregate)),
            builtin_type_ids::STRING,
            ValueKind::Place,
            region,
        )?;

        self.append_template_chunk_to_accumulator(aggregate_value, accumulator, span_ref)
    }

    fn append_text_to_accumulator(
        &mut self,
        text: String,
        accumulator: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let region = self.current_region_or_error(span_ref)?;
        let chunk = self.make_expression(
            span_ref,
            HirExpressionKind::StringLiteral(text),
            builtin_type_ids::STRING,
            ValueKind::Const,
            region,
        )?;

        self.append_template_chunk_to_accumulator(chunk, accumulator, span_ref)
    }

    /// Appends one piece-bearing owned text payload to an accumulator.
    ///
    /// WHAT: converts the payload's ordered pieces into one structural constant chunk and
    ///       appends it with the ordinary chunk path, then marks the payload as structural
    ///       output.
    /// WHY: a payload reaching here carries at least one `Resource` or `SiteRoot` anchor, and
    ///       every anchor is real output; appending the whole list at once keeps authored order
    ///       and never fuses text across an anchor.
    fn append_structural_text_node_to_accumulator(
        &mut self,
        text: &OwnedFoldedString,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let OwnedFoldedString::Pieces(pieces) = text else {
            return_hir_transformation_error!(
                "Piece-bearing runtime-template text node carried neither plain text nor pieces.",
                self.hir_error_location(span_ref)
            );
        };

        let region = self.current_region_or_error(span_ref)?;
        let chunk_kind = HirExpressionKind::StructuralString {
            pieces: self.hir_pieces_from_owned_pieces(pieces, span_ref)?,
        };
        let chunk = self.make_expression(
            span_ref,
            chunk_kind,
            builtin_type_ids::STRING,
            ValueKind::Const,
            region,
        )?;

        self.append_template_chunk_to_accumulator(
            chunk,
            append_context.target_accumulator,
            span_ref,
        )?;

        self.mark_owned_runtime_template_output_if_needed(
            RuntimeTemplateEmission::Output,
            append_context,
            span_ref,
        )?;

        Ok(RuntimeTemplateEmission::Output)
    }

    fn append_expression_to_accumulator(
        &mut self,
        expression: &Expression,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        if let ExpressionKind::StringSlice(text) = &expression.kind
            && self.string_table.resolve(*text).is_empty()
        {
            return Ok(RuntimeTemplateEmission::NoOutput);
        }

        if let Some(emission) =
            self.append_runtime_template_expression_to_accumulator(expression, append_context)?
        {
            return Ok(emission);
        }

        let chunk = self.lower_expression_value_to_current_block(expression)?;
        self.append_template_chunk_to_accumulator(
            chunk,
            append_context.target_accumulator(),
            span_ref,
        )?;

        Ok(RuntimeTemplateEmission::Output)
    }

    fn append_unresolved_slot_node_to_accumulator(
        &mut self,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        if append_context.rejects_unresolved_slots() {
            return_hir_transformation_error!(
                "Runtime template slot application reached HIR with an unresolved slot placeholder. AST slot routing should have converted it to a runtime slot site before HIR lowering.",
                self.hir_error_location(span_ref)
            );
        }

        Ok(RuntimeTemplateEmission::NoOutput)
    }

    // WHAT: Appends an AST-owned runtime-template node into a string accumulator.
    // WHY: runtime templates now hand HIR an owned tree of runtime-template nodes
    // so HIR does not need raw TIR IDs or internal AST template-planning state.
    pub(super) fn append_owned_runtime_template_node_to_accumulator(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        match node {
            OwnedRuntimeTemplateNode::Sequence { children, .. } => {
                let mut emitted_output = false;

                for child in children {
                    let emission = self.append_owned_runtime_template_node_to_accumulator(
                        child,
                        append_context,
                        aggregate_local,
                        span_ref,
                    )?;

                    match emission {
                        RuntimeTemplateEmission::NoOutput => {}

                        RuntimeTemplateEmission::Output => {
                            emitted_output = true;
                        }
                    }

                    let current_block = self.current_block_id_or_error(span_ref)?;
                    if self.block_has_explicit_terminator(current_block, span_ref)? {
                        break;
                    }
                }

                Ok(if emitted_output {
                    RuntimeTemplateEmission::Output
                } else {
                    RuntimeTemplateEmission::NoOutput
                })
            }

            OwnedRuntimeTemplateNode::Text { text, .. } => {
                // Piece-bearing payloads append as one structural constant chunk in authored
                // order; every anchor inside the payload is a real output, so whitespace-only
                // trimming questions only apply to plain-text payloads.
                let Some(text_value) = text.clone().into_text() else {
                    return self.append_structural_text_node_to_accumulator(
                        text,
                        append_context,
                        span_ref,
                    );
                };

                if text_value.is_empty() {
                    return Ok(RuntimeTemplateEmission::NoOutput);
                }
                let whitespace_only = text_value.trim().is_empty();

                self.append_text_to_accumulator(
                    text_value,
                    append_context.target_accumulator,
                    span_ref,
                )?;

                // Preserve whitespace bytes for replay, but don't let whitespace
                // alone select a wrapper that depends on structural output.
                if whitespace_only {
                    return Ok(RuntimeTemplateEmission::NoOutput);
                }

                self.mark_owned_runtime_template_output_if_needed(
                    RuntimeTemplateEmission::Output,
                    append_context,
                    span_ref,
                )?;

                Ok(RuntimeTemplateEmission::Output)
            }

            OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
                let emission =
                    self.append_expression_to_accumulator(expression, append_context, span_ref)?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    span_ref,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
                let emission = self.append_owned_runtime_template_child_to_accumulator(
                    template,
                    append_context,
                    aggregate_local,
                    span_ref,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    span_ref,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => self
                .append_output_conditioned_runtime_wrapper(
                    child,
                    wrapper,
                    append_context,
                    span_ref,
                ),

            OwnedRuntimeTemplateNode::Conditional {
                selector,
                body,
                span,
            } => self.append_owned_runtime_template_conditional(
                selector,
                body,
                append_context,
                aggregate_local,
                span,
            ),

            OwnedRuntimeTemplateNode::Loop {
                header,
                body,
                aggregate_wrapper,
                ..
            } => self.append_owned_runtime_template_loop(
                header,
                body,
                aggregate_wrapper.as_deref(),
                append_context,
                aggregate_local,
                span_ref,
            ),

            OwnedRuntimeTemplateNode::AggregateOutput => {
                let Some(aggregate) = aggregate_local else {
                    return_hir_transformation_error!(
                        "Owned runtime template aggregate output appeared outside an aggregate wrapper context.",
                        self.hir_error_location(span_ref)
                    );
                };

                self.append_aggregate_local_to_accumulator(
                    aggregate,
                    append_context.target_accumulator,
                    span_ref,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    RuntimeTemplateEmission::Output,
                    append_context,
                    span_ref,
                )?;
                Ok(RuntimeTemplateEmission::Output)
            }

            OwnedRuntimeTemplateNode::RuntimeSlotSite { site, .. } => {
                let emission =
                    self.append_runtime_slot_site_to_accumulator(*site, append_context, span_ref)?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    span_ref,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { source, .. } => {
                let emission = self.append_runtime_slot_source_to_accumulator(
                    *source,
                    append_context,
                    span_ref,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    span_ref,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::Slot { .. } => {
                // Wrapper-shaped templates can reach HIR as runtime values when
                // they are not used as helpers. Their slot placeholders are
                // structural insertion points, not renderable chunks, so ordinary
                // rendering skips them just as the old flattened expression
                // path did. Inside an active runtime slot application wrapper
                // the placeholder should have been resolved to a site by AST
                // routing, so the reject policy raises an internal compiler error.
                self.append_unresolved_slot_node_to_accumulator(append_context, span_ref)
            }
        }
    }

    fn mark_owned_runtime_template_output_if_needed(
        &mut self,
        emission: RuntimeTemplateEmission,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if emission == RuntimeTemplateEmission::Output
            && let Some(flag) = append_context.emitted_output()
        {
            self.mark_runtime_template_output_emitted(flag, span_ref)?;
        }

        Ok(())
    }

    fn append_owned_runtime_template_child_to_accumulator(
        &mut self,
        template: &OwnedRuntimeTemplateHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        match &template.body {
            OwnedRuntimeTemplateBody::Render(node) => self
                .append_owned_runtime_template_node_to_accumulator(
                    node,
                    append_context,
                    aggregate_local,
                    span_ref,
                ),

            OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => {
                self.append_runtime_slot_application_with_context(handoff, append_context, span_ref)
            }
        }
    }

    fn append_owned_runtime_template_conditional(
        &mut self,
        selector: &TemplateBranchSelector,
        body: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        match selector {
            TemplateBranchSelector::Bool(condition) => {
                self.lower_if_with_body_emitters(
                    condition,
                    span_ref,
                    None,
                    |builder: &mut HirBuilder<'_>| {
                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            append_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                    |_builder: &mut HirBuilder<'_>| Ok(()),
                )?;
            }

            TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
                self.append_runtime_option_present_template_branch(
                    scrutinee,
                    pattern,
                    span_ref,
                    |builder: &mut HirBuilder<'_>| {
                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            append_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                )?;
            }
        };

        // The branch callback owns emitted-output flag updates. Selection alone says
        // nothing about whether a runtime body produced structural output.
        Ok(if append_context.emitted_output().is_some() {
            RuntimeTemplateEmission::NoOutput
        } else {
            RuntimeTemplateEmission::Output
        })
    }

    fn append_owned_runtime_template_loop(
        &mut self,
        header: &TemplateLoopHeader,
        body: &OwnedRuntimeTemplateNode,
        aggregate_wrapper: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let aggregate = self.initialize_runtime_template_accumulator(span_ref)?;
        let emitted_any_iteration = self.initialize_runtime_template_emitted_flag(span_ref)?;
        let emit_template_iteration =
            |builder: &mut HirBuilder<'_>| -> Result<(), HirConstructionFailure> {
                let iteration_context = append_context
                    .with_target_accumulator(aggregate)
                    .with_emitted_output(emitted_any_iteration);

                builder.append_owned_runtime_template_node_to_accumulator(
                    body,
                    iteration_context,
                    aggregate_local,
                    span_ref,
                )?;
                Ok(())
            };

        match header {
            TemplateLoopHeader::Conditional { condition } => {
                self.lower_while_with_body_emitter(
                    condition,
                    span_ref,
                    None,
                    emit_template_iteration,
                )?;
            }

            TemplateLoopHeader::Range { bindings, range } => {
                self.lower_range_loop_with_body_emitter(
                    bindings,
                    range,
                    span_ref,
                    emit_template_iteration,
                )?;
            }

            TemplateLoopHeader::Collection { bindings, iterable } => {
                self.lower_collection_loop_with_body_emitter(
                    bindings,
                    iterable,
                    span_ref,
                    emit_template_iteration,
                )?;
            }
        }

        self.append_owned_runtime_template_aggregate_wrapper_if_emitted(
            aggregate_wrapper,
            aggregate,
            emitted_any_iteration,
            append_context,
            span_ref,
        )?;

        // The loop's emitted flag is runtime data: zero-iteration collection
        // loops and false conditional loops must not mark the surrounding
        // wrapper as emitted just because HIR built a loop CFG. The aggregate
        // append callback propagates output only on the runtime-emitted path.
        if append_context.emitted_output().is_some() {
            Ok(RuntimeTemplateEmission::NoOutput)
        } else {
            Ok(RuntimeTemplateEmission::Output)
        }
    }

    fn append_owned_runtime_template_aggregate_wrapper_if_emitted(
        &mut self,
        aggregate_wrapper: Option<&OwnedRuntimeTemplateNode>,
        aggregate: LocalId,
        emitted_output: LocalId,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let Some(aggregate_wrapper) = aggregate_wrapper else {
            return Ok(());
        };

        self.append_runtime_template_aggregate_when_emitted(
            super::aggregate::RuntimeTemplateAggregateAppend {
                aggregate,
                emitted_output,
                append_context,
            },
            span_ref,
            |builder, append, span_ref| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    aggregate_wrapper,
                    append.append_context,
                    Some(append.aggregate),
                    span_ref,
                )?;
                Ok(())
            },
        )
    }

    fn append_runtime_template_expression_to_accumulator(
        &mut self,
        expression: &Expression,
        append_context: RuntimeTemplateAppendContext<'_>,
    ) -> Result<Option<RuntimeTemplateEmission>, HirConstructionFailure> {
        let Some(candidate) = runtime_template_append_candidate_for_expression(expression) else {
            return Ok(None);
        };

        match candidate {
            RuntimeTemplateAppendCandidate::SlotApplication(handoff) => self
                .append_runtime_slot_application_with_context(
                    handoff,
                    append_context,
                    &expression.span,
                )
                .map(Some),

            RuntimeTemplateAppendCandidate::TemplateHandoff { handoff } => match &handoff.body {
                OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => self
                    .append_runtime_slot_application_with_context(
                        handoff,
                        append_context,
                        &expression.span,
                    )
                    .map(Some),

                OwnedRuntimeTemplateBody::Render(node) => self
                    .append_owned_runtime_template_node_to_accumulator(
                        node,
                        append_context,
                        None,
                        &expression.span,
                    )
                    .map(Some),
            },
        }
    }

    fn append_runtime_slot_site_to_accumulator(
        &mut self,
        site_id: RuntimeSlotSiteId,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let Some(slot_sites) = append_context.slot_sites else {
            return_hir_transformation_error!(
                "Runtime slot site appeared outside an active runtime slot application.",
                self.hir_error_location(span_ref)
            );
        };
        let Some(site) = slot_sites
            .get(site_id.0)
            .filter(|site| site.site == site_id)
        else {
            return_hir_transformation_error!(
                "Runtime slot application wrapper referenced a missing slot site.",
                self.hir_error_location(span_ref)
            );
        };

        self.append_owned_runtime_template_node_to_accumulator(
            &site.render_root,
            append_context,
            None,
            &site.span,
        )
    }

    fn append_runtime_slot_source_to_accumulator(
        &mut self,
        source_id: RuntimeSlotContributionSourceId,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let Some(source_accumulators) = append_context.source_accumulators else {
            return_hir_transformation_error!(
                "Runtime slot source appeared outside an active runtime slot application.",
                self.hir_error_location(span_ref)
            );
        };
        let Some(source_locals) = source_accumulators.for_source(source_id) else {
            return_hir_transformation_error!(
                "Runtime slot site referenced a missing contribution source.",
                self.hir_error_location(span_ref)
            );
        };

        // Replay authored bytes even when whitespace-only text did not set the structural flag.
        self.append_aggregate_local_to_accumulator(
            source_locals.accumulator,
            append_context.target_accumulator(),
            span_ref,
        )?;

        // Structural emission gates parent flags, not replay of the source buffer.
        let Some(parent_emitted_output) = append_context.emitted_output() else {
            return Ok(RuntimeTemplateEmission::Output);
        };

        if let Some(source_emitted_output) = source_locals.emitted_output {
            self.append_runtime_template_aggregate_when_emitted(
                RuntimeTemplateAggregateAppend {
                    aggregate: source_locals.accumulator,
                    emitted_output: source_emitted_output,
                    append_context,
                },
                span_ref,
                |builder, append, span_ref| {
                    builder.mark_owned_runtime_template_output_if_needed(
                        RuntimeTemplateEmission::Output,
                        append.append_context,
                        span_ref,
                    )
                },
            )?;
        } else {
            // A source without a local tracker was proven to produce output, so
            // propagate that fact directly to the active parent tracker.
            self.mark_runtime_template_output_emitted(parent_emitted_output, span_ref)?;
        }

        Ok(RuntimeTemplateEmission::NoOutput)
    }

    fn append_output_conditioned_runtime_wrapper(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        wrapper_node: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RuntimeTemplateEmission, HirConstructionFailure> {
        let child_accumulator = self.initialize_runtime_template_accumulator(span_ref)?;
        let child_emitted = self.initialize_runtime_template_emitted_flag(span_ref)?;
        let child_context = append_context
            .with_target_accumulator(child_accumulator)
            .with_emitted_output(child_emitted);

        let emission = self.append_owned_runtime_template_node_to_accumulator(
            node,
            child_context,
            None,
            span_ref,
        )?;

        // Append the owned wrapper node only when the child structurally emitted
        // output. The wrapper node carries the same AggregateOutput marker that
        // loop aggregate wrappers use, so passing the child accumulator as the
        // aggregate local lets the owned-node append path splice it in.
        self.append_runtime_template_aggregate_when_emitted(
            RuntimeTemplateAggregateAppend {
                aggregate: child_accumulator,
                emitted_output: child_emitted,
                append_context,
            },
            span_ref,
            |builder, append, span_ref| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    wrapper_node,
                    append.append_context,
                    Some(append.aggregate),
                    span_ref,
                )?;
                Ok(())
            },
        )?;

        if append_context.emitted_output().is_some() && emission == RuntimeTemplateEmission::Output
        {
            return Ok(RuntimeTemplateEmission::NoOutput);
        }

        Ok(emission)
    }

    fn append_template_chunk_to_accumulator(
        &mut self,
        chunk: HirValueId,
        accumulator: LocalId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let string_ty = builtin_type_ids::STRING;
        let region = self.current_region_or_error(span_ref)?;
        let accumulated = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(accumulator)),
            string_ty,
            ValueKind::Place,
            region,
        )?;
        let chunk_as_string =
            self.coerce_expression_to_string(chunk, span_ref, string_ty, region)?;
        let next_value = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: accumulated,
                op: HirBinOp::StringAppend,
                right: chunk_as_string,
            },
            string_ty,
            ValueKind::RValue,
            region,
        )?;

        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(accumulator)),
                value: next_value,
            },
            span_ref,
        )
    }

    pub(super) fn coerce_expression_to_string(
        &mut self,
        expression: HirValueId,
        span_ref: &Option<SourceSpan>,
        string_ty: TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let expression_type = self.module.expressions.expression(expression).ty;
        if expression_type == builtin_type_ids::STRING {
            return Ok(expression);
        }

        if expression_type == self.type_environment.builtins().none {
            return self.make_expression(
                span_ref,
                HirExpressionKind::StringLiteral(String::new()),
                string_ty,
                ValueKind::Const,
                region,
            );
        }

        // `Float` template chunks must use the Moth-owned formatter instead of target-native
        // stringification so casts and templates share one formatting contract.
        // Template chunk formatting is generated scaffolding: the `FormatFloat` stays spanless.
        if expression_type == self.type_environment.builtins().float {
            return self.emit_formatted_float_value(expression, span_ref);
        }
        if let Some(scale) = self.type_environment.number_scale(expression_type) {
            return self.make_expression(
                span_ref,
                HirExpressionKind::Cast {
                    source: expression,
                    policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Number(scale)),
                },
                string_ty,
                ValueKind::RValue,
                region,
            );
        }

        // Profile-sized `Uint` chunks render through the same numeric text policy as
        // explicit casts, so a chunk never reaches `StringAppend` as a raw number.
        // Fixed numeric chunks share this rule; `Byte` is not numeric and keeps the
        // append path; template validation rejects it before lowering.
        if expression_type == self.type_environment.builtins().uint
            || self
                .type_environment
                .fixed_scalar(expression_type)
                .and_then(|scalar| BuiltinCastTarget::Fixed(scalar).numeric_scalar())
                .is_some()
        {
            let numeric = NumericScalar::from_type_id(expression_type, &self.type_environment)
                .unwrap_or(NumericScalar::Uint);
            return self.make_expression(
                span_ref,
                HirExpressionKind::Cast {
                    source: expression,
                    policy: BuiltinCastPolicyId::NumericToString(numeric),
                },
                string_ty,
                ValueKind::RValue,
                region,
            );
        }

        let empty = self.make_expression(
            span_ref,
            HirExpressionKind::StringLiteral(String::new()),
            string_ty,
            ValueKind::Const,
            region,
        )?;

        self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: empty,
                op: crate::compiler_frontend::hir::operators::HirBinOp::StringAppend,
                right: expression,
            },
            string_ty,
            ValueKind::RValue,
            region,
        )
    }

    // ------------------------
    //  Structural string composition
    // ------------------------

    /// Converts one ordered owned piece list into module-local HIR pieces.
    fn hir_pieces_from_owned_pieces(
        &mut self,
        pieces: &[OwnedFoldedStringPiece],
        span_ref: &Option<SourceSpan>,
    ) -> Result<HirStringPieceRange, HirConstructionFailure> {
        let mut hir_pieces = Vec::with_capacity(pieces.len());

        for piece in pieces {
            let hir_piece = match piece {
                OwnedFoldedStringPiece::Text(text) => {
                    ConstStringPiece::Text(self.string_table.intern(text.as_str()))
                }

                OwnedFoldedStringPiece::Resource(origin) => ConstStringPiece::Resource(
                    self.intern_handoff_resource_origin(origin, span_ref)?,
                ),

                OwnedFoldedStringPiece::SiteRoot => ConstStringPiece::SiteRoot,
            };

            hir_pieces.push(hir_piece);
        }

        self.module
            .expressions
            .append_string_pieces(&hir_pieces, *span_ref)
    }
}

enum RuntimeTemplateAppendCandidate<'a> {
    SlotApplication(&'a OwnedRuntimeSlotApplicationHandoff),
    TemplateHandoff {
        handoff: &'a OwnedRuntimeTemplateHandoff,
    },
}

fn runtime_template_append_candidate_for_expression(
    expression: &Expression,
) -> Option<RuntimeTemplateAppendCandidate<'_>> {
    match &expression.kind {
        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            Some(RuntimeTemplateAppendCandidate::SlotApplication(handoff))
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            Some(RuntimeTemplateAppendCandidate::TemplateHandoff { handoff })
        }

        // String-boundary coercions are inserted around template helpers before
        // HIR lowering. Append-mode slot applications need to see through them
        // so they append into the active template accumulator.
        ExpressionKind::Coerced { value, .. } => {
            runtime_template_append_candidate_for_expression(value)
        }

        ExpressionKind::Runtime(rpn) if rpn.items.len() == 1 => match &rpn.items[0] {
            ExpressionRpnItem::Operand(expression) => {
                runtime_template_append_candidate_for_expression(expression)
            }
            ExpressionRpnItem::Operator { .. } => None,
            // Pending syntax never survives evaluation; flag it in debug builds so the invalid
            // boundary is visible instead of silently reporting no append candidate.
            ExpressionRpnItem::PendingNumericLiteral { .. }
            | ExpressionRpnItem::PendingGroup { .. } => {
                debug_assert!(
                    false,
                    "pending expression syntax reached HIR render-append query"
                );
                None
            }
        },

        _ => None,
    }
}
