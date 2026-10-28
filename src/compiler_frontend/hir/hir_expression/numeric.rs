//! Checked numeric lowering helpers.
//!
//! Converts numeric operations into scalar statements or native success/error continuations.
//! Completed source operations arrive with resolved facts. Compiler-generated arithmetic selects
//! its explicit operand conversions here; recoverable delivery remains a lowering decision.

use crate::compiler_frontend::builtins::casts::evidence::type_id_for_builtin_target;
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, numeric_operation_cannot_fail,
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
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

use super::fallible::EmittedFallibleResults;

impl<'a> HirBuilder<'a> {
    /// Realises only the conversion retained by AST resolution. Exact integer
    /// comparisons keep both domains and decimal power keeps its I32 exponent.
    pub(super) fn lower_resolved_numeric_operand(
        &mut self,
        value: HirValueId,
        source: NumericScalar,
        conversion: Option<NumericScalar>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let Some(target) = conversion else {
            return Ok(value);
        };
        let target_type = self.lower_type_id(target.type_id(&self.type_environment))?;
        let region = self.module.expressions.expression(value).region;
        self.make_expression(
            &None,
            HirExpressionKind::Cast {
                source: value,
                policy: BuiltinCastPolicyId::NumericConversion { source, target },
            },
            target_type,
            ValueKind::RValue,
            region,
        )
    }

    /// Emits an operation at its selected precision and preserves its failure continuation.
    pub(crate) fn emit_checked_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
        discharged: bool,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if discharged {
            return self.emit_scalar_numeric_value(
                op,
                NumericFailureMode::Infallible,
                operands,
                success_type,
                span,
            );
        }
        if self.active_handler_accepts_builtin_failure()?
            || self.numeric_failure_is_recoverable()?
        {
            self.emit_recoverable_numeric_value(op, operands, success_type, span)
        } else {
            self.emit_scalar_numeric_value(
                op,
                NumericFailureMode::Trap,
                operands,
                success_type,
                span,
            )
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

        let region = self.current_region_or_error()?;
        let no_span = None;
        self.make_local_load_expression(result_local, success_type, &no_span, region)
    }

