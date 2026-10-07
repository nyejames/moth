//! Function and CFG emission helpers for the JavaScript backend.
//!
//! This module recognises acyclic branch regions and emits their shared continuation once.
//! Cyclic or unsupported control flow retains the dispatcher fallback.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlFlowStrategy {
    Structured,
    Dispatcher,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BranchTermination {
    Jump(BlockId),
    Terminated,
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_function(&mut self, function: &HirFunction) -> Result<(), CompilerError> {
        let function_name = self.function_name(function.id)?.to_owned();

        let mut parameters = Vec::with_capacity(function.params.len());
        for parameter_local in &function.params {
            parameters.push(self.local_name(*parameter_local)?.to_owned());
        }

        self.emit_line(&format!(
            "function {}({}) {{",
            function_name,
            parameters.join(", ")
        ));

        self.indent += 1;

        let reachable_blocks = self.collect_reachable_blocks(function.entry)?;
        self.emit_function_local_declarations(function, &reachable_blocks)?;
        self.emit_parameter_binding_setup(function)?;
        self.validate_jump_argument_contract(&reachable_blocks)?;

        let strategy = self.choose_control_flow_strategy(function)?;
        self.current_function = Some(function.id);
        let emit_body_result: Result<(), CompilerError> = if self.function_is_fallible(function) {
            self.emit_line("try {");
            self.indent += 1;
            match strategy {
                ControlFlowStrategy::Structured => {
                    self.emit_structured_function_body(function)?;
                }
                ControlFlowStrategy::Dispatcher => {
                    self.emit_dispatcher_for_function(function, &reachable_blocks)?;
                }
            }
            self.indent -= 1;
            self.emit_line("} catch (__moth_err) {");
            self.indent += 1;
            self.emit_line("if (__moth_err && __moth_err.__moth_result_propagate === true) {");
            self.indent += 1;
            self.emit_line("return { tag: \"err\", value: __moth_err.value };");
            self.indent -= 1;
            self.emit_line("}");
            self.emit_line("throw __moth_err;");
            self.indent -= 1;
            self.emit_line("}");
            Ok(())
        } else {
            match strategy {
                ControlFlowStrategy::Structured => {
                    self.emit_structured_function_body(function)?;
                }
                ControlFlowStrategy::Dispatcher => {
                    self.emit_dispatcher_for_function(function, &reachable_blocks)?;
                }
            }
            Ok(())
        };
        self.current_function = None;
        emit_body_result?;

        self.indent -= 1;
        self.emit_line("}");

        Ok(())
    }

    fn emit_function_local_declarations(
        &mut self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
    ) -> Result<(), CompilerError> {
        let parameter_set = function.params.iter().copied().collect::<HashSet<_>>();
        let mut local_ids = Vec::new();

        for block_id in reachable_blocks {
            let block = self.block_by_id(*block_id)?;
            for local in &block.locals {
                if !parameter_set.contains(&local.id) {
                    local_ids.push(local.id);
                }
            }
        }

        local_ids.sort_by_key(|local_id| local_id.0);
        local_ids.dedup_by_key(|local_id| local_id.0);

        for local_id in local_ids {
            let local_name = self.local_name(local_id)?;
            self.emit_line(&format!("let {local_name} = __moth_binding(undefined);"));
        }

        if !reachable_blocks.is_empty() || !function.params.is_empty() {
            self.emit_line("");
        }

        Ok(())
    }

    pub(crate) fn function_is_fallible(&self, function: &HirFunction) -> bool {
        self.type_environment
            .is_fallible_carrier(function.return_type)
    }

    fn emit_parameter_binding_setup(
        &mut self,
        function: &HirFunction,
    ) -> Result<(), CompilerError> {
        for parameter_local in &function.params {
            let parameter_name = self.local_name(*parameter_local)?;
            self.emit_line(&format!(
                "{parameter_name} = __moth_param_binding({parameter_name});"
            ));
        }

        if !function.params.is_empty() {
            self.emit_line("");
        }

        Ok(())
    }

    fn choose_control_flow_strategy(
        &self,
        function: &HirFunction,
    ) -> Result<ControlFlowStrategy, CompilerError> {
        if self.has_cfg_cycle(function.entry)? {
            return Ok(ControlFlowStrategy::Dispatcher);
        }

        // Follow lexical regions from entry through their joins, not every CFG block independently.
        // One ownership set catches shared bodies even across separate continuation regions and
        // visits each nested branch once instead of repeatedly inspecting success-chain suffixes.
        let mut owned_blocks = HashSet::new();
        let mut cursor = function.entry;
        loop {
            let block = self.block_by_id(cursor)?;
            let termination = match &block.terminator {
                HirTerminator::Match { arms, .. } => {
                    if !owned_blocks.insert(cursor) {
                        return Ok(ControlFlowStrategy::Dispatcher);
                    }
                    self.inspect_match_termination(arms, &mut owned_blocks)
                }

                _ => self.inspect_branch_termination(cursor, &mut owned_blocks),
            };
            match termination {
                Ok(BranchTermination::Jump(target)) => cursor = target,
                Ok(BranchTermination::Terminated) => return Ok(ControlFlowStrategy::Structured),
                Err(_) => return Ok(ControlFlowStrategy::Dispatcher),
            }
        }
    }

    fn validate_jump_argument_contract(
        &self,
        reachable_blocks: &[BlockId],
    ) -> Result<(), CompilerError> {
        let mut incoming_arity_by_target = HashMap::new();

        for source_block_id in reachable_blocks {
            let block = self.block_by_id(*source_block_id)?;
            match &block.terminator {
                HirTerminator::Jump { target, args } => {
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *target,
                        args.len(),
                        &mut incoming_arity_by_target,
                    )?;
                }

                HirTerminator::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *then_block,
                        0,
                        &mut incoming_arity_by_target,
                    )?;
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *else_block,
                        0,
                        &mut incoming_arity_by_target,
                    )?;
                }

                HirTerminator::FallibleBranch {
                    success_block,
                    error_block,
                    ..
                } => {
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *success_block,
                        0,
                        &mut incoming_arity_by_target,
                    )?;
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *error_block,
                        0,
                        &mut incoming_arity_by_target,
                    )?;
                }

                HirTerminator::Match { arms, .. } => {
                    for arm in arms {
                        self.record_incoming_jump_arity(
                            *source_block_id,
                            arm.body,
                            0,
                            &mut incoming_arity_by_target,
                        )?;
                    }
                }

                HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                    self.record_incoming_jump_arity(
                        *source_block_id,
                        *target,
                        0,
                        &mut incoming_arity_by_target,
                    )?;
                }

                HirTerminator::Return(_)
                | HirTerminator::ReturnSuccess(_)
                | HirTerminator::ReturnError(_)
                | HirTerminator::RuntimeFailure { .. }
                | HirTerminator::Uninitialized
                | HirTerminator::AssertFailure { .. } => {}
            }
        }

        for (target, arity) in incoming_arity_by_target {
            self.ensure_jump_target_parameter_arity(target, arity)?;
        }

        Ok(())
    }

    fn record_incoming_jump_arity(
        &self,
        source: BlockId,
        target: BlockId,
        arity: usize,
        incoming_arity_by_target: &mut HashMap<BlockId, usize>,
    ) -> Result<(), CompilerError> {
        if let Some(existing_arity) = incoming_arity_by_target.get(&target)
            && *existing_arity != arity
        {
            return Err(CompilerError::compiler_error(format!(
                "JavaScript backend: block {} receives inconsistent incoming jump argument counts ({existing_arity} vs {arity}) at predecessor block {}",
                target.0, source.0
            )));
        }

        incoming_arity_by_target.insert(target, arity);
        Ok(())
    }

    fn ensure_jump_target_parameter_arity(
        &self,
        target: BlockId,
        arity: usize,
    ) -> Result<(), CompilerError> {
        let target_block = self.block_by_id(target)?;
        if arity > target_block.locals.len() {
            return Err(CompilerError::compiler_error(format!(
                "JavaScript backend: block {} receives {arity} jump argument(s), but only {} target local(s) are available",
                target.0,
                target_block.locals.len()
            )));
        }
        Ok(())
    }

    fn jump_target_parameter_locals(
        &self,
        target: BlockId,
        arity: usize,
    ) -> Result<Vec<LocalId>, CompilerError> {
        self.ensure_jump_target_parameter_arity(target, arity)?;
        let target_block = self.block_by_id(target)?;
        Ok(target_block
            .locals
            .iter()
            .take(arity)
            .map(|local| local.id)
            .collect())
    }

    pub(crate) fn emit_jump_argument_transfer(
        &mut self,
        target: BlockId,
        args: &[LocalId],
    ) -> Result<(), CompilerError> {
        if args.is_empty() {
            return Ok(());
        }

        let destination_locals = self.jump_target_parameter_locals(target, args.len())?;
        let mut captured_values = Vec::with_capacity(args.len());
        for source_local in args {
            let source_name = self.local_name(*source_local)?.to_owned();
            let captured_name = self.next_temp_identifier("__jump_arg");
            self.emit_line(&format!(
                "const {captured_name} = __moth_read({source_name});"
            ));
            captured_values.push(captured_name);
        }

        for (destination_local, captured_name) in destination_locals.iter().zip(captured_values) {
            let destination_name = self.local_name(*destination_local)?.to_owned();
            if self.local_is_alias_only_at_block_entry(target, *destination_local) {
                self.emit_line(&format!(
                    "__moth_write({destination_name}, {captured_name});"
                ));
            } else {
                self.emit_line(&format!(
                    "__moth_assign_value({destination_name}, {captured_name});"
                ));
            }
        }

        Ok(())
    }

    fn has_cfg_cycle(&self, entry_block: BlockId) -> Result<bool, CompilerError> {
        fn dfs(
            emitter: &JsEmitter<'_>,
            block_id: BlockId,
            visiting: &mut HashSet<BlockId>,
            visited: &mut HashSet<BlockId>,
        ) -> Result<bool, CompilerError> {
            if visiting.contains(&block_id) {
                return Ok(true);
            }

            if visited.contains(&block_id) {
                return Ok(false);
            }

            visiting.insert(block_id);

            let block = emitter.block_by_id(block_id)?;
            for successor in terminator_targets(&block.terminator) {
                if dfs(emitter, successor, visiting, visited)? {
                    return Ok(true);
                }
            }

            visiting.remove(&block_id);
            visited.insert(block_id);

            Ok(false)
        }

        dfs(self, entry_block, &mut HashSet::new(), &mut HashSet::new())
    }

    fn emit_structured_function_body(
        &mut self,
        function: &HirFunction,
    ) -> Result<(), CompilerError> {
        let mut emitted_blocks = HashSet::new();
        self.emit_structured_block(function.entry, &mut emitted_blocks)
    }

    fn emit_structured_block(
        &mut self,
        block_id: BlockId,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<(), CompilerError> {
        if emitted_blocks.contains(&block_id) {
            return Ok(());
        }

        emitted_blocks.insert(block_id);

        let block = self.block_by_id(block_id)?.clone();
        self.emit_block_statements(&block)?;

        match &block.terminator {
            HirTerminator::Jump { target, args } => {
                self.emit_jump_argument_transfer(*target, args)?;
                self.emit_structured_block(*target, emitted_blocks)
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => self.emit_structured_if(condition, *then_block, *else_block, emitted_blocks),

            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => self.emit_structured_fallible_branch(
                result,
                *success_block,
                *error_block,
                emitted_blocks,
            ),

            HirTerminator::Match { scrutinee, arms } => {
                self.emit_structured_match(scrutinee, arms, emitted_blocks)
            }

            HirTerminator::Return(expression) => self.emit_return_terminator(expression),
            HirTerminator::ReturnSuccess(expression) => {
                self.emit_success_return_terminator(expression)
            }
            HirTerminator::ReturnError(expression) => self.emit_error_return_terminator(expression),
            HirTerminator::Uninitialized => Err(CompilerError::compiler_error(
                "JavaScript backend: structured lowering encountered Uninitialized terminator",
            )),
            HirTerminator::RuntimeFailure { message, .. } => {
                self.emit_runtime_failure_terminator(message)
            }
            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => self.emit_assert_failure_terminator(message, *message_evaluation),

            HirTerminator::Break { .. } | HirTerminator::Continue { .. } => {
                Err(CompilerError::compiler_error(
                    "JavaScript backend: structured lowering does not support Break/Continue terminators",
                ))
            }
        }
    }

    fn emit_structured_if(
        &mut self,
        condition: &crate::compiler_frontend::hir::expressions::HirExpression,
        then_block: BlockId,
        else_block: BlockId,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<(), CompilerError> {
        let mut branch_blocks = HashSet::new();
        let then_termination = self.inspect_branch_termination(then_block, &mut branch_blocks)?;
        let else_termination = self.inspect_branch_termination(else_block, &mut branch_blocks)?;
        let merge_target = Self::resolve_branch_merge_target(then_termination, else_termination)?;

        let condition = self.lower_expr(condition)?;

        self.emit_line(&format!("if ({condition}) {{"));
        self.indent += 1;
        self.emit_branch_block(then_block, merge_target, emitted_blocks)?;
        self.indent -= 1;

        self.emit_line("} else {");
        self.indent += 1;
        self.emit_branch_block(else_block, merge_target, emitted_blocks)?;
        self.indent -= 1;
        self.emit_line("}");

        if let Some(merge_target) = merge_target {
            self.emit_structured_block(merge_target, emitted_blocks)?;
        }

        Ok(())
    }

    fn emit_structured_fallible_branch(
        &mut self,
        result: &crate::compiler_frontend::hir::expressions::HirExpression,
        success_block: BlockId,
        error_block: BlockId,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<(), CompilerError> {
        let mut branch_blocks = HashSet::new();
        let success_termination =
            self.inspect_branch_termination(success_block, &mut branch_blocks)?;
        let error_termination = self.inspect_branch_termination(error_block, &mut branch_blocks)?;
        let merge_target =
            Self::resolve_branch_merge_target(success_termination, error_termination)?;

        let condition = self.lower_fallible_success_condition(result)?;

        self.emit_line(&format!("if ({condition}) {{"));
        self.indent += 1;
        self.emit_branch_block(success_block, merge_target, emitted_blocks)?;
        self.indent -= 1;

        self.emit_line("} else {");
        self.indent += 1;
        self.emit_branch_block(error_block, merge_target, emitted_blocks)?;
        self.indent -= 1;
        self.emit_line("}");

        if let Some(merge_target) = merge_target {
            self.emit_structured_block(merge_target, emitted_blocks)?;
        }

        Ok(())
    }

    fn emit_structured_match(
        &mut self,
        scrutinee: &crate::compiler_frontend::hir::expressions::HirExpression,
        arms: &[HirMatchArm],
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<(), CompilerError> {
        let merge_target = self.resolve_match_merge_target(arms)?;
        let scrutinee_type = scrutinee.ty;
        let scrutinee = self.lower_expr(scrutinee)?;
        let scrutinee_temp = self.next_temp_identifier("__match_value");
        let synthetic_merge_wildcard = merge_target.and_then(|target| {
            arms.iter().position(|arm| {
                matches!(arm.pattern, HirPattern::Wildcard)
                    && arm.guard.is_none()
                    && arm.body == target
            })
        });

        self.emit_line(&format!("const {scrutinee_temp} = {scrutinee};"));

        let mut emitted_arm_count = 0usize;
        for (index, arm) in arms.iter().enumerate() {
            if synthetic_merge_wildcard == Some(index) {
                continue;
            }

            let condition = self.lower_match_arm_condition(&scrutinee_temp, scrutinee_type, arm)?;

            if emitted_arm_count == 0 {
                self.emit_line(&format!("if ({condition}) {{"));
            } else {
                self.emit_line(&format!("else if ({condition}) {{"));
            }

            self.indent += 1;
            self.emit_branch_block(arm.body, merge_target, emitted_blocks)?;
            self.indent -= 1;
            self.emit_line("}");
            emitted_arm_count += 1;
        }

        if let Some(merge_target) = merge_target {
            self.emit_structured_block(merge_target, emitted_blocks)?;
        }

        Ok(())
    }

    fn emit_branch_block(
        &mut self,
        block_id: BlockId,
        expected_merge_target: Option<BlockId>,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<BranchTermination, CompilerError> {
        if expected_merge_target == Some(block_id) {
            return Ok(BranchTermination::Jump(block_id));
        }

        if emitted_blocks.contains(&block_id) {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: branch block was emitted more than once during structured lowering",
            ));
        }

        emitted_blocks.insert(block_id);

        let block = self.block_by_id(block_id)?.clone();
        self.emit_block_statements(&block)?;

        match &block.terminator {
            HirTerminator::Jump { target, args } => {
                if expected_merge_target == Some(*target) {
                    self.emit_jump_argument_transfer(*target, args)?;
                    Ok(BranchTermination::Jump(*target))
                } else {
                    Err(CompilerError::compiler_error(
                        "JavaScript backend: structured branch jumped to unexpected target",
                    ))
                }
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.lower_expr(condition)?;
                self.emit_nested_branch(
                    &condition,
                    *then_block,
                    *else_block,
                    expected_merge_target,
                    emitted_blocks,
                )
            }

            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                let condition = self.lower_fallible_success_condition(result)?;
                self.emit_nested_branch(
                    &condition,
                    *success_block,
                    *error_block,
                    expected_merge_target,
                    emitted_blocks,
                )
            }

            HirTerminator::Return(expression) => {
                self.emit_return_terminator(expression)?;
                Ok(BranchTermination::Terminated)
            }

            HirTerminator::ReturnSuccess(expression) => {
                self.emit_success_return_terminator(expression)?;
                Ok(BranchTermination::Terminated)
            }

            HirTerminator::ReturnError(expression) => {
                self.emit_error_return_terminator(expression)?;
                Ok(BranchTermination::Terminated)
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                self.emit_assert_failure_terminator(message, *message_evaluation)?;
                Ok(BranchTermination::Terminated)
            }

            HirTerminator::RuntimeFailure { message, .. } => {
                self.emit_runtime_failure_terminator(message)?;
                Ok(BranchTermination::Terminated)
            }

            _ => Err(CompilerError::compiler_error(
                "JavaScript backend: structured lowering encountered unsupported branch terminator",
            )),
        }
    }

    /// Keep nested edges inside their selected arm, leaving the shared join to its outer owner.
    fn emit_nested_branch(
        &mut self,
        condition: &str,
        then_block: BlockId,
        else_block: BlockId,
        expected_merge_target: Option<BlockId>,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<BranchTermination, CompilerError> {
        self.emit_line(&format!("if ({condition}) {{"));
        self.indent += 1;
        let then_termination =
            self.emit_branch_block(then_block, expected_merge_target, emitted_blocks)?;
        self.indent -= 1;

        self.emit_line("} else {");
        self.indent += 1;
        let else_termination =
            self.emit_branch_block(else_block, expected_merge_target, emitted_blocks)?;
        self.indent -= 1;
        self.emit_line("}");

        Ok(
            Self::resolve_branch_merge_target(then_termination, else_termination)?
                .map_or(BranchTermination::Terminated, BranchTermination::Jump),
        )
    }

    fn inspect_branch_termination(
        &self,
        block_id: BlockId,
        branch_blocks: &mut HashSet<BlockId>,
    ) -> Result<BranchTermination, CompilerError> {
        // Branch bodies need one lexical owner. A shared exit is safe only when represented
        // by a jump target, which the enclosing conditional emits after its arms.
        if !branch_blocks.insert(block_id) {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: structured branches share a non-merge block",
            ));
        }

        let block = self.block_by_id(block_id)?;

        match &block.terminator {
            HirTerminator::Jump { target, .. } => Ok(BranchTermination::Jump(*target)),

            HirTerminator::If {
                then_block: success_block,
                else_block: error_block,
                ..
            }
            | HirTerminator::FallibleBranch {
                success_block,
                error_block,
                ..
            } => {
                let success_termination =
                    self.inspect_branch_termination(*success_block, branch_blocks)?;
                let error_termination =
                    self.inspect_branch_termination(*error_block, branch_blocks)?;
                Ok(
                    Self::resolve_branch_merge_target(success_termination, error_termination)?
                        .map_or(BranchTermination::Terminated, BranchTermination::Jump),
                )
            }

            HirTerminator::Return(_)
            | HirTerminator::ReturnSuccess(_)
            | HirTerminator::ReturnError(_)
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. } => Ok(BranchTermination::Terminated),

            HirTerminator::Uninitialized => Err(CompilerError::compiler_error(
                "JavaScript backend: branch inspection encountered Uninitialized terminator",
            )),

            _ => Err(CompilerError::compiler_error(
                "JavaScript backend: branch terminator is unsupported for structured lowering",
            )),
        }
    }

    fn resolve_branch_merge_target(
        then_termination: BranchTermination,
        else_termination: BranchTermination,
    ) -> Result<Option<BlockId>, CompilerError> {
        match (then_termination, else_termination) {
            (BranchTermination::Jump(then_target), BranchTermination::Jump(else_target)) => {
                if then_target == else_target {
                    Ok(Some(then_target))
                } else {
                    Err(CompilerError::compiler_error(
                        "JavaScript backend: structured if-branches jump to different merge targets",
                    ))
                }
            }

            (BranchTermination::Jump(target), BranchTermination::Terminated)
            | (BranchTermination::Terminated, BranchTermination::Jump(target)) => Ok(Some(target)),

            (BranchTermination::Terminated, BranchTermination::Terminated) => Ok(None),
        }
    }

    fn resolve_match_merge_target(
        &self,
        arms: &[HirMatchArm],
    ) -> Result<Option<BlockId>, CompilerError> {
        Ok(
            match self.inspect_match_termination(arms, &mut HashSet::new())? {
                BranchTermination::Jump(target) => Some(target),
                BranchTermination::Terminated => None,
            },
        )
    }

    fn inspect_match_termination(
        &self,
        arms: &[HirMatchArm],
        owned_blocks: &mut HashSet<BlockId>,
    ) -> Result<BranchTermination, CompilerError> {
        if arms.is_empty() {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: structured match has no arms",
            ));
        }

        // A synthetic wildcard points directly at the continuation. Inspect the other arms
        // first so this block remains owned by the post-match region, regardless of arm order.
        let wildcard = arms
            .iter()
            .position(|arm| matches!(arm.pattern, HirPattern::Wildcard) && arm.guard.is_none());
        let mut termination = BranchTermination::Terminated;
        for (index, arm) in arms.iter().enumerate() {
            if !matches!(
                arm.pattern,
                HirPattern::Literal(_)
                    | HirPattern::OptionNone
                    | HirPattern::OptionValue { .. }
                    | HirPattern::OptionRelational { .. }
                    | HirPattern::OptionPresent
                    | HirPattern::Wildcard
                    | HirPattern::ChoiceVariant { .. }
            ) {
                return Err(CompilerError::compiler_error(
                    "JavaScript backend: match pattern is unsupported for structured lowering",
                ));
            }
            if Some(index) == wildcard {
                continue;
            }

            let arm_termination = self.inspect_branch_termination(arm.body, owned_blocks)?;
            termination = Self::resolve_branch_merge_target(termination, arm_termination)?
                .map_or(BranchTermination::Terminated, BranchTermination::Jump);
        }

        if let Some(index) = wildcard {
            let arm = &arms[index];
            if termination != BranchTermination::Jump(arm.body) {
                let arm_termination = self.inspect_branch_termination(arm.body, owned_blocks)?;
                termination = Self::resolve_branch_merge_target(termination, arm_termination)?
                    .map_or(BranchTermination::Terminated, BranchTermination::Jump);
            }
        }

        Ok(termination)
    }
}
