//! Statement lowering helpers for the JavaScript backend.
//!
//! These routines emit block-local statements after HIR has already made evaluation order and
//! control-flow edges explicit.

use crate::backends::js::JsEmitter;
use crate::backends::js::js_expr::{escape_js_string, js_cast_expression_for_policy};
use crate::backends::js::numeric_carrier::{
    JsFastStoreConversion, JsNumericCarrier, JsNumericConversion, JsNumericOperationLowering,
    binary_float_precision_bits,
};
use crate::backends::js::runtime::fatal_numeric_trap_throw;
use crate::backends::js::value_use::JsValueUse;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, compiler_errors::InfrastructureFailure,
};
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

/// The delivery context in which one numeric operation is classified.
///
/// WHAT: a `NumericOp` statement carries the failure mode the frontend recorded for it; an
///       `Invoke` operation has no statement mode.
/// WHY: only statement delivery can carry the frontend's mandatory `Infallible` totality
///      (operand-range discharge or a dominating guard), so the classifier needs that fact
///      alongside the optional proof table. Naming the delivery keeps every caller from passing
///      a positional boolean that the next caller could invert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericDelivery {
    /// A `NumericOp` statement, with the failure mode the frontend recorded for it.
    Statement(NumericFailureMode),
    /// A recoverable `Invoke` operation, which carries no statement failure mode.
    Invoke,
}

impl NumericDelivery {
    /// Whether the frontend already established this delivery's totality.
    fn is_discharged(self) -> bool {
        matches!(self, Self::Statement(NumericFailureMode::Infallible))
    }
}

