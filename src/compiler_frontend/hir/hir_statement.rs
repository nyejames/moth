//! HIR Statement Lowering
//!
//! Lowers AST statements and control-flow nodes into explicit HIR blocks, statements, and
//! terminators.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only. Function-body terminality is owned by
//! AST; a fallthrough that reaches HIR lowering indicates a compiler invariant failure.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, LoopBindings, MultiBindTarget, MultiBindTargetKind, NodeKind, RangeLoopSpec,
    SourceLocation,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::PlaceExpression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{
    HirTerminator, classify_assertion_message_evaluation,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::return_hir_transformation_error;

use crate::hir_log;

mod control_flow;
mod declarations;
mod entry_start;
mod loop_lowering;
mod match_captures;
mod returns;
mod value_blocks;

impl<'a> HirBuilder<'a> {
    // -------------------------
    //  Top-Level Lowering
    // -------------------------

    // WHAT: routes one top-level AST node into the HIR lowering path that owns it.
    // WHY: declaration registration already built the symbol tables, so top-level lowering should
    //      only accept nodes that materially contribute module/runtime semantics.
    pub(super) fn lower_top_level_node(&mut self, node: &AstNode) -> Result<(), CompilerError> {
        match &node.kind {
            NodeKind::Function(name, signature, body) => {
                self.lower_function_body(name, signature, body, &node.location)
            }

            NodeKind::StructDefinition(_, _) => Ok(()),

            NodeKind::Return(_) | NodeKind::ReturnError(_) => Err(CompilerError::new(
                "HIR invariant: Top-level return reached HIR lowering. Returns must appear inside function bodies in well-formed AST.",
                self.hir_error_location(&node.location),
                ErrorType::HirTransformation,
            )),

            _ => Err(CompilerError::new(
                format!(
                    "HIR invariant: unsupported top-level AST node reached HIR lowering: {:?}",
                    node.kind
                ),
                self.hir_error_location(&node.location),
                ErrorType::HirTransformation,
            )),
        }
    }

    // -------------------------
    //  Function Body Lowering
    // -------------------------

    // WHAT: enters one function's lowering context, lowers its body, then restores builder state.
    // WHY: function lowering needs scoped block/region/current-function state that must not leak
    //      into the next function.
    pub(super) fn lower_function_body(
        &mut self,
        function_name: &InternedPath,
        signature: &FunctionSignature,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let function_id = self.resolve_function_id_or_error(function_name, location)?;

        self.enter_function(function_id, location)?;

        // WHAT: for entry start(), allocate the Vec<String> fragment accumulator before lowering.
        // WHY: PushStartRuntimeFragment nodes in the body push to this local; the implicit return
        //      at end of entry start loads it as the function result.
        self.maybe_initialize_entry_fragment_accumulator(function_id, location)?;

        let lower_result = self.lower_function_body_inner(function_id, signature, body, location);
        self.leave_function();

        lower_result
    }

    fn lower_function_body_inner(
        &mut self,
        function_id: FunctionId,
        signature: &FunctionSignature,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let return_type = self
            .function_by_id_or_error(function_id, location)?
            .return_type;

        self.lower_parameter_locals(function_id, signature, location)?;
        self.lower_statement_sequence(body)?;

        let current_block = self.current_block_id_or_error(location)?;
        if self.block_has_explicit_terminator(current_block, location)? {
            return Ok(());
        }

        if self.is_unit_type(return_type) {
            let region = self.current_region_or_error(location)?;
            let unit = self.unit_expression(location, region);
            self.emit_terminator(current_block, HirTerminator::Return(unit), location)?;
            return Ok(());
        }

        if let Some((ok, _)) = self.type_environment.fallible_carrier_slots(return_type)
            && signature.success_returns().is_empty()
        {
            let region = self.current_region_or_error(location)?;
            let unit = self.unit_expression(location, region);
            if unit.ty != ok {
                return Err(CompilerError::new(
                    "Result function with empty success returns has non-unit ok type",
                    self.hir_error_location(location),
                    ErrorType::HirTransformation,
                ));
            }

            self.emit_terminator(current_block, HirTerminator::ReturnSuccess(unit), location)?;
            return Ok(());
        }

        // WHAT: entry start() has an implicit return of the fragment vec accumulator.
        // WHY: the body contains only PushStartRuntimeFragment nodes with no explicit return;
        //      the return type is Vec<String> which the builder consumes as the fragment list.
        if self.maybe_emit_entry_fragment_return(function_id, current_block, location)? {
            return Ok(());
        }

        // AST terminality validation should have rejected any function that can reach this point.
        // A fallthrough here is therefore an internal compiler invariant failure, not a user
        // diagnostic.
        Err(CompilerError::new(
            "HIR lowering reached a non-terminal function body after AST terminality validation",
            self.hir_error_location(location),
            ErrorType::HirTransformation,
        ))
    }

    // -------------------------
    //  Statement Lowering
    // -------------------------

    // WHAT: lowers a run of AST statements until a terminating control-flow edge is emitted.
    // WHY: once a block has an explicit terminator, later statements in the sequence are dead for
    //      the current CFG path and must not be appended.
    pub(crate) fn lower_statement_sequence(
        &mut self,
        nodes: &[AstNode],
    ) -> Result<(), CompilerError> {
        for node in nodes {
            let current_block = self.current_block_id_or_error(&node.location)?;
            if self.block_has_explicit_terminator(current_block, &node.location)? {
                break;
            }

            self.lower_statement_node(node)?;
        }

        Ok(())
    }

    // WHAT: lowers one AST statement node into HIR statements, blocks, or terminators.
    // WHY: statement lowering is the control-flow dispatcher for the builder and centralizes the
    //      mapping from AST statement kinds to explicit HIR form.
    pub(crate) fn lower_statement_node(&mut self, node: &AstNode) -> Result<(), CompilerError> {
        self.log_statement_input(node);

        let result = match &node.kind {
            NodeKind::VariableDeclaration(var) => {
                self.lower_variable_declaration_statement(var, &node.location)
            }

            NodeKind::Assignment { target, value } => {
                self.lower_assignment_statement(target, value, &node.location)
            }

            NodeKind::MultiBind { targets, value } => {
                self.lower_multi_bind_statement(targets, value, &node.location)
            }

            NodeKind::ExpressionStatement(expr) => {
                self.lower_expression_statement(expr, &node.location)
            }

            NodeKind::Return(values) => self.lower_return_statement(values, &node.location),

            NodeKind::ReturnError(value) => {
                self.lower_error_return_statement(value, &node.location)
            }

            NodeKind::If(condition, then_body, else_body) => {
                self.lower_if_statement(condition, then_body, else_body.as_deref(), &node.location)
            }

            NodeKind::WhileLoop(condition, body) => {
                self.lower_while_statement(condition, body, &node.location)
            }

            NodeKind::Break => self.lower_break_statement(&node.location),

            NodeKind::Continue => self.lower_continue_statement(&node.location),

            NodeKind::Match {
                scrutinee,
                arms,
                default,
                exhaustiveness,
            } => self.lower_match_statement(
                scrutinee,
                arms,
                default.as_deref(),
                *exhaustiveness,
                &node.location,
            ),

            NodeKind::ScopedBlock { body } => {
                self.lower_scoped_block_statement(body, &node.location)
            }

            NodeKind::RangeLoop {
                bindings,
                range,
                body,
            } => self.lower_range_loop_statement(bindings, range, body, &node.location),

            NodeKind::CollectionLoop {
                bindings,
                iterable,
                body,
            } => self.lower_collection_loop_statement(bindings, iterable, body, &node.location),

            NodeKind::ThenValue(produced_values) => {
                self.lower_then_value_statement(produced_values, &node.location)
            }

            NodeKind::Assert { condition, message } => {
                self.lower_assert_statement(condition, message, &node.location)
            }

            NodeKind::PushStartRuntimeFragment(expr) => {
                // WHAT: lower a top-level runtime template push into a PushRuntimeFragment HIR statement.
                // WHY: the fragment accumulator local was allocated at function entry; each
                //      PushStartRuntimeFragment appends one evaluated string to it.
                let Some(vec_local) = self.entry_fragment_vec_local else {
                    return_hir_transformation_error!(
                        "PushStartRuntimeFragment encountered outside entry start() — no fragment vec local is active",
                        self.hir_error_location(&node.location)
                    );
                };

                let value = self.lower_expression_value_to_current_block(expr)?;
                self.emit_statement_kind(
                    HirStatementKind::PushRuntimeFragment { vec_local, value },
                    &node.location,
                )
            }

            _ => return_hir_transformation_error!(
                format!(
                    "Unsupported AST statement node during HIR lowering: {:?}",
                    node.kind
                ),
                self.hir_error_location(&node.location)
            ),
        };

        if result.is_ok() {
            self.log_statement_output(node);
        }

        result
    }

    // -------------------------
    //  Variable Lowering
    // -------------------------

    fn lower_assignment_statement(
        &mut self,
        target: &PlaceExpression,
        value: &Expression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let (target_prelude, target_place) = self.lower_place_expression_to_hir_place(target)?;

        for prelude in target_prelude {
            self.emit_statement_to_current_block(prelude, location)?;
        }

        let lowered_value = self.lower_expression_value_to_current_block(value)?;

        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: target_place,
                value: lowered_value,
            },
            location,
        )
    }

    fn lower_multi_bind_statement(
        &mut self,
        targets: &[MultiBindTarget],
        value: &Expression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        // INVARIANT: AST validation guarantees the RHS is an explicitly supported multi-bind
        // source (currently a multi-return function call). This lowering assumes that invariant
        // and does not handle generic destructuring of arbitrary expressions.
        if targets.len() < 2 {
            return_hir_transformation_error!(
                "Single-target bind unexpectedly reached multi-bind lowering",
                self.hir_error_location(location)
            );
        }

        let rhs_value = self.lower_expression_value_to_current_block(value)?;

        let rhs_type = rhs_value.ty;
        let rhs_local = self.allocate_temp_local(rhs_type, Some(location.to_owned()))?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(rhs_local),
                value: rhs_value,
            },
            location,
        )?;

        let tuple_fields = match self.type_environment.tuple_field_ids(rhs_type) {
            Some(fields) => fields.to_vec(),
            None => {
                return_hir_transformation_error!(
                    "Multi-bind right-hand value lowered to a non-tuple shape",
                    self.hir_error_location(location)
                );
            }
        };

        if tuple_fields.len() != targets.len() {
            return_hir_transformation_error!(
                "Multi-bind slot arity does not match lowered tuple shape",
                self.hir_error_location(location)
            );
        }

        for (slot_index, target) in targets.iter().enumerate() {
            let slot_type = tuple_fields[slot_index];
            let target_type = self.lower_type_id(target.type_id, &target.location)?;

            if slot_type != target_type {
                return_hir_transformation_error!(
                    format!(
                        "Lowered multi-bind slot type mismatch at index {}",
                        slot_index
                    ),
                    self.hir_error_location(&target.location)
                );
            }

            let target_local = match target.kind {
                MultiBindTargetKind::Declaration => self.allocate_named_local(
                    target.id.to_owned(),
                    target_type,
                    target.value_mode.is_mutable(),
                    Some(target.location.to_owned()),
                )?,
                MultiBindTargetKind::Assignment => {
                    let Some(local_id) = self.locals_by_name.get(&target.id).copied() else {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' is missing from local bindings",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.location)
                        );
                    };

                    let Some((block_index, local_index)) =
                        self.local_index_by_id.get(&local_id).copied()
                    else {
                        return_hir_transformation_error!(
                            "Multi-bind assignment target local is not registered in HIR blocks",
                            self.hir_error_location(&target.location)
                        );
                    };

                    let local = &self.module.blocks[block_index].locals[local_index];
                    if !local.mutable {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' lowered as immutable local",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.location)
                        );
                    }

                    if local.ty != target_type {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' lowered with mismatched local type",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.location)
                        );
                    }

                    local_id
                }
            };

            let slot_region = self.current_region_or_error(&target.location)?;
            let tuple_value = self.make_expression(
                &target.location,
                HirExpressionKind::Load(HirPlace::Local(rhs_local)),
                rhs_type,
                ValueKind::RValue,
                slot_region,
            );
            let slot_value = self.make_expression(
                &target.location,
                HirExpressionKind::TupleGet {
                    tuple: Box::new(tuple_value),
                    index: slot_index,
                },
                slot_type,
                ValueKind::RValue,
                slot_region,
            );

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(target_local),
                    value: slot_value,
                },
                &target.location,
            )?;
        }

        Ok(())
    }

    fn lower_expression_statement(
        &mut self,
        expression: &Expression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let value = self.lower_expression_value_to_current_block(expression)?;

        if self.is_unit_type(value.ty) {
            if matches!(
                expression.kind,
                ExpressionKind::HandledFallibleFunctionCall { .. }
                    | ExpressionKind::HandledFallibleHostFunctionCall { .. }
            ) {
                self.emit_statement_kind(HirStatementKind::Expr(value), location)?;
            }
            return Ok(());
        }

        self.emit_statement_kind(HirStatementKind::Expr(value), location)
    }

    // -------------------------
    //  Assert Statement
    // -------------------------

    /// Lower an `assert` statement into HIR control flow.
    ///
    /// WHAT: turns `assert(condition)` or `assert(condition, "message")` into explicit CFG.
    /// WHY: `assert(false, ...)` must be statically terminal; dynamic conditions branch to a
    ///      failure block that terminates with `AssertFailure`.
    fn lower_assert_statement(
        &mut self,
        condition: &Expression,
        message: &Expression,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        // Statically known false → immediate assertion failure, no pass block needed.
        if matches!(condition.kind, ExpressionKind::Bool(false)) {
            let message_value = self.lower_expression_value_to_current_block(message)?;
            let failure_block = self.current_block_id_or_error(location)?;
            return self.emit_terminator(
                failure_block,
                HirTerminator::AssertFailure {
                    message_evaluation: classify_assertion_message_evaluation(&message_value),
                    message: message_value,
                },
                location,
            );
        }

        // Statically known true → no runtime effect.
        if matches!(condition.kind, ExpressionKind::Bool(true)) {
            return Ok(());
        }

        // Dynamic condition: lower it, then branch to pass / failure blocks.
        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error(location)?;

        let parent_region = self.current_region_or_error(location)?;
        let pass_region = self.create_child_region(parent_region);
        let failure_region = self.create_child_region(parent_region);
        let pass_block = self.create_block(pass_region, location, "assert-pass")?;
        let failure_block = self.create_block(failure_region, location, "assert-fail")?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block: pass_block,
                else_block: failure_block,
            },
            location,
        )?;
        self.log_control_flow_edge(condition_block, pass_block, "assert.true");
        self.log_control_flow_edge(condition_block, failure_block, "assert.false");

        self.set_current_block(failure_block, location)?;
        let message_value = self.lower_expression_value_to_current_block(message)?;
        let failure_tail_block = self.current_block_id_or_error(location)?;
        self.emit_terminator(
            failure_tail_block,
            HirTerminator::AssertFailure {
                message_evaluation: classify_assertion_message_evaluation(&message_value),
                message: message_value,
            },
            location,
        )?;

        self.set_current_block(pass_block, location)
    }

    // -------------------------
    //  Loop Statements
    // -------------------------

    fn lower_range_loop_statement(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        // Loop lowering is intentionally split into a dedicated submodule to keep this file
        // focused on statement dispatch and shared lowering helpers.
        self.lower_range_loop_statement_impl(bindings, range, body, location)
    }

    fn lower_collection_loop_statement(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_collection_loop_statement_impl(bindings, iterable, body, location)
    }

    // -------------------------
    //  Statement Emission
    // -------------------------

    pub(super) fn emit_statement_kind(
        &mut self,
        kind: HirStatementKind,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind,
            location: location.clone(),
        };

        self.side_table.map_statement(location, &statement);
        self.emit_statement_to_current_block(statement, location)
    }

    // -------------------------
    //  Diagnostics & Logging
    // -------------------------

    fn log_statement_input(&self, _node: &AstNode) {
        hir_log!(format!("[HIR][Stmt] Lowering {:?}", _node.kind));
    }

    fn log_statement_output(&self, _node: &AstNode) {
        hir_log!(format!("[HIR][Stmt] Lowered {:?}", _node.kind));
    }

    fn log_block_created(&self, _block_id: BlockId, _label: &str, _location: &SourceLocation) {
        hir_log!(format!(
            "[HIR][CFG] Created block {} ({}) @ {:?}",
            _block_id, _label, _location
        ));
    }

    fn log_control_flow_edge(&self, _from: BlockId, _to: BlockId, _label: &str) {
        hir_log!(format!("[HIR][CFG] Edge {} -> {} ({})", _from, _to, _label));
    }

    fn log_terminator_emitted(
        &self,
        _block_id: BlockId,
        _terminator: &HirTerminator,
        _location: &SourceLocation,
    ) {
        hir_log!(format!(
            "[HIR][CFG] Terminator for {} @ {:?}: {}",
            _block_id,
            _location,
            _terminator.display_with_context(
                &crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                    self.string_table,
                )
                .with_side_table(&self.side_table)
                .with_type_environment(&self.type_environment),
            )
        ));
    }
}
