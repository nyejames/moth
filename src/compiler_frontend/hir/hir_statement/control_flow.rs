//! Control-flow lowering helpers for HIR statements.
//!
//! WHAT: lowers structured control-flow constructs into explicit CFG blocks and terminators.
//! WHY: if/match/loop lowering is the densest CFG-building logic in HIR and benefits from a
//! dedicated module boundary.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, MatchExhaustiveness};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchPattern, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, LoopTargets};
use crate::compiler_frontend::hir::hir_statement::match_captures::substitute_local_expressions;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId, RegionId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

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
    scrutinee_hir: &'a HirExpression,
    scrutinee_ast: &'a Expression,
    guard_block: BlockId,
    arm_body_block: BlockId,
    next_dispatch: BlockId,
    location: &'a SourceLocation,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_scoped_block_statement(
        &mut self,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let entry_block = self.current_block_id_or_error(location)?;
        let parent_region = self.current_region_or_error(location)?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, location, "scoped-block")?;

        self.emit_jump_to(entry_block, body_block, location, "block.enter")?;
        self.set_current_block(body_block, location)?;
        self.lower_statement_sequence(body)?;

        let body_tail_block = self.current_block_id_or_error(location)?;
        if self.block_has_explicit_terminator(body_tail_block, location)? {
            return self.set_current_block(body_tail_block, location);
        }

        let after_block = self.create_block(parent_region, location, "scoped-block-after")?;
        self.emit_jump_to(body_tail_block, after_block, location, "block.exit")?;
        self.set_current_block(after_block, location)
    }

    pub(super) fn lower_if_statement(
        &mut self,
        condition: &Expression,
        then_body: &[AstNode],
        else_body: Option<&[AstNode]>,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_if_with_body_emitters(
            condition,
            location,
            |builder| builder.lower_statement_sequence(then_body),
            |builder| {
                if let Some(else_nodes) = else_body {
                    builder.lower_statement_sequence(else_nodes)?;
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
        condition: &Expression,
        location: &SourceLocation,
        emit_then: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
        emit_else: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error(location)?;

        let parent_region = self.current_region_or_error(location)?;
        let then_region = self.create_child_region(parent_region);
        let else_region = self.create_child_region(parent_region);
        let then_block = self.create_block(then_region, location, "if-then")?;
        let else_block = self.create_block(else_region, location, "if-else")?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block,
                else_block,
            },
            location,
        )?;

        self.log_control_flow_edge(condition_block, then_block, "if.true");
        self.log_control_flow_edge(condition_block, else_block, "if.false");

        let mut terminated_anchor: Option<BlockId> = None;

        self.set_current_block(then_block, location)?;
        emit_then(self)?;
        let then_tail_block = self.current_block_id_or_error(location)?;
        let then_terminated = self.block_has_explicit_terminator(then_tail_block, location)?;
        if then_terminated {
            terminated_anchor = Some(then_tail_block);
        }

        self.set_current_block(else_block, location)?;
        emit_else(self)?;

        let else_tail_block = self.current_block_id_or_error(location)?;
        let else_terminated = self.block_has_explicit_terminator(else_tail_block, location)?;
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

            return self.set_current_block(anchor_block, location);
        }

        let merge_block = self.create_block(parent_region, location, "if-merge")?;
        if !then_terminated {
            self.emit_jump_to(then_tail_block, merge_block, location, "if.then.merge")?;
        }
        if !else_terminated {
            self.emit_jump_to(else_tail_block, merge_block, location, "if.else.merge")?;
        }

        self.set_current_block(merge_block, location)
    }

    pub(super) fn lower_break_statement(
        &mut self,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.emit_break_to_current_loop(location)
    }

    pub(crate) fn emit_break_to_current_loop(
        &mut self,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let current_block = self.current_block_id_or_error(location)?;
        let targets = self.current_loop_targets_or_error("break", location)?;

        self.emit_terminator(
            current_block,
            HirTerminator::Break {
                target: targets.break_target,
            },
            location,
        )?;

        self.log_control_flow_edge(current_block, targets.break_target, "loop.break");
        Ok(())
    }

    pub(super) fn lower_continue_statement(
        &mut self,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.emit_continue_to_current_loop(location)
    }

    pub(crate) fn emit_continue_to_current_loop(
        &mut self,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let current_block = self.current_block_id_or_error(location)?;
        let targets = self.current_loop_targets_or_error("continue", location)?;

        self.emit_terminator(
            current_block,
            HirTerminator::Continue {
                target: targets.continue_target,
            },
            location,
        )?;

        self.log_control_flow_edge(current_block, targets.continue_target, "loop.continue");
        Ok(())
    }

    pub(super) fn lower_while_statement(
        &mut self,
        condition: &Expression,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_while_statement_impl(condition, body, location)
    }

    /// Lower an AST match statement into explicit CFG blocks and a `Match` terminator.
    ///
    /// WHAT: creates a block per arm (plus optional default and merge blocks), emits
    /// the `HirTerminator::Match`, then lowers each arm body and wires non-terminal
    /// arms to a shared merge block.
    /// WHY: HIR represents control flow as a flat block graph, so structured match
    /// syntax must be decomposed here. Lazy merge-block creation avoids empty blocks
    /// when every arm terminates explicitly.
    pub(super) fn lower_match_statement(
        &mut self,
        scrutinee: &Expression,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.validate_match_exhaustiveness_contract(exhaustiveness, default, location)?;

        if self.match_guards_need_current_block_lowering(arms) {
            return self.lower_match_statement_with_cfg_guards(
                scrutinee,
                arms,
                default,
                exhaustiveness,
                location,
            );
        }

        self.lower_match_statement_with_inline_guards(
            scrutinee,
            arms,
            default,
            exhaustiveness,
            location,
        )
    }

    fn validate_match_exhaustiveness_contract(
        &self,
        exhaustiveness: MatchExhaustiveness,
        default: Option<&[AstNode]>,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        match exhaustiveness {
            MatchExhaustiveness::HasDefault if default.is_none() => {
                return_hir_transformation_error!(
                    "Match marked as having a default arm but no default body was provided",
                    self.hir_error_location(location)
                );
            }
            MatchExhaustiveness::ExhaustiveChoice if default.is_some() => {
                return_hir_transformation_error!(
                    "Match marked as exhaustive choice but also provided a default arm",
                    self.hir_error_location(location)
                );
            }
            _ => {}
        }

        Ok(())
    }

    fn match_guards_need_current_block_lowering(&self, arms: &[MatchArm]) -> bool {
        arms.iter().any(|arm| {
            arm.guard
                .as_ref()
                .is_some_and(|guard| self.expression_needs_current_block_lowering(guard))
        })
    }

    fn lower_match_statement_with_inline_guards(
        &mut self,
        scrutinee: &Expression,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let scrutinee_value = self.lower_expression_value_to_current_block(scrutinee)?;
        let current_block = self.current_block_id_or_error(location)?;

        let parent_region = self.current_region_or_error(location)?;
        let mut arm_blocks = Vec::with_capacity(arms.len());
        for _ in arms {
            let arm_region = self.create_child_region(parent_region);
            arm_blocks.push(self.create_block(arm_region, location, "match-arm")?);
        }

        // AST owns exhaustiveness validation; HIR only lowers the contract it receives.
        let default_block = match exhaustiveness {
            MatchExhaustiveness::HasDefault => {
                let default_region = self.create_child_region(parent_region);
                Some(self.create_block(default_region, location, "match-default")?)
            }
            MatchExhaustiveness::ExhaustiveChoice => None,
        };
        let mut merge_block = None;

        // Register capture locals and lower each arm's pattern/guard together.
        // WHY: guards are lowered into HirExpression here, but evaluated at runtime in the parent
        // block context. Capture locals must be in `locals_by_name` during guard lowering so
        // variable references resolve. Registering per-arm prevents later arms from overwriting
        // earlier capture bindings before their guards are lowered.
        let mut arm_capture_locals: Vec<Vec<LocalId>> = Vec::with_capacity(arms.len());
        let mut hir_arms = Vec::with_capacity(arms.len() + 1);
        for (index, arm) in arms.iter().enumerate() {
            let arm_block = arm_blocks[index];
            self.set_current_block(arm_block, location)?;
            let locals = self.register_match_arm_capture_locals(arm, scrutinee, location)?;
            arm_capture_locals.push(locals);

            let lowered_pattern = self.lower_match_pattern(&arm.pattern, scrutinee.type_id)?;
            let lowered_guard = self.lower_inline_match_guard(
                arm,
                &arm_capture_locals[index],
                scrutinee,
                &scrutinee_value,
                location,
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

        let scrutinee_for_captures = scrutinee_value.clone();

        self.emit_terminator(
            current_block,
            HirTerminator::Match {
                scrutinee: scrutinee_value,
                arms: hir_arms,
            },
            location,
        )?;

        let mut terminated_anchor: Option<BlockId> = None;

        // Emit capture extraction assignments at the start of each arm block, then lower the body.
        for (index, arm) in arms.iter().enumerate() {
            let arm_block = arm_blocks[index];
            self.set_current_block(arm_block, location)?;

            self.lower_match_arm_body(
                arm,
                &arm_capture_locals[index],
                &scrutinee_for_captures,
                scrutinee,
                location,
                true,
            )?;

            let arm_tail_block = self.current_block_id_or_error(location)?;
            let arm_terminated = self.block_has_explicit_terminator(arm_tail_block, location)?;
            if arm_terminated {
                if terminated_anchor.is_none() {
                    terminated_anchor = Some(arm_tail_block);
                }
            } else {
                let merge_target =
                    self.ensure_match_merge_block(parent_region, location, &mut merge_block)?;
                self.emit_jump_to(arm_tail_block, merge_target, location, "match.arm.merge")?;
            }
        }

        self.lower_match_default_body(
            default_block,
            default,
            parent_region,
            location,
            &mut merge_block,
            &mut terminated_anchor,
        )?;

        self.finish_match_lowering(merge_block, terminated_anchor, location, "Match lowering")
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
    fn lower_match_statement_with_cfg_guards(
        &mut self,
        scrutinee: &Expression,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        exhaustiveness: MatchExhaustiveness,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let scrutinee_value = self.lower_expression_value_to_current_block(scrutinee)?;
        let mut dispatch_block = self.current_block_id_or_error(location)?;
        let parent_region = self.current_region_or_error(location)?;

        let default_block = match exhaustiveness {
            MatchExhaustiveness::HasDefault => {
                let default_region = self.create_child_region(parent_region);
                Some(self.create_block(default_region, location, "match-default")?)
            }
            MatchExhaustiveness::ExhaustiveChoice => None,
        };
        let no_match_block = if default_block.is_none() {
            Some(self.create_block(parent_region, location, "match-no-match")?)
        } else {
            None
        };

        let mut merge_block = None;
        let mut terminated_anchor: Option<BlockId> = None;

        for (index, arm) in arms.iter().enumerate() {
            let arm_region = self.create_child_region(parent_region);
            let arm_body_block = self.create_block(arm_region, location, "match-arm")?;
            let guard_needs_cfg = arm
                .guard
                .as_ref()
                .is_some_and(|guard| self.expression_needs_current_block_lowering(guard));
            let guard_block = if guard_needs_cfg {
                Some(self.create_block(arm_region, location, "match-guard")?)
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
                location,
            )?;

            let capture_registration_block = guard_block.unwrap_or(arm_body_block);
            self.set_current_block(capture_registration_block, location)?;
            let capture_locals =
                self.register_match_arm_capture_locals(arm, scrutinee, location)?;
            let pattern = self.lower_match_pattern(&arm.pattern, scrutinee.type_id)?;
            let inline_guard = if guard_needs_cfg {
                None
            } else {
                self.lower_inline_match_guard(
                    arm,
                    &capture_locals,
                    scrutinee,
                    &scrutinee_value,
                    location,
                )?
            };

            self.emit_terminator(
                dispatch_block,
                HirTerminator::Match {
                    scrutinee: scrutinee_value.clone(),
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
                location,
            )?;

            if let Some(guard_block_id) = guard_block {
                self.lower_cfg_match_guard(CfgMatchGuardLowering {
                    arm,
                    capture_locals: &capture_locals,
                    scrutinee_hir: &scrutinee_value,
                    scrutinee_ast: scrutinee,
                    guard_block: guard_block_id,
                    arm_body_block,
                    next_dispatch,
                    location,
                })?;
            }

            self.set_current_block(arm_body_block, location)?;
            self.lower_match_arm_body(
                arm,
                &capture_locals,
                &scrutinee_value,
                scrutinee,
                location,
                !guard_needs_cfg,
            )?;

            let arm_tail_block = self.current_block_id_or_error(location)?;
            let arm_terminated = self.block_has_explicit_terminator(arm_tail_block, location)?;
            if arm_terminated {
                if terminated_anchor.is_none() {
                    terminated_anchor = Some(arm_tail_block);
                }
            } else {
                let merge_target =
                    self.ensure_match_merge_block(parent_region, location, &mut merge_block)?;
                self.emit_jump_to(arm_tail_block, merge_target, location, "match.arm.merge")?;
            }

            dispatch_block = next_dispatch;
        }

        self.lower_match_default_body(
            default_block,
            default,
            parent_region,
            location,
            &mut merge_block,
            &mut terminated_anchor,
        )?;

        if let Some(no_match_block_id) = no_match_block {
            self.set_current_block(no_match_block_id, location)?;
            self.emit_terminator(
                no_match_block_id,
                HirTerminator::RuntimeFailure {
                    message: "No match arm selected".to_owned(),
                },
                location,
            )?;
        }

        self.finish_match_lowering(
            merge_block,
            terminated_anchor,
            location,
            "CFG match lowering",
        )
    }

    fn next_match_dispatch_block(
        &mut self,
        index: usize,
        arm_count: usize,
        parent_region: RegionId,
        default_block: Option<BlockId>,
        no_match_block: Option<BlockId>,
        location: &SourceLocation,
    ) -> Result<BlockId, CompilerError> {
        if index + 1 < arm_count {
            return self.create_block(parent_region, location, "match-next");
        }

        if let Some(default_block_id) = default_block {
            return Ok(default_block_id);
        }

        if let Some(no_match_block_id) = no_match_block {
            return Ok(no_match_block_id);
        }

        return_hir_transformation_error!(
            "Match dispatch had no next arm, default arm, or fallback runtime-failure block",
            self.hir_error_location(location)
        )
    }

    fn lower_match_default_body(
        &mut self,
        default_block: Option<BlockId>,
        default: Option<&[AstNode]>,
        parent_region: RegionId,
        location: &SourceLocation,
        merge_block: &mut Option<BlockId>,
        terminated_anchor: &mut Option<BlockId>,
    ) -> Result<(), CompilerError> {
        let (Some(default_block_id), Some(default_body)) = (default_block, default) else {
            return Ok(());
        };

        self.set_current_block(default_block_id, location)?;
        self.lower_statement_sequence(default_body)?;

        let default_tail_block = self.current_block_id_or_error(location)?;
        let default_terminated =
            self.block_has_explicit_terminator(default_tail_block, location)?;
        if default_terminated {
            if terminated_anchor.is_none() {
                *terminated_anchor = Some(default_tail_block);
            }

            return Ok(());
        }

        let merge_target = self.ensure_match_merge_block(parent_region, location, merge_block)?;
        self.emit_jump_to(
            default_tail_block,
            merge_target,
            location,
            "match.default.merge",
        )
    }

    fn finish_match_lowering(
        &mut self,
        merge_block: Option<BlockId>,
        terminated_anchor: Option<BlockId>,
        location: &SourceLocation,
        context: &str,
    ) -> Result<(), CompilerError> {
        if let Some(merge_block_id) = merge_block {
            return self.set_current_block(merge_block_id, location);
        }

        if let Some(anchor_block) = terminated_anchor {
            return self.set_current_block(anchor_block, location);
        }

        return_hir_transformation_error!(
            format!("{context} produced no merge block and no terminated anchor block"),
            self.hir_error_location(location)
        )
    }

    fn lower_cfg_match_guard(
        &mut self,
        context: CfgMatchGuardLowering<'_>,
    ) -> Result<(), CompilerError> {
        let CfgMatchGuardLowering {
            arm,
            capture_locals,
            scrutinee_hir,
            scrutinee_ast,
            guard_block,
            arm_body_block,
            next_dispatch,
            location,
        } = context;

        self.set_current_block(guard_block, location)?;

        self.with_arm_capture_bindings(arm, capture_locals, |builder| {
            builder.emit_match_arm_capture_assignments(
                arm,
                capture_locals,
                scrutinee_hir,
                scrutinee_ast,
                location,
            )?;

            let Some(guard) = &arm.guard else {
                return_hir_transformation_error!(
                    "CFG match guard lowering reached an arm without a guard",
                    builder.hir_error_location(location)
                );
            };

            let guard_value = builder.lower_match_guard_value_to_current_block(guard)?;
            let guard_tail_block = builder.current_block_id_or_error(location)?;
            builder.emit_terminator(
                guard_tail_block,
                HirTerminator::If {
                    condition: guard_value,
                    then_block: arm_body_block,
                    else_block: next_dispatch,
                },
                location,
            )?;

            builder.log_control_flow_edge(guard_tail_block, arm_body_block, "match.guard.true");
            builder.log_control_flow_edge(guard_tail_block, next_dispatch, "match.guard.false");
            Ok(())
        })
    }

    fn lower_match_arm_body(
        &mut self,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_hir: &HirExpression,
        scrutinee_ast: &Expression,
        location: &SourceLocation,
        emit_capture_assignments: bool,
    ) -> Result<(), CompilerError> {
        self.with_arm_capture_bindings(arm, capture_locals, |builder| {
            if emit_capture_assignments {
                builder.emit_match_arm_capture_assignments(
                    arm,
                    capture_locals,
                    scrutinee_hir,
                    scrutinee_ast,
                    location,
                )?;
            }

            builder.lower_statement_sequence(&arm.body)
        })
    }

    fn lower_inline_match_guard(
        &mut self,
        arm: &MatchArm,
        capture_locals: &[LocalId],
        scrutinee_ast: &Expression,
        scrutinee_hir: &HirExpression,
        location: &SourceLocation,
    ) -> Result<Option<HirExpression>, CompilerError> {
        let Some(guard) = &arm.guard else {
            return Ok(None);
        };

        let guard_expr = self.lower_match_guard_expression(guard)?;
        if let MatchPattern::ChoiceVariant { captures, .. } = &arm.pattern {
            if !captures.is_empty() && !capture_locals.is_empty() {
                return Ok(Some(self.substitute_match_guard_captures(
                    &guard_expr,
                    arm,
                    capture_locals,
                    scrutinee_ast,
                    scrutinee_hir,
                    location,
                )?));
            }
        } else if matches!(arm.pattern, MatchPattern::OptionPresentCapture { .. })
            && !capture_locals.is_empty()
        {
            let capture_local = capture_locals[0];
            let MatchPattern::OptionPresentCapture {
                inner_type_id,
                binding_location,
                ..
            } = &arm.pattern
            else {
                unreachable!("checked above")
            };
            let field_ty = self.lower_type_id(*inner_type_id, binding_location)?;
            let region = self.current_region_or_error(binding_location)?;
            let payload_get = self.make_expression(
                binding_location,
                HirExpressionKind::VariantPayloadGet {
                    carrier: HirVariantCarrier::Option,
                    source: Box::new(scrutinee_hir.clone()),
                    variant_index:
                        crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX,
                    field_index: 0,
                },
                field_ty,
                ValueKind::RValue,
                region,
            );
            let mut substitutions = rustc_hash::FxHashMap::default();
            substitutions.insert(capture_local, payload_get);
            return Ok(Some(substitute_local_expressions(
                &guard_expr,
                &substitutions,
            )));
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
        condition: &Expression,
    ) -> Result<HirExpression, CompilerError> {
        let lowered_pattern = self.lower_expression(condition)?;
        if !lowered_pattern.prelude.is_empty() {
            return_hir_transformation_error!(
                "Match arm pattern lowering produced side-effect statements; only literal patterns are supported",
                self.hir_error_location(&condition.location)
            );
        }

        if lowered_pattern.value.value_kind != ValueKind::Const {
            return_hir_transformation_error!(
                "Match arm patterns must be compile-time literals",
                self.hir_error_location(&condition.location)
            );
        }

        if !matches!(
            lowered_pattern.value.kind,
            HirExpressionKind::Int(_)
                | HirExpressionKind::Float(_)
                | HirExpressionKind::Bool(_)
                | HirExpressionKind::Char(_)
                | HirExpressionKind::StringLiteral(_)
        ) {
            return_hir_transformation_error!(
                "Match arm patterns currently support only literal int/float/bool/char/string values",
                self.hir_error_location(&condition.location)
            );
        }

        Ok(lowered_pattern.value)
    }

    /// Lower an AST match pattern into its HIR counterpart.
    pub(crate) fn lower_match_pattern(
        &mut self,
        pattern: &MatchPattern,
        scrutinee_type_id: TypeId,
    ) -> Result<HirPattern, CompilerError> {
        match pattern {
            MatchPattern::Literal(expression) => {
                let lowered = self.lower_match_literal_pattern(expression)?;
                Ok(HirPattern::Literal(lowered))
            }

            MatchPattern::OptionNone { .. } => Ok(HirPattern::OptionNone),

            MatchPattern::OptionValue { value, .. } => {
                let lowered = self.lower_match_literal_pattern(value)?;
                Ok(HirPattern::OptionValue { value: lowered })
            }

            MatchPattern::OptionPresentCapture { .. } => Ok(HirPattern::OptionPresent),

            MatchPattern::Relational { op, value, .. } => {
                let lowered_value = self.lower_match_literal_pattern(value)?;
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
                location,
                ..
            } => {
                let choice_id =
                    self.choice_id_for_scrutinee_type(nominal_path, scrutinee_type_id, location)?;
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
        guard: &Expression,
    ) -> Result<HirExpression, CompilerError> {
        let lowered_guard = self.lower_expression(guard)?;
        if !lowered_guard.prelude.is_empty() {
            return_hir_transformation_error!(
                "Match arm guard lowering produced side-effect statements; guards must stay pure boolean expressions",
                self.hir_error_location(&guard.location)
            );
        }

        if lowered_guard.value.ty != self.type_environment.builtins().bool {
            return_hir_transformation_error!(
                "Match arm guards must lower to Bool expressions",
                self.hir_error_location(&guard.location)
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
        guard: &Expression,
    ) -> Result<HirExpression, CompilerError> {
        let guard_value = self.lower_expression_value_to_current_block(guard)?;
        if guard_value.ty != self.type_environment.builtins().bool {
            return_hir_transformation_error!(
                "Match arm guards must lower to Bool expressions",
                self.hir_error_location(&guard.location)
            );
        }

        Ok(guard_value)
    }

    /// Lazily create the shared merge block on the first non-terminal arm that needs it.
    fn ensure_match_merge_block(
        &mut self,
        region: crate::compiler_frontend::hir::ids::RegionId,
        location: &SourceLocation,
        merge_block: &mut Option<BlockId>,
    ) -> Result<BlockId, CompilerError> {
        if let Some(existing) = *merge_block {
            return Ok(existing);
        }

        let created = self.create_block(region, location, "match-merge")?;
        *merge_block = Some(created);
        Ok(created)
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
        source_location: &SourceLocation,
        label: &str,
    ) -> Result<BlockId, CompilerError> {
        let block = HirBlock {
            id: self.allocate_block_id(),
            region,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Uninitialized,
        };

        self.side_table.map_block(source_location, &block);
        self.log_block_created(block.id, label, source_location);

        let id = block.id;
        self.push_block(block);
        Ok(id)
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
        location: &SourceLocation,
    ) -> Result<bool, CompilerError> {
        if self.block_has_incoming_terminator_edge(block_id) {
            return Ok(false);
        }

        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return_hir_transformation_error!(
                format!("Cannot discard unknown HIR block {block_id}."),
                self.hir_error_location(location)
            );
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

    pub(super) fn expression_from_return_values(
        &mut self,
        values: &[HirExpression],
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let region = self.current_region_or_error(location)?;

        match values {
            [] => Ok(self.unit_expression(location, region)),
            [single] => Ok(single.to_owned()),
            many => {
                let field_types = many.iter().map(|value| value.ty).collect::<Vec<_>>();
                let tuple_type = self.type_environment.intern_tuple(field_types);

                Ok(self.make_expression(
                    location,
                    HirExpressionKind::TupleConstruct {
                        elements: many.to_vec(),
                    },
                    tuple_type,
                    ValueKind::RValue,
                    region,
                ))
            }
        }
    }

    pub(crate) fn emit_jump_to(
        &mut self,
        from_block: BlockId,
        target: BlockId,
        location: &SourceLocation,
        edge_label: &str,
    ) -> Result<(), CompilerError> {
        self.emit_jump_with_args(from_block, target, vec![], location, edge_label)
    }

    pub(crate) fn emit_jump_with_args(
        &mut self,
        from_block: BlockId,
        target: BlockId,
        args: Vec<crate::compiler_frontend::hir::ids::LocalId>,
        location: &SourceLocation,
        edge_label: &str,
    ) -> Result<(), CompilerError> {
        self.emit_terminator(from_block, HirTerminator::Jump { target, args }, location)?;

        self.log_control_flow_edge(from_block, target, edge_label);
        Ok(())
    }

    pub(crate) fn emit_terminator(
        &mut self,
        block_id: BlockId,
        terminator: HirTerminator,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.log_terminator_emitted(block_id, &terminator, location);
        self.set_block_terminator(block_id, terminator, location)
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
        location: &SourceLocation,
    ) -> Result<LoopTargets, CompilerError> {
        let Some(targets) = self.loop_targets.last().copied() else {
            return_hir_transformation_error!(
                format!(
                    "'{}' reached HIR lowering without an active loop context",
                    keyword
                ),
                self.hir_error_location(location)
            );
        };

        Ok(targets)
    }

    pub(crate) fn is_unit_type(&self, ty: TypeId) -> bool {
        ty == self.type_environment.builtins().none
    }
}
