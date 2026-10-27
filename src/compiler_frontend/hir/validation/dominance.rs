//! Definite local definitions across executable control-flow edges.
//!
//! Local declarations reserve storage, not values. This must-analysis intersects incoming
//! definitions before checking reads, so loop backedges and Invoke's mutually exclusive
//! destinations cannot expose an earlier execution's stale success or error value.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
use crate::compiler_frontend::hir::utils::for_each_terminator_target;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

type Definitions = FxHashSet<LocalId>;

#[derive(Default)]
struct ExpressionReads {
    pending: Vec<HirValueId>,
    seen: FxHashSet<HirValueId>,
}

impl HirValidator<'_> {
    pub(super) fn validate_function_definitions(&self) -> Result<(), InfrastructureFailure> {
        let mut incoming: FxHashMap<BlockId, Definitions> = FxHashMap::default();
        let mut pending = VecDeque::new();
        let mut queued = FxHashSet::default();
        for function in &self.module.functions {
            // Structural validation has already established disjoint function CFGs.
            incoming.insert(function.entry, function.params.iter().copied().collect());
            pending.push_back(function.entry);
            queued.insert(function.entry);

            // Solve first, then validate reads. A first visited predecessor is not a proof
            // that a definition survives every predecessor or a later loop iteration.
            while let Some(block_id) = pending.pop_front() {
                queued.remove(&block_id);
                let block = &self.module.blocks[self.block_index_by_id[&block_id]];
                let mut outgoing = incoming[&block_id].clone();
                for statement in &block.statements {
                    transfer_statement_definitions(&statement.kind, &mut outgoing);
                }
                match &block.terminator {
                    HirTerminator::Invoke {
                        success,
                        success_results,
                        failure,
                        error_result,
                        ..
                    } => {
                        for result in success_results {
                            outgoing.remove(result);
                        }
                        outgoing.remove(error_result);
                        let mut success_definitions = outgoing.clone();
                        success_definitions.extend(success_results.iter().copied());
                        merge_definitions(
                            *success,
                            &success_definitions,
                            &mut incoming,
                            &mut pending,
                            &mut queued,
                        );
                        outgoing.insert(*error_result);
                        merge_definitions(
                            *failure,
                            &outgoing,
                            &mut incoming,
                            &mut pending,
                            &mut queued,
                        );
                    }
                    HirTerminator::Jump { target, args } => {
                        outgoing.extend(args.iter().map(|argument| argument.destination));
                        merge_definitions(
                            *target,
                            &outgoing,
                            &mut incoming,
                            &mut pending,
                            &mut queued,
                        );
                    }
                    terminator => for_each_terminator_target(terminator, |target| {
                        merge_definitions(
                            target,
                            &outgoing,
                            &mut incoming,
                            &mut pending,
                            &mut queued,
                        );
                    }),
                }
            }
        }

        // Walk the module once, in storage order, for deterministic diagnostics.
        let mut reads = ExpressionReads::default();
        for block in &self.module.blocks {
            let Some(entry_definitions) = incoming.get(&block.id) else {
                continue;
            };
            let mut defined = entry_definitions.clone();
            for statement in &block.statements {
                self.validate_statement_reads(statement, &defined, &mut reads)?;
                transfer_statement_definitions(&statement.kind, &mut defined);
            }
            self.validate_terminator_reads(&block.terminator, &defined, &mut reads)?;
        }
        Ok(())
    }

    fn require_defined_local(
        &self,
        local: LocalId,
        defined: &Definitions,
    ) -> Result<(), InfrastructureFailure> {
        if !defined.contains(&local) {
            return Err(self.error_with_hir(
                format!("Local {local} is not definitely defined before this use (definition does not dominate the use)"),
            ));
        }
        Ok(())
    }

    fn validate_statement_reads(
        &self,
        statement: &HirStatement,
        defined: &Definitions,
        reads: &mut ExpressionReads,
    ) -> Result<(), InfrastructureFailure> {
        let mut expression = |value| self.validate_expression_reads(value, defined, reads);
        match &statement.kind {
            HirStatementKind::Write { target, value } => {
                expression(*value)?;
                if let HirWriteTarget::AssignPlace(place) = target {
                    self.validate_place_reads(*place, defined, reads)?;
                }
            }
            HirStatementKind::Call { args, results, .. } => {
                for argument in self.module.expressions.values(*args) {
                    expression(*argument)?;
                }
                for destination in results {
                    self.validate_destination_reads(*destination, defined)?;
                }
            }
            HirStatementKind::Expr(value) => expression(*value)?,
            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                expression(*value)?;
                self.require_defined_local(*vec_local, defined)?;
            }
            HirStatementKind::MapOp {
                receiver,
                args,
                result,
                ..
            } => {
                expression(*receiver)?;
                for argument in self.module.expressions.values(*args) {
                    expression(*argument)?;
                }
                if let Some(destination) = result {
                    self.validate_destination_reads(*destination, defined)?;
                }
            }
            HirStatementKind::NumericOp {
                operands, result, ..
            } => {
                match operands {
                    HirNumericOperands::Unary { operand } => expression(*operand)?,
                    HirNumericOperands::Binary { left, right } => {
                        expression(*left)?;
                        expression(*right)?;
                    }
                }
                self.validate_destination_reads(*result, defined)?;
            }
            HirStatementKind::RangeStepFailure { result, .. } => {
                self.validate_destination_reads(*result, defined)?
            }
            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                candidate_result,
                in_range_result,
                ..
            } => {
                for value in [current, step, end, ascending] {
                    expression(*value)?;
                }
                self.validate_destination_reads(*candidate_result, defined)?;
                self.validate_destination_reads(*in_range_result, defined)?;
            }
            HirStatementKind::FormatFloat { source, result, .. }
            | HirStatementKind::ValidateFloat { source, result, .. } => {
                expression(*source)?;
                self.validate_destination_reads(*result, defined)?;
            }
        }
        Ok(())
    }

    fn validate_destination_reads(
        &self,
        destination: HirLocalDestination,
        defined: &Definitions,
    ) -> Result<(), InfrastructureFailure> {
        if let HirLocalDestination::Update(local) = destination {
            self.require_defined_local(local, defined)?;
        }
        Ok(())
    }

    fn validate_terminator_reads(
        &self,
        terminator: &HirTerminator,
        defined: &Definitions,
        reads: &mut ExpressionReads,
    ) -> Result<(), InfrastructureFailure> {
        let mut expression = |value| self.validate_expression_reads(value, defined, reads);
        match terminator {
            HirTerminator::Jump { args, .. } => {
                // All sources are read in the incoming state, before any destination exists.
                for argument in args {
                    self.require_defined_local(argument.source, defined)?;
                }
            }
            HirTerminator::If { condition, .. } => expression(*condition)?,
            HirTerminator::Invoke { operation, .. } => {
                for operand in operation.operands(&self.module.expressions) {
                    expression(operand)?;
                }
            }
            HirTerminator::Match { scrutinee, arms } => {
                expression(*scrutinee)?;
                for arm in arms {
                    match &arm.pattern {
                        HirPattern::Literal(value)
                        | HirPattern::OptionValue { value }
                        | HirPattern::OptionRelational { value, .. }
                        | HirPattern::Relational { value, .. } => expression(*value)?,
                        HirPattern::OptionNone
                        | HirPattern::OptionPresent
                        | HirPattern::Wildcard
                        | HirPattern::ChoiceVariant { .. } => {}
                    }
                    if let Some(guard) = arm.guard {
                        expression(guard)?;
                    }
                }
            }
            HirTerminator::Return(values) => {
                for value in self.module.expressions.values(*values) {
                    expression(*value)?;
                }
            }
            HirTerminator::ReturnError(value)
            | HirTerminator::AssertFailure { message: value, .. } => expression(*value)?,
            HirTerminator::RuntimeFailure {
                cause: Some(cause), ..
            } => {
                let (RuntimeFailureCause::StoreConversion { error }
                | RuntimeFailureCause::AuthoredCastConversion { error }) = cause;
                self.require_defined_local(*error, defined)?;
            }
            HirTerminator::Break { .. }
            | HirTerminator::Continue { .. }
            | HirTerminator::Uninitialized
            | HirTerminator::RuntimeFailure { cause: None, .. } => {}
        }
        Ok(())
    }

    fn validate_place_reads(
        &self,
        place: HirPlace,
        defined: &Definitions,
        reads: &mut ExpressionReads,
    ) -> Result<(), InfrastructureFailure> {
        self.require_defined_local(place.root, defined)?;
        for index in self.module.expressions.place_index_values(place) {
            self.validate_expression_reads(index, defined, reads)?;
        }
        Ok(())
    }

    fn validate_expression_reads(
        &self,
        root: HirValueId,
        defined: &Definitions,
        reads: &mut ExpressionReads,
    ) -> Result<(), InfrastructureFailure> {
        reads.pending.clear();
        reads.seen.clear();
        reads.pending.push(root);
        while let Some(value) = reads.pending.pop() {
            if !reads.seen.insert(value) {
                continue;
            }
            match &self.module.expressions.expression(value).kind {
                // WHY: index operands are pushed in projection order, so the last index is checked
                // first. The store's source-order listing would reverse which undefined read is
                // reported first, so Load and Copy keep the projection push.
                HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
                    self.require_defined_local(place.root, defined)?;
                    let indexes = self.module.expressions.place_index_values(*place);
                    reads.pending.extend(indexes);
                }
                _ => {
                    let expressions = &self.module.expressions;
                    let first = expressions.push_child_values_in_source_order(
                        value,
                        &mut reads.pending,
                        |child| child,
                    );
                    reads.pending.extend(first);
                }
            }
        }
        Ok(())
    }
}

