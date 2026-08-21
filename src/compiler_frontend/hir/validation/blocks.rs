//! Block, statement, terminator, and side-table mapping validation for HIR.
//!
//! WHAT: walks executable block contents and checks local regions, source mappings, terminators,
//! and contained expression/place references.
//! WHY: the HIR side table is the bridge back to AST and source locations for later analysis and
//! infrastructure errors.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{
    HirTerminator, classify_assertion_message_evaluation,
};

#[derive(Clone, Copy)]
enum FallibleReturnSlot {
    Success,
    Error,
}

impl FallibleReturnSlot {
    fn terminator_name(self) -> &'static str {
        match self {
            FallibleReturnSlot::Success => "ReturnSuccess",
            FallibleReturnSlot::Error => "ReturnError",
        }
    }

    fn slot_name(self) -> &'static str {
        match self {
            FallibleReturnSlot::Success => "success",
            FallibleReturnSlot::Error => "error",
        }
    }

    fn select_type(self, success_type: TypeId, error_type: TypeId) -> TypeId {
        match self {
            FallibleReturnSlot::Success => success_type,
            FallibleReturnSlot::Error => error_type,
        }
    }
}

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Block & Statement Validation
    // -------------------------

    pub(super) fn validate_blocks(&self) -> Result<(), CompilerError> {
        for block in &self.module.blocks {
            if matches!(block.terminator, HirTerminator::Uninitialized) {
                return Err(self.error_with_hir(
                    format!(
                        "Block {} still has placeholder terminator Uninitialized after HIR lowering",
                        block.id
                    ),
                    Some(HirLocation::Block(block.id)),
                ));
            }

            self.require_region_id(block.region, Some(HirLocation::Block(block.id)))?;

            for local in &block.locals {
                self.require_type_id(local.ty, Some(HirLocation::Local(local.id)))?;
                self.require_region_id(local.region, Some(HirLocation::Local(local.id)))?;
            }

            for statement in &block.statements {
                self.validate_statement_mappings(statement)?;
                self.validate_statement(statement)?;
            }

            self.validate_terminator_mapping(block.id)?;
            self.validate_terminator(block.id, &block.terminator)?;
        }

        Ok(())
    }

    pub(super) fn validate_statement_mappings(
        &self,
        statement: &HirStatement,
    ) -> Result<(), CompilerError> {
        let statement_location = HirLocation::Statement(statement.id);
        if self
            .module
            .side_table
            .ast_source_id_for_hir(statement_location)
            .is_none()
        {
            return Err(self.error_with_text_location(
                format!(
                    "Statement {} is missing AST->HIR side-table mapping",
                    statement.id
                ),
                &statement.location,
            ));
        }

        if self
            .module
            .side_table
            .hir_source_id_for_hir(statement_location)
            .is_none()
        {
            return Err(self.error_with_text_location(
                format!(
                    "Statement {} is missing HIR source side-table mapping",
                    statement.id
                ),
                &statement.location,
            ));
        }

        Ok(())
    }

    pub(super) fn validate_terminator_mapping(
        &self,
        block_id: BlockId,
    ) -> Result<(), CompilerError> {
        let terminator_location = HirLocation::Terminator(block_id);
        if self
            .module
            .side_table
            .ast_source_id_for_hir(terminator_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!("Block {block_id} terminator is missing AST->HIR side-table mapping"),
                Some(terminator_location),
            ));
        }

        if self
            .module
            .side_table
            .hir_source_id_for_hir(terminator_location)
            .is_none()
        {
            return Err(self.error_with_hir(
                format!("Block {block_id} terminator is missing HIR source side-table mapping",),
                Some(terminator_location),
            ));
        }

        Ok(())
    }

    pub(super) fn validate_statement(&self, statement: &HirStatement) -> Result<(), CompilerError> {
        let anchor = Some(HirLocation::Statement(statement.id));
        match &statement.kind {
            HirStatementKind::Assign { target, value } => {
                let _ = self.validate_place(target, anchor)?;
                self.validate_expression(value, anchor)?;
            }

            HirStatementKind::Call { args, result, .. } => {
                for arg in args {
                    self.validate_expression(arg, anchor)?;
                }

                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;
                }
            }

            HirStatementKind::Expr(expression) => {
                self.validate_expression(expression, anchor)?;
            }

            HirStatementKind::MapOp {
                receiver,
                args,
                result,
                ..
            } => {
                self.validate_expression(receiver, anchor)?;
                for arg in args {
                    self.validate_expression(arg, anchor)?;
                }
                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;
                }
            }

            HirStatementKind::Drop(local) => {
                self.require_local_id(*local, anchor)?;
            }

            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                self.require_local_id(*vec_local, anchor)?;
                self.validate_expression(value, anchor)?;
            }

            HirStatementKind::CastOp { source, result, .. } => {
                self.validate_expression(source, anchor)?;
                if let Some(local_id) = result {
                    self.require_local_id(*local_id, anchor)?;
                }
            }

            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                if op.is_unary() != matches!(operands, HirNumericOperands::Unary { .. }) {
                    return Err(self.error_with_hir(
                        format!(
                            "NumericOp::{op:?} operand shape does not match the operation arity"
                        ),
                        anchor,
                    ));
                }

                match operands {
                    HirNumericOperands::Unary { operand } => {
                        self.validate_expression(operand, anchor)?;
                    }
                    HirNumericOperands::Binary { left, right } => {
                        self.validate_expression(left, anchor)?;
                        self.validate_expression(right, anchor)?;
                    }
                }

                self.require_local_id(*result, anchor)?;

                if *failure_mode == NumericFailureMode::ReturnError {
                    let Some(result_type) = self.local_types.get(result).copied() else {
                        return Err(self.error_with_hir(
                            "NumericOp result local has no registered type",
                            anchor,
                        ));
                    };

                    if self
                        .type_environment
                        .fallible_carrier_slots(result_type)
                        .is_none()
                    {
                        return Err(self.error_with_hir(
                            "NumericOp ReturnError result local must have an internal fallible carrier type",
                            anchor,
                        ));
                    }
                }
            }

            HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            } => {
                self.validate_float_effect_statement(
                    "FormatFloat",
                    source,
                    *failure_mode,
                    *result,
                    self.type_environment.builtins().string,
                    anchor,
                )?;
            }

            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            } => {
                self.validate_float_effect_statement(
                    "ValidateFloat",
                    source,
                    *failure_mode,
                    *result,
                    self.type_environment.builtins().float,
                    anchor,
                )?;
            }
        }

        Ok(())
    }

    fn validate_float_effect_statement(
        &self,
        statement_name: &'static str,
        source: &HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        success_type: TypeId,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        self.validate_expression(source, anchor)?;

        if source.ty != self.type_environment.builtins().float {
            return Err(self.error_with_hir(
                format!("{statement_name} source must have Float type"),
                anchor,
            ));
        }

        self.require_local_id(result, anchor)?;
        let Some(result_type) = self.local_types.get(&result).copied() else {
            return Err(self.error_with_hir(
                format!("{statement_name} result local has no registered type"),
                anchor,
            ));
        };

        match failure_mode {
            NumericFailureMode::Trap => {
                if result_type != success_type {
                    return Err(self.error_with_hir(
                        format!("{statement_name} Trap result local has the wrong success type"),
                        anchor,
                    ));
                }
            }

            NumericFailureMode::ReturnError => {
                let Some((carrier_success_type, _)) =
                    self.type_environment.fallible_carrier_slots(result_type)
                else {
                    return Err(self.error_with_hir(
                        format!(
                            "{statement_name} ReturnError result local must have an internal fallible carrier type"
                        ),
                        anchor,
                    ));
                };

                if carrier_success_type != success_type {
                    return Err(self.error_with_hir(
                        format!("{statement_name} ReturnError carrier has the wrong success type"),
                        anchor,
                    ));
                }
            }
        }

        Ok(())
    }

    pub(super) fn validate_terminator(
        &self,
        block_id: BlockId,
        terminator: &HirTerminator,
    ) -> Result<(), CompilerError> {
        let anchor = Some(HirLocation::Terminator(block_id));

        match terminator {
            HirTerminator::Jump { target, args } => {
                self.require_block_id(*target, anchor)?;
                self.require_same_function_cfg_owner(block_id, *target, anchor)?;
                for local in args {
                    self.require_local_id(*local, anchor)?;
                }
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                self.validate_expression(condition, anchor)?;
                self.require_block_id(*then_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *then_block, anchor)?;
                self.require_block_id(*else_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *else_block, anchor)?;
            }

            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                self.validate_expression(result, anchor)?;
                if self
                    .type_environment
                    .fallible_carrier_slots(result.ty)
                    .is_none()
                {
                    return Err(self.error_with_hir(
                        "FallibleBranch result expression must have an internal fallible carrier type",
                        anchor,
                    ));
                }
                self.require_block_id(*success_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *success_block, anchor)?;
                self.require_block_id(*error_block, anchor)?;
                self.require_same_function_cfg_owner(block_id, *error_block, anchor)?;
            }

            HirTerminator::Match { scrutinee, arms } => {
                self.validate_expression(scrutinee, anchor)?;
                for arm in arms {
                    self.validate_match_arm(block_id, arm, anchor)?;
                }
            }

            HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                self.require_block_id(*target, anchor)?;
                self.require_same_function_cfg_owner(block_id, *target, anchor)?;
            }

            HirTerminator::Return(value) => {
                self.validate_expression(value, anchor)?;
            }

            HirTerminator::ReturnSuccess(value) => {
                self.validate_expression(value, anchor)?;
                self.validate_fallible_return_terminator(
                    block_id,
                    value,
                    FallibleReturnSlot::Success,
                    anchor,
                )?;
            }

            HirTerminator::ReturnError(value) => {
                self.validate_expression(value, anchor)?;
                self.validate_fallible_return_terminator(
                    block_id,
                    value,
                    FallibleReturnSlot::Error,
                    anchor,
                )?;
            }

            HirTerminator::Uninitialized => {
                return Err(self.error_with_hir(
                    "Placeholder Uninitialized terminators are not allowed in validated HIR",
                    anchor,
                ));
            }

            HirTerminator::RuntimeFailure { .. } => {
                // Compiler-generated runtime failures are valid terminal terminators.
                // They carry backend-facing text only, not HIR expressions.
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                // Assertion failure is a valid terminal terminator.
                // The message is an ordinary typed optional value evaluated on the failure edge.
                self.validate_expression(message, anchor)?;
                let expected_message_type = self.type_environment.builtins().string;
                if self.type_environment.option_inner_type(message.ty)
                    != Some(expected_message_type)
                {
                    return Err(
                        self.error_with_hir("AssertFailure message must have type String?", anchor)
                    );
                }
                let actual_evaluation = classify_assertion_message_evaluation(message);
                if *message_evaluation != actual_evaluation {
                    return Err(self.error_with_hir(
                        format!(
                            "AssertFailure message evaluation fact {:?} does not match lowered shape {:?}",
                            message_evaluation, actual_evaluation
                        ),
                        anchor,
                    ));
                }
            }
        }

        Ok(())
    }

    fn validate_fallible_return_terminator(
        &self,
        block_id: BlockId,
        value: &HirExpression,
        slot: FallibleReturnSlot,
        anchor: Option<HirLocation>,
    ) -> Result<(), CompilerError> {
        let Some(function_id) = self.block_owner_by_id.get(&block_id).copied() else {
            return Err(self.error_with_hir(
                format!(
                    "Block {block_id} has no function owner for {}",
                    slot.terminator_name()
                ),
                anchor,
            ));
        };
        let function = self
            .module
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .ok_or_else(|| {
                self.error_with_hir(
                    format!(
                        "{} owner function {function_id:?} is missing",
                        slot.terminator_name()
                    ),
                    anchor,
                )
            })?;
        let Some((success_type, error_type)) = self
            .type_environment
            .fallible_carrier_slots(function.return_type)
        else {
            return Err(self.error_with_hir(
                format!(
                    "{} in function {function_id:?} whose return type has no {} slot",
                    slot.terminator_name(),
                    slot.slot_name()
                ),
                anchor,
            ));
        };

        let expected_type = slot.select_type(success_type, error_type);
        if value.ty != expected_type {
            return Err(self.error_with_hir(
                format!(
                    "{} value type {:?} does not match function {} slot {:?}",
                    slot.terminator_name(),
                    value.ty,
                    slot.slot_name(),
                    expected_type
                ),
                anchor,
            ));
        }

        Ok(())
    }
}
