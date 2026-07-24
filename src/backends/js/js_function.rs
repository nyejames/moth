//! Function and CFG emission helpers for the JavaScript backend.
//!
//! This module decides whether a HIR function can stay structured in JS or needs the dispatcher
//! fallback for cyclic control flow.

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

        let strategy = self.choose_control_flow_strategy(function, &reachable_blocks)?;
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
            let initializer = self.reactive_local_initializer(local_id);
            self.emit_line(&format!("let {local_name} = {initializer};"));
        }

        if !reachable_blocks.is_empty() || !function.params.is_empty() {
            self.emit_line("");
        }

        Ok(())
    }

    /// Returns the JS initializer for a local declaration.
    ///
    /// WHAT: reactive source locals receive `__moth_reactive_binding(sourceId, undefined)` so writes
    /// through the binding can schedule source dirtying. Ordinary locals keep the existing
    /// `__moth_binding(undefined)` initializer.
    /// WHY: reactive source identity is HIR metadata, not a type distinction, so JS lowering must
    /// tag the runtime binding with the stable source id.
    fn reactive_local_initializer(&self, local_id: LocalId) -> String {
        let Some(source_id) = self.hir.side_table.reactive_source_id_for_local(local_id) else {
            return "__moth_binding(undefined)".to_owned();
        };

        format!("__moth_reactive_binding({}, undefined)", source_id.0)
    }

    fn function_is_fallible(&self, function: &HirFunction) -> bool {
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
        reachable_blocks: &[BlockId],
    ) -> Result<ControlFlowStrategy, CompilerError> {
        if self.has_cfg_cycle(function.entry)? {
            return Ok(ControlFlowStrategy::Dispatcher);
        }

        for block_id in reachable_blocks {
            let block = self.block_by_id(*block_id)?;

            match &block.terminator {
                HirTerminator::Jump { .. } => {}

                HirTerminator::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    if self.inspect_simple_branch_termination(*then_block).is_err()
                        || self.inspect_simple_branch_termination(*else_block).is_err()
                    {
                        return Ok(ControlFlowStrategy::Dispatcher);
                    }
                }

                HirTerminator::FallibleBranch {
                    success_block,
                    error_block,
                    ..
                } => {
                    if self
                        .inspect_simple_branch_termination(*success_block)
                        .is_err()
                        || self
                            .inspect_simple_branch_termination(*error_block)
                            .is_err()
                    {
                        return Ok(ControlFlowStrategy::Dispatcher);
                    }
                }

                HirTerminator::Match { arms, .. } => {
                    if arms.is_empty() {
                        return Ok(ControlFlowStrategy::Dispatcher);
                    }

                    for arm in arms {
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
                            return Ok(ControlFlowStrategy::Dispatcher);
                        }

                        if self.inspect_simple_branch_termination(arm.body).is_err() {
                            return Ok(ControlFlowStrategy::Dispatcher);
                        }
                    }
                }

                HirTerminator::Break { .. } | HirTerminator::Continue { .. } => {
                    return Ok(ControlFlowStrategy::Dispatcher);
                }

                HirTerminator::Return(_)
                | HirTerminator::ReturnSuccess(_)
                | HirTerminator::ReturnError(_)
                | HirTerminator::RuntimeFailure { .. }
                | HirTerminator::Uninitialized
                | HirTerminator::AssertFailure { .. } => {}
            }
        }

        Ok(ControlFlowStrategy::Structured)
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
                self.emit_line(&format!("__moth_write({destination_name}, {captured_name});"));
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
            HirTerminator::RuntimeFailure { message } => {
                self.emit_runtime_failure_terminator(message)
            }
            HirTerminator::AssertFailure { message } => {
                self.emit_assert_failure_terminator(message)
            }

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
        let then_termination = self.inspect_simple_branch_termination(then_block)?;
        let else_termination = self.inspect_simple_branch_termination(else_block)?;
        let merge_target = Self::resolve_branch_merge_target(then_termination, else_termination)?;

        let condition = self.lower_expr(condition)?;

        self.emit_line(&format!("if ({condition}) {{"));
        self.indent += 1;
        self.emit_simple_branch_block(then_block, merge_target, emitted_blocks)?;
        self.indent -= 1;

        self.emit_line("} else {");
        self.indent += 1;
        self.emit_simple_branch_block(else_block, merge_target, emitted_blocks)?;
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
        let success_termination = self.inspect_simple_branch_termination(success_block)?;
        let error_termination = self.inspect_simple_branch_termination(error_block)?;
        let merge_target =
            Self::resolve_branch_merge_target(success_termination, error_termination)?;

        let condition = self.lower_fallible_success_condition(result)?;

        self.emit_line(&format!("if ({condition}) {{"));
        self.indent += 1;
        self.emit_simple_branch_block(success_block, merge_target, emitted_blocks)?;
        self.indent -= 1;

        self.emit_line("} else {");
        self.indent += 1;
        self.emit_simple_branch_block(error_block, merge_target, emitted_blocks)?;
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

            let condition = self.lower_match_arm_condition(&scrutinee_temp, arm)?;

            if emitted_arm_count == 0 {
                self.emit_line(&format!("if ({condition}) {{"));
            } else {
                self.emit_line(&format!("else if ({condition}) {{"));
            }

            self.indent += 1;
            self.emit_simple_branch_block(arm.body, merge_target, emitted_blocks)?;
            self.indent -= 1;
            self.emit_line("}");
            emitted_arm_count += 1;
        }

        if let Some(merge_target) = merge_target {
            self.emit_structured_block(merge_target, emitted_blocks)?;
        }

        Ok(())
    }

    fn emit_simple_branch_block(
        &mut self,
        block_id: BlockId,
        expected_merge_target: Option<BlockId>,
        emitted_blocks: &mut HashSet<BlockId>,
    ) -> Result<BranchTermination, CompilerError> {
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

            HirTerminator::AssertFailure { message } => {
                self.emit_assert_failure_terminator(message)?;
                Ok(BranchTermination::Terminated)
            }

            HirTerminator::RuntimeFailure { message } => {
                self.emit_runtime_failure_terminator(message)?;
                Ok(BranchTermination::Terminated)
            }

            _ => Err(CompilerError::compiler_error(
                "JavaScript backend: structured lowering encountered unsupported branch terminator",
            )),
        }
    }

    fn inspect_simple_branch_termination(
        &self,
        block_id: BlockId,
    ) -> Result<BranchTermination, CompilerError> {
        let block = self.block_by_id(block_id)?;

        match &block.terminator {
            HirTerminator::Jump { target, .. } => Ok(BranchTermination::Jump(*target)),

            HirTerminator::Return(_)
            | HirTerminator::ReturnSuccess(_)
            | HirTerminator::ReturnError(_)
            | HirTerminator::RuntimeFailure { .. }
            | HirTerminator::AssertFailure { .. } => Ok(BranchTermination::Terminated),

            HirTerminator::Uninitialized => Err(CompilerError::compiler_error(
                "JavaScript backend: branch inspection encountered Uninitialized terminator",
            )),

            _ => Err(CompilerError::compiler_error(
                "JavaScript backend: branch terminator is not simple enough for structured lowering",
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
        let mut jump_targets = Vec::new();

        for arm in arms {
            if let BranchTermination::Jump(target) =
                self.inspect_simple_branch_termination(arm.body)?
            {
                jump_targets.push(target);
            }
        }

        jump_targets.sort_by_key(|target| target.0);
        jump_targets.dedup_by_key(|target| target.0);

        match jump_targets.as_slice() {
            [] => Ok(None),
            [single] => Ok(Some(*single)),
            _ => Err(CompilerError::compiler_error(
                "JavaScript backend: structured match arms jump to different merge targets",
            )),
        }
    }
}
