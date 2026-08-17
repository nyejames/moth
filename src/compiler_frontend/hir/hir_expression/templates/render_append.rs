//! Runtime-template append helpers.
//!
//! WHAT: appends AST-owned runtime-template nodes into a string accumulator and performs final
//! string coercion for dynamic chunks.
//! WHY: inline control-flow templates and aggregate wrapping share the same HIR concatenation
//! semantics, and runtime slot source/site plans use that same append path after AST has finished
//! routing and validation.

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
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::ids::{LocalId, RegionId};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
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
    location: &'a SourceLocation,
}

impl<'a> HirBuilder<'a> {
    /// Lowers a reactive linear template from a TIR-owned handoff node.
    ///
    /// WHAT: builds a lazy string expression tree from the owned runtime-template
    /// node. Direct `$(source)` splices and nested reactive template values stay
    /// inside the returned expression, while ordinary dynamic reads are
    /// materialized into snapshot locals once.
    /// WHY: this keeps reactive linear lowering on the owned handoff path while
    /// preserving per-segment subscription markers without HIR consuming raw
    /// TIR IDs.
    pub(super) fn lower_runtime_reactive_linear_template_expression_from_owned_node(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let string_ty = builtin_type_ids::STRING;
        let region = self.current_region_or_error(location)?;
        let mut rendered = self.make_expression(
            location,
            HirExpressionKind::StringLiteral(String::new()),
            string_ty,
            ValueKind::Const,
            region,
        );

        self.append_owned_runtime_template_node_to_reactive_linear_expression(
            node,
            location,
            &mut rendered,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value: rendered,
        })
    }

    fn append_owned_runtime_template_node_to_reactive_linear_expression(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        location: &SourceLocation,
        rendered: &mut HirExpression,
    ) -> Result<(), CompilerError> {
        let string_ty = builtin_type_ids::STRING;

        match node {
            OwnedRuntimeTemplateNode::Sequence { children, .. } => {
                for child in children {
                    self.append_owned_runtime_template_node_to_reactive_linear_expression(
                        child, location, rendered,
                    )?;
                }
            }

            OwnedRuntimeTemplateNode::Text {
                text,
                location: text_location,
                ..
            } => {
                let text_value = self.string_table.resolve(*text).to_owned();
                let region = self.current_region_or_error(text_location)?;
                let chunk = self.make_expression(
                    text_location,
                    HirExpressionKind::StringLiteral(text_value),
                    string_ty,
                    ValueKind::Const,
                    region,
                );

                let region = self.current_region_or_error(location)?;
                *rendered = self.make_expression(
                    location,
                    HirExpressionKind::BinOp {
                        left: Box::new(rendered.clone()),
                        op: HirBinOp::StringAppend,
                        right: Box::new(chunk),
                    },
                    string_ty,
                    ValueKind::RValue,
                    region,
                );
            }

            OwnedRuntimeTemplateNode::DynamicExpression {
                expression,
                reactive_subscription,
                ..
            } => {
                let chunk = self.lower_reactive_linear_expression_chunk(
                    expression,
                    reactive_subscription.is_some(),
                )?;

                let region = self.current_region_or_error(location)?;
                *rendered = self.make_expression(
                    location,
                    HirExpressionKind::BinOp {
                        left: Box::new(rendered.clone()),
                        op: HirBinOp::StringAppend,
                        right: Box::new(chunk),
                    },
                    string_ty,
                    ValueKind::RValue,
                    region,
                );
            }

            OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
                let chunk = self.lower_reactive_linear_child_template_chunk(template)?;

                let region = self.current_region_or_error(location)?;
                *rendered = self.make_expression(
                    location,
                    HirExpressionKind::BinOp {
                        left: Box::new(rendered.clone()),
                        op: HirBinOp::StringAppend,
                        right: Box::new(chunk),
                    },
                    string_ty,
                    ValueKind::RValue,
                    region,
                );
            }

            OwnedRuntimeTemplateNode::ConditionalWrapper { .. } => {
                return_hir_transformation_error!(
                    "Reactive linear template lowering received an output-conditioned wrapper node.",
                    self.hir_error_location(location)
                );
            }

            OwnedRuntimeTemplateNode::Slot { .. } => {}

            OwnedRuntimeTemplateNode::BranchChain { .. }
            | OwnedRuntimeTemplateNode::Loop { .. }
            | OwnedRuntimeTemplateNode::AggregateOutput
            | OwnedRuntimeTemplateNode::LoopControl { .. }
            | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
            | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. } => {
                return_hir_transformation_error!(
                    "Reactive linear template lowering received a non-linear owned node. Reactive control-flow and slot sites must use their dedicated lowering paths.",
                    self.hir_error_location(location)
                );
            }
        }

        Ok(())
    }

    fn lower_reactive_linear_child_template_chunk(
        &mut self,
        template: &OwnedRuntimeTemplateHandoff,
    ) -> Result<HirExpression, CompilerError> {
        match &template.body {
            OwnedRuntimeTemplateBody::Render(node) => {
                if is_owned_runtime_template_node_control_flow(node) {
                    return_hir_transformation_error!(
                        "Reactive linear template lowering received a nested control-flow child template.",
                        self.hir_error_location(&template.location)
                    );
                }

                // A nested reactive template value stays lazy like a direct subscription.
                if self.owned_runtime_template_node_has_runtime_dependency(node) {
                    self.lower_runtime_reactive_linear_template_expression_from_owned_node(
                        node,
                        &template.location,
                    )
                    .map(|lowered| lowered.value)
                } else {
                    let chunk = self.lower_runtime_linear_template_expression_from_owned_node(
                        node,
                        &template.location,
                    )?;
                    self.materialize_reactive_template_snapshot_chunk(chunk, &template.location)
                }
            }

            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => {
                return_hir_transformation_error!(
                    "Reactive linear template lowering received a nested runtime slot application.",
                    self.hir_error_location(&template.location)
                )
            }
        }
    }

    pub(super) fn owned_runtime_template_node_has_runtime_dependency(
        &self,
        node: &OwnedRuntimeTemplateNode,
    ) -> bool {
        match node {
            OwnedRuntimeTemplateNode::Sequence { children, .. } => children
                .iter()
                .any(|child| self.owned_runtime_template_node_has_runtime_dependency(child)),

            OwnedRuntimeTemplateNode::DynamicExpression {
                expression,
                reactive_subscription,
                ..
            } => {
                reactive_subscription.is_some()
                    || expression
                        .reactive_template
                        .as_ref()
                        .is_some_and(|metadata| metadata.has_runtime_dependency())
            }

            OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
                self.owned_runtime_template_handoff_has_runtime_dependency(template)
            }

            OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
                self.owned_runtime_template_node_has_runtime_dependency(child)
                    || self.owned_runtime_template_node_has_runtime_dependency(wrapper)
            }

            OwnedRuntimeTemplateNode::BranchChain {
                branches, fallback, ..
            } => {
                branches.iter().any(|branch| {
                    self.owned_runtime_template_node_has_runtime_dependency(&branch.body)
                }) || fallback.as_ref().is_some_and(|fallback| {
                    self.owned_runtime_template_node_has_runtime_dependency(fallback)
                })
            }

            OwnedRuntimeTemplateNode::Loop {
                body,
                aggregate_wrapper,
                ..
            } => {
                self.owned_runtime_template_node_has_runtime_dependency(body)
                    || aggregate_wrapper.as_ref().is_some_and(|wrapper| {
                        self.owned_runtime_template_node_has_runtime_dependency(wrapper)
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

    fn owned_runtime_template_handoff_has_runtime_dependency(
        &self,
        handoff: &OwnedRuntimeTemplateHandoff,
    ) -> bool {
        match &handoff.body {
            OwnedRuntimeTemplateBody::Render(node) => {
                self.owned_runtime_template_node_has_runtime_dependency(node)
            }
            OwnedRuntimeTemplateBody::RuntimeSlotApplication(_) => false,
        }
    }

    pub(super) fn initialize_runtime_template_accumulator(
        &mut self,
        location: &SourceLocation,
    ) -> Result<LocalId, CompilerError> {
        let string_ty = builtin_type_ids::STRING;
        let accumulator = self.allocate_temp_local(string_ty, Some(location.clone()))?;
        let region = self.current_region_or_error(location)?;
        let empty_string = self.make_expression(
            location,
            HirExpressionKind::StringLiteral(String::new()),
            string_ty,
            ValueKind::Const,
            region,
        );

        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(accumulator),
                value: empty_string,
            },
            location,
        )?;

        Ok(accumulator)
    }

    fn append_aggregate_local_to_accumulator(
        &mut self,
        aggregate: LocalId,
        accumulator: LocalId,
        fallback_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let region = self.current_region_or_error(fallback_location)?;
        let aggregate_value = self.make_expression(
            fallback_location,
            HirExpressionKind::Load(HirPlace::Local(aggregate)),
            builtin_type_ids::STRING,
            ValueKind::Place,
            region,
        );

        self.append_template_chunk_to_accumulator(aggregate_value, accumulator, fallback_location)
    }

    fn append_string_id_to_accumulator(
        &mut self,
        text: StringId,
        accumulator: LocalId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let text_value = self.string_table.resolve(text).to_owned();
        let region = self.current_region_or_error(location)?;
        let chunk = self.make_expression(
            location,
            HirExpressionKind::StringLiteral(text_value),
            builtin_type_ids::STRING,
            ValueKind::Const,
            region,
        );

        self.append_template_chunk_to_accumulator(chunk, accumulator, location)
    }

    fn append_expression_to_accumulator(
        &mut self,
        expression: &Expression,
        append_context: RuntimeTemplateAppendContext<'_>,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
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
            fallback_location,
        )?;

        Ok(TemplateBodyEmission::Output)
    }

    fn append_unresolved_slot_node_to_accumulator(
        &mut self,
        append_context: RuntimeTemplateAppendContext<'_>,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        if append_context.rejects_unresolved_slots() {
            return_hir_transformation_error!(
                "Runtime template slot application reached HIR with an unresolved slot placeholder. AST slot routing should have converted it to a runtime slot site before HIR lowering.",
                self.hir_error_location(location)
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
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        match node {
            OwnedRuntimeTemplateNode::Sequence { children, .. } => {
                let mut emitted_output = false;

                for child in children {
                    let emission = self.append_owned_runtime_template_node_to_accumulator(
                        child,
                        append_context,
                        aggregate_local,
                        fallback_location,
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

                    let current_block = self.current_block_id_or_error(fallback_location)?;
                    if self.block_has_explicit_terminator(current_block, fallback_location)? {
                        break;
                    }
                }

                Ok(if emitted_output {
                    TemplateBodyEmission::Output
                } else {
                    TemplateBodyEmission::NoOutput
                })
            }

            OwnedRuntimeTemplateNode::Text { text, location, .. } => {
                if self.string_table.resolve(*text).is_empty() {
                    return Ok(TemplateBodyEmission::NoOutput);
                }

                self.append_string_id_to_accumulator(
                    *text,
                    append_context.target_accumulator,
                    location,
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
                let is_whitespace = self.string_table.resolve(*text).trim().is_empty();
                if is_whitespace {
                    return Ok(TemplateBodyEmission::NoOutput);
                }

                self.mark_owned_runtime_template_output_if_needed(
                    TemplateBodyEmission::Output,
                    append_context,
                    location,
                )?;
                Ok(TemplateBodyEmission::Output)
            }

            OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
                let emission = self.append_expression_to_accumulator(
                    expression,
                    append_context,
                    fallback_location,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    fallback_location,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
                let emission = self.append_owned_runtime_template_child_to_accumulator(
                    template,
                    append_context,
                    aggregate_local,
                    fallback_location,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    fallback_location,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::ConditionalWrapper {
                child,
                wrapper,
                location,
            } => self.append_output_conditioned_runtime_wrapper(
                child,
                wrapper,
                append_context,
                location,
            ),

            OwnedRuntimeTemplateNode::BranchChain {
                branches,
                fallback,
                location,
            } => self.append_owned_runtime_template_branch_chain(
                branches,
                fallback.as_deref(),
                append_context,
                aggregate_local,
                location,
            ),

            OwnedRuntimeTemplateNode::Loop {
                header,
                body,
                aggregate_wrapper,
                location,
            } => self.append_owned_runtime_template_loop(
                header,
                body,
                aggregate_wrapper.as_deref(),
                append_context,
                aggregate_local,
                location,
            ),

            OwnedRuntimeTemplateNode::AggregateOutput => {
                let Some(aggregate) = aggregate_local else {
                    return_hir_transformation_error!(
                        "Owned runtime template aggregate output appeared outside an aggregate wrapper context.",
                        self.hir_error_location(fallback_location)
                    );
                };

                self.append_aggregate_local_to_accumulator(
                    aggregate,
                    append_context.target_accumulator,
                    fallback_location,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    TemplateBodyEmission::Output,
                    append_context,
                    fallback_location,
                )?;
                Ok(TemplateBodyEmission::Output)
            }

            OwnedRuntimeTemplateNode::LoopControl { kind, location } => {
                if let Some(flush) = append_context.loop_control_flush {
                    self.flush_runtime_slot_application_for_loop_control(flush, *kind, location)?;
                    return Ok(match kind {
                        TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                        TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                    });
                }

                self.emit_template_loop_control(*kind, location)?;
                Ok(match kind {
                    TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                    TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                })
            }

            OwnedRuntimeTemplateNode::RuntimeSlotSite { site, .. } => {
                let emission = self.append_runtime_slot_site_to_accumulator(
                    *site,
                    append_context,
                    fallback_location,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    fallback_location,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { source } => {
                let emission = self.append_runtime_slot_source_to_accumulator(
                    *source,
                    append_context,
                    fallback_location,
                )?;
                self.mark_owned_runtime_template_output_if_needed(
                    emission,
                    append_context,
                    fallback_location,
                )?;
                Ok(emission)
            }

            OwnedRuntimeTemplateNode::Slot { location } => {
                // Wrapper-shaped templates can reach HIR as runtime values when
                // they are not used as helpers. Their slot placeholders are
                // structural insertion points, not renderable chunks, so linear
                // rendering skips them just as the old flattened expression
                // path did. Inside an active runtime slot application wrapper
                // the placeholder should have been resolved to a site by AST
                // routing, so the reject policy raises an internal compiler error.
                self.append_unresolved_slot_node_to_accumulator(append_context, location)
            }
        }
    }

    fn mark_owned_runtime_template_output_if_needed(
        &mut self,
        emission: TemplateBodyEmission,
        append_context: RuntimeTemplateAppendContext<'_>,
        fallback_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        if emission == TemplateBodyEmission::Output
            && let Some(flag) = append_context.emitted_output
        {
            self.mark_runtime_template_output_emitted(flag, fallback_location)?;
        }

        Ok(())
    }

    fn append_owned_runtime_template_child_to_accumulator(
        &mut self,
        template: &OwnedRuntimeTemplateHandoff,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        match &template.body {
            OwnedRuntimeTemplateBody::Render(node) => self
                .append_owned_runtime_template_node_to_accumulator(
                    node,
                    append_context,
                    aggregate_local,
                    fallback_location,
                ),

            OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => self
                .append_runtime_slot_application_with_context(
                    handoff,
                    append_context,
                    fallback_location,
                ),
        }
    }

    fn append_owned_runtime_template_branch_chain(
        &mut self,
        branches: &[OwnedRuntimeTemplateBranch],
        fallback: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        self.append_owned_runtime_template_branch_chain_from_index(
            branches,
            fallback,
            0,
            append_context,
            aggregate_local,
            location,
        )
    }

    fn append_owned_runtime_template_branch_chain_from_index(
        &mut self,
        branches: &[OwnedRuntimeTemplateBranch],
        fallback: Option<&OwnedRuntimeTemplateNode>,
        branch_index: usize,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let Some(branch) = branches.get(branch_index) else {
            return self.append_owned_runtime_template_fallback_branch(
                fallback,
                append_context,
                aggregate_local,
                location,
            );
        };

        match &branch.selector {
            crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector::Bool(condition) => {
                self.lower_if_with_body_emitters(
                    condition,
                    &branch.location,
                    |builder| {
                        builder.append_owned_runtime_template_node_to_accumulator(
                            &branch.body,
                            append_context,
                            aggregate_local,
                            &branch.location,
                        )?;
                        Ok(())
                    },
                    |builder| {
                        builder.append_owned_runtime_template_branch_chain_from_index(
                            branches,
                            fallback,
                            branch_index + 1,
                            append_context,
                            aggregate_local,
                            location,
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
                        location,
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
    ) -> Result<TemplateBodyEmission, CompilerError> {
        self.append_runtime_option_present_template_branch(
            scrutinee,
            pattern,
            &branch.location,
            |builder| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    &branch.body,
                    append.append_context,
                    append.aggregate_local,
                    &branch.location,
                )?;
                Ok(())
            },
            |builder| {
                builder.append_owned_runtime_template_branch_chain_from_index(
                    append.branches,
                    append.fallback,
                    append.branch_index + 1,
                    append.append_context,
                    append.aggregate_local,
                    append.location,
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
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let Some(fallback) = fallback else {
            return Ok(TemplateBodyEmission::NoOutput);
        };

        self.append_owned_runtime_template_node_to_accumulator(
            fallback,
            append_context,
            aggregate_local,
            location,
        )
    }

    fn append_owned_runtime_template_loop(
        &mut self,
        header: &TemplateLoopHeader,
        body: &OwnedRuntimeTemplateNode,
        aggregate_wrapper: Option<&OwnedRuntimeTemplateNode>,
        append_context: RuntimeTemplateAppendContext<'_>,
        aggregate_local: Option<LocalId>,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let aggregate = self.initialize_runtime_template_accumulator(fallback_location)?;
        let emitted_any_iteration =
            self.initialize_runtime_template_emitted_flag(fallback_location)?;

        match header {
            TemplateLoopHeader::Conditional { condition } => {
                self.lower_while_with_body_emitter(condition, fallback_location, |builder| {
                    let iteration_context = append_context
                        .with_target_accumulator(aggregate)
                        .with_emitted_output(Some(emitted_any_iteration));

                    builder.append_owned_runtime_template_node_to_accumulator(
                        body,
                        iteration_context,
                        aggregate_local,
                        fallback_location,
                    )?;
                    Ok(())
                })?;
            }

            TemplateLoopHeader::Range { bindings, range } => {
                self.lower_range_loop_with_body_emitter(
                    bindings,
                    range,
                    fallback_location,
                    |builder| {
                        let iteration_context = append_context
                            .with_target_accumulator(aggregate)
                            .with_emitted_output(Some(emitted_any_iteration));

                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            iteration_context,
                            aggregate_local,
                            fallback_location,
                        )?;
                        Ok(())
                    },
                )?;
            }

            TemplateLoopHeader::Collection { bindings, iterable } => {
                self.lower_collection_loop_with_body_emitter(
                    bindings,
                    iterable,
                    fallback_location,
                    |builder| {
                        let iteration_context = append_context
                            .with_target_accumulator(aggregate)
                            .with_emitted_output(Some(emitted_any_iteration));

                        builder.append_owned_runtime_template_node_to_accumulator(
                            body,
                            iteration_context,
                            aggregate_local,
                            fallback_location,
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
            fallback_location,
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
        fallback_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let Some(aggregate_wrapper) = aggregate_wrapper else {
            return Ok(());
        };

        self.append_runtime_template_aggregate_when_emitted(
            super::aggregate::RuntimeTemplateAggregateAppend {
                aggregate,
                emitted_output,
                append_context,
            },
            fallback_location,
            |builder, append, fallback_location| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    aggregate_wrapper,
                    append.append_context,
                    Some(append.aggregate),
                    fallback_location,
                )?;
                Ok(())
            },
        )
    }

    fn lower_reactive_linear_expression_chunk(
        &mut self,
        expression: &Expression,
        direct_subscription: bool,
    ) -> Result<HirExpression, CompilerError> {
        let lowered = self.lower_expression_value_to_current_block(expression)?;
        let location = &expression.location;
        let string_ty = builtin_type_ids::STRING;
        let region = self.current_region_or_error(location)?;
        let chunk_as_string = if direct_subscription {
            self.coerce_reactive_subscription_to_string(lowered, location, string_ty, region)
        } else {
            self.coerce_expression_to_string(lowered, location, string_ty, region)
        }?;

        if direct_subscription
            || expression
                .reactive_template
                .as_ref()
                .is_some_and(|metadata| metadata.has_runtime_dependency())
        {
            return Ok(chunk_as_string);
        }

        // Non-reactive chunks inside a reactive template are snapshots. Store the rendered chunk
        // once so later rerenders do not accidentally turn ordinary `[source]` reads live.
        self.materialize_reactive_template_snapshot_chunk(chunk_as_string, location)
    }

    /// Coerces a direct reactive subscription chunk without eager statement materialization.
    ///
    /// WHAT: Float subscriptions use a lazy expression-level cast so the JS snapshot function
    ///       formats the current source value on every rerender.
    /// WHY: ordinary `FormatFloat` statements are correct for non-reactive chunks, but direct
    ///      `$(source)` pieces must stay inside the returned expression tree instead of becoming
    ///      one-time snapshot statements.
    fn coerce_reactive_subscription_to_string(
        &mut self,
        expression: HirExpression,
        location: &SourceLocation,
        string_ty: TypeId,
        region: RegionId,
    ) -> Result<HirExpression, CompilerError> {
        if expression.ty == self.type_environment.builtins().float {
            return Ok(self.make_expression(
                location,
                HirExpressionKind::Cast {
                    source: Box::new(expression),
                    policy: BuiltinCastPolicyId::FloatToString,
                },
                string_ty,
                ValueKind::RValue,
                region,
            ));
        }

        self.coerce_expression_to_string(expression, location, string_ty, region)
    }

    fn materialize_reactive_template_snapshot_chunk(
        &mut self,
        chunk: HirExpression,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let string_ty = builtin_type_ids::STRING;
        let snapshot = self.allocate_temp_local(string_ty, Some(location.clone()))?;

        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(snapshot),
                value: chunk,
            },
            location,
        )?;

        let region = self.current_region_or_error(location)?;
        Ok(self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(snapshot)),
            string_ty,
            ValueKind::Place,
            region,
        ))
    }

    fn append_runtime_template_expression_to_accumulator(
        &mut self,
        expression: &Expression,
        append_context: RuntimeTemplateAppendContext<'_>,
    ) -> Result<Option<TemplateBodyEmission>, CompilerError> {
        let Some(candidate) = runtime_template_append_candidate_for_expression(expression) else {
            return Ok(None);
        };

        let (handoff, append_linear_handoff_directly) = match candidate {
            RuntimeTemplateAppendCandidate::SlotApplication(handoff) => {
                return self
                    .append_runtime_slot_application_with_context(
                        handoff,
                        append_context,
                        &expression.location,
                    )
                    .map(Some);
            }

            RuntimeTemplateAppendCandidate::TemplateHandoff { handoff } => match &handoff.body {
                OwnedRuntimeTemplateBody::RuntimeSlotApplication(handoff) => {
                    return self
                        .append_runtime_slot_application_with_context(
                            handoff,
                            append_context,
                            &expression.location,
                        )
                        .map(Some);
                }

                OwnedRuntimeTemplateBody::Render(node) => {
                    if is_owned_runtime_template_node_control_flow(node) {
                        return self
                            .append_nested_runtime_template_control_flow(
                                node,
                                append_context,
                                &expression.location,
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
                            &expression.location,
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
                            &expression.location,
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
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let condition_block = self.current_block_id_or_error(location)?;
        let parent_region = self.current_region_or_error(location)?;
        let flush_region = self.create_child_region(parent_region);
        let skip_region = self.create_child_region(parent_region);
        let flush_block = self.create_block(flush_region, location, "runtime-slot-flush")?;
        let skip_block = self.create_block(skip_region, location, "runtime-slot-skip")?;
        let condition = self.make_local_load_expression(
            flush.contribution_emitted_flag,
            builtin_type_ids::BOOL,
            location,
            parent_region,
        );

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition,
                then_block: flush_block,
                else_block: skip_block,
            },
            location,
        )?;

        // If a slot contribution produced output before loop control, replay the
        // wrapper on this terminating path before jumping to the surrounding
        // template loop target. The skip path still emits the same loop control
        // without rendering an empty wrapper.
        self.set_current_block(flush_block, location)?;
        let wrapper_context = RuntimeTemplateAppendContext::new(flush.target_accumulator)
            .with_runtime_slot_sites(flush.source_accumulators, flush.slot_sites)
            .with_emitted_output(flush.parent_emitted_flag)
            .rejecting_unresolved_slots();
        self.append_owned_runtime_template_node_to_accumulator(
            flush.wrapper_plan,
            wrapper_context,
            None,
            location,
        )?;

        let flush_tail = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(flush_tail, location)? {
            self.emit_template_loop_control(control_kind, location)?;
        }

        self.set_current_block(skip_block, location)?;
        self.emit_template_loop_control(control_kind, location)
    }

    fn append_runtime_slot_site_to_accumulator(
        &mut self,
        site_id: RuntimeSlotSiteId,
        append_context: RuntimeTemplateAppendContext<'_>,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let Some(slot_sites) = append_context.slot_sites else {
            return_hir_transformation_error!(
                "Runtime slot site appeared outside an active runtime slot application.",
                self.hir_error_location(fallback_location)
            );
        };
        let Some(site) = slot_sites
            .get(site_id.0)
            .filter(|site| site.site == site_id)
        else {
            return_hir_transformation_error!(
                "Runtime slot application wrapper referenced a missing slot site.",
                self.hir_error_location(fallback_location)
            );
        };

        self.append_owned_runtime_template_node_to_accumulator(
            &site.render_root,
            append_context,
            None,
            &site.location,
        )
    }

    fn append_runtime_slot_source_to_accumulator(
        &mut self,
        source_id: RuntimeSlotContributionSourceId,
        append_context: RuntimeTemplateAppendContext<'_>,
        fallback_location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let Some(source_accumulators) = append_context.source_accumulators else {
            return_hir_transformation_error!(
                "Runtime slot source appeared outside an active runtime slot application.",
                self.hir_error_location(fallback_location)
            );
        };
        let Some(source_accumulator) = source_accumulators.local_for(source_id) else {
            return_hir_transformation_error!(
                "Runtime slot site referenced a missing contribution source.",
                self.hir_error_location(fallback_location)
            );
        };

        let region = self.current_region_or_error(fallback_location)?;
        let source_value = self.make_expression(
            fallback_location,
            HirExpressionKind::Load(HirPlace::Local(source_accumulator)),
            builtin_type_ids::STRING,
            ValueKind::Place,
            region,
        );
        self.append_template_chunk_to_accumulator(
            source_value,
            append_context.target_accumulator,
            fallback_location,
        )?;

        Ok(TemplateBodyEmission::Output)
    }

    fn emit_template_loop_control(
        &mut self,
        control_kind: TemplateLoopControlKind,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        match control_kind {
            TemplateLoopControlKind::Break => self.emit_break_to_current_loop(location),
            TemplateLoopControlKind::Continue => self.emit_continue_to_current_loop(location),
        }
    }

    fn append_nested_runtime_template_control_flow(
        &mut self,
        node: &OwnedRuntimeTemplateNode,
        append_context: RuntimeTemplateAppendContext<'_>,
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        match node {
            OwnedRuntimeTemplateNode::LoopControl {
                kind,
                location: control_location,
            } => {
                if let Some(flush) = append_context.loop_control_flush {
                    self.flush_runtime_slot_application_for_loop_control(
                        flush,
                        *kind,
                        control_location,
                    )?;
                    return Ok(match kind {
                        TemplateLoopControlKind::Break => TemplateBodyEmission::Break,
                        TemplateLoopControlKind::Continue => TemplateBodyEmission::Continue,
                    });
                }

                self.emit_template_loop_control(*kind, control_location)?;
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
                    location,
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
        location: &SourceLocation,
    ) -> Result<TemplateBodyEmission, CompilerError> {
        let child_accumulator = self.initialize_runtime_template_accumulator(location)?;
        let child_emitted = self.initialize_runtime_template_emitted_flag(location)?;
        let child_context = append_context
            .with_target_accumulator(child_accumulator)
            .with_emitted_output(Some(child_emitted));

        let emission = self.append_owned_runtime_template_node_to_accumulator(
            node,
            child_context,
            None,
            location,
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
            location,
            |builder, append, fallback_location| {
                builder.append_owned_runtime_template_node_to_accumulator(
                    wrapper_node,
                    append.append_context,
                    Some(append.aggregate),
                    fallback_location,
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
        chunk: HirExpression,
        accumulator: LocalId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let string_ty = builtin_type_ids::STRING;
        let region = self.current_region_or_error(location)?;
        let accumulated = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(accumulator)),
            string_ty,
            ValueKind::Place,
            region,
        );
        let chunk_as_string =
            self.coerce_expression_to_string(chunk, location, string_ty, region)?;
        let next_value = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(accumulated),
                op: HirBinOp::StringAppend,
                right: Box::new(chunk_as_string),
            },
            string_ty,
            ValueKind::RValue,
            region,
        );

        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(accumulator),
                value: next_value,
            },
            location,
        )
    }

    pub(super) fn coerce_expression_to_string(
        &mut self,
        expression: HirExpression,
        location: &SourceLocation,
        string_ty: TypeId,
        region: RegionId,
    ) -> Result<HirExpression, CompilerError> {
        if expression.ty == builtin_type_ids::STRING {
            return Ok(expression);
        }

        if expression.ty == self.type_environment.builtins().none {
            return Ok(self.make_expression(
                location,
                HirExpressionKind::StringLiteral(String::new()),
                string_ty,
                ValueKind::Const,
                region,
            ));
        }

        // `Float` template chunks must use the Moth-owned formatter instead of target-native
        // stringification so casts and templates share one formatting contract.
        if expression.ty == self.type_environment.builtins().float {
            return self.emit_formatted_float_value(expression, location);
        }

        let empty = self.make_expression(
            location,
            HirExpressionKind::StringLiteral(String::new()),
            string_ty,
            ValueKind::Const,
            region,
        );

        Ok(self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(empty),
                op: crate::compiler_frontend::hir::operators::HirBinOp::StringAppend,
                right: Box::new(expression),
            },
            string_ty,
            ValueKind::RValue,
            region,
        ))
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
