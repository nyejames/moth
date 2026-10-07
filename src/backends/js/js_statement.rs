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
use crate::compiler_frontend::analysis::borrow_checker::LocalMode;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, HirMapOp};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_block_statements(
        &mut self,
        block: &crate::compiler_frontend::hir::blocks::HirBlock,
    ) -> Result<(), CompilerError> {
        for statement in &block.statements {
            self.emit_statement(statement)?;
        }

        Ok(())
    }

    pub(crate) fn emit_statement(&mut self, statement: &HirStatement) -> Result<(), CompilerError> {
        match &statement.kind {
            HirStatementKind::Assign { target, value } => {
                self.emit_assignment(statement, target, value)?;
            }

            HirStatementKind::Call {
                target,
                args,
                result,
            } => {
                self.emit_call_statement(target, args, result)?;
            }

            HirStatementKind::CastOp {
                policy,
                source,
                result,
            } => {
                let source_expr = self.lower_expr(source)?;
                let cast_expression =
                    self.lower_cast_op_expression(*policy, &source_expr, statement.id)?;
                if let Some(result_local) = result {
                    let result_name = self.local_name(*result_local)?;
                    self.emit_line(&format!(
                        "__moth_assign_value({result_name}, {cast_expression});"
                    ));
                } else {
                    self.emit_line(&format!("{cast_expression};"));
                }
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
                self.emit_map_op_statement(*op, receiver, args, result)?;
            }

            HirStatementKind::NumericOp {
                op,
                failure_mode,
                operands,
                result,
            } => {
                // WHAT: dispatch a checked operation through the helper family selected from its
                //       semantic result domain.
                // WHY: the profile's carrier determines both JS arithmetic and domain bounds.
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
                    [current, step, end, ascending],
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
                // WHAT: dispatch a finite-Float formatting operation to the JS runtime helper that
                //       implements Moth's formatting contract, then either wrap the carrier for
                //       trap mode or assign the carrier directly for builtin-Error recovery mode.
                // WHY: Float formatting is a language builtin with an explicit failure mode; the
                //      backend must map it to the helper that normalizes JS number text to the
                //      Moth contract instead of using target-native stringification directly.
                self.emit_format_float_statement(*failure_mode, source, *result)?;
            }

            HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            } => {
                // WHAT: validates a finite Float at its selected profile precision.
                // WHY: external values enter the same finite, rounded carrier as every other Float.
                self.emit_validate_float_statement(*failure_mode, source, *result)?;
            }

            HirStatementKind::Expr(expression) => {
                let expression = self.lower_expr(expression)?;
                self.emit_line(&format!("{expression};"));
            }

            HirStatementKind::Drop(_) => {
                // No-op for GC backend.
            }

            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                // WHAT: lower a fragment push into a JS vec push call against the unwrapped array.
                // WHY: locals are stored as binding wrappers `{ value: ... }` so `.push` cannot be
                //      called on the binding itself. __moth_read returns the underlying array.
                let vec_name = self.local_name(*vec_local)?.to_owned();
                let value_expr =
                    self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                self.emit_line(&format!("__moth_read({vec_name}).push({value_expr});"));
            }
        }

        Ok(())
    }

    /// Lower a `HirStatementKind::MapOp` into the appropriate runtime helper call.
    ///
    /// WHAT: dispatches `get`, `contains`, `set`, `remove`, `clear`, and `length` to their
    /// corresponding `__moth_map_*` helpers, validates arity against the HIR contract, and emits
    /// a result assignment when the statement carries a destination local.
    /// WHY: map operations are language builtins, not external calls; the backend must map them
    ///      to the JS runtime helpers that enforce the branded-map representation.
    fn emit_map_op_statement(
        &mut self,
        op: HirMapOp,
        receiver: &HirExpression,
        args: &[HirExpression],
        result: &Option<LocalId>,
    ) -> Result<(), CompilerError> {
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
            return Err(CompilerError::compiler_error(format!(
                "JS backend received MapOp::{op:?} with {actual} args instead of {expected}",
                actual = args.len(),
                expected = expected_arity,
            )));
        }

        // Lower each HIR argument to a JS expression.
        let mut lowered_args = Vec::with_capacity(args.len());
        for arg in args {
            lowered_args.push(self.lower_expr(arg)?);
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

        // Emit either an assignment to a destination local or a standalone call.
        if let Some(result_local) = result {
            let result_name = self.local_name(*result_local)?;
            self.emit_line(&format!("__moth_assign_value({result_name}, {call});"));
        } else {
            self.emit_line(&format!("{call};"));
        }

        Ok(())
    }

    /// Lower a checked HIR numeric operation through its profile-selected carrier family.
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
        result: LocalId,
        statement: HirNodeId,
    ) -> Result<(), CompilerError> {
        // Guard against arity mismatch between HIR and the backend.
        let is_unary = op.is_unary();
        let operands_are_unary = matches!(operands, HirNumericOperands::Unary { .. });
        if is_unary != operands_are_unary {
            return Err(CompilerError::compiler_error(format!(
                "JS backend received NumericOp::{op} with operand arity that does not match the operation"
            )));
        }

        // Lower each HIR operand to a JS expression once, in source order.
        let mut lowered_args = match operands {
            HirNumericOperands::Unary { operand } => vec![self.lower_expr(operand)?],
            HirNumericOperands::Binary { left, right } => {
                vec![self.lower_expr(left)?, self.lower_expr(right)?]
            }
        };

        let carrier = JsNumericCarrier::for_scalar(op.domain, self.config.numeric_profile)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JS backend has no numeric carrier for {:?}",
                    op.domain
                ))
            })?;

        // WHAT: proven-safe integer operations skip the checked helper and its runtime range
        //       predicate entirely. Trap mode assigns the raw scalar result; ReturnError mode
        //       assigns the same native result inside the existing `{tag, value}` success
        //       carrier its unchanged HIR consumers branch on.
        // WHY: the numeric proof table guarantees every relevant failure is impossible, so the
        //       checked machinery would be unreachable at runtime while the carrier itself stays
        //       required for recovery-mode consumers.
        if self
            .numeric_proofs
            .integer_operation_is_safe(statement, self.config.numeric_profile)
            && let Some(proven_expression) =
                self.proven_integer_operation_expression(op, &carrier, &lowered_args)
        {
            let assigned_value = match failure_mode {
                NumericFailureMode::Trap | NumericFailureMode::Infallible => proven_expression,
                NumericFailureMode::ReturnError => {
                    format!("{{ tag: \"ok\", value: {proven_expression} }}")
                }
            };
            let result_name = self.local_name(result)?;
            self.emit_line(&format!(
                "__moth_assign_value({result_name}, {assigned_value});"
            ));
            return Ok(());
        }

        let (helper_name, bounds) = js_numeric_helper_for_op(op, self.config.numeric_profile)?;
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

        let helper_call = format!("{helper_name}({})", lowered_args.join(", "));
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Renders exact native-carrier arithmetic for one proven-safe integer operation.
    ///
    /// WHAT: proven operations evaluate their already-lowered operands once, in source order, and
    ///       produce the raw scalar with no checked helper call. Trap mode assigns that raw
    ///       scalar directly; ReturnError mode wraps it in the existing `{tag, value}` success
    ///       carrier. Number carriers use exact JS Number arithmetic because the proven result
    ///       lies inside its semantic domain (at most U32 wide, well below the exact-Number
    ///       limit); 64-bit carriers stay on exact BigInt arithmetic with the same truncating
    ///       division and dividend-signed remainder semantics the checked helpers implement.
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

    /// Lowers one CastOp policy, eliding the checked-narrowing helpers when the proof table
    /// proves the narrowing safe.
    fn lower_cast_op_expression(
        &mut self,
        policy: BuiltinCastPolicyId,
        value: &str,
        statement: HirNodeId,
    ) -> Result<String, CompilerError> {
        if let Some(proven_expression) =
            self.proven_narrowing_expression(policy, value, statement)?
        {
            return Ok(proven_expression);
        }

        js_cast_expression_for_policy(policy, value, self.config.numeric_profile)
    }

    /// Renders the carrier-only conversion for one proven-safe fallible integer narrowing.
    ///
    /// WHAT: proven narrowing keeps the original Number/BigInt carrier conversion and the
    ///       `{tag, value}` success-carrier shape the unchanged HIR requires for branch and
    ///       unwrap consumers, and omits only the range predicate the checked cast helper
    ///       performs.
    /// WHY: the proof table guarantees the source value is inside the target range, so the
    ///       checked helper's runtime range test is unreachable machinery, while the carrier
    ///       itself is still consumed downstream.
    fn proven_narrowing_expression(
        &self,
        policy: BuiltinCastPolicyId,
        value: &str,
        statement: HirNodeId,
    ) -> Result<Option<String>, CompilerError> {
        if !self.integer_narrowing_is_proven(statement, policy) {
            return Ok(None);
        }

        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return Ok(None);
        };
        let conversion = JsNumericConversion::classify(source, target, self.config.numeric_profile)
            .expect("proven narrowing policy must classify");
        let source_carrier = JsNumericCarrier::for_scalar(source, self.config.numeric_profile)
            .expect("classified narrowing source has a JS carrier");
        let proven = conversion
            .proven_safe_integer_narrowing(source_carrier)
            .expect("proven narrowing applies to checked integer conversions");

        Ok(Some(format!(
            "{{ tag: \"ok\", value: {} }}",
            proven.expression(value, source, target)
        )))
    }

    /// Whether one CastOp statement's numeric conversion is a proven-safe integer narrowing.
    ///
    /// WHAT: mirrors the statement lowering gate — only fallible integer-to-integer numeric
    ///       conversions whose statement the proof table proves are eligible.
    /// WHY: the demand scan and the statement lowering must agree exactly on which cast policies
    ///       no longer need their checked runtime helpers.
    pub(crate) fn integer_narrowing_is_proven(
        &self,
        statement: HirNodeId,
        policy: BuiltinCastPolicyId,
    ) -> bool {
        let BuiltinCastPolicyId::NumericConversion { source, target } = policy else {
            return false;
        };
        let Ok(conversion) =
            JsNumericConversion::classify(source, target, self.config.numeric_profile)
        else {
            return false;
        };
        if !matches!(
            conversion,
            JsNumericConversion::CheckedIntegerToInteger { .. }
        ) {
            return false;
        }

        self.numeric_proofs
            .integer_narrowing_is_safe(statement, self.config.numeric_profile)
    }

    /// Emit the range candidate into JS-local scratch, then commit only a finite in-bound value.
    fn emit_float_range_candidate_statement(
        &mut self,
        expressions: [&HirExpression; 4],
        inclusive: bool,
        domain: NumericScalar,
        candidate_result: LocalId,
        in_range_result: LocalId,
    ) -> Result<(), CompilerError> {
        let [current, step, end, ascending] = expressions;

        let current_expr = self.lower_expr(current)?;
        let step_expr = self.lower_expr(step)?;
        let end_expr = self.lower_expr(end)?;
        let ascending_expr = self.lower_expr(ascending)?;
        let precision = domain
            .binary_float_precision(self.config.numeric_profile)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JS backend received a non-float range candidate domain {domain:?}"
                ))
            })?;

        let direction_name = self.next_temp_identifier("__float_range_ascending");
        let candidate_name = self.next_temp_identifier("__float_range_candidate");
        let in_range_name = self.next_temp_identifier("__float_range_valid");
        let unrounded_candidate = format!(
            "({direction_name} ? ({current_expr} + {step_expr}) : ({current_expr} - {step_expr}))"
        );
        let candidate_expr = match precision {
            BinaryFloatPrecision::Binary32 => format!("Math.fround({unrounded_candidate})"),
            BinaryFloatPrecision::Binary64 => unrounded_candidate,
            BinaryFloatPrecision::Binary16 => {
                return Err(CompilerError::compiler_error(
                    "JS backend does not support F16 range candidates",
                ));
            }
        };
        let (ascending_comparison, descending_comparison) =
            if inclusive { ("<=", ">=") } else { ("<", ">") };
        let bound_check = format!(
            "({direction_name} ? ({candidate_name} {ascending_comparison} {end_expr}) : ({candidate_name} {descending_comparison} {end_expr}))"
        );
        let candidate_local = self.local_name(candidate_result)?;
        let in_range_local = self.local_name(in_range_result)?;
        let direction_assignment = format!("const {direction_name} = {ascending_expr};");
        let candidate_assignment = format!("const {candidate_name} = {candidate_expr};");
        let in_range_assignment =
            format!("const {in_range_name} = Number.isFinite({candidate_name}) && {bound_check};");
        let in_range_result_assignment =
            format!("__moth_assign_value({in_range_local}, {in_range_name});");
        let candidate_result_assignment = format!(
            "if ({in_range_name}) __moth_assign_value({candidate_local}, {candidate_name});"
        );

        self.emit_line(&direction_assignment);
        self.emit_line(&candidate_assignment);
        self.emit_line(&in_range_assignment);
        self.emit_line(&in_range_result_assignment);
        self.emit_line(&candidate_result_assignment);

        Ok(())
    }

    /// Lower a `HirStatementKind::FormatFloat` into the profile-precision binary-float formatter.
    ///
    /// WHAT: emits `__moth_format_float(source, precision, "Float")` and assigns either the scalar
    ///       formatted string (trap mode) or the fallible carrier (return-error mode).
    /// WHY: template and cast formatting share one precision-aware helper and result contract.
    fn emit_format_float_statement(
        &mut self,
        failure_mode: NumericFailureMode,
        source: &HirExpression,
        result: LocalId,
    ) -> Result<(), CompilerError> {
        let source_expr = self.lower_expr(source)?;
        let precision =
            binary_float_precision_bits(self.config.numeric_profile.float_precision.into());
        let helper_call = format!("__moth_format_float({source_expr}, {precision}, \"Float\")");
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Lower a `HirStatementKind::ValidateFloat` into the profile-aware finite-Float helper call.
    fn emit_validate_float_statement(
        &mut self,
        failure_mode: NumericFailureMode,
        source: &HirExpression,
        result: LocalId,
    ) -> Result<(), CompilerError> {
        let source_expr = self.lower_expr(source)?;
        let helper_call = format!("__moth_float_validate({source_expr})");
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Emit the result assignment shared by checked numeric helper calls.
    ///
    /// WHAT: wraps `helper_call` in `__moth_numeric_trap` for trap mode or assigns the carrier
    ///       directly for return-error mode, then assigns the value to `result`.
    /// WHY: `NumericOp`, `FormatFloat`, and `ValidateFloat` all use the same result-local carrier
    ///      contract; keeping the assignment logic in one helper prevents near-duplicate lowering
    ///      code for each statement kind.
    fn emit_numeric_carrier_assignment(
        &mut self,
        helper_call: String,
        failure_mode: NumericFailureMode,
        result: LocalId,
    ) -> Result<(), CompilerError> {
        let assigned_value = match failure_mode {
            NumericFailureMode::Trap | NumericFailureMode::Infallible => {
                format!("__moth_numeric_trap({helper_call})")
            }
            NumericFailureMode::ReturnError => helper_call,
        };

        let result_name = self.local_name(result)?;
        self.emit_line(&format!(
            "__moth_assign_value({result_name}, {assigned_value});"
        ));

        Ok(())
    }

    fn emit_assignment(
        &mut self,
        statement: &HirStatement,
        target: &HirPlace,
        value: &HirExpression,
    ) -> Result<(), CompilerError> {
        match target {
            HirPlace::Local(local_id) => self.emit_local_assignment(statement, *local_id, value),
            _ => {
                let target_ref = self.lower_place(target)?;
                let emitted_value =
                    self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                self.emit_line(&format!("__moth_write({target_ref}, {emitted_value});"));

                Ok(())
            }
        }
    }

    /// Emits a write to a local, choosing how it treats the binding the local already holds.
    ///
    /// WHY: JS declares one binding wrapper per local at function entry, but a declaration
    /// inside a re-entered block creates a new dynamic binding on every execution. Where the
    /// borrow analysis proves the local is unbound, this assignment creates that binding with a
    /// fresh wrapper. Reusing the old wrapper would write through an alias retained from an
    /// earlier execution, mutating that execution's referent.
    fn emit_local_assignment(
        &mut self,
        statement: &HirStatement,
        local_id: LocalId,
        value: &HirExpression,
    ) -> Result<(), CompilerError> {
        let local_name = self.local_name(local_id)?.to_owned();
        let mode = self.local_mode_before_statement(statement, local_id);
        let creates_binding = mode.is_some_and(LocalMode::is_definitely_uninit);
        let alias_only = mode.is_some_and(Self::snapshot_local_is_alias_only);

        match &value.kind {
            HirExpressionKind::Load(place) => {
                let source = self.lower_place(place)?;
                if creates_binding {
                    self.emit_line(&format!("{local_name} = __moth_alias_binding({source});"));
                } else if alias_only {
                    self.emit_line(&format!(
                        "__moth_write({local_name}, __moth_read({source}));",
                    ));
                } else {
                    self.emit_line(&format!("__moth_assign_borrow({local_name}, {source});"));
                }
            }
            _ => {
                let lowered = self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                if creates_binding {
                    self.emit_line(&format!("{local_name} = __moth_binding({lowered});"));
                } else if alias_only {
                    self.emit_line(&format!("__moth_write({local_name}, {lowered});"));
                } else {
                    self.emit_line(&format!("__moth_assign_value({local_name}, {lowered});"));
                }
            }
        }

        Ok(())
    }

    fn local_mode_before_statement(
        &self,
        statement: &HirStatement,
        local_id: LocalId,
    ) -> Option<LocalMode> {
        self.borrow_analysis
            .analysis
            .statement_entry_states
            .get(&statement.id)?
            .locals
            .iter()
            .find(|local| local.local == local_id)
            .map(|local| local.mode)
    }

    pub(crate) fn local_is_alias_only_at_block_entry(
        &self,
        block_id: BlockId,
        local_id: LocalId,
    ) -> bool {
        let Some(snapshot) = self
            .borrow_analysis
            .analysis
            .block_entry_states
            .get(&block_id)
        else {
            return false;
        };

        let Some(local_snapshot) = snapshot.locals.iter().find(|local| local.local == local_id)
        else {
            return false;
        };

        Self::snapshot_local_is_alias_only(local_snapshot.mode)
    }

    fn snapshot_local_is_alias_only(mode: LocalMode) -> bool {
        mode.contains(LocalMode::ALIAS) && !mode.contains(LocalMode::SLOT)
    }

    pub(crate) fn emit_return_terminator(
        &mut self,
        expression: &HirExpression,
    ) -> Result<(), CompilerError> {
        if self.is_unit_expression(expression) {
            self.emit_line("return;");
            return Ok(());
        }

        let value = self.lower_moth_return_value(expression)?;
        self.emit_line(&format!("return {value};"));
        Ok(())
    }

    pub(crate) fn emit_success_return_terminator(
        &mut self,
        expression: &HirExpression,
    ) -> Result<(), CompilerError> {
        let Some(function_id) = self.current_function else {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnSuccess emitted outside a function",
            ));
        };
        let function = self
            .hir
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JavaScript backend: current function {function_id:?} is missing"
                ))
            })?;
        let Some((success_type, _)) = self
            .type_environment
            .fallible_carrier_slots(function.return_type)
        else {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnSuccess emitted in a non-fallible function",
            ));
        };
        if expression.ty != success_type {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnSuccess value type does not match function success slot",
            ));
        }

        let value = self.lower_moth_return_value(expression)?;
        self.emit_line(&format!("return {{ tag: \"ok\", value: {value} }};"));
        Ok(())
    }

    pub(crate) fn emit_error_return_terminator(
        &mut self,
        expression: &HirExpression,
    ) -> Result<(), CompilerError> {
        let Some(function_id) = self.current_function else {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnError emitted outside a function",
            ));
        };
        let function = self
            .hir
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "JavaScript backend: current function {function_id:?} is missing"
                ))
            })?;
        let Some((_, error_type)) = self
            .type_environment
            .fallible_carrier_slots(function.return_type)
        else {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnError emitted in a non-fallible function",
            ));
        };
        if expression.ty != error_type {
            return Err(CompilerError::compiler_error(
                "JavaScript backend: ReturnError value type does not match function error slot",
            ));
        }

        let value = self.lower_expr(expression)?;
        self.emit_line(&format!("return {{ tag: \"err\", value: {value} }};"));
        Ok(())
    }

    pub(crate) fn emit_assert_failure_terminator(
        &mut self,
        message: &HirExpression,
        message_evaluation: HirAssertionMessageEvaluation,
    ) -> Result<(), CompilerError> {
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

    pub(crate) fn emit_runtime_failure_terminator(
        &mut self,
        message: &str,
    ) -> Result<(), CompilerError> {
        self.emit_line(&format!("throw new Error({});", escape_js_string(message)));

        Ok(())
    }

    pub(crate) fn emit_dispatcher_for_function(
        &mut self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
    ) -> Result<(), CompilerError> {
        let state_identifier = self.next_temp_identifier("__bb");

        self.emit_line(&format!("let {state_identifier} = {};", function.entry.0));
        self.emit_line("while (true) {");
        self.indent += 1;
        self.emit_line(&format!("switch ({state_identifier}) {{"));
        self.indent += 1;

        for block_id in reachable_blocks {
            let block = match self.block_by_id(*block_id) {
                Ok(block) => block.clone(),
                Err(error) => {
                    self.indent -= 2;
                    return Err(error);
                }
            };

            self.emit_line(&format!("case {}: {{", block.id.0));
            self.indent += 1;

            if let Err(error) = self.emit_block_statements(&block) {
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
    ) -> Result<(), CompilerError> {
        match terminator {
            HirTerminator::Jump { target, args } => {
                self.emit_jump_argument_transfer(*target, args)?;
                self.emit_line(&format!("{state_identifier} = {};", target.0));
                self.emit_line("continue;");
            }

            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.lower_expr(condition)?;
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

            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                let condition = self.lower_fallible_success_condition(result)?;
                self.emit_line(&format!("if ({condition}) {{"));
                self.with_indent(|emitter| {
                    emitter.emit_line(&format!("{state_identifier} = {};", success_block.0));
                });
                self.emit_line("} else {");
                self.with_indent(|emitter| {
                    emitter.emit_line(&format!("{state_identifier} = {};", error_block.0));
                });
                self.emit_line("}");
                self.emit_line("continue;");
            }

            HirTerminator::Match { scrutinee, arms } => {
                if arms.is_empty() {
                    return Err(CompilerError::compiler_error(
                        "JavaScript backend: Match terminator has no arms",
                    ));
                }
                let scrutinee_type = scrutinee.ty;
                let scrutinee = self.lower_expr(scrutinee)?;
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
                self.emit_return_terminator(value)?;
            }

            HirTerminator::ReturnSuccess(value) => {
                self.emit_success_return_terminator(value)?;
            }

            HirTerminator::ReturnError(value) => {
                self.emit_error_return_terminator(value)?;
            }

            HirTerminator::Uninitialized => {
                return Err(CompilerError::compiler_error(
                    "Uninitialized terminator reached JS backend lowering",
                ));
            }

            HirTerminator::RuntimeFailure { message, .. } => {
                self.emit_runtime_failure_terminator(message)?;
            }

            HirTerminator::AssertFailure {
                message,
                message_evaluation,
            } => {
                self.emit_assert_failure_terminator(message, *message_evaluation)?;
            }
        }

        Ok(())
    }

    pub(crate) fn lower_match_arm_condition(
        &mut self,
        scrutinee_expression: &str,
        scrutinee_type: TypeId,
        arm: &HirMatchArm,
    ) -> Result<String, CompilerError> {
        let pattern_condition = match &arm.pattern {
            HirPattern::Literal(value) => {
                let literal = self.lower_expr(value)?;
                self.lower_typed_equality(
                    scrutinee_expression.to_owned(),
                    scrutinee_type,
                    literal,
                    value.ty,
                )
            }
            HirPattern::OptionNone => {
                format!("({scrutinee_expression}).tag === \"none\"")
            }
            HirPattern::OptionValue { value } => {
                let literal = self.lower_expr(value)?;
                let inner_type = self
                    .type_environment
                    .option_inner_type(scrutinee_type)
                    .unwrap_or(value.ty);
                let inner_equality = self.lower_typed_equality(
                    format!("({scrutinee_expression}).value"),
                    inner_type,
                    literal,
                    value.ty,
                );
                format!("((({scrutinee_expression}).tag === \"some\") && {inner_equality})")
            }
            HirPattern::OptionRelational { op, value } => {
                let rhs = self.lower_expr(value)?;
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
                let rhs = self.lower_expr(value)?;
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

        if let Some(guard) = &arm.guard {
            let guard = self.lower_expr(guard)?;
            Ok(format!("({pattern_condition}) && ({guard})"))
        } else {
            Ok(pattern_condition)
        }
    }
}

/// Selects the checked JS helper family and domain bounds for one HIR numeric operation.
fn js_numeric_helper_for_op(
    op: HirNumericOp,
    numeric_profile: moth_lexical::numeric::profile::NumericProfile,
) -> Result<(String, Option<(String, String)>), CompilerError> {
    let carrier = JsNumericCarrier::for_scalar(op.domain, numeric_profile).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "JS backend has no numeric carrier for {:?}",
            op.domain
        ))
    })?;

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
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "JS backend received unreachable numeric operation {op}"
            )));
        }
    };
    let family = carrier.helper_family().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "JS backend received unreachable numeric operation domain {:?}",
            op.domain
        ))
    })?;

    Ok((
        format!("__moth_{family}_{operation}"),
        carrier.integer_bounds_js(),
    ))
}