fn merge_definitions(
    target: BlockId,
    edge: &Definitions,
    incoming: &mut FxHashMap<BlockId, Definitions>,
    pending: &mut VecDeque<BlockId>,
    queued: &mut FxHashSet<BlockId>,
) {
    let changed = match incoming.entry(target) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(edge.clone());
            true
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            let previous_len = entry.get().len();
            entry.get_mut().retain(|local| edge.contains(local));
            entry.get().len() != previous_len
        }
    };
    if changed && queued.insert(target) {
        pending.push_back(target);
    }
}

fn transfer_statement_definitions(statement: &HirStatementKind, defined: &mut Definitions) {
    match statement {
        HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(local),
            ..
        } => {
            defined.insert(*local);
        }
        HirStatementKind::Call { results, .. } => {
            defined.extend(results.iter().map(|result| result.local()));
        }
        HirStatementKind::MapOp { result, .. } => {
            if let Some(result) = result {
                defined.insert(result.local());
            }
        }
        HirStatementKind::NumericOp { result, .. }
        | HirStatementKind::RangeStepFailure { result, .. }
        | HirStatementKind::FormatFloat { result, .. }
        | HirStatementKind::ValidateFloat { result, .. } => {
            defined.insert(result.local());
        }
        HirStatementKind::FloatRangeCandidate {
            candidate_result,
            in_range_result,
            ..
        } => {
            defined.insert(candidate_result.local());
            defined.insert(in_range_result.local());
        }
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(_),
            ..
        }
        | HirStatementKind::Expr(_)
        | HirStatementKind::PushRuntimeFragment { .. } => {}
    }
}
