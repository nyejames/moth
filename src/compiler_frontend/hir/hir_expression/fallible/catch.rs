//! Catch handler CFG lowering.
//!
//! WHAT: lowers `catch:` and `catch |err|:` blocks into explicit success/error CFG branches,
//! including binding, merge behavior, and catch block fallthrough.
//! WHY: catch recovery is the most complex fallible path because it must join the success and
//! error edges back into a single continuation block with a consistent value.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::FallibleHandling;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, ValueBlockTarget};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::return_hir_transformation_error;

use super::super::LoweredExpression;
use super::{FallibleBranchingContext, FallibleCarrierBranchingContext};

struct FallibleSuccessAssignment<'a> {
    success_payload: HirExpression,
    carrier_local: LocalId,
    carrier_type: FrontendTypeId,
    ok_type: FrontendTypeId,
    result_type_ids: &'a [FrontendTypeId],
    result_locals: &'a [LocalId],
    location: &'a crate::compiler_frontend::tokenizer::tokens::SourceLocation,
}

impl<'a> HirBuilder<'a> {
    /// Lowers a handled fallible call with catch branching.
    ///
    /// WHAT: emits the call, then delegates to `lower_fallible_carrier_with_branching` to build
    /// the success/error/merge blocks.
    pub(crate) fn lower_handled_fallible_call_with_branching(
        &mut self,
        target: CallTarget,
        args: &[CallArgument],
        context: FallibleBranchingContext<'_>,
    ) -> Result<LoweredExpression, CompilerError> {
        let FallibleBranchingContext {
            result_type_ids,
            handling,
            carrier_type,
            ok_type,
            err_type,
            value_required,
            location,
            ..
        } = context;
        let validate_float_success =
            matches!(&target, CallTarget::External(_)) && self.type_id_is_float(ok_type);
        let result_local =
            self.emit_result_call_to_current_block(target, args, carrier_type, location)?;
        let current_block = self.current_block_id_or_error(location)?;

        self.lower_fallible_carrier_with_branching(FallibleCarrierBranchingContext {
            current_block,
            result_local,
            handled_result: FallibleBranchingContext {
                result_type_ids,
                handling,
                carrier_type,
                ok_type,
                err_type,
                value_required,
                location,
                validate_float_success,
            },
        })
    }

