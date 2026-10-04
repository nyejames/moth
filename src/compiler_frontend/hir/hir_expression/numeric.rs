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
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::fallible::EmittedFallibleCarrier;

impl<'a> HirBuilder<'a> {
    /// Emits a checked numeric operation and returns the scalar success expression.
    ///
    /// WHAT: allocates a result local, emits `HirStatementKind::NumericOp`, and, in `ReturnError`
    ///       mode, branches on the internal fallible carrier before returning the unwrapped success
    ///       value. In `Trap` mode the result local receives the scalar success value and a local
    ///       load is returned.
    /// WHY: callers (runtime RPN lowering, loop lowering) should not duplicate the failure-mode
    ///      selection, carrier allocation, and branch-emission logic.
    pub(crate) fn emit_checked_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = if self.active_handler_accepts_builtin_failure(span)? {
            NumericFailureMode::ReturnError
        } else {
            self.select_numeric_failure_mode(span)?
        };

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_numeric_value(op, operands, success_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_numeric_value(op, operands, success_type, span)
            }
        }
    }

    /// Emits a trapping numeric operation and returns the scalar success local load.
    fn emit_trapping_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(success_type, None)?;
        self.emit_numeric_op_statement(op, NumericFailureMode::Trap, operands, result_local, span)?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        Ok(self.make_local_load_expression(result_local, success_type, &no_span, region))
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
    ) -> Result<HirExpression, CompilerError> {
        let builtin_error_type = self.builtin_error_type_id(span)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(success_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, None)?;

        self.emit_numeric_op_statement(
            op,
            NumericFailureMode::ReturnError,
            operands,
            result_local,
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
        result: LocalId,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
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
        source: HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = if self.active_handler_accepts_builtin_failure(span)? {
            NumericFailureMode::ReturnError
        } else {
            self.select_numeric_failure_mode(span)?
        };
        let string_type = self.lower_type_id(self.type_environment.builtins().string, span)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_formatted_float_value(source, string_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_formatted_float_value(source, string_type, span)
            }
        }
    }

    /// Emits a trapping `FormatFloat` and returns the scalar `String` local load.
    fn emit_trapping_formatted_float_value(
        &mut self,
        source: HirExpression,
        string_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(string_type, None)?;
        self.emit_format_float_statement(source, NumericFailureMode::Trap, result_local, span)?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        Ok(self.make_local_load_expression(result_local, string_type, &no_span, region))
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
        source: HirExpression,
        string_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
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
        source: HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = if self.active_handler_accepts_builtin_failure(span)? {
            NumericFailureMode::ReturnError
        } else {
            self.select_numeric_failure_mode(span)?
        };
        let float_type = self.lower_type_id(self.type_environment.builtins().float, span)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_validated_float_value(source, float_type, span)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_validated_float_value(source, float_type, span)
            }
        }
    }

    /// Emits a trapping `ValidateFloat` and returns the scalar `Float` local load.
    fn emit_trapping_validated_float_value(
        &mut self,
        source: HirExpression,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(float_type, None)?;
        self.emit_validate_float_statement(source, NumericFailureMode::Trap, result_local, span)?;

        let region = self.current_region_or_error(span)?;
        let no_span = None;
        Ok(self.make_local_load_expression(result_local, float_type, &no_span, region))
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
        source: HirExpression,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
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
    ) -> Result<bool, CompilerError> {
        let Some(handler) = self.active_catch_handler else {
            return Ok(false);
        };
        Ok(handler.error_type == self.builtin_error_type_id(span)?)
    }

    fn lower_numeric_carrier_to_success_value(
        &mut self,
        carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        if self.active_handler_accepts_builtin_failure(span)? {
            self.lower_carrier_to_active_catch_success(carrier, span)
        } else {
            self.lower_fallible_carrier_to_success_value(carrier, span)
        }
    }

    /// Emits the `ValidateFloat` statement itself.
    fn emit_validate_float_statement(
        &mut self,
        source: HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        self.emit_statement_to_current_block(statement, span)
    }

    /// Emits the `FormatFloat` statement itself.
    fn emit_format_float_statement(
        &mut self,
        source: HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        self.emit_statement_to_current_block(statement, span)
    }

    /// Emits a checked numeric operation and assigns its success value into `target`.
    ///
    /// WHAT: uses the same failure-mode selection as source-authored arithmetic, then stores the
    ///       success value into an existing local.
    /// WHY: compiler-generated arithmetic, such as range-loop counter updates, must preserve the
    ///      same recoverable-vs-trapping semantics as the enclosing source context instead of
    ///      silently taking a separate trap-only path.
    pub(crate) fn emit_checked_numeric_assignment(
        &mut self,
        target: LocalId,
        op: HirNumericOp,
        left: HirExpression,
        right: HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let (left, right) = self.lower_checked_numeric_binary_operands(op, left, right, span)?;
        let operands = HirNumericOperands::Binary { left, right };
        let failure_mode = self.select_numeric_failure_mode(span)?;
        let no_span = None;

        if matches!(failure_mode, NumericFailureMode::Trap) {
            return self.emit_numeric_op_statement(op, failure_mode, operands, target, &no_span);
        }

        let success_type = self.checked_numeric_result_type(op, &no_span)?;
        let success_value =
            self.emit_recoverable_numeric_value(op, operands, success_type, &no_span)?;
        self.emit_assign_local_statement(target, success_value, span)
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
    ) -> Result<TypeId, CompilerError> {
        let domain_type = op.domain.type_id(&self.type_environment);
        self.lower_type_id(domain_type, span)
    }

    /// Selects the numeric failure mode for the current function context.
    ///
    /// WHAT: returns `ReturnError` when the signature's fallible carrier has builtin `Error`.
    ///       Other signatures initially use `Trap`.
    /// WHY: the private failure lane later installs inferred delivery for private no-slot
    ///      functions. Custom error slots require frontend-validated local recovery or explicit
    ///      mapping, never automatic conversion.
    pub(crate) fn select_numeric_failure_mode(
        &mut self,
        span: &Option<SourceSpan>,
    ) -> Result<NumericFailureMode, CompilerError> {
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
        left: &HirExpression,
        right: &HirExpression,
    ) -> Option<(HirNumericOp, TypeId)> {
        let operator = op.numeric_operator()?;
        let left = NumericScalar::from_type_id(left.ty, &self.type_environment)?;
        let right = NumericScalar::from_type_id(right.ty, &self.type_environment)?;
        let domain = binary_operation_domain(operator, left, right)?;

        Some((
            HirNumericOp { operator, domain },
            domain.type_id(&self.type_environment),
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
        left: HirExpression,
        right: HirExpression,
        span: &Option<SourceSpan>,
    ) -> Result<(HirExpression, HirExpression), CompilerError> {
        let left = self.convert_numeric_operand_to_domain(left, op.domain, span)?;

        if op.operator == NumericOperator::Power && matches!(op.domain, NumericScalar::Number(_)) {
            let int_type = self.type_environment.builtins().int;
            if right.ty != int_type {
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
        operand: &HirExpression,
    ) -> Option<(HirNumericOp, TypeId)> {
        let operand = NumericScalar::from_type_id(operand.ty, &self.type_environment)?;
        let domain = negation_domain(operand)?;

        Some((
            HirNumericOp {
                operator: NumericOperator::Negate,
                domain,
            },
            domain.type_id(&self.type_environment),
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
        value: HirExpression,
        domain: NumericScalar,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let target_type = domain.type_id(&self.type_environment);
        if value.ty == target_type {
            return Ok(value);
        }

        let Some(source) = NumericScalar::from_type_id(value.ty, &self.type_environment) else {
            return_hir_transformation_error!(
                "Numeric operation operand has no numeric conversion domain",
                self.hir_error_location(span)
            );
        };

        let region = value.region;
        let no_span = None;
        Ok(self.make_expression(
            &no_span,
            HirExpressionKind::Cast {
                source: Box::new(value),
                policy: BuiltinCastPolicyId::NumericConversion {
                    source,
                    target: domain,
                },
            },
            target_type,
            ValueKind::RValue,
            region,
        ))
    }
}
