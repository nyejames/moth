//! Statement lowering helpers for the JavaScript backend.
//!
//! These routines emit block-local statements after HIR has already made evaluation order and
//! control-flow edges explicit.

use crate::backends::js::JsEmitter;
use crate::backends::js::js_expr::{escape_js_string, js_cast_expression_for_policy};
use crate::backends::js::numeric_carrier::{
    JsNumericCarrier, JsNumericConversion, binary_float_precision_bits, number_scale_factor_js,
};
use crate::backends::js::value_use::JsValueUse;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, compiler_errors::InfrastructureFailure,
};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirAssertionMessageEvaluation, HirFallibleOperation, HirTerminator,
};

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_block_statements(
        &mut self,
        block: &crate::compiler_frontend::hir::blocks::HirBlock,
    ) -> Result<(), InfrastructureFailure> {
        for statement in &block.statements {
            self.emit_statement(statement)?;
        }

        Ok(())
    }

    pub(crate) fn emit_statement(
        &mut self,
        statement: &HirStatement,
    ) -> Result<(), InfrastructureFailure> {
        match &statement.kind {
            HirStatementKind::Write { target, value } => {
                self.emit_write(*target, *value)?;
            }

            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                self.emit_call_statement(target, *args, results)?;
            }

            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                // WHAT: dispatch a map builtin to the JS runtime helper that handles branded-map
                // values.
                // WHY: map ops are not ordinary calls; helper selection and arity validation stay
                // local to `emit_map_op_statement`.
                self.emit_map_op_statement(*op, *receiver, *args, result)?;
            }

            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                // WHAT: dispatch a checked operation through the helper family selected from its
                //       semantic result domain.
                // WHY: the domain's carrier determines both JS arithmetic and the domain bounds.
                self.emit_numeric_op_statement(
                    *op,
                    *failure_mode,
                    operands,
                    *result,
                    statement.id,
                )?;
            }

            HirStatementKind::RangeStepFailure {
                cause,
                failure_mode,
                result,
            } => {
                let error_code = cause.builtin_error_code();
                let helper_call = format!(
                    "__moth_error_result({:?}, {})",
                    error_code.default_message(),
                    error_code.as_u32(),
                );
                self.emit_numeric_carrier_assignment(helper_call, *failure_mode, *result)?;
            }
            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                inclusive,
                domain,
                candidate_result,
                in_range_result,
            } => {
                self.emit_float_range_candidate_statement(
                    [*current, *step, *end, *ascending],
                    *inclusive,
                    *domain,
                    *candidate_result,
                    *in_range_result,
                )?;
            }
            HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            } => {
                // WHAT: format at the source precision and consume the helper's ABI envelope.
                // WHY: binary-float formatting is a language builtin with an explicit failure mode; the
                //      backend must map it to the helper that normalizes JS number text to the
                //      Moth contract instead of using target-native stringification directly.
                self.emit_format_float_statement(*failure_mode, *source, *result)?;
            }

            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            } => {
                // WHAT: validates a finite binary-float at its exact binary precision.
                // WHY: external values enter the same finite, rounded carrier as other binary floats.
                self.emit_validate_float_statement(*failure_mode, *source, *result)?;
            }

            HirStatementKind::Expr(expression) => {
                let expression = self.lower_expr(*expression)?;
                self.emit_line(&format!("{expression};"));
            }

            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                // WHAT: lower a fragment push into a JS vec push call against the unwrapped array.
                // WHY: locals are stored as binding wrappers `{ value: ... }` so `.push` cannot be
                //      called on the binding itself. __moth_read returns the underlying array.
                let vec_name = self.local_name(*vec_local)?.to_owned();
                let value_expr =
                    self.lower_expression_for_use(*value, JsValueUse::AssignmentValue)?;
                self.emit_line(&format!("__moth_read({vec_name}).push({value_expr});"));
            }
        }

        Ok(())
    }

    /// Emit an infallible map operation using the existing runtime ABI.
    fn emit_map_op_statement(
        &mut self,
        op: HirMapOp,
        receiver: HirValueId,
        args: HirValueRange,
        result: &Option<HirLocalDestination>,
    ) -> Result<(), InfrastructureFailure> {
        let call = self.lower_map_operation(op, receiver, args)?;
        if let Some(result) = result {
            self.emit_value_destination(*result, &call)?;
        } else {
            self.emit_line(&format!("{call};"));
        }
        Ok(())
    }

    fn lower_map_operation(
        &mut self,
        op: HirMapOp,
        receiver: HirValueId,
        args: HirValueRange,
    ) -> Result<String, InfrastructureFailure> {
        let hir = self.hir;
        let args = hir.expressions.values(args);
        // Lower the receiver map first so helper-call argument order mirrors HIR order.
        let receiver_expr = self.lower_expr(receiver)?;

        // Select the JS helper and its HIR arity contract.
        let (helper_name, expected_arity) = match op {
            HirMapOp::Get => ("__moth_map_get", 1),
            HirMapOp::Contains => ("__moth_map_contains", 1),
            HirMapOp::Set => ("__moth_map_set", 2),
            HirMapOp::Remove => ("__moth_map_remove", 1),
            HirMapOp::Clear => ("__moth_map_clear", 0),
            HirMapOp::Length => ("__moth_map_length", 0),
        };

        // Guard against arity mismatch between HIR and the backend.
        if args.len() != expected_arity {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend received MapOp::{op:?} with {actual} args instead of {expected}",
                actual = args.len(),
                expected = expected_arity,
            );
        }

        // Lower each HIR argument to a JS expression.
        let mut lowered_args = Vec::with_capacity(args.len());
        for arg in args {
            lowered_args.push(self.lower_expr(*arg)?);
        }

        // Assemble the helper call, with or without extra arguments.
        let call = if lowered_args.is_empty() {
            format!("{helper_name}({receiver_expr})")
        } else {
            format!(
                "{helper_name}({receiver_expr}, {})",
                lowered_args.join(", ")
            )
        };

        Ok(call)
    }

    /// Lower a checked HIR numeric operation through its domain-selected carrier family.
    ///
    /// WHAT: validates HIR arity, lowers operands, selects the operation helper and appends
    ///       semantic integer bounds when that family uses an integer carrier.
    /// WHY: one helper family covers all domains with the same JS representation without losing
    ///      each operation result's range.
    fn emit_numeric_op_statement(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: &HirNumericOperands,
        result: HirLocalDestination,
        statement: HirNodeId,
    ) -> Result<(), InfrastructureFailure> {
        let (value, proven) = self.lower_numeric_operation(op, operands, statement)?;
        if proven {
            self.emit_value_destination(result, &value)
        } else {
            self.emit_numeric_carrier_assignment(value, failure_mode, result)
        }
    }

    fn lower_numeric_operation(
        &mut self,
        op: HirNumericOp,
        operands: &HirNumericOperands,
        statement: HirNodeId,
    ) -> Result<(String, bool), InfrastructureFailure> {
        // Guard against arity mismatch between HIR and the backend.
        let is_unary = op.is_unary();
        let operands_are_unary = matches!(operands, HirNumericOperands::Unary { .. });
        if is_unary != operands_are_unary {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend received NumericOp::{op} with operand arity that does not match the operation"
            );
        }

        // Lower each HIR operand to a JS expression once, in source order.
        let mut lowered_args = match operands {
            HirNumericOperands::Unary { operand } => vec![self.lower_expr(*operand)?],
            HirNumericOperands::Binary { left, right } => {
                vec![self.lower_expr(*left)?, self.lower_expr(*right)?]
            }
        };

        let carrier = JsNumericCarrier::for_scalar(op.domain).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend has no numeric carrier for {:?}",
                op.domain
            )
        });

        // A proof removes only checked helper machinery, never the Invoke's failure edge.
        if self.numeric_proofs.integer_operation_is_safe(statement)
            && let Some(proven_expression) =
                self.proven_integer_operation_expression(op, &carrier, &lowered_args)
        {
            return Ok((proven_expression, true));
        }

        let (helper_name, bounds) = js_numeric_helper_for_op(op);
        if let NumericScalar::Number(scale) = op.domain
            && matches!(
                op.operator,
                NumericOperator::Multiply | NumericOperator::Divide | NumericOperator::Power
            )
        {
            lowered_args.push(number_scale_factor_js(scale.get()));
        }
        if let Some((min, max)) = bounds {
            lowered_args.push(min);
            lowered_args.push(max);
        }

        Ok((format!("{helper_name}({})", lowered_args.join(", ")), false))
    }

    /// Renders exact native-carrier arithmetic for one proven-safe integer operation.
    ///
    /// Proven operations evaluate operands once and preserve their exact Number or BigInt
    /// domain, including truncating division and dividend-signed remainder.
    /// WHY: JS still produces `-0` on the Number carrier where Moth has one canonical integer
    ///       zero — `0 * negative`, `0 / negative`, `negative % divisor` dividing evenly and
    ///       negating `0` — so those operations renormalise with `+ 0` without re-evaluating any
    ///       operand, matching the checked helpers' success boundary. Negation also groups its
    ///       operand inside the emitted unary minus, because a leading-minus operand spelling
    ///       (a negative literal) would otherwise fuse into an invalid JS `--` update token.
    ///
    /// Returns `None` for every operation the proof table never proves (binary-float and Dec
    /// domains, `Power`, and unsupported operator/domain combinations), leaving them on the
    /// checked lowering path.
    fn proven_integer_operation_expression(
        &self,
        op: HirNumericOp,
        carrier: &JsNumericCarrier,
        lowered_args: &[String],
    ) -> Option<String> {
        match (carrier, op.operator, lowered_args) {
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Add, [left, right]) => {
                Some(format!("({left} + {right})"))
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Subtract, [left, right]) => {
                Some(format!("({left} - {right})"))
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Multiply, [left, right]) => {
                Some(format!("({left} * {right} + 0)"))
            }
            (
                JsNumericCarrier::ExactInteger { .. },
                NumericOperator::IntegerDivide,
                [left, right],
            ) => Some(format!("(Math.trunc({left} / {right}) + 0)")),
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Remainder, [left, right]) => {
                Some(format!("({left} % {right} + 0)"))
            }
            (JsNumericCarrier::ExactInteger { .. }, NumericOperator::Negate, [left]) => {
                Some(format!("(-({left}) + 0)"))
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Add, [left, right]) => {
                Some(format!("({left} + {right})"))
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Subtract, [left, right]) => {
                Some(format!("({left} - {right})"))
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Multiply, [left, right]) => {
                Some(format!("({left} * {right})"))
            }
            (
                JsNumericCarrier::BigInteger { .. },
                NumericOperator::IntegerDivide,
                [left, right],
            ) => Some(format!("({left} / {right})")),
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Remainder, [left, right]) => {
                Some(format!("({left} % {right})"))
            }
            (JsNumericCarrier::BigInteger { .. }, NumericOperator::Negate, [left]) => {
                Some(format!("(-({left}))"))
            }
            _ => None,
        }
    }

    /// Render the raw carrier conversion for a proven-safe fallible integer narrowing.
    fn proven_narrowing_expression(
        &self,
        policy: BuiltinCastPolicyId,
        value: &str,
        statement: HirNodeId,
    ) -> Option<String> {
        if !self.integer_narrowing_is_proven(statement, policy) {
            return None;
        }

        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return None;
        };
        let conversion = JsNumericConversion::classify(source, target)
            .expect("proven narrowing policy must classify");
        let source_carrier = JsNumericCarrier::for_scalar(source)
            .expect("classified narrowing source has a JS carrier");
        let proven = conversion
            .proven_safe_integer_narrowing(source_carrier)
            .expect("proven narrowing applies to checked integer conversions");

        Some(proven.expression(value, source, target))
    }

    /// Whether one Invoke cast's numeric conversion is a proven-safe integer narrowing.
    ///
    /// WHAT: only fallible integer-to-integer conversions proven at their Invoke node qualify.
    /// WHY: demand scans and operation lowering must agree on elided checked helpers.
    pub(crate) fn integer_narrowing_is_proven(
        &self,
        statement: HirNodeId,
        policy: BuiltinCastPolicyId,
    ) -> bool {
        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return false;
        };
        let Ok(conversion) = JsNumericConversion::classify(source, target) else {
            return false;
        };
        if !matches!(
            conversion,
            JsNumericConversion::CheckedIntegerToInteger { .. }
        ) {
            return false;
        }

        self.numeric_proofs.integer_narrowing_is_safe(statement)
    }

    /// Emit the range candidate into JS-local scratch, then commit only a finite in-bound value.
    fn emit_float_range_candidate_statement(
        &mut self,
        expressions: [HirValueId; 4],
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: HirLocalDestination,
        in_range_result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let [current, step, end, ascending] = expressions;

        let current_expr = self.lower_expr(current)?;
        let step_expr = self.lower_expr(step)?;
        let end_expr = self.lower_expr(end)?;
        let ascending_expr = self.lower_expr(ascending)?;
        let precision = domain.binary_float_precision().unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend received a non-float range candidate domain {domain:?}"
            )
        });

        let direction_name = self.next_temp_identifier("__float_range_ascending");
        let candidate_name = self.next_temp_identifier("__float_range_candidate");
        let in_range_name = self.next_temp_identifier("__float_range_valid");
        let unrounded_candidate = format!(
            "({direction_name} ? ({current_expr} + {step_expr}) : ({current_expr} - {step_expr}))"
        );
        // An F16 range step completes at F16 at each candidate, so the selected domain
        // keeps its precision instead of widening to F32.
        let candidate_expr = match precision {
            BinaryFloatPrecision::Binary16 => format!("Math.f16round({unrounded_candidate})"),
            BinaryFloatPrecision::Binary32 => format!("Math.fround({unrounded_candidate})"),
            BinaryFloatPrecision::Binary64 => unrounded_candidate,
        };
        let (ascending_comparison, descending_comparison) =
            if inclusive { ("<=", ">=") } else { ("<", ">") };
        let bound_check = format!(
            "({direction_name} ? ({candidate_name} {ascending_comparison} {end_expr}) : ({candidate_name} {descending_comparison} {end_expr}))"
        );
        let direction_assignment = format!("const {direction_name} = {ascending_expr};");
        let candidate_assignment = format!("const {candidate_name} = {candidate_expr};");
        let in_range_assignment =
            format!("const {in_range_name} = Number.isFinite({candidate_name}) && {bound_check};");

        self.emit_line(&direction_assignment);
        self.emit_line(&candidate_assignment);
        self.emit_line(&in_range_assignment);
        self.emit_value_destination(in_range_result, &in_range_name)?;
        self.emit_line(&format!("if ({in_range_name}) {{"));
        self.indent += 1;
        self.emit_value_destination(candidate_result, &candidate_name)?;
        self.indent -= 1;
        self.emit_line("}");

        Ok(())
    }

    /// Lower a `HirStatementKind::FormatFloat` into the exact-precision binary-float formatter.
    ///
    /// WHAT: derives the source precision and canonical name, then consumes the runtime
    /// formatter's ABI envelope as a scalar string.
    /// WHY: template and cast formatting share one precision-aware helper and result contract,
    ///      and the source row owns the resolved precision.
    fn emit_format_float_statement(
        &mut self,
        failure_mode: NumericFailureMode,
        source: HirValueId,
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let source_expr = self.lower_expr(source)?;
        let (scalar, precision) = self.float_source_scalar(source, "FormatFloat");
        let helper_call = format!(
            "__moth_format_float({source_expr}, {}, \"{scalar}\")",
            binary_float_precision_bits(precision)
        );
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Lower a `HirStatementKind::ValidateFloat` into the exact-precision finite binary-float helper.
    ///
    /// WHAT: derives the validated source's own binary-float precision and emits it as the
    ///       helper's precision argument.
    /// WHY: a validated boundary value carries a fixed `F16`/`F32`/`F64` type, so the exact
    ///      source type owns the rounding step.
    fn emit_validate_float_statement(
        &mut self,
        failure_mode: NumericFailureMode,
        source: HirValueId,
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let source_expr = self.lower_expr(source)?;
        let (_scalar, precision) = self.float_source_scalar(source, "binary-float validation");
        let helper_call = format!(
            "__moth_float_validate({source_expr}, {})",
            binary_float_precision_bits(precision)
        );
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// The exact binary-float domain and precision of one float statement's source expression.
    ///
    /// WHAT: derives the domain from the source row's own type through the canonical
    ///       `TypeId`-to-domain owner.
    /// WHY: FormatFloat, ValidateFloat and the prelude demand scan must agree on one precision
    ///      source, and an exact `F16`/`F32`/`F64` value is never rounded to another precision.
    pub(crate) fn float_source_scalar(
        &self,
        source: HirValueId,
        statement_name: &'static str,
    ) -> (NumericScalar, BinaryFloatPrecision) {
        let source_type = self.hir.expressions.expression(source).ty;
        let scalar = NumericScalar::from_type_id(source_type, self.type_environment)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend received {statement_name} source with non-binary-float type {source_type:?}"
                )
            });
        let precision = scalar.binary_float_precision().unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend received {statement_name} source with non-binary-float type {source_type:?}"
            )
        });

        (scalar, precision)
    }

    /// Runtime helpers return ABI envelopes; trap statements consume them immediately.
    fn emit_numeric_carrier_assignment(
        &mut self,
        helper_call: String,
        failure_mode: NumericFailureMode,
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let assigned_value = match failure_mode {
            NumericFailureMode::Trap | NumericFailureMode::Infallible => {
                format!("__moth_numeric_trap({helper_call})")
            }
        };

        self.emit_value_destination(result, &assigned_value)
    }

    pub(crate) fn emit_value_destination(
        &mut self,
        destination: HirLocalDestination,
        value: &str,
    ) -> Result<(), InfrastructureFailure> {
        let local_name = self.local_name(destination.local())?.to_owned();
        match destination {
            HirLocalDestination::Define(_) => {
                self.emit_line(&format!("{local_name} = __moth_binding({value});"));
            }
            HirLocalDestination::Update(_) => {
                self.emit_line(&format!("__moth_assign_value({local_name}, {value});"));
            }
        }

        Ok(())
    }

    fn emit_write(
        &mut self,
        target: HirWriteTarget,
        value: HirValueId,
    ) -> Result<(), InfrastructureFailure> {
        match target {
            HirWriteTarget::DefineLocal(local) => self.emit_local_definition(local, value),
            HirWriteTarget::AssignPlace(place) => self.emit_place_assignment(place, value),
        }
    }

    fn emit_local_definition(
        &mut self,
        local: LocalId,
        value: HirValueId,
    ) -> Result<(), InfrastructureFailure> {
        let local_name = self.local_name(local)?.to_owned();
        let value_expression = self.hir.expressions.expression(value);
        match value_expression.value_kind {
            ValueKind::Place => {
                let HirExpressionKind::Load(place) = &value_expression.kind else {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JavaScript backend received a place-valued definition without a direct place load"
                    );
                };
                let source = self.lower_place(place)?;
                self.emit_line(&format!("{local_name} = __moth_alias_binding({source});"));
            }
            ValueKind::RValue | ValueKind::Const => {
                let lowered = self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                self.emit_line(&format!("{local_name} = __moth_binding({lowered});"));
            }
        }

        Ok(())
    }

    fn emit_place_assignment(
        &mut self,
        target: HirPlace,
        value: HirValueId,
    ) -> Result<(), InfrastructureFailure> {
        if self
            .hir
            .expressions
            .projections(target.projections)
            .is_empty()
        {
            let local_name = self.local_name(target.root)?.to_owned();
            let value_expression = self.hir.expressions.expression(value);
            match value_expression.value_kind {
                ValueKind::Place => {
                    let HirExpressionKind::Load(source_place) = &value_expression.kind else {
                        crate::compiler_bug!(
                            CompilerStage::BackendLowering,
                            "JavaScript backend received a place-valued update without a direct place load"
                        );
                    };
                    let source = self.lower_place(source_place)?;
                    self.emit_line(&format!("__moth_assign_borrow({local_name}, {source});"));
                }
                ValueKind::RValue | ValueKind::Const => {
                    let lowered =
                        self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                    self.emit_line(&format!("__moth_assign_value({local_name}, {lowered});"));
                }
            }
        } else {
            let target_ref = self.lower_place(&target)?;
            let emitted_value =
                self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
            self.emit_line(&format!("__moth_write({target_ref}, {emitted_value});"));
        }

        Ok(())
    }

    /// Consume helper/call ABI envelopes at the boundary and define only the selected edge.
    pub(crate) fn prepare_invoke(
        &mut self,
        id: HirNodeId,
        operation: &HirFallibleOperation,
        success_results: &[LocalId],
        error_result: LocalId,
    ) -> Result<(String, bool), InfrastructureFailure> {
        let (value, proven) = match operation {
            HirFallibleOperation::Call { target, args } => {
                (self.lower_call_expression(target, *args)?, false)
            }
            HirFallibleOperation::MapOp { op, receiver, args } => {
                (self.lower_map_operation(*op, *receiver, *args)?, false)
            }
            HirFallibleOperation::NumericOp { op, operands } => {
                self.lower_numeric_operation(*op, operands, id)?
            }
            HirFallibleOperation::CastOp { policy, source } => {
                let source = self.lower_expr(*source)?;
                if let Some(value) = self.proven_narrowing_expression(*policy, &source, id) {
                    (value, true)
                } else {
                    (js_cast_expression_for_policy(*policy, &source)?, false)
                }
            }
            HirFallibleOperation::RangeStepFailure { cause } => {
                let code = cause.builtin_error_code();
                (
                    format!(
                        "__moth_error_result({:?}, {})",
                        code.default_message(),
                        code.as_u32()
                    ),
                    false,
                )
            }
            HirFallibleOperation::FormatFloat { source } => {
                let value = self.lower_expr(*source)?;
                let (scalar, precision) = self.float_source_scalar(*source, "FormatFloat");
                (
                    format!(
                        "__moth_format_float({value}, {}, \"{scalar}\")",
                        binary_float_precision_bits(precision)
                    ),
                    false,
                )
            }
            HirFallibleOperation::ValidateFloat { source } => {
                let value = self.lower_expr(*source)?;
                let (_, precision) = self.float_source_scalar(*source, "ValidateFloat");
                (
                    format!(
                        "__moth_float_validate({value}, {})",
                        binary_float_precision_bits(precision)
                    ),
                    false,
                )
            }
        };
        let captured = self.next_temp_identifier("__invoke_result");
        self.emit_line(&format!("const {captured} = {value};"));
        // Clear both outcomes on every execution, including repeated loop visits. Capture the
        // operation first because its operands may read a previous edge-local binding.
        for local in success_results.iter().chain(std::iter::once(&error_result)) {
            let name = self.local_name(*local)?.to_owned();
            self.emit_line(&format!("{name} = undefined;"));
        }
        Ok((captured, proven))
    }

    pub(crate) fn emit_invoke_success_transfer(
        &mut self,
        captured: &str,
        proven: bool,
        results: &[LocalId],
    ) -> Result<(), InfrastructureFailure> {
        for (index, result) in results.iter().enumerate() {
            let value = if proven {
                captured.to_owned()
            } else if results.len() == 1 {
                format!("{captured}.value")
            } else {
                format!("{captured}.value[{index}]")
            };
            self.emit_value_destination(HirLocalDestination::Define(*result), &value)?;
        }
        Ok(())
    }

    pub(crate) fn emit_invoke_error_transfer(
        &mut self,
        captured: &str,
        result: LocalId,
    ) -> Result<(), InfrastructureFailure> {
        self.emit_value_destination(
            HirLocalDestination::Define(result),
            &format!("{captured}.value"),
        )
    }

    pub(crate) fn emit_return_terminator(
        &mut self,
        values: HirValueRange,
    ) -> Result<(), InfrastructureFailure> {
        let hir = self.hir;
        let expressions = hir.expressions.values(values);
        let function_id = self.current_function.unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: Return outside a function"
            )
        });
        let function = self
            .hir
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JavaScript backend: missing current function"
                )
            });
        if expressions.len() != function.returns.len()
            || expressions
                .iter()
                .zip(function.returns.iter())
                .any(|(value, ty)| self.hir.expressions.expression(*value).ty != *ty)
        {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: Return slots mismatch"
            );
        }
        let fallible = function.error_return.is_some();
        let values = expressions
            .iter()
            .map(|value| self.lower_moth_return_value(*value))
            .collect::<Result<Vec<_>, _>>()?;
        let value = match values.as_slice() {
            [] => "undefined".to_owned(),
            [value] => value.to_owned(),
            _ => format!("[{}]", values.join(", ")),
        };
        if fallible {
            self.emit_line(&format!("return {{ tag: \"ok\", value: {value} }};"));
        } else if values.is_empty() {
            self.emit_line("return;");
        } else {
            self.emit_line(&format!("return {value};"));
        }
        Ok(())
    }

    pub(crate) fn emit_error_return_terminator(
        &mut self,
        expression: HirValueId,
    ) -> Result<(), InfrastructureFailure> {
        let Some(function_id) = self.current_function else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: ReturnError emitted outside a function"
            );
        };
        let function = self
            .hir
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JavaScript backend: current function {function_id:?} is missing"
                )
            });
        let Some(error_type) = function.error_return else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: ReturnError emitted in a non-fallible function"
            );
        };
        if self.hir.expressions.expression(expression).ty != error_type {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: ReturnError value type does not match function error slot"
            );
        }

        let value = self.lower_expr(expression)?;
        self.emit_line(&format!("return {{ tag: \"err\", value: {value} }};"));
        Ok(())
    }

    pub(crate) fn emit_assert_failure_terminator(
        &mut self,
        message: HirValueId,
        message_evaluation: HirAssertionMessageEvaluation,
    ) -> Result<(), InfrastructureFailure> {
        self.used_assertions = true;
        if matches!(message_evaluation, HirAssertionMessageEvaluation::Default) {
            self.emit_line("throw __moth_assertion_error(\"assertion failed\");");
            return Ok(());
        }

        let message_identifier = self.next_temp_identifier("__assert_message");
        let message_value = self.lower_expr(message)?;
        self.emit_line(&format!("let {message_identifier} = {message_value};"));
        self.emit_line(&format!(
            "throw __moth_assertion_error(({message_identifier}.tag === \"some\" ? {message_identifier}.value : \"assertion failed\"));"
        ));

        Ok(())
    }

    pub(crate) fn emit_runtime_failure_terminator(&mut self, message: &str) {
        self.emit_line(&format!("throw new Error({});", escape_js_string(message)));
    }

    pub(crate) fn emit_dispatcher_for_function(
        &mut self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
    ) -> Result<(), InfrastructureFailure> {
        let state_identifier = self.next_temp_identifier("__bb");

        self.emit_line(&format!("let {state_identifier} = {};", function.entry.0));
        self.emit_line("while (true) {");
        self.indent += 1;
        self.emit_line(&format!("switch ({state_identifier}) {{"));
        self.indent += 1;

        for block_id in reachable_blocks {
            let block = match self.block_by_id(*block_id) {
                Ok(block) => block,
                Err(error) => {
                    self.indent -= 2;
                    return Err(error);
                }
            };

            self.emit_line(&format!("case {}: {{", block.id.0));
            self.indent += 1;

            if let Err(error) = self.emit_block_statements(block) {
                self.indent -= 3;
                return Err(error);
            }

            if let Err(error) =
                self.emit_dispatcher_terminator(&state_identifier, &block.terminator)
            {
                self.indent -= 3;
                return Err(error);
            }

            self.indent -= 1;
            self.emit_line("}");
        }

        self.emit_line("default: {");
        self.with_indent(|emitter| {
            emitter.emit_line(&format!(
                "throw new Error(\"Invalid control-flow block: \" + {state_identifier});",
            ));
        });
        self.emit_line("}");

        self.indent -= 1;
        self.emit_line("}");
        self.indent -= 1;
        self.emit_line("}");

        Ok(())
    }

    fn emit_dispatcher_terminator(
        &mut self,
        state_identifier: &str,
        terminator: &HirTerminator,
    ) -> Result<(), InfrastructureFailure> {
        match terminator {
            HirTerminator::Jump { target, args } => {
                self.emit_jump_argument_transfer(args)?;
                self.emit_line(&format!("{state_identifier} = {};", target.0));
                self.emit_line("continue;");
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.lower_expr(*condition)?;
                self.emit_line(&format!("if ({condition}) {{"));
                self.with_indent(|emitter| {
                    emitter.emit_line(&format!("{state_identifier} = {};", then_block.0));
                });
                self.emit_line("} else {");
                self.with_indent(|emitter| {
                    emitter.emit_line(&format!("{state_identifier} = {};", else_block.0));
                });
                self.emit_line("}");
                self.emit_line("continue;");
            }

            HirTerminator::Invoke {
                id,
                operation,
                success,
                success_results,
                failure,
                error_result,
                ..
            } => {
                let (captured, proven) =
                    self.prepare_invoke(*id, operation, success_results, *error_result)?;
                let condition = if proven {
                    "true".to_owned()
                } else {
                    format!("{captured}.tag === \"ok\"")
                };
                self.emit_line(&format!("if ({condition}) {{"));
                self.indent += 1;
                self.emit_invoke_success_transfer(&captured, proven, success_results)?;
                self.emit_line(&format!("{state_identifier} = {};", success.0));
                self.indent -= 1;
                self.emit_line("} else {");
                self.indent += 1;
                self.emit_invoke_error_transfer(&captured, *error_result)?;
                self.emit_line(&format!("{state_identifier} = {};", failure.0));
                self.indent -= 1;
                self.emit_line("}");
                self.emit_line("continue;");
            }

            HirTerminator::Match { scrutinee, arms } => {
                // Unreachable: `enforce_match_exhaustiveness` rejects a match with no arms and
                // no `else`, so HIR never emits an empty `Match` terminator.
                if arms.is_empty() {
                    crate::compiler_bug!(
                        CompilerStage::BackendLowering,
                        "JavaScript backend: Match terminator has no arms"
                    );
                }
                let scrutinee_type = self.hir.expressions.expression(*scrutinee).ty;
                let scrutinee = self.lower_expr(*scrutinee)?;
                let scrutinee_temp = self.next_temp_identifier("__match");
                self.emit_line(&format!("const {scrutinee_temp} = {scrutinee};"));

                // If the last arm is an unguarded wildcard, emit it as `else` instead of
                // `else if (true)` and skip the unreachable fallback throw.
                let has_unconditional_fallback = matches!(
                    arms.last(),
                    Some(HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        ..
                    })
                );
                let emit_count = if has_unconditional_fallback {
                    arms.len() - 1
                } else {
                    arms.len()
                };

                for (index, arm) in arms.iter().enumerate().take(emit_count) {
                    let condition =
                        self.lower_match_arm_condition(&scrutinee_temp, scrutinee_type, arm)?;
                    if index == 0 {
                        self.emit_line(&format!("if ({condition}) {{"));
                    } else {
                        self.emit_line(&format!("else if ({condition}) {{"));
                    }

                    self.with_indent(|emitter| {
                        emitter.emit_line(&format!("{state_identifier} = {};", arm.body.0));
                    });
                    self.emit_line("}");
                }
                if has_unconditional_fallback {
                    if let Some(wildcard_arm) = arms.last() {
                        self.emit_line("else {");
                        self.with_indent(|emitter| {
                            emitter.emit_line(&format!(
                                "{state_identifier} = {};",
                                wildcard_arm.body.0
                            ));
                        });
                        self.emit_line("}");
                    }
                } else {
                    self.emit_line("else {");
                    self.with_indent(|emitter| {
                        emitter.emit_line("throw new Error(\"No match arm selected\");");
                    });
                    self.emit_line("}");
                }
                self.emit_line("continue;");
            }

            HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                self.emit_line(&format!("{state_identifier} = {};", target.0));
                self.emit_line("continue;");
            }

            HirTerminator::Return(value) => {
                self.emit_return_terminator(*value)?;
            }

            HirTerminator::ReturnError(value) => {
                self.emit_error_return_terminator(*value)?;
            }

            HirTerminator::Uninitialized => {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "Uninitialized terminator reached JS backend lowering"
                );
            }

            HirTerminator::RuntimeFailure { message, .. } => {
                self.emit_runtime_failure_terminator(message);
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                self.emit_assert_failure_terminator(*message, *message_evaluation)?;
            }
        }

        Ok(())
    }

    pub(crate) fn lower_match_arm_condition(
        &mut self,
        scrutinee_expression: &str,
        scrutinee_type: TypeId,
        arm: &HirMatchArm,
    ) -> Result<String, InfrastructureFailure> {
        let pattern_condition = match &arm.pattern {
            HirPattern::Literal(value) => {
                let value_type = self.hir.expressions.expression(*value).ty;
                let literal = self.lower_expr(*value)?;
                self.lower_typed_equality(
                    scrutinee_expression.to_owned(),
                    scrutinee_type,
                    literal,
                    value_type,
                )
            }
            HirPattern::OptionNone => {
                format!("({scrutinee_expression}).tag === \"none\"")
            }
            HirPattern::OptionValue { value } => {
                let value_type = self.hir.expressions.expression(*value).ty;
                let literal = self.lower_expr(*value)?;
                let inner_type = self
                    .type_environment
                    .option_inner_type(scrutinee_type)
                    .unwrap_or(value_type);
                let inner_equality = self.lower_typed_equality(
                    format!("({scrutinee_expression}).value"),
                    inner_type,
                    literal,
                    value_type,
                );
                format!("((({scrutinee_expression}).tag === \"some\") && {inner_equality})")
            }
            HirPattern::OptionRelational { op, value } => {
                let rhs = self.lower_expr(*value)?;
                let js_op = match op {
                    HirRelationalPatternOp::LessThan => "<",
                    HirRelationalPatternOp::LessThanOrEqual => "<=",
                    HirRelationalPatternOp::GreaterThan => ">",
                    HirRelationalPatternOp::GreaterThanOrEqual => ">=",
                };
                format!(
                    "((({scrutinee_expression}).tag === \"some\") && (({scrutinee_expression}).value {js_op} {rhs}))"
                )
            }
            HirPattern::Wildcard => "true".to_owned(),
            HirPattern::OptionPresent => {
                format!("({scrutinee_expression}).tag === \"some\"")
            }
            HirPattern::Relational { op, value } => {
                let rhs = self.lower_expr(*value)?;
                let js_op = match op {
                    HirRelationalPatternOp::LessThan => "<",
                    HirRelationalPatternOp::LessThanOrEqual => "<=",
                    HirRelationalPatternOp::GreaterThan => ">",
                    HirRelationalPatternOp::GreaterThanOrEqual => ">=",
                };
                format!("{scrutinee_expression} {js_op} {rhs}")
            }
            HirPattern::ChoiceVariant { variant_index, .. } => {
                format!("{scrutinee_expression}.tag === {variant_index}")
            }
        };

        if let Some(guard) = arm.guard {
            let guard = self.lower_expr(guard)?;
            Ok(format!("({pattern_condition}) && ({guard})"))
        } else {
            Ok(pattern_condition)
        }
    }
}