    /// Lowers the catch/recovery path for a fallible carrier.
    ///
    /// WHAT: creates success/error/merge blocks, assigns the success payload to a merge local,
    /// runs the catch handler body with the active value target, and joins both edges at
    /// the merge block.
    /// WHY: catch is the only fallible path that must resume with a value after handling the
    /// error. The merge block guarantees that later code sees a single definition regardless of
    /// which path was taken.
    pub(crate) fn lower_fallible_carrier_with_branching(
        &mut self,
        context: FallibleCarrierBranchingContext<'_>,
    ) -> Result<LoweredExpression, CompilerError> {
        let FallibleCarrierBranchingContext {
            current_block,
            result_local,
            handled_result,
        } = context;
        let FallibleBranchingContext {
            result_type_ids,
            handling,
            carrier_type,
            ok_type,
            err_type,
            value_required,
            location,
            validate_float_success,
        } = handled_result;

        let region = self.current_region_or_error(location)?;
        let result_for_test =
            self.make_local_load_expression(result_local, carrier_type, location, region);

        let success_block = self.create_block(region, location, "fallible-handled-ok")?;
        let error_region = self.create_child_region(region);
        let error_block = self.create_block(error_region, location, "fallible-handled-err")?;
        let merge_block = self.create_block(region, location, "fallible-handled-merge")?;

        self.emit_terminator(
            current_block,
            HirTerminator::FallibleBranch {
                result: result_for_test,
                success_block,
                error_block,
            },
            location,
        )?;

        let result_locals = if value_required && !self.is_unit_type(ok_type) {
            self.allocate_fallible_catch_result_locals(result_type_ids, location)?
        } else {
            vec![]
        };

        // Success edge: unwrap the payload, store it in the merge local, and jump to merge.
        self.set_current_block(success_block, location)?;
        if !result_locals.is_empty() {
            let success_region = self.current_region_or_error(location)?;
            let success_result = self.make_local_load_expression(
                result_local,
                carrier_type,
                location,
                success_region,
            );
            let success_payload = self.make_expression(
                location,
                HirExpressionKind::FallibleUnwrapSuccess {
                    result: Box::new(success_result),
                },
                ok_type,
                ValueKind::RValue,
                success_region,
            );
            let success_payload = if validate_float_success {
                self.emit_validated_float_value(success_payload, location)?
            } else {
                success_payload
            };
            self.assign_fallible_success_payload_to_result_locals(FallibleSuccessAssignment {
                success_payload,
                carrier_local: result_local,
                carrier_type,
                ok_type,
                result_type_ids,
                result_locals: &result_locals,
                location,
            })?;
        }

        self.emit_jump_to(
            success_block,
            merge_block,
            location,
            "fallible-handled.success.merge",
        )?;

        // Error edge: enter the catch handler region, bind the error local, then run
        // the handler body. Any `ThenValue` in that body assigns the shared result
        // locals and jumps to the value-block merge.
        self.set_current_block(error_block, location)?;
        let error_region = self.current_region_or_error(location)?;
        let error_result =
            self.make_local_load_expression(result_local, carrier_type, location, error_region);
        let error_payload = self.make_expression(
            location,
            HirExpressionKind::FallibleUnwrapError {
                result: Box::new(error_result),
            },
            err_type,
            ValueKind::RValue,
            error_region,
        );

        match handling {
            FallibleHandling::Handler { error, body } => {
                if let Some(error_binding) = error {
                    let handler_error_local = self.allocate_named_local(
                        error_binding.error_binding.to_owned(),
                        err_type,
                        false,
                        Some(location.to_owned()),
                    )?;

                    self.emit_assign_local_statement(handler_error_local, error_payload, location)?;
                }

                if !result_locals.is_empty() {
                    self.with_active_value_block_target(
                        ValueBlockTarget {
                            result_locals: result_locals.clone(),
                            merge_block,
                        },
                        |builder| builder.lower_statement_sequence(body),
                    )?;
                } else {
                    self.lower_statement_sequence(body)?;
                }

                let error_tail_block = self.current_block_id_or_error(location)?;
                if self.block_has_explicit_terminator(error_tail_block, location)? {
                    self.set_current_block(merge_block, location)?;
                    let value = if result_locals.is_empty() {
                        self.unit_expression(location, self.current_region_or_error(location)?)
                    } else {
                        self.value_block_result_expression(
                            &result_locals,
                            result_type_ids,
                            location,
                            region,
                        )?
                    };
                    return Ok(LoweredExpression {
                        prelude: vec![],
                        value,
                    });
                }

                if !result_locals.is_empty() {
                    return_hir_transformation_error!(
                        "Catch handler reached HIR fallthrough while a value continuation is required",
                        self.hir_error_location(location)
                    );
                }
            }
            FallibleHandling::Propagate => {
                return_hir_transformation_error!(
                    "Propagation handling unexpectedly reached fallible branching lowering",
                    self.hir_error_location(location)
                );
            }
        }

        let error_tail_block = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(error_tail_block, location)? {
            self.emit_jump_to(
                error_tail_block,
                merge_block,
                location,
                "fallible-handled.error.merge",
            )?;
        }

        self.set_current_block(merge_block, location)?;
        let merge_region = self.current_region_or_error(location)?;
        let value = if result_locals.is_empty() {
            self.unit_expression(location, merge_region)
        } else {
            self.value_block_result_expression(&result_locals, result_type_ids, location, region)?
        };

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    fn allocate_fallible_catch_result_locals(
        &mut self,
        result_type_ids: &[FrontendTypeId],
        location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
    ) -> Result<Vec<LocalId>, CompilerError> {
        let mut result_locals = Vec::with_capacity(result_type_ids.len());
        for type_id in result_type_ids {
            let lowered_ty = self.lower_type_id(*type_id, location)?;
            let local = self.allocate_temp_local(lowered_ty, Some(location.to_owned()))?;
            result_locals.push(local);
        }

        Ok(result_locals)
    }

    fn assign_fallible_success_payload_to_result_locals(
        &mut self,
        assignment: FallibleSuccessAssignment<'_>,
    ) -> Result<(), CompilerError> {
        let FallibleSuccessAssignment {
            success_payload,
            carrier_local,
            carrier_type,
            ok_type,
            result_type_ids,
            result_locals,
            location,
        } = assignment;

        if result_locals.len() == 1 {
            self.emit_assign_local_statement(result_locals[0], success_payload, location)?;
            return Ok(());
        }

        for (slot_index, slot_local) in result_locals.iter().enumerate() {
            let slot_type = self.lower_type_id(result_type_ids[slot_index], location)?;
            let region = self.current_region_or_error(location)?;
            let success_result =
                self.make_local_load_expression(carrier_local, carrier_type, location, region);
            let tuple_value = self.make_expression(
                location,
                HirExpressionKind::FallibleUnwrapSuccess {
                    result: Box::new(success_result),
                },
                ok_type,
                ValueKind::RValue,
                region,
            );
            let slot_value = self.make_expression(
                location,
                HirExpressionKind::TupleGet {
                    tuple: Box::new(tuple_value),
                    index: slot_index,
                },
                slot_type,
                ValueKind::RValue,
                region,
            );

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(*slot_local),
                    value: slot_value,
                },
                location,
            )?;
        }

        Ok(())
    }
}
