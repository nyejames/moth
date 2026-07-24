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
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = self.select_numeric_failure_mode(location)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_numeric_value(op, operands, success_type, location)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_numeric_value(op, operands, success_type, location)
            }
        }
    }

    /// Emits a trapping numeric operation and returns the scalar success local load.
    fn emit_trapping_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(success_type, Some(location.to_owned()))?;
        self.emit_numeric_op_statement(
            op,
            NumericFailureMode::Trap,
            operands,
            result_local,
            location,
        )?;

        let region = self.current_region_or_error(location)?;
        Ok(self.make_local_load_expression(result_local, success_type, location, region))
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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let builtin_error_type = self.builtin_error_type_id(location)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(success_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;

        self.emit_numeric_op_statement(
            op,
            NumericFailureMode::ReturnError,
            operands,
            result_local,
            location,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: success_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_fallible_carrier_to_success_value(carrier, location)
    }

    /// Emits the `NumericOp` statement itself.
    pub(crate) fn emit_numeric_op_statement(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: HirNumericOperands,
        result: LocalId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &statement);
        self.emit_statement_to_current_block(statement, location)
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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = self.select_numeric_failure_mode(location)?;
        let string_type = self.lower_type_id(self.type_environment.builtins().string, location)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_formatted_float_value(source, string_type, location)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_formatted_float_value(source, string_type, location)
            }
        }
    }

    /// Emits a trapping `FormatFloat` and returns the scalar `String` local load.
    fn emit_trapping_formatted_float_value(
        &mut self,
        source: HirExpression,
        string_type: TypeId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(string_type, Some(location.to_owned()))?;
        self.emit_format_float_statement(source, NumericFailureMode::Trap, result_local, location)?;

        let region = self.current_region_or_error(location)?;
        Ok(self.make_local_load_expression(result_local, string_type, location, region))
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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let builtin_error_type = self.builtin_error_type_id(location)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(string_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;

        self.emit_format_float_statement(
            source,
            NumericFailureMode::ReturnError,
            result_local,
            location,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: string_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_fallible_carrier_to_success_value(carrier, location)
    }

    /// Validates a `Float` value from an external/backend boundary before exposing it as an
    /// ordinary Moth `Float`.
    ///
    /// WHAT: allocates a result local, emits `HirStatementKind::ValidateFloat`, and, in
    ///       `ReturnError` mode, branches on the internal fallible carrier before returning the
    ///       unwrapped finite `Float`. In `Trap` mode the result local receives the scalar `Float`
    ///       success value and a local load is returned.
    /// WHY: external functions and backend boundaries may return non-finite `f64` values; Moth
    ///      `Float` is finite `f64`, so every entering Float must be checked explicitly.
    pub(crate) fn emit_validated_float_value(
        &mut self,
        source: HirExpression,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let failure_mode = self.select_numeric_failure_mode(location)?;
        let float_type = self.lower_type_id(self.type_environment.builtins().float, location)?;

        match failure_mode {
            NumericFailureMode::Trap => {
                self.emit_trapping_validated_float_value(source, float_type, location)
            }
            NumericFailureMode::ReturnError => {
                self.emit_recoverable_validated_float_value(source, float_type, location)
            }
        }
    }

    /// Emits a trapping `ValidateFloat` and returns the scalar `Float` local load.
    fn emit_trapping_validated_float_value(
        &mut self,
        source: HirExpression,
        float_type: TypeId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let result_local = self.allocate_temp_local(float_type, Some(location.to_owned()))?;
        self.emit_validate_float_statement(
            source,
            NumericFailureMode::Trap,
            result_local,
            location,
        )?;

        let region = self.current_region_or_error(location)?;
        Ok(self.make_local_load_expression(result_local, float_type, location, region))
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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let builtin_error_type = self.builtin_error_type_id(location)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(float_type, builtin_error_type);
        let result_local = self.allocate_temp_local(carrier_type, Some(location.to_owned()))?;

        self.emit_validate_float_statement(
            source,
            NumericFailureMode::ReturnError,
            result_local,
            location,
        )?;

        let carrier = EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type: float_type,
            err_type: builtin_error_type,
            validate_float_success: false,
        };
        self.lower_fallible_carrier_to_success_value(carrier, location)
    }

    /// Emits the `ValidateFloat` statement itself.
    fn emit_validate_float_statement(
        &mut self,
        source: HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &statement);
        self.emit_statement_to_current_block(statement, location)
    }

    /// Emits the `FormatFloat` statement itself.
    fn emit_format_float_statement(
        &mut self,
        source: HirExpression,
        failure_mode: NumericFailureMode,
        result: LocalId,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &statement);
        self.emit_statement_to_current_block(statement, location)
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
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let (left, right) =
            self.lower_checked_numeric_binary_operands(op, left, right, location)?;
        let operands = HirNumericOperands::Binary { left, right };
        let failure_mode = self.select_numeric_failure_mode(location)?;

        if matches!(failure_mode, NumericFailureMode::Trap) {
            return self.emit_numeric_op_statement(op, failure_mode, operands, target, location);
        }

        let success_type = self.checked_numeric_result_type(op, location)?;
        let success_value =
            self.emit_recoverable_numeric_value(op, operands, success_type, location)?;
        self.emit_assign_local_statement(target, success_value, location)
    }

    /// Returns the scalar success type for a checked numeric operation.
    pub(crate) fn checked_numeric_result_type(
        &mut self,
        op: HirNumericOp,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        let int_type = self.lower_type_id(self.type_environment.builtins().int, location)?;
        let float_type = self.lower_type_id(self.type_environment.builtins().float, location)?;

        Ok(match op {
            HirNumericOp::IntAdd
            | HirNumericOp::IntSub
            | HirNumericOp::IntMul
            | HirNumericOp::IntDiv
            | HirNumericOp::IntMod
            | HirNumericOp::IntPow
            | HirNumericOp::IntNeg => int_type,

            HirNumericOp::FloatAdd
            | HirNumericOp::FloatSub
            | HirNumericOp::FloatMul
            | HirNumericOp::FloatDiv
            | HirNumericOp::FloatMod
            | HirNumericOp::FloatPow
            | HirNumericOp::FloatNeg => float_type,
        })
    }

    /// Selects the numeric failure mode for the current function context.
    ///
    /// WHAT: returns `ReturnError` only when the enclosing function has an internal fallible carrier
    ///       whose error slot is exactly builtin `Error`. Top-level `start()`, non-fallible functions,
    ///       and custom error channels all use `Trap`.
    /// WHY: only builtin `Error!` can represent numeric failures as user-recoverable values; other
    ///      contexts have no channel for the failure.
    fn select_numeric_failure_mode(
        &mut self,
        location: &SourceLocation,
    ) -> Result<NumericFailureMode, CompilerError> {
        let current_function_id = self.current_function_id_or_error(location)?;

        // Entry `start()` is implicitly non-fallible regardless of its carrier shape.
        if Some(current_function_id) == Some(self.module.start_function) {
            return Ok(NumericFailureMode::Trap);
        }

        let function = self.function_by_id_or_error(current_function_id, location)?;
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
        )
    }

    /// Classifies a runtime binary operator and its operand types as a checked numeric operation.
    ///
    /// WHAT: returns the `HirNumericOp` and the scalar result type when the operator is numeric
    ///       arithmetic. String concatenation and non-numeric operators return `None` so callers can
    ///       fall back to plain `BinOp`.
    /// WHY: keeps the tree-lowering branch focused on control flow while numeric policy lives here.
    pub(crate) fn classify_checked_numeric_binop(
        &mut self,
        op: &Operator,
        left: &HirExpression,
        right: &HirExpression,
    ) -> Option<(HirNumericOp, TypeId)> {
        let int_type = self.type_environment.builtins().int;
        let float_type = self.type_environment.builtins().float;
        let string_type = self.type_environment.builtins().string;

        let left_is_int = left.ty == int_type;
        let left_is_float = left.ty == float_type;
        let right_is_int = right.ty == int_type;
        let right_is_float = right.ty == float_type;
        let any_operand_is_string = left.ty == string_type || right.ty == string_type;
        let operands_are_numeric =
            (left_is_int || left_is_float) && (right_is_int || right_is_float);

        if !operands_are_numeric {
            return None;
        }

        match op {
            // String concatenation stays as plain HirBinOp::Add.
            Operator::Add if any_operand_is_string => None,

            Operator::Add if left_is_int && right_is_int => Some((HirNumericOp::IntAdd, int_type)),
            Operator::Subtract if left_is_int && right_is_int => {
                Some((HirNumericOp::IntSub, int_type))
            }
            Operator::Multiply if left_is_int && right_is_int => {
                Some((HirNumericOp::IntMul, int_type))
            }
            Operator::IntDivide if left_is_int && right_is_int => {
                Some((HirNumericOp::IntDiv, int_type))
            }
            Operator::Modulus if left_is_int && right_is_int => {
                Some((HirNumericOp::IntMod, int_type))
            }
            Operator::Exponent if left_is_int && right_is_int => {
                Some((HirNumericOp::IntPow, int_type))
            }

            // Real division always lowers to FloatDiv; operands are converted below.
            Operator::Divide => Some((HirNumericOp::FloatDiv, float_type)),

            // Mixed or pure Float arithmetic for the remaining binary operators.
            Operator::Add => Some((HirNumericOp::FloatAdd, float_type)),
            Operator::Subtract => Some((HirNumericOp::FloatSub, float_type)),
            Operator::Multiply => Some((HirNumericOp::FloatMul, float_type)),
            Operator::Modulus => Some((HirNumericOp::FloatMod, float_type)),
            Operator::Exponent => Some((HirNumericOp::FloatPow, float_type)),

            _ => None,
        }
    }

    /// Converts `Int` operands to `Float` for mixed arithmetic and real division.
    ///
    /// WHAT: given a classified float-family `HirNumericOp`, any `Int` operand is wrapped in an
    ///       infallible `Int -> Float` cast. Pure `Float` operands pass through unchanged.
    /// WHY: the backend expects uniform `Float*` checked operations and `Int / Int` is real
    ///      division.
    pub(crate) fn lower_checked_numeric_binary_operands(
        &mut self,
        op: HirNumericOp,
        left: HirExpression,
        right: HirExpression,
        location: &SourceLocation,
    ) -> Result<(HirExpression, HirExpression), CompilerError> {
        let float_type = self.type_environment.builtins().float;

        let needs_float = matches!(
            op,
            HirNumericOp::FloatAdd
                | HirNumericOp::FloatSub
                | HirNumericOp::FloatMul
                | HirNumericOp::FloatDiv
                | HirNumericOp::FloatMod
                | HirNumericOp::FloatPow
        );

        if !needs_float {
            return Ok((left, right));
        }

        let left = self.convert_int_to_float_if_needed(left, float_type, location)?;
        let right = self.convert_int_to_float_if_needed(right, float_type, location)?;
        Ok((left, right))
    }

    /// Classifies unary numeric negation as a checked numeric operation.
    ///
    /// WHAT: returns the `HirNumericOp` and result type when the operand is `Int` or `Float`.
    ///       Non-numeric negation returns `None` so callers fall back to plain `UnaryOp`.
    pub(crate) fn classify_checked_numeric_negation(
        &self,
        operand: &HirExpression,
    ) -> Option<(HirNumericOp, TypeId)> {
        let int_type = self.type_environment.builtins().int;
        let float_type = self.type_environment.builtins().float;

        if operand.ty == int_type {
            Some((HirNumericOp::IntNeg, int_type))
        } else if operand.ty == float_type {
            Some((HirNumericOp::FloatNeg, float_type))
        } else {
            None
        }
    }

    /// Wraps an `Int` operand in an infallible `Int -> Float` cast when needed.
    ///
    /// WHAT: mixed `Int`/`Float` arithmetic and `Int / Int` real division convert `Int` operands to
    ///       `Float` explicitly so the backend sees a uniform `Float*` checked operation.
    /// WHY: HIR already owns the `IntToFloat` cast policy and JS lowering treats it as identity,
    ///      so reusing it avoids inventing a new conversion expression shape.
    fn convert_int_to_float_if_needed(
        &mut self,
        value: HirExpression,
        float_type: TypeId,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let int_type = self.type_environment.builtins().int;
        if value.ty != int_type {
            return Ok(value);
        }

        let region = value.region;
        Ok(self.make_expression(
            location,
            HirExpressionKind::Cast {
                source: Box::new(value),
                policy: BuiltinCastPolicyId::IntToFloat,
            },
            float_type,
            ValueKind::RValue,
            region,
        ))
    }
}
