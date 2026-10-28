//! Control-flow lowering helpers for HIR statements.
//!
//! WHAT: lowers structured control-flow constructs into explicit CFG blocks and terminators.
//! WHY: if/match/loop lowering is the densest CFG-building logic in HIR and benefits from a
//! dedicated module boundary.
//!
//! ## Diagnostic boundary
//!
//! `InfrastructureFailure::hir_transformation` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionStore, ExpressionView,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, PatternValue, PatternValueKind, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidMatchGuardReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirVariantCarrier, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, LoopTargets};
use crate::compiler_frontend::hir::hir_statement::match_captures::substitute_local_expressions;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;

fn lower_relational_pattern_op(op: RelationalPatternOp) -> HirRelationalPatternOp {
    match op {
        RelationalPatternOp::LessThan => HirRelationalPatternOp::LessThan,
        RelationalPatternOp::LessThanOrEqual => HirRelationalPatternOp::LessThanOrEqual,
        RelationalPatternOp::GreaterThan => HirRelationalPatternOp::GreaterThan,
        RelationalPatternOp::GreaterThanOrEqual => HirRelationalPatternOp::GreaterThanOrEqual,
    }
}

struct CfgMatchGuardLowering<'a> {
    arm: &'a MatchArm,
    capture_locals: &'a [LocalId],
    scrutinee_hir: HirValueId,
    scrutinee_ast: ExpressionView<'a>,
    guard_block: BlockId,
    arm_body_block: BlockId,
    next_dispatch: BlockId,
    span: &'a Option<SourceSpan>,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_lexical_scope(
        &mut self,
        expressions: &ExpressionStore,
        body: &[AstNode],
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let entry_block = self.current_block_id_or_error()?;
        let parent_region = self.current_region_or_error()?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, span, "lexical-scope");

        self.emit_jump_to(entry_block, body_block, span, "lexical-scope.enter")?;
        self.set_current_block(body_block)?;
        self.lower_statement_sequence(expressions, body)?;

        let body_tail_block = self.current_block_id_or_error()?;
        if self.block_has_explicit_terminator(body_tail_block)? {
            return Ok(self.set_current_block(body_tail_block)?);
        }

        let after_block = self.create_block(parent_region, span, "lexical-scope.after");
        self.emit_jump_to(body_tail_block, after_block, span, "lexical-scope.exit")?;
        Ok(self.set_current_block(after_block)?)
    }

    pub(super) fn lower_if_statement(
        &mut self,
        expressions: &ExpressionStore,
        condition: ExpressionView<'_>,
        then_body: &[AstNode],
        else_body: Option<&[AstNode]>,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if self.expression_constant_bool(condition).is_some() {
            increment_frontend_counter(FrontendCounter::HirStaticBoolIfNodes);
            return Err(InfrastructureFailure::hir_transformation(
                "Stage 4 passed a statically decided Bool `if` to HIR",
            )
            .into());
        }

        self.lower_if_with_body_emitters(
            condition,
            span,
            authored_span,
            |builder| builder.lower_statement_sequence(expressions, then_body),
            |builder| {
                if let Some(else_nodes) = else_body {
                    builder.lower_statement_sequence(expressions, else_nodes)?;
                }

                Ok(())
            },
        )
    }

    /// Lowers a Bool `if` into branch blocks while delegating body emission to callers.
    ///
    /// WHAT: constructs the condition branch, then/else regions, terminal-path handling,
    ///       and merge block once for every HIR producer that needs ordinary `if` CFG.
    /// WHY: runtime template `if` needs the same lazy branch/merge shape as statement `if`,
    ///      but branch bodies append render pieces instead of lowering statement nodes.
    pub(crate) fn lower_if_with_body_emitters(
        &mut self,
        condition: ExpressionView<'_>,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
        emit_then: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
        emit_else: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        increment_frontend_counter(FrontendCounter::HirRuntimeIfNodes);

        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error()?;

        let parent_region = self.current_region_or_error()?;
        let then_region = self.create_child_region(parent_region);
        let else_region = self.create_child_region(parent_region);
        let then_block = self.create_block(then_region, span, "if-then");
        let else_block = self.create_block(else_region, span, "if-else");

        self.emit_terminator_with_span(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block,
                else_block,
            },
            span,
            authored_span,
        )?;

        self.log_control_flow_edge(condition_block, then_block, "if.true");
        self.log_control_flow_edge(condition_block, else_block, "if.false");

        let mut terminated_anchor: Option<BlockId> = None;

        self.set_current_block(then_block)?;
        emit_then(self)?;
        let then_tail_block = self.current_block_id_or_error()?;
        let then_terminated = self.block_has_explicit_terminator(then_tail_block)?;
        if then_terminated {
            terminated_anchor = Some(then_tail_block);
        }

        self.set_current_block(else_block)?;
        emit_else(self)?;

        let else_tail_block = self.current_block_id_or_error()?;
        let else_terminated = self.block_has_explicit_terminator(else_tail_block)?;
        if else_terminated && terminated_anchor.is_none() {
            terminated_anchor = Some(else_tail_block);
        }

        if then_terminated && else_terminated {
            // No continuation path exists after this branch.
            let anchor_block = if let Some(anchor) = terminated_anchor {
                anchor
            } else {
                then_block
            };

            return Ok(self.set_current_block(anchor_block)?);
        }

        let merge_block = self.create_block(parent_region, span, "if-merge");
        if !then_terminated {
            self.emit_jump_to(then_tail_block, merge_block, span, "if.then.merge")?;
        }
        if !else_terminated {
            self.emit_jump_to(else_tail_block, merge_block, span, "if.else.merge")?;
        }

        Ok(self.set_current_block(merge_block)?)
    }

    pub(super) fn lower_break_statement(
        &mut self,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.emit_break_to_current_loop(span, authored_span)
    }

    pub(crate) fn emit_break_to_current_loop(
        &mut self,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let current_block = self.current_block_id_or_error()?;
        let targets = self.current_loop_targets_or_error("break")?;

        self.emit_terminator_with_span(
            current_block,
            HirTerminator::Break {
                target: targets.break_target,
            },
            span,
            authored_span,
        )?;

        self.log_control_flow_edge(current_block, targets.break_target, "loop.break");
        Ok(())
    }

    pub(super) fn lower_continue_statement(
        &mut self,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.emit_continue_to_current_loop(span, authored_span)
    }

    pub(crate) fn emit_continue_to_current_loop(
        &mut self,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let current_block = self.current_block_id_or_error()?;
        let targets = self.current_loop_targets_or_error("continue")?;

        self.emit_terminator_with_span(
            current_block,
            HirTerminator::Continue {
                target: targets.continue_target,
            },
            span,
            authored_span,
        )?;

        self.log_control_flow_edge(current_block, targets.continue_target, "loop.continue");
        Ok(())
    }

    pub(super) fn lower_while_statement(
        &mut self,
        expressions: &ExpressionStore,
        condition: ExpressionView<'_>,
        body: &[AstNode],
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.lower_while_statement_impl(expressions, condition, body, span, authored_span)
    }

    /// Lower an AST match statement into explicit CFG blocks and a `Match` terminator.
    ///
    /// WHAT: creates a block per arm (plus optional default and merge blocks), emits
    /// the `HirTerminator::Match`, then lowers each arm body and wires non-terminal
    /// arms to a shared merge block.
    /// WHY: HIR represents control flow as a flat block graph, so structured match
    /// syntax must be decomposed here. Lazy merge-block creation avoids empty blocks
    /// when every arm terminates explicitly.
    #[allow(
        clippy::too_many_arguments,
        reason = "match dispatch keeps the mutable builder, scrutinee, arms, default, exhaustiveness and spans separate"
    )]
    pub(super) fn lower_match_statement(
        &mut self,
        expressions: &ExpressionStore,
        scrutinee: ExpressionView<'_>,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.validate_match_exhaustiveness_contract(exhaustiveness, default)?;

        if self.match_guards_need_current_block_lowering(expressions, arms)? {
            return self.lower_match_statement_with_cfg_guards(
                expressions,
                scrutinee,
                arms,
                default,
                exhaustiveness,
                span,
                authored_span,
            );
        }

        self.lower_match_statement_with_inline_guards(
            expressions,
            scrutinee,
            arms,
            default,
            exhaustiveness,
            span,
            authored_span,
        )
    }

    fn validate_match_exhaustiveness_contract(
        &self,
        exhaustiveness: MatchExhaustiveness,
        default: Option<&[AstNode]>,
    ) -> Result<(), HirConstructionFailure> {
        match exhaustiveness {
            MatchExhaustiveness::HasDefault if default.is_none() => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Match marked as having a default arm but no default body was provided",
                )
                .into());
            }
            MatchExhaustiveness::ExhaustiveChoice if default.is_some() => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Match marked as exhaustive choice but also provided a default arm",
                )
                .into());
            }
            _ => {}
        }

        Ok(())
    }

    fn match_guards_need_current_block_lowering(
        &self,
        expressions: &ExpressionStore,
        arms: &[MatchArm],
    ) -> Result<bool, HirConstructionFailure> {
        for arm in arms {
            if let Some(id) = arm.guard
                && self.expression_needs_current_block_lowering(expressions.view(id)?)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "inline guard lowering keeps the mutable builder, scrutinee, arms, default, exhaustiveness and spans separate"
    )]
    fn lower_match_statement_with_inline_guards(
        &mut self,
        expressions: &ExpressionStore,
        scrutinee: ExpressionView<'_>,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let scrutinee_value = self.lower_expression_value_to_current_block(scrutinee)?;
        let current_block = self.current_block_id_or_error()?;

        let parent_region = self.current_region_or_error()?;
        let mut arm_blocks = Vec::with_capacity(arms.len());
        for _ in arms {
            let arm_region = self.create_child_region(parent_region);
            arm_blocks.push(self.create_block(arm_region, span, "match-arm"));
        }

        // AST owns exhaustiveness validation; HIR only lowers the contract it receives.
        let default_block = match exhaustiveness {
            MatchExhaustiveness::HasDefault => {
                let default_region = self.create_child_region(parent_region);
                Some(self.create_block(default_region, span, "match-default"))
            }
            MatchExhaustiveness::ExhaustiveChoice => None,
        };
        let mut merge_block = None;

        // Register capture locals and lower each arm's pattern/guard together.
        // WHY: guards are lowered into dense HIR rows here, but evaluated at runtime in the parent
        // block context. Capture locals must be in `locals_by_name` during guard lowering so
        // variable references resolve. Registering per-arm prevents later arms from overwriting
        // earlier capture bindings before their guards are lowered.
        let mut arm_capture_locals: Vec<Vec<LocalId>> = Vec::with_capacity(arms.len());
        let mut hir_arms = Vec::with_capacity(arms.len() + 1);
        for (index, arm) in arms.iter().enumerate() {
            let arm_block = arm_blocks[index];
            self.set_current_block(arm_block)?;
            let locals = self.register_match_arm_capture_locals(arm, scrutinee, span)?;
            arm_capture_locals.push(locals);

            let lowered_pattern =
                self.lower_match_pattern(expressions, &arm.pattern, scrutinee.value_type_id()?)?;
            let lowered_guard = self.lower_inline_match_guard(
                expressions,
                arm,
                &arm_capture_locals[index],
                scrutinee,
                scrutinee_value,
                span,
            )?;

            hir_arms.push(HirMatchArm {
                pattern: lowered_pattern,
                guard: lowered_guard,
                body: arm_block,
            });
        }

        if let Some(default_block_id) = default_block {
            hir_arms.push(HirMatchArm {
                pattern: HirPattern::Wildcard,
                guard: None,
                body: default_block_id,
            });
        }

        let scrutinee_for_captures = scrutinee_value;

        self.emit_terminator_with_span(
            current_block,
            HirTerminator::Match {
                scrutinee: scrutinee_value,
                arms: hir_arms,
            },
            span,
            authored_span,
        )?;

        let mut terminated_anchor: Option<BlockId> = None;

        // Emit capture extraction assignments at the start of each arm block, then lower the body.
        for (index, arm) in arms.iter().enumerate() {
            let arm_block = arm_blocks[index];
            self.set_current_block(arm_block)?;

            self.lower_match_arm_body(
                expressions,
                arm,
                &arm_capture_locals[index],
                scrutinee_for_captures,
                scrutinee,
                span,
                true,
            )?;

            let arm_tail_block = self.current_block_id_or_error()?;
            let arm_terminated = self.block_has_explicit_terminator(arm_tail_block)?;
            if arm_terminated {
                if terminated_anchor.is_none() {
                    terminated_anchor = Some(arm_tail_block);
                }
            } else {
                let merge_target =
                    self.ensure_match_merge_block(parent_region, span, &mut merge_block);
                self.emit_jump_to(arm_tail_block, merge_target, span, "match.arm.merge")?;
            }
        }

        self.lower_match_default_body(
            expressions,
            default_block,
            default,
            parent_region,
            span,
            &mut merge_block,
            &mut terminated_anchor,
        )?;

        self.finish_match_lowering(merge_block, terminated_anchor, "Match lowering")
    }

    /// Lower a match whose guards need active CFG mutation before arm selection completes.
    ///
    /// WHAT: expands the match into a sequence of tiny pattern-dispatch `Match` terminators. A
    /// pattern hit for a CFG guard enters a guard block, evaluates the guard with the normal
    /// current-block expression path, then branches to the arm body or the next dispatch block.
    /// WHY: fallible guard expressions such as `value if check()! =>` contain error edges.
    /// Those edges cannot be hidden inside the pure guard expression stored on a `Match`
    /// terminator, and evaluating the guard before pattern dispatch would run side effects for
    /// arms that did not match.
    #[allow(
        clippy::too_many_arguments,
        reason = "fallible guard lowering keeps the mutable builder, scrutinee, arms, default, exhaustiveness and spans separate"
    )]
    fn lower_match_statement_with_cfg_guards(
        &mut self,
        expressions: &ExpressionStore,
        scrutinee: ExpressionView<'_>,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let scrutinee_value = self.lower_expression_value_to_current_block(scrutinee)?;
        let mut dispatch_block = self.current_block_id_or_error()?;
        let parent_region = self.current_region_or_error()?;

        let default_block = match exhaustiveness {
            MatchExhaustiveness::HasDefault => {
                let default_region = self.create_child_region(parent_region);
                Some(self.create_block(default_region, span, "match-default"))
            }
            MatchExhaustiveness::ExhaustiveChoice => None,
        };
        let no_match_block = if default_block.is_none() {
            Some(self.create_block(parent_region, span, "match-no-match"))
        } else {
            None
        };

        let mut merge_block = None;
        let mut terminated_anchor: Option<BlockId> = None;

        for (index, arm) in arms.iter().enumerate() {
            let arm_region = self.create_child_region(parent_region);
            let arm_body_block = self.create_block(arm_region, span, "match-arm");
            let guard_needs_cfg = arm
                .guard
                .map(|id| expressions.view(id))
                .transpose()?
                .is_some_and(|guard| self.expression_needs_current_block_lowering(guard));
            let guard_block = if guard_needs_cfg {
                Some(self.create_block(arm_region, span, "match-guard"))
            } else {
                None
            };
            let matched_block = guard_block.unwrap_or(arm_body_block);
            let next_dispatch = self.next_match_dispatch_block(
                index,
                arms.len(),
                parent_region,
                default_block,
                no_match_block,
                span,
            )?;

            let capture_registration_block = guard_block.unwrap_or(arm_body_block);
            self.set_current_block(capture_registration_block)?;
            let capture_locals = self.register_match_arm_capture_locals(arm, scrutinee, span)?;
            let pattern =
                self.lower_match_pattern(expressions, &arm.pattern, scrutinee.value_type_id()?)?;
            let inline_guard = if guard_needs_cfg {
                None
            } else {
                self.lower_inline_match_guard(
                    expressions,
                    arm,
                    &capture_locals,
                    scrutinee,
                    scrutinee_value,
                    span,
                )?
            };

            self.emit_terminator_with_span(
                dispatch_block,
                HirTerminator::Match {
                    scrutinee: scrutinee_value,
                    arms: vec![
                        HirMatchArm {
                            pattern,
                            guard: inline_guard,
                            body: matched_block,
                        },
                        HirMatchArm {
                            pattern: HirPattern::Wildcard,
                            guard: None,
                            body: next_dispatch,
                        },
                    ],
                },
                span,
                authored_span,
            )?;

            if let Some(guard_block_id) = guard_block {
                self.lower_cfg_match_guard(
                    expressions,
                    CfgMatchGuardLowering {
                        arm,
                        capture_locals: &capture_locals,
                        scrutinee_hir: scrutinee_value,
                        scrutinee_ast: scrutinee,
                        guard_block: guard_block_id,
                        arm_body_block,
                        next_dispatch,
                        span,
                    },
                )?;
            }

            self.set_current_block(arm_body_block)?;
            self.lower_match_arm_body(
                expressions,
                arm,
                &capture_locals,
                scrutinee_value,
                scrutinee,
                span,
                !guard_needs_cfg,
            )?;

            let arm_tail_block = self.current_block_id_or_error()?;
            let arm_terminated = self.block_has_explicit_terminator(arm_tail_block)?;
            if arm_terminated {
                if terminated_anchor.is_none() {
                    terminated_anchor = Some(arm_tail_block);
                }
            } else {
                let merge_target =
                    self.ensure_match_merge_block(parent_region, span, &mut merge_block);
                self.emit_jump_to(arm_tail_block, merge_target, span, "match.arm.merge")?;
            }

            dispatch_block = next_dispatch;
        }

        self.lower_match_default_body(
            expressions,
            default_block,
            default,
            parent_region,
            span,
            &mut merge_block,
            &mut terminated_anchor,
        )?;

        if let Some(no_match_block_id) = no_match_block {
            self.set_current_block(no_match_block_id)?;
            self.emit_terminator(
                no_match_block_id,
                HirTerminator::RuntimeFailure {
                    message: "No match arm selected".to_owned(),
                    cause: None,
                },
                span,
            )?;
        }

        self.finish_match_lowering(merge_block, terminated_anchor, "CFG match lowering")
    }

    fn next_match_dispatch_block(
        &mut self,
        index: usize,
        arm_count: usize,
        parent_region: RegionId,
        default_block: Option<BlockId>,
        no_match_block: Option<BlockId>,
        span: &Option<SourceSpan>,
    ) -> Result<BlockId, HirConstructionFailure> {
        if index + 1 < arm_count {
            return Ok(self.create_block(parent_region, span, "match-next"));
        }

        if let Some(default_block_id) = default_block {
            return Ok(default_block_id);
        }

        if let Some(no_match_block_id) = no_match_block {
            return Ok(no_match_block_id);
        }

        Err(InfrastructureFailure::hir_transformation(
            "Match dispatch had no next arm, default arm, or fallback runtime-failure block",
        )
        .into())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "default arm lowering keeps the mutable builder, block, body, region, span and mutable block anchors separate"
    )]
    fn lower_match_default_body(
        &mut self,
        expressions: &ExpressionStore,
        default_block: Option<BlockId>,
        default: Option<&[AstNode]>,
        parent_region: RegionId,
        span: &Option<SourceSpan>,
        merge_block: &mut Option<BlockId>,
        terminated_anchor: &mut Option<BlockId>,
    ) -> Result<(), HirConstructionFailure> {
        let (Some(default_block_id), Some(default_body)) = (default_block, default) else {
            return Ok(());
        };

        self.set_current_block(default_block_id)?;
        self.lower_statement_sequence(expressions, default_body)?;

        let default_tail_block = self.current_block_id_or_error()?;
        let default_terminated = self.block_has_explicit_terminator(default_tail_block)?;
        if default_terminated {
            if terminated_anchor.is_none() {
                *terminated_anchor = Some(default_tail_block);
            }

            return Ok(());
        }

        let merge_target = self.ensure_match_merge_block(parent_region, span, merge_block);
        self.emit_jump_to(
            default_tail_block,
            merge_target,
            span,
            "match.default.merge",
        )
    }

    fn finish_match_lowering(
        &mut self,
        merge_block: Option<BlockId>,
        terminated_anchor: Option<BlockId>,
        context: &str,
    ) -> Result<(), HirConstructionFailure> {
        if let Some(merge_block_id) = merge_block {
            return Ok(self.set_current_block(merge_block_id)?);
        }

        if let Some(anchor_block) = terminated_anchor {
            return Ok(self.set_current_block(anchor_block)?);
        }

        Err(InfrastructureFailure::hir_transformation(format!(
            "{context} produced no merge block and no terminated anchor block"
        ))
        .into())
    }

    fn lower_cfg_match_guard(
        &mut self,
        expressions: &ExpressionStore,
        context: CfgMatchGuardLowering<'_>,
    ) -> Result<(), HirConstructionFailure> {
        let CfgMatchGuardLowering {
            arm,
            capture_locals,
            scrutinee_hir,
            scrutinee_ast,
            guard_block,
            arm_body_block,
            next_dispatch,
            span,
        } = context;

        self.set_current_block(guard_block)?;

        self.with_arm_capture_bindings(arm, capture_locals, |builder| {
            builder.emit_match_arm_capture_assignments(
                arm,
                capture_locals,
                scrutinee_hir,
                scrutinee_ast,
                span,
            )?;

            let Some(guard) = &arm.guard else {
                return Err(InfrastructureFailure::hir_transformation(
                    "CFG match guard lowering reached an arm without a guard",
                )
                .into());
            };

            let guard = expressions.view(*guard)?;
            let guard_value = builder.lower_match_guard_value_to_current_block(guard)?;
            let guard_tail_block = builder.current_block_id_or_error()?;
            // Authored guard branch carries the guard expression span.
            builder.emit_terminator_with_span(
                guard_tail_block,
                HirTerminator::If {
                    condition: guard_value,
                    then_block: arm_body_block,
                    else_block: next_dispatch,
                },
                span,
                guard.span(),
            )?;
            builder.log_control_flow_edge(guard_tail_block, next_dispatch, "match.guard.false");
            Ok(())
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "arm body lowering keeps the mutable builder, HIR and AST scrutinee, capture locals, span and flag separate"
    )]
    fn lower_match_arm_body(
        &mut self,
        expressions: &ExpressionStore,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_hir: HirValueId,
        scrutinee_ast: ExpressionView<'_>,
        span: &Option<SourceSpan>,
        emit_capture_assignments: bool,
    ) -> Result<(), HirConstructionFailure> {
        self.with_arm_capture_bindings(arm, capture_locals, |builder| {
            if emit_capture_assignments {
                builder.emit_match_arm_capture_assignments(
                    arm,
                    capture_locals,
                    scrutinee_hir,
                    scrutinee_ast,
                    span,
                )?;
            }

            builder.lower_statement_sequence(expressions, &arm.body)
        })
    }

    fn lower_inline_match_guard(
        &mut self,
        expressions: &ExpressionStore,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_ast: ExpressionView<'_>,
        scrutinee_hir: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<Option<HirValueId>, HirConstructionFailure> {
        let Some(guard) = &arm.guard else {
            return Ok(None);
        };

        let guard_expr = self.lower_match_guard_expression(expressions.view(*guard)?)?;
        if let MatchPattern::ChoiceVariant { captures, .. } = &arm.pattern {
            if !captures.is_empty() && !capture_locals.is_empty() {
                return Ok(Some(self.substitute_match_guard_captures(
                    guard_expr,
                    arm,
                    capture_locals,
                    scrutinee_ast,
                    scrutinee_hir,
                    span,
                )?));
            }
        } else if matches!(arm.pattern, MatchPattern::OptionPresentCapture { .. })
            && !capture_locals.is_empty()
        {
            let capture_local = capture_locals[0];
            let MatchPattern::OptionPresentCapture {
                inner_type_id,
                binding_span: authored_binding_span,
                ..
            } = &arm.pattern
            else {
                crate::compiler_bug!(
                    CompilerStage::HirConstruction,
                    "OptionPresentCapture was matched immediately above; the inner destructure cannot fail"
                )
            };
            let binding_span = (*authored_binding_span).or(*span);
            let field_ty = self.lower_type_id(*inner_type_id)?;
            let region = self.current_region_or_error()?;
            let payload_get = self.make_expression(
                &binding_span,
                HirExpressionKind::VariantPayloadGet {
                    carrier: HirVariantCarrier::Option,
                    source: scrutinee_hir,
                    variant_index:
                        crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX,
                    field_index: 0,
                },
                field_ty,
                ValueKind::RValue,
                region,
            )?;
            let mut substitutions = rustc_hash::FxHashMap::default();
            substitutions.insert(capture_local, payload_get);
            return Ok(Some(substitute_local_expressions(
                &mut self.module.expressions,
                &mut self.side_table,
                guard_expr,
                &substitutions,
            )?));
        }

        Ok(Some(guard_expr))
    }

    /// Validate and lower a match arm pattern, rejecting non-literal expressions.
    ///
    /// WHAT: lowers the pattern expression and verifies it has no side-effect prelude,
    /// is a compile-time constant, and is one of the supported literal kinds.
    /// WHY: match dispatch relies on constant comparison values; catching non-literals
    /// here produces clear HIR-stage errors instead of miscompilation.
    fn lower_match_literal_pattern(
        &mut self,
        expressions: &ExpressionStore,
        pattern: &PatternValue,
        scrutinee_type_id: TypeId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let value = match pattern.kind {
            PatternValueKind::Constant(value) => {
                self.lower_const_store_expression(value, &pattern.span)?
            }
            PatternValueKind::Runtime(id) => {
                let lowered = self.lower_expression(expressions.view(id)?)?;
                if !lowered.prelude.is_empty() {
                    crate::compiler_bug!(
                        CompilerStage::HirConstruction,
                        "Match arm pattern lowering produced side-effect statements; only literal patterns are supported",
                    );
                }
                lowered.value
            }
        };
        let row = self.module.expressions.expression(value);
        if row.value_kind != ValueKind::Const {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Match arm patterns must be compile-time literals",
            );
        }
        match &row.kind {
            HirExpressionKind::FixedScalar(value) => {
                if self.type_environment.fixed_scalar(scrutinee_type_id) != Some(value.scalar()) {
                    crate::compiler_bug!(
                        CompilerStage::HirConstruction,
                        "Fixed-scalar match pattern type does not match the scrutinee",
                    );
                }
            }
            HirExpressionKind::Number(value) => {
                if self.type_environment.number_scale(scrutinee_type_id) != Some(value.scale()) {
                    crate::compiler_bug!(
                        CompilerStage::HirConstruction,
                        "Dec match pattern scale does not match the scrutinee type",
                    );
                }
            }
            HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {}
            _ => crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Match arm patterns currently support only literal fixed-scalar/number/bool/char/string values",
            ),
        }
        Ok(value)
    }

    /// Lower an AST match pattern into its HIR counterpart.
    pub(crate) fn lower_match_pattern(
        &mut self,
        expressions: &ExpressionStore,
        pattern: &MatchPattern,
        scrutinee_type_id: TypeId,
    ) -> Result<HirPattern, HirConstructionFailure> {
        let pattern_value_type_id = self
            .type_environment
            .option_inner_type(scrutinee_type_id)
            .unwrap_or(scrutinee_type_id);
        match pattern {
            MatchPattern::Literal(expression) => {
                let lowered = self.lower_match_literal_pattern(
                    expressions,
                    expression,
                    pattern_value_type_id,
                )?;
                Ok(HirPattern::Literal(lowered))
            }

            MatchPattern::OptionNone { .. } => Ok(HirPattern::OptionNone),

            MatchPattern::OptionValue { value, .. } => {
                let lowered =
                    self.lower_match_literal_pattern(expressions, value, pattern_value_type_id)?;
                Ok(HirPattern::OptionValue { value: lowered })
            }

            MatchPattern::OptionPresentCapture { .. } => Ok(HirPattern::OptionPresent),

            MatchPattern::Relational { op, value, .. } => {
                let lowered_value =
                    self.lower_match_literal_pattern(expressions, value, pattern_value_type_id)?;
                let hir_op = lower_relational_pattern_op(*op);

                if self
                    .type_environment
                    .option_inner_type(scrutinee_type_id)
                    .is_some()
                {
                    return Ok(HirPattern::OptionRelational {
                        op: hir_op,
                        value: lowered_value,
                    });
                }

                Ok(HirPattern::Relational {
                    op: hir_op,
                    value: lowered_value,
                })
            }
            MatchPattern::ChoiceVariant {
                nominal_path,
                tag,
                span,
                ..
            } => {
                let choice_id =
                    self.choice_id_for_scrutinee_type(nominal_path, scrutinee_type_id, span)?;
                Ok(HirPattern::ChoiceVariant {
                    choice_id,
                    variant_index: *tag,
                })
            }
        }
    }

    /// Lower a match arm guard and ensure it remains a pure boolean expression.
    fn lower_match_guard_expression(
        &mut self,
        guard: ExpressionView<'_>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let lowered_guard = self.lower_expression(guard)?;
        if !lowered_guard.prelude.is_empty() {
            return Err(choices_control_flow::invalid_match_guard(
                InvalidMatchGuardReason::SideEffectingPrelude,
                guard.span(),
            )
            .into());
        }

        if self.module.expressions.expression(lowered_guard.value).ty
            != self.type_environment.builtins().bool
        {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Match arm guards must lower to Bool expressions",
            );
        }

        Ok(lowered_guard.value)
    }

    /// Lower a match arm guard when a preceding pattern dispatch has already selected the arm.
    ///
    /// WHAT: emits any required guard CFG, including fallible propagation, into the active guard
    /// block and returns the boolean value used by the guard's final `If` terminator.
    /// WHY: guards that contain `expr!` or short-circuit control flow cannot stay embedded in a
    /// `Match` terminator expression, but they still must run only after their pattern matches.
    fn lower_match_guard_value_to_current_block(
        &mut self,
        guard: ExpressionView<'_>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let guard_value = self.lower_expression_value_to_current_block(guard)?;
        if self.module.expressions.expression(guard_value).ty
            != self.type_environment.builtins().bool
        {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Match arm guards must lower to Bool expressions",
            );
        }

        Ok(guard_value)
    }

    /// Lazily create the shared merge block on the first non-terminal arm that needs it.
    fn ensure_match_merge_block(
        &mut self,
        region: crate::compiler_frontend::hir::ids::RegionId,
        span: &Option<SourceSpan>,
        merge_block: &mut Option<BlockId>,
    ) -> BlockId {
        if let Some(existing) = *merge_block {
            return existing;
        }

        let created = self.create_block(region, span, "match-merge");
        *merge_block = Some(created);
        created
    }

    pub(crate) fn create_child_region(
        &mut self,
        parent: crate::compiler_frontend::hir::ids::RegionId,
    ) -> crate::compiler_frontend::hir::ids::RegionId {
        let region_id = self.allocate_region_id();
        self.push_region(HirRegion::lexical(region_id, Some(parent)));
        region_id
    }

    pub(crate) fn create_block(
        &mut self,
        region: crate::compiler_frontend::hir::ids::RegionId,
        source_span: &Option<SourceSpan>,
        label: &str,
    ) -> BlockId {
        let block = HirBlock {
            id: self.allocate_block_id(),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Uninitialized,
        };

        self.side_table.map_block(source_span.to_owned(), &block);
        self.log_block_created(block.id, label, source_span);

        let id = block.id;
        self.push_block(block);
        id
    }

    /// Removes a preallocated block when body lowering proved that no CFG edge can reach it.
    ///
    /// WHAT: range and collection loops preallocate their step block so `continue`
    /// statements have a target while the body is lowered. If every body path exits
    /// through `break`, `return`, or another terminal edge, that step block remains
    /// empty and unreachable.
    ///
    /// WHY: HIR validation intentionally rejects ownerless blocks. Pruning the unused
    /// step block keeps loop lowering strict without inventing a semantic edge that
    /// would execute unreachable step work.
    pub(crate) fn discard_unreachable_empty_block(
        &mut self,
        block_id: BlockId,
    ) -> Result<bool, HirConstructionFailure> {
        if self.block_has_incoming_terminator_edge(block_id) {
            return Ok(false);
        }

        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Cannot discard unknown HIR block {block_id}."
            ))
            .into());
        };

        let block = &self.module.blocks[index];
        if !block.locals.is_empty()
            || !block.statements.is_empty()
            || !matches!(block.terminator, HirTerminator::Uninitialized)
        {
            return Ok(false);
        }

        self.module.blocks.remove(index);
        self.block_index_by_id.remove(&block_id);
        for position in index..self.module.blocks.len() {
            let remaining_id = self.module.blocks[position].id;
            self.block_index_by_id.insert(remaining_id, position);
        }
        Ok(true)
    }

    fn block_has_incoming_terminator_edge(&self, target: BlockId) -> bool {
        self.module.blocks.iter().any(|block| {
            terminator_targets(&block.terminator)
                .into_iter()
                .any(|successor| successor == target)
        })
    }

    pub(crate) fn emit_jump_to(
        &mut self,
        from_block: BlockId,
        target: BlockId,
        span: &Option<SourceSpan>,
        edge_label: &str,
    ) -> Result<(), HirConstructionFailure> {
        self.emit_jump_with_args(from_block, target, vec![], span, edge_label)
    }

    /// Jumps to `target` from the block lowering is currently in.
    ///
    /// WHAT: reads the live continuation block, then terminates that block with a jump.
    /// WHY: expression and checked-operation lowering may split the block it started in. A `!`
    /// propagation, or compiler-generated checked arithmetic inside a builtin `Error!` function,
    /// ends the current block with an `Invoke` and continues in a fresh success block.
    /// A caller that remembered the earlier block id would try to give it a second terminator.
    pub(crate) fn emit_jump_from_current_block(
        &mut self,
        target: BlockId,
        span: &Option<SourceSpan>,
        edge_label: &str,
    ) -> Result<(), HirConstructionFailure> {
        let continuation_block = self.current_block_id_or_error()?;
        self.emit_jump_to(continuation_block, target, span, edge_label)
    }

    pub(crate) fn emit_jump_with_args(
        &mut self,
        from_block: BlockId,
        target: BlockId,
        args: Vec<HirJumpArgument>,
        span: &Option<SourceSpan>,
        edge_label: &str,
    ) -> Result<(), HirConstructionFailure> {
        self.emit_terminator(from_block, HirTerminator::Jump { target, args }, span)?;

        self.log_control_flow_edge(from_block, target, edge_label);
        Ok(())
    }

    pub(crate) fn emit_terminator(
        &mut self,
        block_id: BlockId,
        terminator: HirTerminator,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.emit_terminator_with_span(block_id, terminator, span, None)
    }

    pub(crate) fn emit_terminator_with_span(
        &mut self,
        block_id: BlockId,
        terminator: HirTerminator,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.log_terminator_emitted(block_id, &terminator, span);
        self.set_block_terminator(block_id, terminator, span)?;
        if let Some(authored_span) = authored_span {
            self.side_table.map_terminator_span(block_id, authored_span);
        }
        Ok(())
    }

    pub(super) fn push_loop_targets(&mut self, break_target: BlockId, continue_target: BlockId) {
        self.loop_targets.push(LoopTargets {
            break_target,
            continue_target,
        });
    }

    pub(super) fn pop_loop_targets(&mut self) {
        let _ = self.loop_targets.pop();
    }

    pub(super) fn current_loop_targets_or_error(
        &self,
        keyword: &str,
    ) -> Result<LoopTargets, HirConstructionFailure> {
        let Some(targets) = self.loop_targets.last().copied() else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "'{}' reached HIR lowering without an active loop context",
                keyword
            ))
            .into());
        };

        Ok(targets)
    }
}
