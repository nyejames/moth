//! Statement lowering helpers for the JavaScript backend.
//!
//! These routines emit block-local statements after HIR has already made evaluation order and
//! control-flow edges explicit.

use crate::backends::js::JsEmitter;
use crate::backends::js::js_expr::{escape_js_string, js_cast_helper_for_policy};
use crate::backends::js::value_use::JsValueUse;
use crate::compiler_frontend::analysis::borrow_checker::LocalMode;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, HirMapOp};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, HirNodeId, LocalId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern, HirRelationalPatternOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;

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
        self.emit_location_comment(&statement.location);

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
                let helper = js_cast_helper_for_policy(*policy).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JavaScript backend: CastOp policy {:?} has no runtime helper",
                        policy
                    ))
                })?;
                let call = format!("{helper}({source_expr})");
                if let Some(result_local) = result {
                    let result_name = self.local_name(*result_local)?;
                    self.emit_line(&format!("__moth_assign_value({result_name}, {call});"));
                } else {
                    self.emit_line(&format!("{call};"));
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
                // WHAT: dispatch a checked numeric operation to the JS runtime helper that mirrors
                //       AST const-eval semantics, then either wrap the carrier for trap mode or
                //       assign the carrier directly for builtin-Error recovery mode.
                // WHY: numeric operations are language builtins with explicit failure modes; the
                //      backend must map them to the helpers that enforce i32/finite-f64 semantics.
                self.emit_numeric_op_statement(*op, *failure_mode, operands, *result)?;
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
                // WHAT: dispatch a finite-Float boundary check to the JS runtime helper, then either
                //       wrap the carrier for trap mode or assign the carrier directly for
                //       builtin-Error recovery mode.
                // WHY: Moth `Float` is finite `f64`; values from external/backend boundaries
                //      must be validated before use, and the backend must expose that check through
                //      the same carrier contract as other checked numeric operations.
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
                //      Assignment value context preserves reactive template objects for Phase 7
                //      mounting instead of snapshotting them to plain strings here.
                let vec_name = self.local_name(*vec_local)?.to_owned();
                let value_expr =
                    self.lower_expression_for_use(value, JsValueUse::AssignmentValue)?;
                self.emit_line(&format!("__moth_read({vec_name}).push({value_expr});"));
            }
        }

        self.emit_reactive_invalidations_for_statement(statement.id);

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

    /// Lower a `HirStatementKind::NumericOp` into the appropriate checked runtime helper call.
    ///
    /// WHAT: dispatches `Int*` and `Float*` operations to their `__moth_int_*` / `__moth_float_*`
    ///       helpers, validates operand arity against the HIR contract, and emits the result
    ///       assignment. Trap mode wraps the helper's fallible carrier in `__moth_numeric_trap` so
    ///       the result local receives only the scalar success value; ReturnError mode assigns the
    ///       carrier directly.
    /// WHY: numeric operations are compiler-owned builtins with explicit failure modes; the backend
    ///      must map them to the JS helpers that enforce Alpha `Int = i32` and `Float = finite f64`
    ///      semantics.
    fn emit_numeric_op_statement(
        &mut self,
        op: HirNumericOp,
        failure_mode: NumericFailureMode,
        operands: &HirNumericOperands,
        result: LocalId,
    ) -> Result<(), CompilerError> {
        // Guard against arity mismatch between HIR and the backend.
        let is_unary = op.is_unary();
        let operands_are_unary = matches!(operands, HirNumericOperands::Unary { .. });
        if is_unary != operands_are_unary {
            return Err(CompilerError::compiler_error(format!(
                "JS backend received NumericOp::{op:?} with operand arity that does not match the operation"
            )));
        }

        // Lower each HIR operand to a JS expression.
        let lowered_args = match operands {
            HirNumericOperands::Unary { operand } => vec![self.lower_expr(operand)?],
            HirNumericOperands::Binary { left, right } => {
                vec![self.lower_expr(left)?, self.lower_expr(right)?]
            }
        };

        // Select the JS helper name for this operation.
        let helper_name = js_numeric_helper_for_op(op);

        // Assemble the helper call.
        let helper_call = format!("{helper_name}({})", lowered_args.join(", "));
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Lower a `HirStatementKind::FormatFloat` into the Moth Float formatting helper call.
    ///
    /// WHAT: emits `__moth_format_float(source)` and assigns either the scalar formatted string
    ///       (trap mode) or the fallible carrier (return-error mode) to the result local.
    /// WHY: formatting shares the same result-local carrier contract as `NumericOp`; trap mode
    ///      extracts the success value or throws, while return-error mode keeps the carrier for
    ///      later `FallibleBranch` lowering.
    fn emit_format_float_statement(
        &mut self,
        failure_mode: NumericFailureMode,
        source: &HirExpression,
        result: LocalId,
    ) -> Result<(), CompilerError> {
        let source_expr = self.lower_expr(source)?;
        let helper_call = format!("__moth_format_float({source_expr})");
        self.emit_numeric_carrier_assignment(helper_call, failure_mode, result)
    }

    /// Lower a `HirStatementKind::ValidateFloat` into the finite-Float validation helper call.
    ///
    /// WHAT: emits `__moth_float_validate(source)` and assigns either the scalar finite `Float`
    ///       (trap mode) or the fallible carrier (return-error mode) to the result local.
    /// WHY: Float boundary validation shares the same result-local carrier contract as
    ///      `NumericOp`; trap mode extracts the success value or throws, while return-error mode
    ///      keeps the carrier for later `FallibleBranch` lowering.
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
            NumericFailureMode::Trap => format!("__moth_numeric_trap({helper_call})"),
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

    fn emit_local_assignment(
        &mut self,
        statement: &HirStatement,
        local_id: LocalId,
        value: &HirExpression,
    ) -> Result<(), CompilerError> {
        let local_name = self.local_name(local_id)?.to_owned();
        let alias_only = self.local_is_alias_only_before_statement(statement, local_id);

        match &value.kind {
            HirExpressionKind::Load(place) => {
                let source = self.lower_place(place)?;
                if self.local_is_reactive_source(local_id) {
                    // Reactive declarations own stable source storage. Assignment updates that
                    // storage with the source's current value rather than rebinding it as an alias.
                    self.emit_line(&format!(
                        "__moth_assign_value({local_name}, __moth_read({source}));",
                    ));
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
                if alias_only {
                    self.emit_line(&format!("__moth_write({local_name}, {lowered});"));
                } else {
                    self.emit_line(&format!("__moth_assign_value({local_name}, {lowered});"));
                }
            }
        }

        Ok(())
    }

    fn emit_reactive_invalidations_for_statement(&mut self, statement_id: HirNodeId) {
        let Some(invalidations) = self
            .borrow_analysis
            .analysis
            .reactive_invalidations
            .get(&statement_id)
        else {
            return;
        };

        let mut source_ids = invalidations
            .iter()
            .map(|fact| fact.source.0)
            .collect::<Vec<_>>();
        source_ids.sort_unstable();
        source_ids.dedup();

        // Borrow validation owns conservative invalidation detection. JS lowering only schedules
        // the dirty sources after the statement's ordinary semantics have run.
        for source_id in source_ids {
            self.emit_line(&format!("__moth_reactive_schedule({source_id});"));
        }
    }

    fn local_is_alias_only_before_statement(
        &self,
        statement: &HirStatement,
        local_id: LocalId,
    ) -> bool {
        let Some(snapshot) = self
            .borrow_analysis
            .analysis
            .statement_entry_states
            .get(&statement.id)
        else {
            return false;
        };

        let Some(local_snapshot) = snapshot.locals.iter().find(|local| local.local == local_id)
        else {
            return false;
        };

        Self::snapshot_local_is_alias_only(local_snapshot.mode)
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

    fn current_function_returns_alias_reference(&self) -> bool {
        let Some(function_id) = self.current_function else {
            return false;
        };

        self.hir
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .is_some_and(|function| {
                function.return_aliases.len() == 1 && function.return_aliases[0].is_some()
            })
    }

    pub(crate) fn emit_return_terminator(
        &mut self,
        expression: &HirExpression,
    ) -> Result<(), CompilerError> {
        if self.is_unit_expression(expression) {
            self.emit_line("return;");
            return Ok(());
        }

        let value = if self.current_function_returns_alias_reference() {
            self.lower_return_value_expression(expression)?
        } else {
            self.lower_expr(expression)?
        };
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

        let value = if self.current_function_returns_alias_reference() {
            self.lower_return_value_expression(expression)?
        } else {
            self.lower_expr(expression)?
        };
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
        message: &Option<String>,
    ) -> Result<(), CompilerError> {
        let js_message = match message {
            Some(text) => format!("throw new Error({});", escape_js_string(text)),
            None => "throw new Error(\"assertion failed\");".to_string(),
        };
        self.emit_line(&js_message);

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

                let scrutinee = self.lower_expr(scrutinee)?;
                let scrutinee_temp = self.next_temp_identifier("__match");
                self.emit_line(&format!("const {scrutinee_temp} = {scrutinee};"));

                // If the last arm is an unguarded wildcard or capture, emit it as `else`
                // instead of `else if (true)` and skip the unreachable fallback throw.
                let has_unconditional_fallback = matches!(
                    arms.last(),
                    Some(HirMatchArm {
                        pattern: HirPattern::Wildcard | HirPattern::Capture,
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
                    let condition = self.lower_match_arm_condition(&scrutinee_temp, arm)?;
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

            HirTerminator::RuntimeFailure { message } => {
                self.emit_runtime_failure_terminator(message)?;
            }

            HirTerminator::AssertFailure { message } => {
                self.emit_assert_failure_terminator(message)?;
            }
        }

        Ok(())
    }

    pub(crate) fn lower_match_arm_condition(
        &mut self,
        scrutinee_expression: &str,
        arm: &HirMatchArm,
    ) -> Result<String, CompilerError> {
        let pattern_condition = match &arm.pattern {
            HirPattern::Literal(value) => {
                let literal = self.lower_expr(value)?;
                format!("{scrutinee_expression} === {literal}")
            }
            HirPattern::OptionNone => {
                format!("({scrutinee_expression}).tag === \"none\"")
            }
            HirPattern::OptionValue { value } => {
                let literal = self.lower_expr(value)?;
                let inner_equality = self.lower_option_inner_equality(
                    format!("({scrutinee_expression}).value"),
                    value.ty,
                    literal,
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
            HirPattern::Capture => "true".to_owned(),
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

/// Returns the JS runtime helper name for a checked numeric HIR operation.
///
/// WHAT: maps each `HirNumericOp` to the `__moth_int_*` or `__moth_float_*` helper emitted by
///       `emit_runtime_numeric_helpers`.
/// WHY: keeps the helper name decision in one place so statement lowering and runtime emission
///      cannot drift.
fn js_numeric_helper_for_op(op: HirNumericOp) -> &'static str {
    match op {
        HirNumericOp::IntAdd => "__moth_int_add",
        HirNumericOp::IntSub => "__moth_int_sub",
        HirNumericOp::IntMul => "__moth_int_mul",
        HirNumericOp::IntDiv => "__moth_int_div",
        HirNumericOp::IntMod => "__moth_int_mod",
        HirNumericOp::IntPow => "__moth_int_pow",
        HirNumericOp::IntNeg => "__moth_int_neg",
        HirNumericOp::FloatAdd => "__moth_float_add",
        HirNumericOp::FloatSub => "__moth_float_sub",
        HirNumericOp::FloatMul => "__moth_float_mul",
        HirNumericOp::FloatDiv => "__moth_float_div",
        HirNumericOp::FloatMod => "__moth_float_mod",
        HirNumericOp::FloatPow => "__moth_float_pow",
        HirNumericOp::FloatNeg => "__moth_float_neg",
    }
}
