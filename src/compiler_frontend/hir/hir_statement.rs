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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
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
                self.lower_function_body(name, signature, body, &node.span)
            }

            NodeKind::StructDefinition(_, _) => Ok(()),

            NodeKind::Return(_) | NodeKind::ReturnError(_) => Err(CompilerError::new(
                "HIR invariant: Top-level return reached HIR lowering. Returns must appear inside function bodies in well-formed AST.",
                self.hir_error_location(&node.span),
                ErrorType::HirTransformation,
            )),

            _ => Err(CompilerError::new(
                format!(
                    "HIR invariant: unsupported top-level AST node reached HIR lowering: {:?}",
                    node.kind
                ),
                self.hir_error_location(&node.span),
                ErrorType::HirTransformation,
            )),
        }
    }

    // -------------------------
    //  Function Body Lowering
    // -------------------------

    // WHAT: enters one function's lowering context, lowers its body, then restores builder state.
    // WHY: function lowering needs lexical scope/region/current-function state that must not leak
    //      into the next function.
    pub(super) fn lower_function_body(
        &mut self,
        function_name: &PathId,
        signature: &FunctionSignature,
        body: &[AstNode],
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let function_id = self.resolve_function_id_or_error(function_name, span)?;

        self.enter_function(function_id, span)?;

        // WHAT: for entry start(), allocate the Vec<String> fragment accumulator before lowering.
        // WHY: PushStartRuntimeFragment nodes in the body push to this local; the implicit return
        //      at end of entry start loads it as the function result.
        self.maybe_initialize_entry_fragment_accumulator(function_id, span)?;

        let lower_result = self.lower_function_body_inner(function_id, signature, body, span);
        self.leave_function();

        lower_result
    }

    fn lower_function_body_inner(
        &mut self,
        function_id: FunctionId,
        signature: &FunctionSignature,
        body: &[AstNode],
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let return_type = self.function_by_id_or_error(function_id, span)?.return_type;

        self.lower_parameter_locals(function_id, signature, span)?;
        self.lower_statement_sequence(body)?;

        let current_block = self.current_block_id_or_error(span)?;
        if self.block_has_explicit_terminator(current_block, span)? {
            return Ok(());
        }

        if self.is_unit_type(return_type) {
            let region = self.current_region_or_error(span)?;
            let unit = self.unit_expression(span, region);
            self.emit_terminator(current_block, HirTerminator::Return(unit), span)?;
            return Ok(());
        }

        if let Some((ok, _)) = self.type_environment.fallible_carrier_slots(return_type)
            && signature.success_returns().is_empty()
        {
            let region = self.current_region_or_error(span)?;
            let unit = self.unit_expression(span, region);
            if unit.ty != ok {
                return Err(CompilerError::new(
                    "Result function with empty success returns has non-unit ok type",
                    self.hir_error_location(span),
                    ErrorType::HirTransformation,
                ));
            }

            self.emit_terminator(current_block, HirTerminator::ReturnSuccess(unit), span)?;
            return Ok(());
        }

        // WHAT: entry start() has an implicit return of the fragment vec accumulator.
        // WHY: the body contains only PushStartRuntimeFragment nodes with no explicit return;
        //      the return type is Vec<String> which the builder consumes as the fragment list.
        if self.maybe_emit_entry_fragment_return(function_id, current_block, span)? {
            return Ok(());
        }

        // AST terminality validation should have rejected any function that can reach this point.
        // A fallthrough here is therefore an internal compiler invariant failure, not a user
        // diagnostic.
        Err(CompilerError::new(
            "HIR lowering reached a non-terminal function body after AST terminality validation",
            self.hir_error_location(span),
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
            let current_block = self.current_block_id_or_error(&node.span)?;
            if self.block_has_explicit_terminator(current_block, &node.span)? {
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
                self.lower_variable_declaration_statement(var, &node.span, node.span)
            }

            NodeKind::Assignment { target, value } => {
                self.lower_assignment_statement(target, value, &node.span, node.span)
            }

            NodeKind::MultiBind { targets, value } => {
                self.lower_multi_bind_statement(targets, value, &node.span, node.span)
            }

            NodeKind::ExpressionStatement(expr) => {
                self.lower_expression_statement(expr, &node.span, node.span)
            }

            NodeKind::Return(values) => self.lower_return_statement(values, &node.span, node.span),

            NodeKind::ReturnError(value) => {
                self.lower_error_return_statement(value, &node.span, node.span)
            }

            NodeKind::If(condition, then_body, else_body, _) => self.lower_if_statement(
                condition,
                then_body,
                else_body.as_deref(),
                &node.span,
                node.span,
            ),

            NodeKind::WhileLoop(condition, body) => {
                self.lower_while_statement(condition, body, &node.span, node.span)
            }

            NodeKind::Break => self.lower_break_statement(&node.span, node.span),

            NodeKind::Continue => self.lower_continue_statement(&node.span, node.span),

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
                &node.span,
                node.span,
            ),

            NodeKind::LexicalScope { body } => self.lower_lexical_scope(body, &node.span),

            NodeKind::StructDefinition(_, _) => Ok(()),

            NodeKind::RangeLoop {
                bindings,
                range,
                body,
            } => self.lower_range_loop_statement(bindings, range, body, &node.span, node.span),

            NodeKind::CollectionLoop {
                bindings,
                iterable,
                body,
            } => self
                .lower_collection_loop_statement(bindings, iterable, body, &node.span, node.span),

            NodeKind::ThenValue(produced_values) => {
                self.lower_then_value_statement(produced_values, &node.span, node.span)
            }

            NodeKind::Assert { condition, message } => {
                self.lower_assert_statement(condition, message, &node.span, node.span)
            }

            NodeKind::PushStartRuntimeFragment(expr) => {
                // WHAT: lower a top-level runtime template push into a PushRuntimeFragment HIR statement.
                // WHY: the fragment accumulator local was allocated at function entry; each
                //      PushStartRuntimeFragment appends one evaluated string to it.
                let Some(vec_local) = self.entry_fragment_vec_local else {
                    return_hir_transformation_error!(
                        "PushStartRuntimeFragment encountered outside entry start() — no fragment vec local is active",
                        self.hir_error_location(&node.span)
                    );
                };

                let value = self.lower_expression_value_to_current_block(expr)?;
                self.emit_statement_kind_with_span(
                    HirStatementKind::PushRuntimeFragment { vec_local, value },
                    &node.span,
                    node.span,
                )
            }

            _ => return_hir_transformation_error!(
                format!(
                    "Unsupported AST statement node during HIR lowering: {:?}",
                    node.kind
                ),
                self.hir_error_location(&node.span)
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
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let (target_prelude, target_place) = self.lower_place_expression_to_hir_place(target)?;

        for prelude in target_prelude {
            self.emit_statement_to_current_block(prelude, span)?;
        }

        let lowered_value = self.lower_expression_value_to_current_block(value)?;

        // Authored assignment: the statement span covers the whole `target = value` operation.
        // Place-lowering preludes above keep their own expression spans; only this Assign
        // carries the statement span.
        self.emit_statement_kind_with_span(
            HirStatementKind::Assign {
                target: target_place,
                value: lowered_value,
            },
            span,
            statement_span,
        )
    }

    fn lower_multi_bind_statement(
        &mut self,
        targets: &[MultiBindTarget],
        value: &Expression,
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // INVARIANT: AST validation guarantees the RHS is an explicitly supported multi-bind
        // source (currently a multi-return function call). This lowering assumes that invariant
        // and does not handle generic destructuring of arbitrary expressions.
        if targets.len() < 2 {
            return_hir_transformation_error!(
                "Single-target bind unexpectedly reached multi-bind lowering",
                self.hir_error_location(span)
            );
        }

        let rhs_value = self.lower_expression_value_to_current_block(value)?;

        let rhs_type = rhs_value.ty;
        let rhs_local = self.allocate_temp_local(rhs_type, None)?;
        // Generated tuple spill: the hidden rhs local is compiler scaffolding, so the
        // assignment stays spanless even though the RHS value itself is authored.
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(rhs_local),
                value: rhs_value,
            },
            span,
        )?;

        let tuple_fields = match self.type_environment.tuple_field_ids(rhs_type) {
            Some(fields) => fields.to_vec(),
            None => {
                return_hir_transformation_error!(
                    "Multi-bind right-hand value lowered to a non-tuple shape",
                    self.hir_error_location(span)
                );
            }
        };

        if tuple_fields.len() != targets.len() {
            return_hir_transformation_error!(
                "Multi-bind slot arity does not match lowered tuple shape",
                self.hir_error_location(span)
            );
        }

        for (slot_index, target) in targets.iter().enumerate() {
            let slot_type = tuple_fields[slot_index];
            let target_type = self.lower_type_id(target.type_id, &target.span)?;

            if slot_type != target_type {
                return_hir_transformation_error!(
                    format!(
                        "Lowered multi-bind slot type mismatch at index {}",
                        slot_index
                    ),
                    self.hir_error_location(&target.span)
                );
            }

            let target_local = match target.kind {
                MultiBindTargetKind::Declaration => self.allocate_named_local(
                    target.id.to_owned(),
                    target_type,
                    target.value_mode.is_mutable(),
                    target.span,
                )?,
                MultiBindTargetKind::Assignment => {
                    let Some(local_id) = self.locals_by_name.get(&target.id).copied() else {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' is missing from local bindings",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.span)
                        );
                    };

                    let Some((block_index, local_index)) =
                        self.local_index_by_id.get(&local_id).copied()
                    else {
                        return_hir_transformation_error!(
                            "Multi-bind assignment target local is not registered in HIR blocks",
                            self.hir_error_location(&target.span)
                        );
                    };

                    let local = &self.module.blocks[block_index].locals[local_index];
                    if !local.mutable {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' lowered as immutable local",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.span)
                        );
                    }

                    if local.ty != target_type {
                        return_hir_transformation_error!(
                            format!(
                                "Multi-bind assignment target '{}' lowered with mismatched local type",
                                self.symbol_name_for_diagnostics(&target.id)
                            ),
                            self.hir_error_location(&target.span)
                        );
                    }

                    local_id
                }
            };

            let slot_region = self.current_region_or_error(&target.span)?;
            let tuple_value = self.make_expression(
                &target.span,
                HirExpressionKind::Load(HirPlace::Local(rhs_local)),
                rhs_type,
                ValueKind::RValue,
                slot_region,
            );
            let slot_value = self.make_expression(
                &target.span,
                HirExpressionKind::TupleGet {
                    tuple: Box::new(tuple_value),
                    index: slot_index,
                },
                slot_type,
                ValueKind::RValue,
                slot_region,
            );

            // Authored per-slot bind: each target carries its own binding span. Fall back
            // to the statement span only when the target is identity-free.
            self.emit_statement_kind_with_span(
                HirStatementKind::Assign {
                    target: HirPlace::Local(target_local),
                    value: slot_value,
                },
                &target.span,
                target.span.or(statement_span),
            )?;
        }

        Ok(())
    }

    fn lower_expression_statement(
        &mut self,
        expression: &Expression,
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let value = self.lower_expression_value_to_current_block(expression)?;

        if self.is_unit_type(value.ty) {
            if matches!(
                expression.kind,
                ExpressionKind::HandledFallibleFunctionCall { .. }
                    | ExpressionKind::HandledFallibleHostFunctionCall { .. }
            ) {
                // Authored unit-valued handled call in statement position keeps the
                // statement span; the fallback covers identity-free statements.
                self.emit_statement_kind_with_span(
                    HirStatementKind::Expr(value),
                    span,
                    statement_span.or(expression.span),
                )?;
            }
            return Ok(());
        }

        // Authored expression statement carries the statement span.
        self.emit_statement_kind_with_span(
            HirStatementKind::Expr(value),
            span,
            statement_span.or(expression.span),
        )
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
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // Statically known false → immediate assertion failure, no pass block needed.
        if matches!(condition.kind, ExpressionKind::Bool(false)) {
            let message_value = self.lower_expression_value_to_current_block(message)?;
            let failure_block = self.current_block_id_or_error(span)?;
            return self.emit_terminator_with_span(
                failure_block,
                HirTerminator::AssertFailure {
                    message_evaluation: classify_assertion_message_evaluation(&message_value),
                    message: message_value,
                },
                span,
                statement_span,
            );
        }

        // Statically known true → no runtime effect.
        if matches!(condition.kind, ExpressionKind::Bool(true)) {
            return Ok(());
        }

        // Dynamic condition: lower it, then branch to pass / failure blocks.
        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error(span)?;

        let parent_region = self.current_region_or_error(span)?;
        let pass_region = self.create_child_region(parent_region);
        let failure_region = self.create_child_region(parent_region);
        let pass_block = self.create_block(pass_region, span, "assert-pass")?;
        let failure_block = self.create_block(failure_region, span, "assert-fail")?;

        // Authored assert header: the condition branch carries the statement span so
        // diagnostics can point at the authored `assert(...)`.
        self.emit_terminator_with_span(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block: pass_block,
                else_block: failure_block,
            },
            span,
            statement_span.or(condition.span),
        )?;
        self.log_control_flow_edge(condition_block, pass_block, "assert.true");
        self.log_control_flow_edge(condition_block, failure_block, "assert.false");

        self.set_current_block(failure_block, span)?;
        let message_value = self.lower_expression_value_to_current_block(message)?;
        let failure_tail_block = self.current_block_id_or_error(span)?;
        self.emit_terminator_with_span(
            failure_tail_block,
            HirTerminator::AssertFailure {
                message_evaluation: classify_assertion_message_evaluation(&message_value),
                message: message_value,
            },
            span,
            statement_span,
        )?;

        self.set_current_block(pass_block, span)
    }

    // -------------------------
    //  Loop Statements
    // -------------------------

    fn lower_range_loop_statement(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &[AstNode],
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // Loop lowering is intentionally split into a dedicated submodule to keep this file
        // focused on statement dispatch and shared lowering helpers.
        self.lower_range_loop_statement_impl(bindings, range, body, span, statement_span)
    }

    fn lower_collection_loop_statement(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        body: &[AstNode],
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.lower_collection_loop_statement_impl(bindings, iterable, body, span, statement_span)
    }

    // -------------------------
    //  Statement Emission
    // -------------------------

    pub(super) fn emit_statement_kind(
        &mut self,
        kind: HirStatementKind,
        span: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.emit_statement_kind_with_span(kind, span, None)
    }

    pub(super) fn emit_statement_kind_with_span(
        &mut self,
        kind: HirStatementKind,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let statement = HirStatement {
            id: self.allocate_node_id(),
            kind,
            span: authored_span,
        };

        self.side_table.map_statement(span.to_owned(), &statement);
        self.emit_statement_to_current_block(statement, span)
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

    fn log_block_created(&self, _block_id: BlockId, _label: &str, _span: &Option<SourceSpan>) {
        hir_log!(format!(
            "[HIR][CFG] Created block {} ({}) @ {:?}",
            _block_id, _label, _span
        ));
    }

    fn log_control_flow_edge(&self, _from: BlockId, _to: BlockId, _label: &str) {
        hir_log!(format!("[HIR][CFG] Edge {} -> {} ({})", _from, _to, _label));
    }

    fn log_terminator_emitted(
        &self,
        _block_id: BlockId,
        _terminator: &HirTerminator,
        _span: &Option<SourceSpan>,
    ) {
        hir_log!(format!(
            "[HIR][CFG] Terminator for {} @ {:?}: {}",
            _block_id,
            _span,
            _terminator.display_with_context(
                &crate::compiler_frontend::hir::hir_display::HirDisplayContext::new(
                    self.string_table,
                    self.path_fork,
                )
                .with_side_table(&self.side_table)
                .with_type_environment(&self.type_environment),
            )
        ));
    }
}
