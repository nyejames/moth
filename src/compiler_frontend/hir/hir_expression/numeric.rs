//! Checked numeric lowering helpers.
//!
//! WHAT: converts runtime numeric AST operators into HIR `NumericOp` statements with the correct
//!       failure mode and returns the scalar success value.
//! WHY: numeric failures are semantic HIR effects; this module centralizes the decision of which
//!      `HirNumericOp` to emit, how to convert mixed operands, and whether failures trap or return
//!      a builtin `Error!` carrier.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::builtins::casts::evidence::type_id_for_builtin_target;
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, negation_domain, numeric_operation_cannot_fail,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::fallible::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Emits a checked numeric operation and returns the scalar success expression.
    ///
    /// WHAT: allocates a result local, emits `HirStatementKind::NumericOp`, and, in `ReturnError`
    ///       mode, branches on the internal fallible carrier before returning the unwrapped success
    ///       value. In `Trap` mode the result local receives the scalar success value and a local
    ///       load is returned. Semantically discharged (`Infallible`) operations lower exactly
    ///       like `Trap` with a dead check and no failure edge.
    /// WHY: callers (runtime RPN lowering, loop lowering) should not duplicate the failure-mode
    ///      selection, carrier allocation, and branch-emission logic.
    pub(crate) fn emit_checked_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
        discharged: bool,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let failure_mode = if discharged {
            NumericFailureMode::Infallible
        } else if self.active_handler_accepts_builtin_failure(span)? {
            NumericFailureMode::ReturnError
        } else {
            self.select_numeric_failure_mode(span)?
        };

        match failure_mode {
            NumericFailureMode::Trap | NumericFailureMode::Infallible => {
                self.emit_scalar_numeric_value(op, failure_mode, operands, success_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_numeric_value(op, operands, success_type, span)
            }
        }
    }

    /// Emits a scalar-result numeric operation and returns the success local load.
    ///
    /// WHAT: emits `HirStatementKind::NumericOp` with `Trap` or the semantically discharged
    ///       `Infallible` mode and returns a load of the scalar result local. Discharged
    ///       operations keep the trapping operation and evaluation; only the failure edge is
    ///       gone, so backends share one lowering with a dead check.
    /// WHY: `Trap` and `Infallible` differ only in the recorded mode, so one helper owns both
    ///      instead of two identical emitters.
    fn emit_scalar_numeric_value(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        debug_assert!(matches!(
            failure_mode,
            NumericFailureMode::Trap | NumericFailureMode::Infallible
        ));
        let result_local = self.allocate_temp_local(success_type, None)?;
        self.emit_numeric_op_statement(
            op,
            failure_mode,
            operands,
            HirLocalDestination::Define(result_local),
            span,
        )?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        self.make_local_load_expression(result_local, success_type, &no_span, region)
    }

    /// Shares arithmetic's failure owner and carrier shape so private-lane installation can
    /// retarget the producer without inspecting guard conditions or rendered messages.
    pub(crate) fn emit_range_step_failure(
        &mut self,
        cause: RangeStepFailureCause,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let failure_mode = if self.active_handler_accepts_builtin_failure(span)? {
            NumericFailureMode::ReturnError
        } else {
            self.select_numeric_failure_mode(span)?
        };
        let success_type = self.type_environment.builtins().bool;
        let error_type = if failure_mode == NumericFailureMode::ReturnError {
            Some(self.builtin_error_type_id(span)?)
        } else {
            None
        };
        let result_type = match error_type {
            Some(error_type) => self
                .type_environment
                .intern_fallible_carrier(success_type, error_type),
            None => success_type,
        };
        let result_local = self.allocate_temp_local(result_type, None)?;
        self.emit_statement_kind_with_span(
            HirStatementKind::RangeStepFailure {
                cause,
                failure_mode,
                result: HirLocalDestination::Define(result_local),
            },
            span,
            *span,
        )?;
        let block = self.current_block_id_or_error(span)?;
        if let Some(error_type) = error_type {
            let region = self.current_region_or_error(span)?;
            let error_result =
                self.make_local_load_expression(result_local, result_type, &None, region)?;
            let error_payload = self.make_expression(
                span,
                HirExpressionKind::FallibleUnwrapError {
                    result: error_result,
                },
                error_type,
                ValueKind::RValue,
                region,
            );
            self.emit_terminator(block, HirTerminator::ReturnError(error_payload?), span)?;
        } else {
            self.emit_terminator(
                block,
                HirTerminator::RuntimeFailure {
                    message: cause.builtin_error_code().default_message().to_owned(),
                    cause: None,
                },
                span,
            )?;
        }
        Ok(())
    }

    /// Emits a recoverable numeric operation and returns the unwrapped success value.
    ///
    /// WHAT: stores the internal fallible carrier in a temp local, emits `FallibleBranch` and a
    ///       `ReturnError` edge, then continues on the success block and returns
    ///       `FallibleUnwrapSuccess`.
    /// WHY: this mirrors the existing fallible-carrier helpers for calls and casts, keeping the
    ///      error path visible to borrow validation.
    fn emit_recoverable_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let builtin_error_type = self.builtin_error_type_id(span)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(success_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, None)?;

        self.emit_numeric_op_statement(
            op,
            NumericFailureMode::ReturnError,
            operands,
            HirLocalDestination::Define(result_local),
            span,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: success_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_numeric_carrier_to_success_value(carrier, span)
    }

    /// Emits the `NumericOp` statement itself.
    pub(crate) fn emit_numeric_op_statement(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: HirNumericOperands,
        result: HirLocalDestination,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        self.emit_statement_to_current_block(statement, span)
    }

    /// Formats a `Float` expression into a `String` using Moth's formatting contract.
    ///
    /// WHAT: allocates a result local, emits `HirStatementKind::FormatFloat`, and, in
    ///       `ReturnError` mode, branches on the internal fallible carrier before returning the
    ///       unwrapped formatted string. In `Trap` mode the result local receives the scalar `String`
    ///       success value and a local load is returned.
    /// WHY: `Float -> String` casts and runtime Float template interpolation must share one
    ///      Moth-owned formatter instead of relying on target-native stringification.
    pub(crate) fn emit_formatted_float_value(
        &mut self,
        source: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let failure_mode = self.select_float_integrity_failure_mode(span)?;
        let string_type = self.lower_type_id(self.type_environment.builtins().string, span)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_formatted_float_value(source, string_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_formatted_float_value(source, string_type, span)
            }
            NumericFailureMode::Infallible => {
                return_hir_transformation_error!(
                    "Float formatting has no semantically discharged mode",
                    self.hir_error_location(span)
                );
            }
        }
    }

    /// Emits a trapping `FormatFloat` and returns the scalar `String` local load.
    fn emit_trapping_formatted_float_value(
        &mut self,
        source: HirValueId,
        string_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let result_local = self.allocate_temp_local(string_type, None)?;
        self.emit_format_float_statement(source, NumericFailureMode::Trap, result_local, span)?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        self.make_local_load_expression(result_local, string_type, &no_span, region)
    }

    /// Emits a recoverable `FormatFloat` and returns the unwrapped formatted string.
    ///
    /// WHAT: stores the internal fallible carrier in a temp local, emits `FallibleBranch` and a
    ///       `ReturnError` edge, then continues on the success block and returns
    ///       `FallibleUnwrapSuccess`.
    /// WHY: this mirrors the existing fallible-carrier helpers for calls and casts, keeping the
    ///      error path visible to borrow validation.
    fn emit_recoverable_formatted_float_value(
        &mut self,
        source: HirValueId,
        string_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let builtin_error_type = self.builtin_error_type_id(span)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(string_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, None)?;

        self.emit_format_float_statement(
            source,
            NumericFailureMode::ReturnError,
            result_local,
            span,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: string_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_numeric_carrier_to_success_value(carrier, span)
    }

    /// Validates a `Float` value from an external/backend boundary before exposing it as an
    /// ordinary Moth `Float`.
    ///
    /// WHAT: allocates a result local, emits `HirStatementKind::ValidateFloat`, and, in
    ///       `ReturnError` mode, branches on the internal fallible carrier before returning the
    ///       unwrapped finite `Float`. In `Trap` mode the result local receives the scalar `Float`
    ///       success value and a local load is returned.
    /// WHY: incoming `Float` values must be rounded at the selected profile precision and
    ///      validated as finite before ordinary Moth code observes them.
    pub(crate) fn emit_validated_float_value(
        &mut self,
        source: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let failure_mode = self.select_float_integrity_failure_mode(span)?;
        let float_type = self.lower_type_id(self.type_environment.builtins().float, span)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_validated_float_value(source, float_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_validated_float_value(source, float_type, span)
            }
            NumericFailureMode::Infallible => {
                return_hir_transformation_error!(
                    "Float validation has no semantically discharged mode",
                    self.hir_error_location(span)
                );
            }
        }
    }

    /// Emits a trapping `ValidateFloat` and returns the scalar `Float` local load.
    fn emit_trapping_validated_float_value(
        &mut self,
        source: HirValueId,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let result_local = self.allocate_temp_local(float_type, None)?;
        self.emit_validate_float_statement(source, NumericFailureMode::Trap, result_local, span)?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        self.make_local_load_expression(result_local, float_type, &no_span, region)
    }

    /// Emits a recoverable `ValidateFloat` and returns the unwrapped finite `Float`.
    ///
    /// WHAT: stores the internal fallible carrier in a temp local, emits `FallibleBranch` and a
    ///       `ReturnError` edge, then continues on the success block and returns
    ///       `FallibleUnwrapSuccess`.
    /// WHY: this mirrors the existing fallible-carrier helpers for calls and casts, keeping the
    ///      error path visible to borrow validation.
    fn emit_recoverable_validated_float_value(
        &mut self,
        source: HirValueId,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let builtin_error_type = self.builtin_error_type_id(span)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(float_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, None)?;

        self.emit_validate_float_statement(
            source,
            NumericFailureMode::ReturnError,
            result_local,
            span,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: float_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_numeric_carrier_to_success_value(carrier, span)
    }

    /// A catch continuation accepts builtin numeric failure only. Custom handlers keep
    /// injected float boundary checks on the function-boundary continuation.
    fn active_handler_accepts_builtin_failure(
        &mut self,
        span: &Option<SourceSpan>,
    ) -> Result<bool, HirConstructionFailure> {
        let Some(handler) = self.active_catch_handler else {
            return Ok(false);
        };
        Ok(handler.error_type == self.builtin_error_type_id(span)?)
    }

    fn lower_numeric_carrier_to_success_value(
        &mut self,
        carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.active_handler_accepts_builtin_failure(span)? {
            self.lower_carrier_to_active_catch_success(carrier, span)
        } else {
            self.lower_fallible_carrier_to_success_value(carrier, span)
        }
    }

    /// Emits the `ValidateFloat` statement itself.
    fn emit_validate_float_statement(
        &mut self,
        source: HirValueId,
        failure_mode: NumericFailureMode,
        result: LocalId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result: HirLocalDestination::Define(result),
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        self.emit_statement_to_current_block(statement, span)
    }

    /// Emits the `FormatFloat` statement itself.
    fn emit_format_float_statement(
        &mut self,
        source: HirValueId,
        failure_mode: NumericFailureMode,
        result: LocalId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result: HirLocalDestination::Define(result),
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        self.emit_statement_to_current_block(statement, span)
    }

    /// Emits a checked numeric operation and writes its success value to `target`.
    ///
    /// WHAT: uses the same failure-mode selection as source-authored arithmetic, then stores the
    ///       success value to an explicitly classified local destination.
    /// WHY: compiler-generated arithmetic must preserve both its recoverable-vs-trapping semantics
    ///      and whether the caller defines scratch storage or updates persistent loop state.
    pub(crate) fn emit_checked_numeric_assignment(
        &mut self,
        target: HirLocalDestination,
        op: HirNumericOp,
        left: HirValueId,
        right: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let discharged =
            self.pre_conversion_operands_cannot_fail(op.operator, left, Some(right), op.domain);
        let (left, right) = self.lower_checked_numeric_binary_operands(op, left, right, span)?;
        let operands = HirNumericOperands::Binary { left, right };
        if discharged {
            // Generated loop arithmetic over discharged domains keeps its span but needs no
            // failure edge, exactly like source-authored proven-safe operations.
            return self.emit_numeric_op_statement(
                op,
                NumericFailureMode::Infallible,
                operands,
                target,
                span,
            );
        }
        let failure_mode = self.select_numeric_failure_mode(span)?;

        // Generated updates still belong to the authored loop/assignment. Preserve that
        // producer span so a target capability rejection never points at anonymous scaffolding.
        if matches!(failure_mode, NumericFailureMode::Trap) {
            return self.emit_numeric_op_statement(op, failure_mode, operands, target, span);
        }

        let success_type = self.checked_numeric_result_type(op, span)?;
        let success_value =
            self.emit_recoverable_numeric_value(op, operands, success_type, span)?;
        let write_target = match target {
            HirLocalDestination::Define(local) => HirWriteTarget::DefineLocal(local),
            HirLocalDestination::Update(local) => {
                HirWriteTarget::AssignPlace(HirPlace::local(local))
            }
        };
        self.emit_write_statement(write_target, success_value, span)
    }

    /// Returns the scalar success type for a checked numeric operation.
    ///
    /// WHAT: derives the result type from the operation domain instead of matching every
    ///       operator/variant pair.
    /// WHY: the domain already owns the canonical type for every numeric scalar.
    fn checked_numeric_result_type(
        &mut self,
        op: HirNumericOp,
        span: &Option<SourceSpan>,
    ) -> Result<TypeId, HirConstructionFailure> {
        let domain_type = op.domain.type_id(&self.type_environment);
        self.lower_type_id(domain_type, span)
    }

    /// Formatting and incoming-value guards are integrity checks, not implicit numeric failure.
    /// Only a source-declared builtin Error! contract makes these guards recoverable. The exact
    /// start identity is retained from the compiler-generated AST entry, not inferred from a name.
    fn select_float_integrity_failure_mode(
        &mut self,
        span: &Option<SourceSpan>,
    ) -> Result<NumericFailureMode, HirConstructionFailure> {
        let function_id = self.current_function_id_or_error(span)?;
        if self.module.start_function == Some(function_id) {
            return Ok(NumericFailureMode::Trap);
        }

        self.select_numeric_failure_mode(span)
    }

    /// Selects the numeric failure mode for the current function context.
    ///
    /// WHAT: returns `ReturnError` when the signature's fallible carrier has builtin `Error`.
    ///       Callers check for an enclosing builtin-accepting catch first and bypass this
    ///       selector; every other context initially uses `Trap`: non-fallible signatures,
    ///       custom error slots, and private no-slot helpers (until the lane installer
    ///       retargets their code-carrying producers). The synthetic entry `start()` selects
    ///       `ReturnError` here through its builtin slot; only its float integrity guards
    ///       take a separate trap-only path.
    /// WHY: the private failure lane later installs inferred delivery for private no-slot
    ///      functions. Custom error slots require frontend-validated local recovery or explicit
    ///      mapping, never automatic conversion. Code-less operations (`Float` negation,
    ///      exact `Dec` arithmetic) keep their `Trap` permanently because they never become
    ///      lane producers.
    pub(crate) fn select_numeric_failure_mode(
        &mut self,
        span: &Option<SourceSpan>,
    ) -> Result<NumericFailureMode, HirConstructionFailure> {
        let current_function_id = self.current_function_id_or_error(span)?;
        let function = self.function_by_id_or_error(current_function_id, span)?;
        let Some((_, error_type)) = self
            .type_environment
            .fallible_carrier_slots(function.return_type)
        else {
            return Ok(NumericFailureMode::Trap);
        };

        let Some(builtin_error_type) = self.maybe_builtin_error_type_id() else {
            return Ok(NumericFailureMode::Trap);
        };

        if error_type == builtin_error_type {
            Ok(NumericFailureMode::ReturnError)
        } else {
            Ok(NumericFailureMode::Trap)
        }
    }

    /// Looks up builtin `Error` when it is registered in the current test/module environment.
    ///
    /// WHAT: returns `None` instead of a lowering error when the type is absent.
    /// WHY: selecting trap mode for custom-error or synthetic test environments must not require
    ///      builtin `Error`; only recoverable numeric emission needs the type and validates it
    ///      through `builtin_error_type_id`.
    fn maybe_builtin_error_type_id(&mut self) -> Option<TypeId> {
        type_id_for_builtin_target(
            BuiltinCastTarget::Error,
            &self.type_environment,
            self.string_table,
            self.path_fork,
        )
    }

    /// Classifies a runtime binary operator and its operand types as a checked numeric operation.
    ///
    /// WHAT: derives the shared numeric computation domain from the source operator and operand
    ///       types. Non-numeric operators or unsupported operand pairs return `None`.
    /// WHY: HIR consumes the same promotion policy as AST typing and constant folding, so every
    ///      stage selects one shared numeric operation domain.
    pub(crate) fn classify_checked_numeric_binop(
        &mut self,
        op: &Operator,
        left: HirValueId,
        right: HirValueId,
    ) -> Option<(HirNumericOp, TypeId, bool)> {
        let operator = op.numeric_operator()?;
        let left_type = self.module.expressions.expression(left).ty;
        let right_type = self.module.expressions.expression(right).ty;
        let domain = binary_operation_domain(
            operator,
            NumericScalar::from_type_id(left_type, &self.type_environment)?,
            NumericScalar::from_type_id(right_type, &self.type_environment)?,
        )?;
        let discharged =
            self.pre_conversion_operands_cannot_fail(operator, left, Some(right), domain);

        Some((
            HirNumericOp { operator, domain },
            domain.type_id(&self.type_environment),
            discharged,
        ))
    }

    /// Converts binary operands to the selected checked-operation domain.
    ///
    /// WHAT: makes operation-domain promotions explicit, except that Dec power keeps its Int
    ///       exponent in the canonical profile type.
    /// WHY: backends consume typed operations without reconstructing source promotions, while the
    ///      exponent is semantically distinct from the Dec base and must never be rescaled.
    pub(crate) fn lower_checked_numeric_binary_operands(
        &mut self,
        op: HirNumericOp,
        left: HirValueId,
        right: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<(HirValueId, HirValueId), HirConstructionFailure> {
        let left = self.convert_numeric_operand_to_domain(left, op.domain, span)?;

        if op.operator == NumericOperator::Power && matches!(op.domain, NumericScalar::Number(_)) {
            let int_type = self.type_environment.builtins().int;
            if self.module.expressions.expression(right).ty != int_type {
                return_hir_transformation_error!(
                    "Dec power exponent must remain the canonical Int type",
                    self.hir_error_location(span)
                );
            }

            return Ok((left, right));
        }

        let right = self.convert_numeric_operand_to_domain(right, op.domain, span)?;
        Ok((left, right))
    }

    /// Classifies unary numeric negation through the shared domain policy.
    ///
    /// WHAT: returns the checked operation and promoted result type for a numeric operand; callers
    ///       treat unsupported negation as an internal lowering invariant failure.
    /// WHY: signed narrow integers and F16 promote before negation, while unsigned scalars remain
    ///      unsupported.
    pub(crate) fn classify_checked_numeric_negation(
        &self,
        operand: HirValueId,
    ) -> Option<(HirNumericOp, TypeId, bool)> {
        let operand_type = self.module.expressions.expression(operand).ty;
        let operand_scalar = NumericScalar::from_type_id(operand_type, &self.type_environment)?;
        let domain = negation_domain(operand_scalar)?;
        let discharged = self.pre_conversion_operands_cannot_fail(
            NumericOperator::Negate,
            operand,
            None,
            domain,
        );

        Some((
            HirNumericOp {
                operator: NumericOperator::Negate,
                domain,
            },
            domain.type_id(&self.type_environment),
            discharged,
        ))
    }

    /// Converts an operand to a checked numeric operation's domain when required.
    ///
    /// WHAT: emits the standard `NumericConversion` expression for a source scalar whose
    ///       canonical type differs from `domain`, leaving already-matching operands unchanged.
    /// WHY: shared operator policy determines promotions; HIR makes each required conversion
    ///      explicit. Dec power keeps its exponent in the canonical profile Int type.
    pub(crate) fn convert_numeric_operand_to_domain(
        &mut self,
        value: HirValueId,
        domain: NumericScalar,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let target_type = domain.type_id(&self.type_environment);
        let value_row = self.module.expressions.expression(value);
        if value_row.ty == target_type {
            return Ok(value);
        }

        let Some(source) = NumericScalar::from_type_id(value_row.ty, &self.type_environment) else {
            return_hir_transformation_error!(
                "Numeric operation operand has no numeric conversion domain",
                self.hir_error_location(span)
            );
        };

        let region = value_row.region;
        let no_span = None;
        self.make_expression(
            &no_span,
            HirExpressionKind::Cast {
                source: value,
                policy: BuiltinCastPolicyId::NumericConversion {
                    source,
                    target: domain,
                },
            },
            target_type,
            ValueKind::RValue,
            region,
        )
    }

    /// Whether pre-conversion HIR operand types discharge a checked operation.
    ///
    /// WHAT: reads the same `NumericScalar` domains from the same pre-conversion operand types
    ///       as the AST consumer and applies the shared predicate with the same inputs.
    /// WHY: HIR must emit `Infallible` with exactly the AST's answer; `classify_*` and
    ///      `emit_checked_numeric_assignment` share this one derivation instead of each
    ///      re-deriving the predicate inputs.
    fn pre_conversion_operands_cannot_fail(
        &self,
        operator: NumericOperator,
        left: HirValueId,
        right: Option<HirValueId>,
        domain: NumericScalar,
    ) -> bool {
        let environment: &TypeEnvironment = &self.type_environment;
        let left_type = self.module.expressions.expression(left).ty;
        let Some(left) = NumericScalar::from_type_id(left_type, environment) else {
            return false;
        };
        let right = match right {
            Some(operand) => match NumericScalar::from_type_id(
                self.module.expressions.expression(operand).ty,
                environment,
            ) {
                Some(scalar) => Some(scalar),
                None => return false,
            },
            None if operator.is_unary() => None,
            None => return false,
        };
        numeric_operation_cannot_fail(operator, left, right, domain)
    }
}
