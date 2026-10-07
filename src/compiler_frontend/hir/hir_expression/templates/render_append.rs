//! Runtime-template append helpers.
//!
//! WHAT: appends AST-owned runtime-template nodes into a string accumulator and performs final
//! string coercion for dynamic chunks.
//! WHY: inline control-flow templates and aggregate wrapping share the same HIR concatenation
//! semantics, and runtime slot source/site plans use that same append path after AST has finished
//! routing and validation.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyEmission, TemplateLoopControlKind, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateBranch,
    OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
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
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::aggregate::RuntimeTemplateAggregateAppend;
use super::append_context::{RuntimeSlotLoopControlFlush, RuntimeTemplateAppendContext};
use super::is_owned_runtime_template_node_control_flow;

#[derive(Clone, Copy)]
struct OwnedRuntimeBranchChainAppend<'a, 'context> {
    branches: &'a [OwnedRuntimeTemplateBranch],
    fallback: Option<&'a OwnedRuntimeTemplateNode>,
    branch_index: usize,
    append_context: RuntimeTemplateAppendContext<'context>,
    aggregate_local: Option<LocalId>,
    span_ref: &'a Option<SourceSpan>,
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
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
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
            TemplateBodyEmission::Output,
            append_context,
            span_ref,
        )?;

        Ok(TemplateBodyEmission::Output)
    }

    fn append_expression_to_accumulator(
        &mut self,
        expression: &Expression,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        if let ExpressionKind::StringSlice(text) = &expression.kind
            && self.string_table.resolve(*text).is_empty()
        {
            return Ok(TemplateBodyEmission::NoOutput);
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

        Ok(TemplateBodyEmission::Output)
    }

    fn append_unresolved_slot_node_to_accumulator(
        &mut self,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        if append_context.rejects_unresolved_slots() {
            return_hir_transformation_error!(
                "Runtime template slot application reached HIR with an unresolved slot placeholder. AST slot routing should have converted it to a runtime slot site before HIR lowering.",
                self.hir_error_location(span_ref)
            );
        }

        Ok(TemplateBodyEmission::NoOutput)
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
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
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
                        TemplateBodyEmission::NoOutput => {}

                        TemplateBodyEmission::Output => {
                            emitted_output = true;
                        }

                        TemplateBodyEmission::Break | TemplateBodyEmission::Continue => {
                            return Ok(emission);
                        }
                    }

                    let current_block = self.current_block_id_or_error(span_ref)?;
                    if self.block_has_explicit_terminator(current_block, span_ref)? {
                        break;
                    }
                }

                Ok(if emitted_output {
                    TemplateBodyEmission::Output
                } else {
                    TemplateBodyEmission::NoOutput
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
                    return Ok(TemplateBodyEmission::NoOutput);
                }
                let whitespace_only = text_value.trim().is_empty();

                self.append_text_to_accumulator(
                    text_value,
                    append_context.target_accumulator,
                    span_ref,
                )?;

                // Whitespace-only text is appended to the accumulator for
                // rendering, but must not mark the runtime-slot emitted flag.
                // When a `continue` or `break` follows whitespace inside a
                // contribution source, loop control should discard the entire
                // iteration's output. If whitespace set the emitted flag, the
                // flush would replay the wrapper around the whitespace,
                // producing spurious wrapper tags (e.g. `<li>\n</li>`).
                // Treating whitespace as non-output for the emitted flag keeps
                // the wrapper conditional on meaningful content only.
                if whitespace_only {
                    return Ok(TemplateBodyEmission::NoOutput);
                }

                self.mark_owned_runtime_template_output_if_needed(
                    TemplateBodyEmission::Output,
                    append_context,
                    span_ref,
                )?;

                Ok(TemplateBodyEmission::Output)
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

            OwnedRuntimeTemplateNode::BranchChain {
                branches, fallback, ..
            } => self.append_owned_runtime_template_branch_chain(
                branches,
                fallback.as_deref(),
                append_context,
                aggregate_local,
                span_ref,
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
                    TemplateBodyEmission::Output,
                    append_context,
                    span_ref,
                )?;
                Ok(TemplateBodyEmission::Output)
            }

            OwnedRuntimeTemplateNode::LoopControl { kind, .. } => {
                if let Some(flush) = append_context.loop_control_flush {
                    self.flush_runtime_slot_application_for_loop_control(flush, *kind, span_ref)?;
                    return Ok(match kind {
                        TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                        TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                    });
                }

                self.emit_template_loop_control(*kind, span_ref)?;
                Ok(match kind {
                    TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                    TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                })
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
                // structural insertion points, not renderable chunks, so linear
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
        emission: TemplateBodyEmission,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if emission == TemplateBodyEmission::Output
            && let Some(flag) = append_context.emitted_output
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
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
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

    fn append_owned_runtime_template_branch_chain(
        &mut self,
        branches: &[OwnedRuntimeTemplateBranch],
        fallback: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        self.append_owned_runtime_template_branch_chain_from_index(
            branches,
            fallback,
            0,
            append_context,
            aggregate_local,
            span_ref,
        )
    }

    fn append_owned_runtime_template_branch_chain_from_index(
        &mut self,
        branches: &[OwnedRuntimeTemplateBranch],
        fallback: Option<&OwnedRuntimeTemplateNode>,
        branch_index: usize,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        let Some(branch) = branches.get(branch_index) else {
            return self.append_owned_runtime_template_fallback_branch(
                fallback,
                append_context,
                aggregate_local,
                span_ref,
            );
        };

        match &branch.selector {
            crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector::Bool(condition) => {
                self.lower_if_with_body_emitters(
                    condition,
                    &branch.span,
                    None,
                    |builder: &mut HirBuilder<'_>| {
                        builder.append_owned_runtime_template_node_to_accumulator(
                            &branch.body,
                            append_context,
                            aggregate_local,
                            &branch.span,
                        )?;
                        Ok(())
                    },
                    |builder: &mut HirBuilder<'_>| {
                        builder.append_owned_runtime_template_branch_chain_from_index(
                            branches,
                            fallback,
                            branch_index + 1,
                            append_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                )?;
                Ok(TemplateBodyEmission::Output)
            }

            crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => self
                .append_owned_runtime_option_present_branch_chain_arm(
                    branch,
                    scrutinee,
                    pattern,
                    OwnedRuntimeBranchChainAppend {
                        branches,
                        fallback,
                        branch_index,
                        append_context,
                        aggregate_local,
                        span_ref,
                    },
                ),
        }
    }

    fn append_owned_runtime_option_present_branch_chain_arm(
        &mut self,
        branch: &OwnedRuntimeTemplateBranch,
        scrutinee: &Expression,
        pattern: &MatchPattern,
        append: OwnedRuntimeBranchChainAppend<'_, '_>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        self.append_runtime_option_present_template_branch(
            scrutinee,
            pattern,
            &branch.span,
            |builder: &mut HirBuilder<'_>| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    &branch.body,
                    append.append_context,
                    append.aggregate_local,
                    &branch.span,
                )?;
                Ok(())
            },
            |builder: &mut HirBuilder<'_>| {
                builder.append_owned_runtime_template_branch_chain_from_index(
                    append.branches,
                    append.fallback,
                    append.branch_index + 1,
                    append.append_context,
                    append.aggregate_local,
                    append.span_ref,
                )?;
                Ok(())
            },
        )?;
        Ok(TemplateBodyEmission::Output)
    }

    fn append_owned_runtime_template_fallback_branch(
        &mut self,
        fallback: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        let Some(fallback) = fallback else {
            return Ok(TemplateBodyEmission::NoOutput);
        };

        self.append_owned_runtime_template_node_to_accumulator(
            fallback,
            append_context,
            aggregate_local,
            span_ref,
        )
    }

    fn append_owned_runtime_template_loop(
        &mut self,
        header: &TemplateLoopHeader,
        body: &OwnedRuntimeTemplateNode,
        aggregate_wrapper: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        let aggregate = self.initialize_runtime_template_accumulator(span_ref)?;
        let emitted_any_iteration = self.initialize_runtime_template_emitted_flag(span_ref)?;

        match header {
            TemplateLoopHeader::Conditional { condition } => {
                self.lower_while_with_body_emitter(
                    condition,
                    span_ref,
                    None,
                    |builder: &mut HirBuilder<'_>| {
                        let iteration_context = append_context
                            .with_target_accumulator(aggregate)
                            .with_emitted_output(Some(emitted_any_iteration));

                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            iteration_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                )?;
            }

            TemplateLoopHeader::Range { bindings, range } => {
                self.lower_range_loop_with_body_emitter(
                    bindings,
                    range,
                    span_ref,
                    |builder: &mut HirBuilder<'_>| {
                        let iteration_context = append_context
                            .with_target_accumulator(aggregate)
                            .with_emitted_output(Some(emitted_any_iteration));

                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            iteration_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                )?;
            }

            TemplateLoopHeader::Collection { bindings, iterable } => {
                self.lower_collection_loop_with_body_emitter(
                    bindings,
                    iterable,
                    span_ref,
                    |builder: &mut HirBuilder<'_>| {
                        let iteration_context = append_context
                            .with_target_accumulator(aggregate)
                            .with_emitted_output(Some(emitted_any_iteration));

                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            iteration_context,
                            aggregate_local,
                            span_ref,
                        )?;
                        Ok(())
                    },
                )?;
            }
        }

        self.append_owned_runtime_template_aggregate_wrapper_if_emitted(
            aggregate_wrapper,
            aggregate,
            emitted_any_iteration,
            append_context,
            aggregate_local,
            span_ref,
        )?;

        // The loop's emitted flag is runtime data: zero-iteration collection
        // loops and false conditional loops must not mark the surrounding
        // wrapper as emitted just because HIR built a loop CFG. When a parent
        // emitted flag exists, `append_runtime_template_aggregate_when_emitted`
        // marks it only on the runtime-emitted path.
        if append_context.emitted_output().is_some() {
            Ok(TemplateBodyEmission::NoOutput)
        } else {
            Ok(TemplateBodyEmission::Output)
        }
    }

    fn append_owned_runtime_template_aggregate_wrapper_if_emitted(
        &mut self,
        aggregate_wrapper: Option<&OwnedRuntimeTemplateNode>,
        aggregate: LocalId,
        emitted_output: LocalId,
        append_context: RuntimeTemplateAppendContext<'_>,
        _aggregate_local: Option<LocalId>,
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
    ) -> Result<Option<TemplateBodyEmission>, HirConstructionFailure> {
        let Some(candidate) = runtime_template_append_candidate_for_expression(expression) else {
            return Ok(None);
        };

        let (handoff, append_linear_handoff_directly) = match candidate {
            RuntimeTemplateAppendCandidate::SlotApplication(handoff) => {
                return self
                    .append_runtime_slot_application_with_context(
                        handoff,
                        append_context,
                        &expression.span,
                    )
                    .map(Some);
            }

            RuntimeTemplateAppendCandidate::TemplateHandoff { handoff } => match &handoff.body {
                OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => {
                    return self
                        .append_runtime_slot_application_with_context(
                            handoff,
                            append_context,
                            &expression.span,
                        )
                        .map(Some);
                }

                OwnedRuntimeTemplateBody::Render(node) => {
                    if is_owned_runtime_template_node_control_flow(node) {
                        return self
                            .append_nested_runtime_template_control_flow(
                                node,
                                append_context,
                                &expression.span,
                            )
                            .map(Some);
                    }

                    (handoff, true)
                }
            },
        };

        match &handoff.body {
            OwnedRuntimeTemplateBody::Render(node) => {
                // Slot helper wrappers can reach this point as linear templates
                // around an inner runtime slot application. Append only that
                // shape directly so ordinary template expressions keep their
                // value-lowering codegen.
                if owned_runtime_template_node_contains_runtime_slot_application(node) {
                    return self
                        .append_owned_runtime_template_node_to_accumulator(
                            node,
                            append_context,
                            None,
                            &expression.span,
                        )
                        .map(Some);
                }

                // Owned expression handoffs are already the final AST/HIR
                // boundary shape. Append linear owned nodes directly so simple
                // nested templates such as `[value]` keep the same accumulator
                // shape they had before the raw `Template` bridge was removed.
                if append_linear_handoff_directly {
                    return self
                        .append_owned_runtime_template_node_to_accumulator(
                            node,
                            append_context,
                            None,
                            &expression.span,
                        )
                        .map(Some);
                }

                Ok(None)
            }

            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => {
                unreachable!("runtime slot application handoffs return before render handling")
            }
        }
    }

    fn flush_runtime_slot_application_for_loop_control(
        &mut self,
        flush: RuntimeSlotLoopControlFlush<'_>,
        control_kind: TemplateLoopControlKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let condition_block = self.current_block_id_or_error(span_ref)?;
        let parent_region = self.current_region_or_error(span_ref)?;
        let flush_region = self.create_child_region(parent_region);
        let skip_region = self.create_child_region(parent_region);
        let flush_block = self.create_block(flush_region, span_ref, "runtime-slot-flush")?;
        let skip_block = self.create_block(skip_region, span_ref, "runtime-slot-skip")?;
        let condition = self.make_local_load_expression(
            flush.contribution_emitted_flag,
            builtin_type_ids::BOOL,
            span_ref,
            parent_region,
        )?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition,
                then_block: flush_block,
                else_block: skip_block,
            },
            span_ref,
        )?;

        // If a slot contribution produced output before loop control, replay the
        // wrapper on this terminating path before jumping to the surrounding
        // template loop target. The skip path still emits the same loop control
        // without rendering an empty wrapper.
        self.set_current_block(flush_block, span_ref)?;
        let wrapper_context = RuntimeTemplateAppendContext::new(flush.target_accumulator)
            .with_runtime_slot_sites(flush.source_accumulators, flush.slot_sites)
            .with_emitted_output(flush.parent_emitted_flag)
            .rejecting_unresolved_slots();
        self.append_owned_runtime_template_node_to_accumulator(
            flush.wrapper_plan,
            wrapper_context,
            None,
            span_ref,
        )?;

        let flush_tail = self.current_block_id_or_error(span_ref)?;
        if !self.block_has_explicit_terminator(flush_tail, span_ref)? {
            self.emit_template_loop_control(control_kind, span_ref)?;
        }

        self.set_current_block(skip_block, span_ref)?;
        self.emit_template_loop_control(control_kind, span_ref)
    }

    fn append_runtime_slot_site_to_accumulator(
        &mut self,
        site_id: RuntimeSlotSiteId,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
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
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        let Some(source_accumulators) = append_context.source_accumulators else {
            return_hir_transformation_error!(
                "Runtime slot source appeared outside an active runtime slot application.",
                self.hir_error_location(span_ref)
            );
        };
        let Some(source_accumulator) = source_accumulators.local_for(source_id) else {
            return_hir_transformation_error!(
                "Runtime slot site referenced a missing contribution source.",
                self.hir_error_location(span_ref)
            );
        };

        let region = self.current_region_or_error(span_ref)?;
        let source_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(source_accumulator)),
            builtin_type_ids::STRING,
            ValueKind::Place,
            region,
        )?;
        self.append_template_chunk_to_accumulator(
            source_value,
            append_context.target_accumulator,
            span_ref,
        )?;

        Ok(TemplateBodyEmission::Output)
    }

    fn emit_template_loop_control(
        &mut self,
        control_kind: TemplateLoopControlKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        match control_kind {
            TemplateLoopControlKind::Break => self.emit_break_to_current_loop(span_ref, None),
            TemplateLoopControlKind::Continue => self.emit_continue_to_current_loop(span_ref, None),
        }
    }

    fn append_nested_runtime_template_control_flow(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        match node {
            OwnedRuntimeTemplateNode::LoopControl {
                kind,
                span: control_span,
                ..
            } => {
                if let Some(flush) = append_context.loop_control_flush {
                    self.flush_runtime_slot_application_for_loop_control(
                        flush,
                        *kind,
                        control_span,
                    )?;
                    return Ok(match kind {
                        TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                        TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                    });
                }

                self.emit_template_loop_control(*kind, control_span)?;
                Ok(match kind {
                    TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                    TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                })
            }

            _ => {
                let emission = self.append_owned_runtime_template_node_to_accumulator(
                    node,
                    append_context,
                    None,
                    span_ref,
                )?;

                if append_context.emitted_output().is_some()
                    && emission == TemplateBodyEmission::Output
                {
                    return Ok(TemplateBodyEmission::NoOutput);
                }

                Ok(emission)
            }
        }
    }

    fn append_output_conditioned_runtime_wrapper(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        wrapper_node: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TemplateBodyEmission, HirConstructionFailure> {
        let child_accumulator = self.initialize_runtime_template_accumulator(span_ref)?;
        let child_emitted = self.initialize_runtime_template_emitted_flag(span_ref)?;
        let child_context = append_context
            .with_target_accumulator(child_accumulator)
            .with_emitted_output(Some(child_emitted));

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

        if matches!(
            emission,
            TemplateBodyEmission::Break | TemplateBodyEmission::Continue
        ) {
            return Ok(emission);
        }

        if append_context.emitted_output().is_some() && emission == TemplateBodyEmission::Output {
            return Ok(TemplateBodyEmission::NoOutput);
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
        // HIR lowering. Append-mode slot applications must see through that
        // wrapper so loop control does not escape through expression lowering
        // before the outer template accumulator receives the rendered wrapper.
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

fn expression_contains_runtime_slot_application(expression: &Expression) -> bool {
    let Some(candidate) = runtime_template_append_candidate_for_expression(expression) else {
        return false;
    };

    let handoff = match candidate {
        RuntimeTemplateAppendCandidate::SlotApplication(_) => return true,
        RuntimeTemplateAppendCandidate::TemplateHandoff { handoff, .. } => handoff,
    };

    match &handoff.body {
        OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => true,
        OwnedRuntimeTemplateBody::Render(node) => {
            owned_runtime_template_node_contains_runtime_slot_application(node)
        }
    }
}

fn owned_runtime_template_node_contains_runtime_slot_application(
    node: &OwnedRuntimeTemplateNode,
) -> bool {
    match node {
        OwnedRuntimeTemplateNode::Sequence { children, .. } => children
            .iter()
            .any(owned_runtime_template_node_contains_runtime_slot_application),

        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            expression_contains_runtime_slot_application(expression)
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => match &template.body {
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => true,
            OwnedRuntimeTemplateBody::Render(node) => {
                owned_runtime_template_node_contains_runtime_slot_application(node)
            }
        },

        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            owned_runtime_template_node_contains_runtime_slot_application(child)
                || owned_runtime_template_node_contains_runtime_slot_application(wrapper)
        }

        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            branches.iter().any(|branch| {
                owned_runtime_template_node_contains_runtime_slot_application(&branch.body)
            }) || fallback.as_ref().is_some_and(|fallback| {
                owned_runtime_template_node_contains_runtime_slot_application(fallback)
            })
        }

        OwnedRuntimeTemplateNode::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            owned_runtime_template_node_contains_runtime_slot_application(body)
                || aggregate_wrapper.as_ref().is_some_and(|wrapper| {
                    owned_runtime_template_node_contains_runtime_slot_application(wrapper)
                })
        }

        OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => false,
    }
}