/// Selects the checked JS helper family and domain bounds for one HIR numeric operation.
fn js_numeric_helper_for_op(op: HirNumericOp) -> (String, Option<(String, String)>) {
    let carrier = JsNumericCarrier::for_scalar(op.domain).unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "JS backend has no numeric carrier for {:?}",
            op.domain
        )
    });

    let operation = match (carrier, op.operator) {
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Add,
        ) => "add",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Subtract,
        ) => "sub",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Multiply,
        ) => "mul",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::IntegerDivide,
        ) => "div",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Remainder,
        ) => "mod",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Power,
        ) => "pow",
        (
            JsNumericCarrier::ExactInteger { .. } | JsNumericCarrier::BigInteger { .. },
            NumericOperator::Negate,
        ) => "neg",
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Add) => "add",
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Subtract) => "sub",
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Multiply) => "mul",
        (JsNumericCarrier::ScaledInteger { scale }, NumericOperator::Divide) if scale.get() > 0 => {
            "div"
        }
        (JsNumericCarrier::ScaledInteger { scale }, NumericOperator::IntegerDivide)
            if scale.get() == 0 =>
        {
            "idiv"
        }
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Remainder) => "mod",
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Power) => "pow",
        (JsNumericCarrier::ScaledInteger { .. }, NumericOperator::Negate) => "neg",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Add) => "add",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Subtract) => "sub",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Multiply) => "mul",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Divide) => "div",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Remainder) => "mod",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Power) => "pow",
        (JsNumericCarrier::BinaryFloat { .. }, NumericOperator::Negate) => "neg",
        _ => crate::compiler_bug!(
            CompilerStage::BackendLowering,
            "JS backend received unreachable numeric operation {op}"
        ),
    };
    let family = carrier.helper_family();

    (
        format!("__moth_{family}_{operation}"),
        carrier.integer_bounds_js(),
    )
}