    /// Emits the terminal range guard with arithmetic's builtin failure owner.
    pub(crate) fn emit_range_step_failure(
        &mut self,
        cause: RangeStepFailureCause,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if self.active_handler_accepts_builtin_failure()?
            || self.numeric_failure_is_recoverable()?
        {
            let error_type = self.builtin_error_type_id()?;
            let results = self.emit_fallible_operation(
                HirFallibleOperation::RangeStepFailure { cause },
                &[],
                error_type,
                span,
            )?;
            // The range guard always fails. The structural success edge must still terminate.
            self.lower_numeric_results_to_success_values(results, span)?;
            let block = self.current_block_id_or_error()?;
            self.emit_terminator(
                block,
                HirTerminator::RuntimeFailure {
                    message: cause.builtin_error_code().default_message().to_owned(),
                    cause: None,
                },
                span,
            )?;
        } else {
            let result_local =
                self.allocate_temp_local(self.type_environment.builtins().bool, None)?;
            self.emit_statement_kind_with_span(
                HirStatementKind::RangeStepFailure {
                    cause,
                    failure_mode: NumericFailureMode::Trap,
                    result: HirLocalDestination::Define(result_local),
                },
                span,
                *span,
            )?;
            let block = self.current_block_id_or_error()?;
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

    /// Numeric recovery shares the native continuation machinery with calls and casts.
    fn emit_recoverable_numeric_value(
        &mut self,
        op: HirNumericOp,
        operands: HirNumericOperands,
        success_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let error_type = self.builtin_error_type_id()?;
        let results = self.emit_fallible_operation(
            HirFallibleOperation::NumericOp { op, operands },
            &[success_type],
            error_type,
            span,
        )?;
        self.lower_numeric_results_to_success_value(results, span)
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
        self.emit_statement_to_current_block(statement)
    }

    /// Formats a binary-float expression into a `String` using Moth's formatting contract.
    ///
    /// The formatter owns Moth stringification and checks incoming float integrity.
    /// WHY: runtime binary-float template interpolation must use one Moth-owned formatter instead
    ///      of relying on target-native stringification.
    pub(crate) fn emit_formatted_float_value(
        &mut self,
        source: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let recoverable = self.float_integrity_failure_is_recoverable()?;
        let string_type = self.lower_type_id(self.type_environment.builtins().string)?;
        if recoverable {
            self.emit_recoverable_formatted_float_value(source, string_type, span)
        } else {
            self.emit_trapping_formatted_float_value(source, string_type, span)
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

        let region = self.current_region_or_error()?;
        let no_span = None;
        self.make_local_load_expression(result_local, string_type, &no_span, region)
    }

    /// Formats through native success/error edges without a carrier local.
    fn emit_recoverable_formatted_float_value(
        &mut self,
        source: HirValueId,
        string_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let error_type = self.builtin_error_type_id()?;
        let results = self.emit_fallible_operation(
            HirFallibleOperation::FormatFloat { source },
            &[string_type],
            error_type,
            span,
        )?;
        self.lower_numeric_results_to_success_value(results, span)
    }

    /// Validates an external/backend-boundary binary float before ordinary Moth code observes it.
    ///
    /// The source row owns the exact result precision. Only successful external values
    /// reach this check, and fatal boundary guards do not become catch producers.
    pub(crate) fn emit_validated_float_value(
        &mut self,
        source: HirValueId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let recoverable = self.float_integrity_failure_is_recoverable()?;
        let float_type = self.module.expressions.expression(source).ty;
        if recoverable {
            self.emit_recoverable_validated_float_value(source, float_type, span)
        } else {
            self.emit_trapping_validated_float_value(source, float_type, span)
        }
    }

    /// Emits a trapping `ValidateFloat` and returns the scalar result local load.
    fn emit_trapping_validated_float_value(
        &mut self,
        source: HirValueId,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let result_local = self.allocate_temp_local(float_type, None)?;
        self.emit_validate_float_statement(source, NumericFailureMode::Trap, result_local, span)?;

        let region = self.current_region_or_error()?;
        let no_span = None;
        self.make_local_load_expression(result_local, float_type, &no_span, region)
    }

    /// Validates through native success/error edges at the source's exact float type.
    fn emit_recoverable_validated_float_value(
        &mut self,
        source: HirValueId,
        float_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let error_type = self.builtin_error_type_id()?;
        let results = self.emit_fallible_operation(
            HirFallibleOperation::ValidateFloat { source },
            &[float_type],
            error_type,
            span,
        )?;
        self.lower_numeric_results_to_success_value(results, span)
    }

    /// A catch continuation accepts builtin numeric failure only. Custom handlers keep
    /// injected float boundary checks on the function-boundary continuation.
    fn active_handler_accepts_builtin_failure(&mut self) -> Result<bool, HirConstructionFailure> {
        let Some(handler) = self.active_catch_handler else {
            return Ok(false);
        };
        Ok(handler.error_type == self.builtin_error_type_id()?)
    }

    fn lower_numeric_results_to_success_value(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if self.active_handler_accepts_builtin_failure()? {
            self.lower_results_to_active_catch_success_value(results, span)
        } else {
            self.lower_fallible_results_to_success_value(results, span)
        }
    }

    fn lower_numeric_results_to_success_values(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        if self.active_handler_accepts_builtin_failure()? {
            self.lower_results_to_active_catch_success_values(results, span)
        } else {
            self.lower_fallible_results_to_success_values(results, span)
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
        self.emit_statement_to_current_block(statement)
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
        self.emit_statement_to_current_block(statement)
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
        let (left, right) = self.lower_checked_numeric_binary_operands(op, left, right)?;
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
        let recoverable = self.active_handler_accepts_builtin_failure()?
            || self.numeric_failure_is_recoverable()?;

        // Generated updates retain the authored span and commit only on the success edge.
        if !recoverable {
            return self.emit_numeric_op_statement(
                op,
                NumericFailureMode::Trap,
                operands,
                target,
                span,
            );
        }

        let success_type = self.checked_numeric_result_type(op)?;
        let success_value =
            self.emit_recoverable_numeric_value(op, operands, success_type, span)?;
        let write_target = match target {
            HirLocalDestination::Define(local) => HirWriteTarget::DefineLocal(local),
            HirLocalDestination::Update(local) => {
                HirWriteTarget::AssignPlace(HirPlace::local(local))
            }
        };
        self.emit_write_statement(write_target, success_value)
    }

    /// Returns the scalar success type for a checked numeric operation.
    ///
    /// WHAT: derives the result type from the operation domain instead of matching every
    ///       operator/variant pair.
    /// WHY: the domain already owns the canonical type for every numeric scalar.
    fn checked_numeric_result_type(
        &mut self,
        op: HirNumericOp,
    ) -> Result<TypeId, HirConstructionFailure> {
        let domain_type = op.domain.type_id(&self.type_environment);
        self.lower_type_id(domain_type)
    }

    /// Formatting and incoming-value guards are integrity checks, not implicit numeric failure.
    /// Only a source-declared builtin Error! contract makes these guards recoverable. The exact
    /// start identity is retained from the compiler-generated AST entry, not inferred from a name.
    fn float_integrity_failure_is_recoverable(&mut self) -> Result<bool, HirConstructionFailure> {
        let function_id = self.current_function_id_or_error()?;
        if self.module.start_function == Some(function_id) {
            return Ok(false);
        }

        self.numeric_failure_is_recoverable()
    }

    /// Selects function-boundary delivery, independent of numeric statement modes.
    ///
    /// Builtin Error contracts return implicit failures. Custom errors need explicit mapping.
    /// Private no-slot functions initially trap until the inferred lane retargets their
    /// code-carrying producers. Float integrity guards use their separate start policy.
    pub(crate) fn numeric_failure_is_recoverable(
        &mut self,
    ) -> Result<bool, HirConstructionFailure> {
        let current_function_id = self.current_function_id_or_error()?;
        let function = self.function_by_id_or_error(current_function_id)?;
        let Some(error_type) = function.error_return else {
            return Ok(false);
        };
        Ok(self.maybe_builtin_error_type_id() == Some(error_type))
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

    /// Converts binary operands to the selected checked-operation domain.
    ///
    /// WHAT: makes operation-domain promotions explicit, except that Dec power keeps its exact
    ///       `I32` exponent.
    /// WHY: backends consume typed operations without reconstructing source promotions, while the
    ///      exponent is semantically distinct from the Dec base and must never be rescaled.
    pub(crate) fn lower_checked_numeric_binary_operands(
        &mut self,
        op: HirNumericOp,
        left: HirValueId,
        right: HirValueId,
    ) -> Result<(HirValueId, HirValueId), HirConstructionFailure> {
        let left = self.convert_numeric_operand_to_domain(left, op.domain)?;

        if op.operator == NumericOperator::Power && matches!(op.domain, NumericScalar::Number(_)) {
            let exponent_type =
                NumericScalar::Fixed(FixedScalar::I32).type_id(&self.type_environment);
            if self.module.expressions.expression(right).ty != exponent_type {
                return Err(InfrastructureFailure::hir_transformation(
                    "Dec power exponent must remain the exact I32 type",
                )
                .into());
            }

            return Ok((left, right));
        }

        let right = self.convert_numeric_operand_to_domain(right, op.domain)?;
        Ok((left, right))
    }

    /// Converts an operand to a checked numeric operation's domain when required.
    ///
    /// WHAT: emits the standard `NumericConversion` expression for a source scalar whose
    ///       canonical type differs from `domain`, leaving already-matching operands unchanged.
    /// WHY: shared operator policy determines promotions; HIR makes each required conversion
    ///      explicit. Dec power keeps its exponent in the exact `I32` type.
    pub(crate) fn convert_numeric_operand_to_domain(
        &mut self,
        value: HirValueId,
        domain: NumericScalar,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let target_type = domain.type_id(&self.type_environment);
        let value_row = self.module.expressions.expression(value);
        if value_row.ty == target_type {
            return Ok(value);
        }

        let Some(source) = NumericScalar::from_type_id(value_row.ty, &self.type_environment) else {
            return Err(InfrastructureFailure::hir_transformation(
                "Numeric operation operand has no numeric conversion domain",
            )
            .into());
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

    /// Whether compiler-generated arithmetic's pre-conversion operand types discharge failure.
    /// Source-authored operations instead consume the AST's retained discharge decision.
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