/// How one lowered Invoke delivers its operation value to the success edge.
///
/// WHAT: `Captured` holds the value in the shared `__invoke_result` temp that the success edge
///       transfers from; `StoredScalar` is a scalar numeric or proven integer-narrowing result
///       that `prepare_invoke` already stored in its single success destination.
/// WHY: a scalar operation result has no live failure edge and no transfer, so naming that
///      delivery keeps callers from reading a temp that was never emitted.
pub(crate) enum PreparedInvoke {
    Captured { value: String },
    StoredScalar,
}

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
                // WHAT: dispatch the operation through the shared numeric classifier.
                // WHY: scalar fast, total and discharged results are assigned directly, while
                //      only a checked carrier goes through `__moth_numeric_trap`; the statement
                //      mode carries the mandatory `Infallible` totality, not a second delivery test.
                self.emit_numeric_op_statement(
                    *op,
                    *failure_mode,
                    operands,
                    *result,
                    statement.id,
                )?;
            }

            HirStatementKind::FastStoreConversion {
                conversion,
                source,
                result,
            } => {
                // WHAT: lower the fast write-back directly to its helper expression.
                // WHY: these conversions never return the checked `{ tag, value }` carrier and
                //      never route through `__moth_numeric_trap`; the trapping helpers throw the
                //      canonical fatal message before the enclosing store executes.
                let source = self.lower_expr(*source)?;
                let conversion = JsFastStoreConversion::classify(*conversion)?;
                self.emit_value_destination(*result, &conversion.expression(&source))?;
            }

            HirStatementKind::LoopStepFailure {
                cause,
                semantics,
                result,
                ..
            } => {
                let error_code = cause.builtin_error_code();
                if !semantics.is_fast() {
                    let helper_call = format!(
                        "__moth_error_result({:?}, {})",
                        error_code.default_message(),
                        error_code.as_u32(),
                    );
                    self.emit_numeric_carrier_assignment(helper_call, *result)?;
                } else {
                    // A fast range guard is a fatal trap: throw the canonical cause directly
                    // instead of constructing the recoverable `{ tag, value }` carrier that a
                    // Moth handler or an inferred private lane could consume.
                    self.emit_line(&fatal_numeric_trap_throw(error_code));
                }
            }
            HirStatementKind::RangeCandidate {
                current,
                step,
                end,
                inclusive,
                domain,
                candidate_result,
                in_range_result,
            } => {
                self.emit_range_candidate_statement(
                    [*current, *step, *end],
                    *inclusive,
                    *domain,
                    *candidate_result,
                    *in_range_result,
                )?;
            }
            HirStatementKind::FormatFloat { source, result, .. } => {
                // WHAT: format at the source precision and consume the helper's ABI envelope.
                // WHY: binary-float formatting is a language builtin with an explicit failure mode; the
                //      backend must map it to the helper that normalizes JS number text to the
                //      Moth contract instead of using target-native stringification directly.
                self.emit_format_float_statement(*source, *result)?;
            }

            HirStatementKind::ValidateFloat { source, result, .. } => {
                // WHAT: validates a finite binary-float at its exact binary precision.
                // WHY: external values enter the same finite, rounded carrier as other binary floats.
                self.emit_validate_float_statement(*source, *result)?;
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

    /// Lower one numeric statement and assign its result.
    ///
    /// WHAT: a plain scalar value is assigned directly, while a checked carrier is consumed
    ///       through `__moth_numeric_trap`.
    /// WHY: the shared classifier already decided the value shape, so delivery only picks the
    ///      matching assignment form.
    fn emit_numeric_op_statement(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: &HirNumericOperands,
        result: HirLocalDestination,
        statement: HirNodeId,
    ) -> Result<(), InfrastructureFailure> {
        let (value, plain_scalar) = self.lower_numeric_operation(
            op,
            operands,
            statement,
            NumericDelivery::Statement(failure_mode),
        )?;
        if plain_scalar {
            self.emit_value_destination(result, &value)
        } else {
            self.emit_numeric_carrier_assignment(value, result)
        }
    }

    /// Classify one numeric operation through the shared lowering owner.
    ///
    /// WHAT: combines the statement's mandatory `Infallible` totality with the optional sparse
    ///       proof table into the single lazy proof input the carrier classifier consults.
    /// WHY: statement lowering, the prelude demand scan and Invoke edge liveness must reach the
    ///      same fast/scalar/proven/checked decision. A discharged statement answers the proof
    ///      input directly, so it never consults the sparse table.
    pub(crate) fn classify_numeric_operation(
        &self,
        op: HirNumericOp,
        node: HirNodeId,
        delivery: NumericDelivery,
    ) -> Result<JsNumericOperationLowering, InfrastructureFailure> {
        JsNumericOperationLowering::classify(op, || {
            delivery.is_discharged() || self.numeric_proofs.integer_operation_is_safe(node)
        })
    }

    /// Classify and render one HIR numeric operation over its lowered operands.
    ///
    /// WHAT: validates HIR arity, lowers each operand once in source order, then lets the carrier
    ///       owner choose fast, scalar, proven-integer or checked lowering. The flag reports
    ///       whether the rendered value is a plain scalar rather than a carrier.
    /// WHY: the same classifier decides the prelude demand scan, so statement lowering cannot
    ///      call a helper the prelude omitted or skip one it demanded.
    fn lower_numeric_operation(
        &mut self,
        op: HirNumericOp,
        operands: &HirNumericOperands,
        operation: HirNodeId,
        delivery: NumericDelivery,
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
        let lowered_args = match operands {
            HirNumericOperands::Unary { operand } => vec![self.lower_expr(*operand)?],
            HirNumericOperands::Binary { left, right } => {
                vec![self.lower_expr(*left)?, self.lower_expr(*right)?]
            }
        };

        let lowering = self.classify_numeric_operation(op, operation, delivery)?;

        Ok((lowering.expression(lowered_args), lowering.is_scalar()))
    }

    /// Return the raw carrier conversion for a proven-safe integer narrowing Invoke.
    ///
    /// Demand scans, edge liveness and lowering share this query so they agree on both the
    /// checked helper and the failure edge that can be omitted.
    pub(crate) fn proven_integer_narrowing(
        &self,
        operation: HirNodeId,
        policy: BuiltinCastPolicyId,
    ) -> Option<(JsNumericConversion, NumericScalar, NumericScalar)> {
        if !self.numeric_proofs.integer_narrowing_is_safe(operation) {
            return None;
        }
        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return None;
        };
        let conversion = JsNumericConversion::classify(source, target).ok()?;
        let source_carrier = JsNumericCarrier::for_scalar(source)?;
        let conversion = conversion.proven_safe_integer_narrowing(source_carrier)?;
        Some((conversion, source, target))
    }

    /// Emit one range candidate, dispatching on the counter domain.
    ///
    /// WHAT: integer domains use exact arithmetic on the counter and step carriers; binary floats
    ///       round the signed successor at the counter precision.
    /// WHY: only a finite in-bound successor is committed, so an integer lowering never
    ///      materialises an out-of-domain value and a non-finite float never becomes a counter
    ///      value.
    fn emit_range_candidate_statement(
        &mut self,
        expressions: [HirValueId; 3],
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: HirLocalDestination,
        in_range_result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        if domain.is_integer() {
            return self.emit_integer_range_candidate_statement(
                expressions,
                inclusive,
                domain,
                candidate_result,
                in_range_result,
            );
        }

        let [current, step, end] = expressions;

        let current_expr = self.lower_expr(current)?;
        let step_expr = self.lower_expr(step)?;
        let end_expr = self.lower_expr(end)?;
        let precision = domain.binary_float_precision().unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS backend received a non-float range candidate domain {domain:?}"
            )
        });

        let step_name = self.next_temp_identifier("__range_candidate_step");
        let direction_name = self.next_temp_identifier("__range_candidate_ascending");
        let candidate_name = self.next_temp_identifier("__range_candidate_value");
        let in_range_name = self.next_temp_identifier("__range_candidate_valid");
        let unrounded_candidate = format!("({current_expr} + {step_name})");
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
        let step_assignment = format!("const {step_name} = {step_expr};");
        let direction_assignment = format!("const {direction_name} = {step_name} > 0;");
        let candidate_assignment = format!("const {candidate_name} = {candidate_expr};");
        let in_range_assignment =
            format!("const {in_range_name} = Number.isFinite({candidate_name}) && {bound_check};");

        self.emit_line(&step_assignment);
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

    /// Emit the exact integer range candidate.
    ///
    /// WHAT: adds the signed step to the counter with exact arithmetic, selects the bound
    ///       comparison from the step sign and commits the successor only when in range.
    /// WHY: a signed step cannot be added through the counter domain without overflow or a
    ///      narrowing cast, and a mixed Number/BigInt carrier pair would need a runtime type
    ///      test. Both carriers are static HIR facts, so `BigInt` arithmetic is selected
    ///      whenever either operand is wide and exact `Number` arithmetic stays available when
    ///      both are at most 32 bits.
    fn emit_integer_range_candidate_statement(
        &mut self,
        expressions: [HirValueId; 3],
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: HirLocalDestination,
        in_range_result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let [current, step, end] = expressions;
        let current_expr = self.lower_expr(current)?;
        let step_expr = self.lower_expr(step)?;
        let end_expr = self.lower_expr(end)?;

        let counter_carrier = self.integer_range_candidate_carrier(domain, "counter");
        let step_type = self.hir.expressions.expression(step).ty;
        let step_scalar = NumericScalar::from_type_id(step_type, self.type_environment)
            .unwrap_or_else(|| {
                crate::compiler_bug!(
                    CompilerStage::BackendLowering,
                    "JS backend received a range candidate step without a numeric type"
                )
            });
        let step_carrier = self.integer_range_candidate_carrier(step_scalar, "step");

        let counter_is_bigint = matches!(counter_carrier, JsNumericCarrier::BigInteger { .. });
        let step_is_bigint = matches!(step_carrier, JsNumericCarrier::BigInteger { .. });
        // A Number carrier is exact only up to 32 bits, so one wide operand moves every value
        // onto the exact BigInt carrier.
        let arithmetic_is_bigint = counter_is_bigint || step_is_bigint;

        let current_name = self.next_temp_identifier("__range_candidate_current");
        let step_name = self.next_temp_identifier("__range_candidate_step");
        let end_name = self.next_temp_identifier("__range_candidate_end");
        let next_name = self.next_temp_identifier("__range_candidate_next");
        let in_range_name = self.next_temp_identifier("__range_candidate_valid");

        self.emit_line(&format!("const {current_name} = {current_expr};"));
        self.emit_line(&format!("const {step_name} = {step_expr};"));
        self.emit_line(&format!("const {end_name} = {end_expr};"));

        let (arithmetic_current, arithmetic_step, arithmetic_end) = if arithmetic_is_bigint {
            let mut promote_to_bigint = |operand: &str| {
                let promoted = self.next_temp_identifier("__range_candidate_wide");
                self.emit_line(&format!("const {promoted} = BigInt({operand});"));
                promoted
            };
            let arithmetic_current = if counter_is_bigint {
                current_name.clone()
            } else {
                promote_to_bigint(&current_name)
            };
            let arithmetic_step = if step_is_bigint {
                step_name.clone()
            } else {
                promote_to_bigint(&step_name)
            };
            let arithmetic_end = if counter_is_bigint {
                end_name.clone()
            } else {
                promote_to_bigint(&end_name)
            };
            (arithmetic_current, arithmetic_step, arithmetic_end)
        } else {
            (current_name.clone(), step_name.clone(), end_name.clone())
        };

        let zero_literal = if arithmetic_is_bigint { "0n" } else { "0" };
        let (ascending_comparison, descending_comparison) =
            if inclusive { ("<=", ">=") } else { ("<", ">") };
        self.emit_line(&format!(
            "const {next_name} = {arithmetic_current} + {arithmetic_step};"
        ));
        self.emit_line(&format!(
            "const {in_range_name} = ({arithmetic_step} > {zero_literal}) ? ({next_name} {ascending_comparison} {arithmetic_end}) : ({next_name} {descending_comparison} {arithmetic_end});"
        ));

        self.emit_value_destination(in_range_result, &in_range_name)?;
        self.emit_line(&format!("if ({in_range_name}) {{"));
        self.indent += 1;
        // The guard proved the successor fits the counter domain, so a Number carrier stays
        // exact when converted back from BigInt arithmetic.
        let candidate_value = if arithmetic_is_bigint && !counter_is_bigint {
            format!("Number({next_name})")
        } else {
            next_name.clone()
        };
        self.emit_value_destination(candidate_result, &candidate_value)?;
        self.indent -= 1;
        self.emit_line("}");

        Ok(())
    }

    /// The JS carrier one integer range candidate operand uses.
    ///
    /// WHY: the candidate selects its arithmetic from static HIR types, so a non-integer or
    ///      unbounded carrier is an internal contradiction rather than a runtime fallback.
    fn integer_range_candidate_carrier(
        &self,
        scalar: NumericScalar,
        operand: &'static str,
    ) -> JsNumericCarrier {
        match JsNumericCarrier::for_scalar(scalar) {
            Some(
                carrier @ (JsNumericCarrier::ExactInteger { .. }
                | JsNumericCarrier::BigInteger { .. }),
            ) => carrier,
            _ => crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JS range candidate {operand} is not an integer domain: {scalar}"
            ),
        }
    }

    /// Lower a `HirStatementKind::FormatFloat` into the exact-precision binary-float formatter.
    ///
    /// WHAT: derives the source precision and canonical name, then consumes the runtime
    /// formatter's ABI envelope as a scalar string.
    /// WHY: template and cast formatting share one precision-aware helper and result contract,
    ///      and the source row owns the resolved precision.
    fn emit_format_float_statement(
        &mut self,
        source: HirValueId,
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let source_expr = self.lower_expr(source)?;
        let (scalar, precision) = self.float_source_scalar(source, "FormatFloat");
        let helper_call = format!(
            "__moth_format_float({source_expr}, {}, \"{scalar}\")",
            binary_float_precision_bits(precision)
        );
        self.emit_numeric_carrier_assignment(helper_call, result)
    }

    /// Lower a `HirStatementKind::ValidateFloat` into the exact-precision finite binary-float helper.
    ///
    /// WHAT: derives the validated source's own binary-float precision and emits it as the
    ///       helper's precision argument.
    /// WHY: a validated boundary value carries a fixed `F16`/`F32`/`F64` type, so the exact
    ///      source type owns the rounding step.
    fn emit_validate_float_statement(
        &mut self,
        source: HirValueId,
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        let source_expr = self.lower_expr(source)?;
        let (_scalar, precision) = self.float_source_scalar(source, "binary-float validation");
        let helper_call = format!(
            "__moth_float_validate({source_expr}, {})",
            binary_float_precision_bits(precision)
        );
        self.emit_numeric_carrier_assignment(helper_call, result)
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
        result: HirLocalDestination,
    ) -> Result<(), InfrastructureFailure> {
        self.emit_value_destination(result, &format!("__moth_numeric_trap({helper_call})"))
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
                // A computed place captures the storage its base held when it was constructed, so
                // the definition stays rooted in the allocation the projection named here even if
                // the source binding cell is rebound later. Binding through an alias preserves
                // write-through to that captured field or element.
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

    /// Lower one Invoke operation once and describe how its value reaches the success edge.
    ///
    /// WHAT: emits the capture and slot clearing a live edge needs, or stores a scalar numeric
    ///       or proven integer-narrowing result directly in its single success destination, then
    ///       returns the [`PreparedInvoke`] the caller selects its branch shape from.
    /// WHY: callers must not re-derive whether a scalar result has a live failure edge; the same
    ///      classification drives branch shape, slot clearing and live-edge pruning. A direct
    ///      assignment evaluates its right-hand side before storing, so operands still read the
    ///      previous binding.
    pub(crate) fn prepare_invoke(
        &mut self,
        id: HirNodeId,
        operation: &HirFallibleOperation,
        success_results: &[LocalId],
        error_result: LocalId,
    ) -> Result<PreparedInvoke, InfrastructureFailure> {
        let value = match operation {
            HirFallibleOperation::Call { target, args } => {
                self.lower_call_expression(target, *args)?
            }
            HirFallibleOperation::MapOp { op, receiver, args } => {
                self.lower_map_operation(*op, *receiver, *args)?
            }
            HirFallibleOperation::NumericOp { op, operands } => {
                let (value, scalar) =
                    self.lower_numeric_operation(*op, operands, id, NumericDelivery::Invoke)?;
                if scalar {
                    // A scalar numeric result has no live failure edge, so the operation's value
                    // initialises its one success destination directly. HIR validation fixes a
                    // numeric Invoke at one success destination, and the assignment evaluates its
                    // right-hand side before storing, so operands still read the previous binding
                    // without a staging temp or a clear-then-write pair.
                    self.store_scalar_invoke_result(success_results, &value, "numeric")?;
                    return Ok(PreparedInvoke::StoredScalar);
                }
                value
            }
            HirFallibleOperation::CastOp { policy, source } => {
                let source = self.lower_expr(*source)?;
                if let Some((conversion, source_type, target)) =
                    self.proven_integer_narrowing(id, *policy)
                {
                    let value = conversion.expression(&source, source_type, target);
                    // A proven narrowing performs the same physical Number/BigInt conversion as
                    // the checked helper, but its range predicate can never fail. The same proof
                    // query makes the failure edge dead, so this is the scalar-Invoke path.
                    self.store_scalar_invoke_result(success_results, &value, "narrowing")?;
                    return Ok(PreparedInvoke::StoredScalar);
                }
                js_cast_expression_for_policy(*policy, &source)?
            }
            HirFallibleOperation::LoopStepFailure { cause } => {
                let code = cause.builtin_error_code();
                format!(
                    "__moth_error_result({:?}, {})",
                    code.default_message(),
                    code.as_u32()
                )
            }
            HirFallibleOperation::FormatFloat { source } => {
                let value = self.lower_expr(*source)?;
                let (scalar, precision) = self.float_source_scalar(*source, "FormatFloat");
                format!(
                    "__moth_format_float({value}, {}, \"{scalar}\")",
                    binary_float_precision_bits(precision)
                )
            }
            HirFallibleOperation::ValidateFloat { source } => {
                let value = self.lower_expr(*source)?;
                let (_, precision) = self.float_source_scalar(*source, "ValidateFloat");
                format!(
                    "__moth_float_validate({value}, {})",
                    binary_float_precision_bits(precision)
                )
            }
        };
        let captured = self.next_temp_identifier("__invoke_result");
        self.emit_line(&format!("const {captured} = {value};"));
        // Clear both outcomes on every execution, including repeated loop visits. Capture the
        // operation first because its operands may read a previous edge-local binding. A dead
        // failure edge never reads its slot, so only a reachable one is cleared.
        for local in success_results.iter().chain(std::iter::once(&error_result)) {
            let name = self.local_name(*local)?.to_owned();
            self.emit_line(&format!("{name} = undefined;"));
        }
        Ok(PreparedInvoke::Captured { value: captured })
    }

    /// Store one scalar Invoke result in its sole success destination.
    ///
    /// WHY: a scalar numeric or proven-narrowing result has no live failure edge, so its value
    ///      belongs in the success slot directly. HIR validation fixes these Invokes at one
    ///      success destination, and the storing assignment evaluates its right-hand side first.
    fn store_scalar_invoke_result(
        &mut self,
        success_results: &[LocalId],
        value: &str,
        kind: &str,
    ) -> Result<(), InfrastructureFailure> {
        let [result] = success_results else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: scalar {kind} Invoke must have exactly one success destination"
            )
        };
        self.emit_value_destination(HirLocalDestination::Define(*result), value)
    }

    /// Store a captured operation value in its success destinations.
    ///
    /// A scalar operation result never reaches this transfer: `prepare_invoke` stores it in place
    /// and no capture temp exists for it.
    pub(crate) fn emit_invoke_success_transfer(
        &mut self,
        captured: &str,
        results: &[LocalId],
    ) -> Result<(), InfrastructureFailure> {
        for (index, result) in results.iter().enumerate() {
            let value = if results.len() == 1 {
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
        let Some(returns) = function.return_contract.ordinary() else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: Return emitted in a Never function; HIR validation rejects this"
            );
        };
        if expressions.len() != returns.success().len()
            || expressions
                .iter()
                .zip(returns.success())
                .any(|(value, ty)| self.hir.expressions.expression(*value).ty != *ty)
        {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: Return slots mismatch"
            );
        }
        let fallible = returns.is_fallible();
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
        let Some(error_type) = function
            .return_contract
            .ordinary()
            .and_then(|returns| returns.error())
        else {
            crate::compiler_bug!(
                CompilerStage::BackendLowering,
                "JavaScript backend: ReturnError emitted in a function without an ordinary error slot"
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
                let prepared =
                    self.prepare_invoke(*id, operation, success_results, *error_result)?;
                match prepared {
                    PreparedInvoke::StoredScalar => {
                        // A scalar operation result always succeeds and `prepare_invoke` has
                        // already stored its value, so the failure edge cannot execute: continue
                        // to the success block.
                        self.emit_line(&format!("{state_identifier} = {};", success.0));
                        self.emit_line("continue;");
                    }
                    PreparedInvoke::Captured { value: captured } => {
                        let condition = format!("{captured}.tag === \"ok\"");
                        self.emit_line(&format!("if ({condition}) {{"));
                        self.indent += 1;
                        self.emit_invoke_success_transfer(&captured, success_results)?;
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
                }
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

            HirTerminator::NeverCall { target, args } => {
                // The fallback throws, so dispatcher control cannot continue to another block.
                self.emit_never_call_terminator(target, *args)?;
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
            HirPattern::OptionRelational { op, value } | HirPattern::Relational { op, value } => {
                let compared_type = self.hir.expressions.expression(*value).ty;
                let rhs = self.lower_expr(*value)?;
                let js_op = match op {
                    HirRelationalPatternOp::LessThan => "<",
                    HirRelationalPatternOp::LessThanOrEqual => "<=",
                    HirRelationalPatternOp::GreaterThan => ">",
                    HirRelationalPatternOp::GreaterThanOrEqual => ">=",
                };
                if matches!(arm.pattern, HirPattern::OptionRelational { .. }) {
                    let left = format!("({scrutinee_expression}).value");
                    let comparison =
                        self.lower_ordered_comparison(&left, &rhs, compared_type, js_op);
                    format!("((({scrutinee_expression}).tag === \"some\") && {comparison})")
                } else {
                    self.lower_ordered_comparison(scrutinee_expression, &rhs, compared_type, js_op)
                }
            }
            HirPattern::Wildcard => "true".to_owned(),
            HirPattern::OptionPresent => {
                format!("({scrutinee_expression}).tag === \"some\"")
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
