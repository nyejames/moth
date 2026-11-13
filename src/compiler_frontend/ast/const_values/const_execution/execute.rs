//! The bounded typed-AST frame machine.
//!
//! WHAT: executes prepared const instructions and typed expressions with explicit heap frames,
//!       call depth and budget charges. It owns expression evaluation, range/collection loop
//!       bindings and the TIR template-folding adapter.
//! WHY: a const call can execute before HIR exists. One frame machine sequences the interpreter
//!      context and reuses declaration-time instruction compilation for every call.
//! MUST NOT: decide whether a body is eligible or rewrite an unreachable AST.

use super::{
    AliasTarget, ConstFunctionExecutable, ConstValueId, ProjectedConstant,
    budget::{
        ExecutionBudget, MAX_EXPRESSION_DEPTH, MAX_HANDLER_DEPTH, MAX_LOOP_ITERATIONS,
        charge_new_value_text,
    },
    instructions::{ConstInstruction, ConstInstructionKind},
    labels,
    places::{direct_alias_target, expand_alias_target, fallible_handler_error_binding},
    rewrite::RewriteMode,
    state::{
        ActiveHandler, ConstExecutionServices, ConstInterpreter, ExecutionState, FrameBindings,
    },
    values::{
        ProjectionWrite, assign_constant_projections, constant_operation, expression_with_value,
        hard_limit_error, project_constant_value, template_error, unsupported_error,
    },
};
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetUnit, ConstantFoldError,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, PlaceId,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateFoldBinding, build_collection_iteration_bindings,
    build_range_iteration_bindings,
};
use crate::compiler_frontend::ast::templates::template_folding::{
    RequiredCallSiteRewriter, TemplateEmission,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    fold_prepared_template, prepare_tir_view_with_source_scope_and_bindings,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::rc::Rc;

/// One suspended loop or program frame in the iterative interpreter.
enum ExecFrame {
    Program {
        instructions: Rc<[ConstInstruction]>,
        ip: usize,
    },
    While {
        condition: ExprId,
        body: Rc<[ConstInstruction]>,
    },
    Range {
        cursor: ConstRangeCursor,
        loop_bindings: Box<LoopBindings>,
        body: Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
    },
    Collection(CollectionFrame),
}

/// One active collection-loop frame.
struct CollectionFrame {
    values: Vec<ConstValueId>,
    next: usize,
    loop_bindings: Box<LoopBindings>,
    body: Rc<[ConstInstruction]>,
}

/// Why one instruction stream is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameMode {
    /// A const function body: falling off the end without a `return` is an unsupported body.
    FunctionBody,
    /// A selected catch handler body: the enclosing expression owns the continuation.
    HandlerBody,
}

impl ConstInterpreter<'_> {
    /// Run one prepared function body with freshly bound parameters.
    pub(super) fn execute_body(
        &mut self,
        executable: &ConstFunctionExecutable,
        mut parameter_values: FrameBindings,
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let saved_aliases = std::mem::take(&mut self.state.aliases);
        let saved_handler_bodies = self.state.handler_bodies.take();
        let saved_rewritten_handlers = std::mem::take(&mut self.state.rewritten_handlers);
        self.state.handler_bodies = Some(Rc::clone(&executable.handler_bodies));
        let result = self.execute_frames(
            &executable.instructions,
            FrameMode::FunctionBody,
            &mut parameter_values,
        );
        self.state.aliases = saved_aliases;
        self.state.handler_bodies = saved_handler_bodies;
        self.state.rewritten_handlers = saved_rewritten_handlers;
        let result = result?;
        Ok(self.state.pending_handler_return.take().unwrap_or(result))
    }

    /// Run a prepared catch handler body and return the values its `then` statements produced.
    pub(super) fn execute_handler_body(
        &mut self,
        instructions: &Rc<[ConstInstruction]>,
        bindings: &mut FrameBindings,
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        if self.state.handler_depth >= MAX_HANDLER_DEPTH {
            return Err(hard_limit_error(
                self.budget.span,
                ConstBudgetCategory::Work,
                ConstBudgetUnit::HandlerLevels,
                MAX_HANDLER_DEPTH,
            ));
        }
        self.state.handler_depth += 1;
        let result = self.execute_frames(instructions, FrameMode::HandlerBody, bindings);
        self.state.handler_depth -= 1;
        result
    }

    /// Run the frame machine until it returns, falls through, or hits a refusal.
    fn execute_frames(
        &mut self,
        instructions: &Rc<[ConstInstruction]>,
        mode: FrameMode,
        bindings: &mut FrameBindings,
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let mut frames = vec![ExecFrame::Program {
            instructions: Rc::clone(instructions),
            ip: 0,
        }];

        while let Some(frame) = frames.pop() {
            match mode {
                FrameMode::HandlerBody => {
                    if let Some(returns) = self.state.pending_handler_return.clone() {
                        return Ok(returns);
                    }
                }
                FrameMode::FunctionBody => {
                    if let Some(returns) = self.state.pending_handler_return.take() {
                        return Ok(returns);
                    }
                }
            }
            self.budget.step()?;
            match frame {
                ExecFrame::Program { instructions, ip } => {
                    if ip >= instructions.len() {
                        continue;
                    }
                    let instruction = &instructions[ip];
                    self.budget.span = instruction.span;
                    frames.push(ExecFrame::Program {
                        instructions: Rc::clone(&instructions),
                        ip: ip + 1,
                    });
                    if let Some(returns) =
                        self.execute_instruction(&instruction.kind, &mut frames, bindings)?
                    {
                        return Ok(returns);
                    }
                }
                ExecFrame::While { condition, body } => {
                    self.run_while_frame(condition, body, &mut frames, bindings)?;
                }
                ExecFrame::Range {
                    cursor,
                    loop_bindings,
                    body,
                    span,
                } => {
                    self.run_range_frame(cursor, loop_bindings, body, span, &mut frames, bindings)?;
                }
                ExecFrame::Collection(frame) => {
                    self.run_collection_frame(frame, &mut frames, bindings)?;
                }
            }
        }

        match mode {
            FrameMode::HandlerBody => Ok(Vec::new()),
            FrameMode::FunctionBody => Err(unsupported_error(
                labels::WITHOUT_FINAL_RETURN,
                None,
                self.string_table,
            )),
        }
    }

    /// Run one instruction.
    ///
    /// Returns the values a `return` instruction produces; `None` keeps executing the frame
    /// stack. `break` and `continue` repair the stack here instead of escaping it.
    fn execute_instruction(
        &mut self,
        instruction: &ConstInstructionKind,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        match instruction {
            ConstInstructionKind::Bind {
                path,
                expression,
                value_mode,
            } => self.execute_bind(*path, *expression, *value_mode, bindings)?,
            ConstInstructionKind::BindValue { path, value } => {
                bindings.insert(*path, *value);
            }
            ConstInstructionKind::Assign { place, expression } => {
                self.execute_assign(*place, *expression, bindings)?;
            }
            ConstInstructionKind::Eval(expression) => {
                if !self.execute_collection_mutation(*expression, bindings)? {
                    self.evaluate_expression(*expression, bindings)?;
                }
            }
            ConstInstructionKind::If {
                condition,
                then_body,
                else_body,
            } => self.execute_if(*condition, then_body, else_body, frames, bindings)?,
            ConstInstructionKind::While { condition, body } => {
                self.execute_while(*condition, body, frames, bindings)?;
            }
            ConstInstructionKind::Range {
                bindings: loop_bindings,
                range,
                body,
                span,
            } => self.execute_range_loop(loop_bindings, range, body, *span, frames, bindings)?,
            ConstInstructionKind::Collection {
                bindings: loop_bindings,
                iterable,
                body,
                span,
            } => self.execute_collection_loop(
                loop_bindings,
                *iterable,
                body,
                *span,
                frames,
                bindings,
            )?,
            ConstInstructionKind::ReturnFromFunction(values) => {
                return self.execute_return_from_function(values, bindings);
            }
            ConstInstructionKind::Return(values) => return self.execute_return(values, bindings),
            ConstInstructionKind::Break => execute_break(frames, self.string_table)?,
            ConstInstructionKind::Continue => execute_continue(frames, self.string_table)?,
            ConstInstructionKind::Assert { condition, message } => {
                self.execute_assert(*condition, *message, bindings)?;
            }
        }
        Ok(None)
    }

    /// Bind one declaration, recording a mutable alias target when the value names a place.
    fn execute_bind(
        &mut self,
        path: PathId,
        expression: ExprId,
        value_mode: ValueMode,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let alias_target = if value_mode.is_mutable() {
            direct_alias_target(expression, self.builder, bindings)?
        } else {
            None
        };
        let value = self.evaluate_expression(expression, bindings)?;
        if let Some((root, projections)) = alias_target {
            let (root, projections) = expand_alias_target(root, projections, &self.state.aliases);
            self.state
                .aliases
                .insert(path, AliasTarget { root, projections });
        } else {
            self.state.aliases.remove(&path);
        }
        bindings.insert(path, value);
        Ok(())
    }

    /// Assign one evaluated value through a place, refreshing the bound root and its aliases.
    fn execute_assign(
        &mut self,
        place: PlaceId,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let value = self.evaluate_expression(expression, bindings)?;
        if self.state.pending_handler_return.is_some() {
            return Ok(());
        }
        let (root, projections) = self.evaluate_place_projections(place, bindings)?;
        let (root, projections) = expand_alias_target(root, projections, &self.state.aliases);
        if projections.is_empty() && !bindings.contains_key(&root) {
            // A declaration lowering can assign its result to a fresh temporary place before the
            // body has bound that place.
            self.write_binding_value(root, value, bindings)?;
        } else {
            let root_value = bindings.get(&root).copied().ok_or_else(|| {
                unsupported_error(
                    labels::ASSIGNMENT_TARGET,
                    self.builder
                        .place(place)
                        .ok()
                        .and_then(|view| view.metadata().span),
                    self.string_table,
                )
            })?;
            let mut projected_write = ProjectionWrite {
                constants: &mut *self.constants,
                path_fork: self.path_fork,
                budget: &mut *self.budget,
                string_table: &mut *self.string_table,
            };
            let updated =
                assign_constant_projections(root_value, &projections, value, &mut projected_write)?;
            self.write_binding_value(root, updated, bindings)?;
        }
        Ok(())
    }

    fn execute_if(
        &mut self,
        condition: ExprId,
        then_body: &Rc<[ConstInstruction]>,
        else_body: &Rc<[ConstInstruction]>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let condition = self.evaluate_bool(condition, bindings)?;
        let body = if condition { then_body } else { else_body };
        if !body.is_empty() {
            frames.push(ExecFrame::Program {
                instructions: Rc::clone(body),
                ip: 0,
            });
        }
        Ok(())
    }

    fn execute_while(
        &mut self,
        condition: ExprId,
        body: &Rc<[ConstInstruction]>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        if self.evaluate_bool(condition, bindings)? {
            frames.push(ExecFrame::While {
                condition,
                body: Rc::clone(body),
            });
            frames.push(ExecFrame::Program {
                instructions: Rc::clone(body),
                ip: 0,
            });
        }
        Ok(())
    }

    fn execute_range_loop(
        &mut self,
        loop_bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let range = self.evaluated_range_spec(range, bindings)?;
        let cursor = ConstRangeCursor::new_lenient(
            &range,
            self.builder,
            self.constants,
            MAX_LOOP_ITERATIONS,
            span,
            None,
        )
        .map_err(template_error)?;
        frames.push(ExecFrame::Range {
            cursor,
            loop_bindings: Box::new(loop_bindings.clone()),
            body: Rc::clone(body),
            span,
        });
        Ok(())
    }

    fn execute_collection_loop(
        &mut self,
        loop_bindings: &LoopBindings,
        iterable: ExprId,
        body: &Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let iterable = self.evaluate_expression(iterable, bindings)?;
        if self.state.pending_handler_return.is_some() {
            return Ok(());
        }
        let Some(ConstValuePayload::Collection(values)) = self.constants.payload(iterable) else {
            return Err(unsupported_error(
                labels::COLLECTION_LOOP,
                span,
                self.string_table,
            ));
        };
        self.budget.retain_values(values.len(), span)?;
        self.budget.retain_memory_bytes(
            values
                .len()
                .saturating_mul(std::mem::size_of::<ConstValueId>()),
            span,
        )?;
        let values = values.clone();
        if !values.is_empty() {
            frames.push(ExecFrame::Collection(CollectionFrame {
                values,
                next: 0,
                loop_bindings: Box::new(loop_bindings.clone()),
                body: Rc::clone(body),
            }));
        }
        Ok(())
    }

    /// Evaluate a handler `return` and keep it pending for the owning function boundary.
    fn execute_return_from_function(
        &mut self,
        values: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        let returns = self.evaluate_return_values(values, bindings)?;
        self.state.pending_handler_return = Some(returns.clone());
        Ok(Some(returns))
    }

    /// Evaluate an ordinary `return`; a pending handler return wins at the function boundary.
    fn execute_return(
        &mut self,
        values: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        let mut returns = Vec::with_capacity(values.len());
        for &expression in values {
            returns.push(self.evaluate_expression(expression, bindings)?);
            if self.state.pending_handler_return.is_some() {
                // Leave the pending return for the function boundary; this instruction must not
                // consume it as ordinary recovery.
                return Ok(Some(Vec::new()));
            }
        }
        Ok(Some(returns))
    }

    fn evaluate_return_values(
        &mut self,
        values: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let mut returns = Vec::with_capacity(values.len());
        for &expression in values {
            returns.push(self.evaluate_expression(expression, bindings)?);
        }
        Ok(returns)
    }

    fn execute_assert(
        &mut self,
        condition: ExprId,
        message: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let condition_value = self.evaluate_bool(condition, bindings)?;
        if self.state.pending_handler_return.is_some() {
            return Ok(());
        }
        if !condition_value {
            // Runtime assertion lowering evaluates the message only on the failure branch, so a
            // passing assertion must not execute it.
            self.evaluate_expression(message, bindings)?;
            return Err(ConstantFoldError::Diagnostic(
                choices_control_flow::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::ConstAssertionFailed,
                    None,
                    self.builder
                        .view(condition)
                        .ok()
                        .and_then(|view| view.span()),
                ),
            ));
        }
        Ok(())
    }

    fn run_while_frame(
        &mut self,
        condition: ExprId,
        body: Rc<[ConstInstruction]>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        if self.evaluate_bool(condition, bindings)? {
            frames.push(ExecFrame::While {
                condition,
                body: Rc::clone(&body),
            });
            frames.push(ExecFrame::Program {
                instructions: body,
                ip: 0,
            });
        }
        Ok(())
    }

    fn run_range_frame(
        &mut self,
        mut cursor: ConstRangeCursor,
        loop_bindings: Box<LoopBindings>,
        body: Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some(counter) = cursor.next_counter().map_err(template_error)? else {
            return Ok(());
        };
        let zero_based_index = cursor.iteration_count() - 1;
        self.bind_range_iteration(&loop_bindings, counter, zero_based_index, bindings)?;
        frames.push(ExecFrame::Range {
            cursor,
            loop_bindings,
            body: Rc::clone(&body),
            span,
        });
        frames.push(ExecFrame::Program {
            instructions: body,
            ip: 0,
        });
        Ok(())
    }

    fn run_collection_frame(
        &mut self,
        mut frame: CollectionFrame,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some(&item_value) = frame.values.get(frame.next) else {
            return Ok(());
        };
        self.bind_collection_iteration(&frame.loop_bindings, item_value, frame.next, bindings)?;
        frame.next += 1;
        let body = Rc::clone(&frame.body);
        frames.push(ExecFrame::Collection(frame));
        frames.push(ExecFrame::Program {
            instructions: body,
            ip: 0,
        });
        Ok(())
    }

    /// Evaluate one range bound and replace it with its constant expression.
    fn evaluated_range_spec(
        &mut self,
        range: &RangeLoopSpec,
        bindings: &mut FrameBindings,
    ) -> Result<RangeLoopSpec, ConstantFoldError> {
        Ok(RangeLoopSpec {
            start: self.evaluate_range_bound(range.start, bindings)?,
            end: self.evaluate_range_bound(range.end, bindings)?,
            end_kind: range.end_kind,
            step: range
                .step
                .map(|step| self.evaluate_range_bound(step, bindings))
                .transpose()?,
            domain: range.domain,
        })
    }

    fn evaluate_range_bound(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<ExprId, ConstantFoldError> {
        let (type_id, span, value_mode) = {
            let view = self.builder.view(expression)?;
            let type_id = view.success_shape().single().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "range bound used before its single numeric slot was available",
                )
            })?;
            (type_id, view.span(), view.value_mode())
        };
        let value = self.evaluate_expression(expression, bindings)?;
        if self.state.pending_handler_return.is_some() {
            return Ok(expression);
        }
        expression_with_value(self.builder, value, type_id, span, value_mode)
    }

    fn bind_range_iteration(
        &mut self,
        loop_bindings: &LoopBindings,
        counter: moth_lexical::numeric::fixed_scalar::FixedScalarValue,
        zero_based_index: usize,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let built = build_range_iteration_bindings(
            loop_bindings,
            counter,
            zero_based_index,
            &Default::default(),
            self.builder,
            self.constants,
        )
        .map_err(template_error)?;
        for binding in built {
            let value = constant_operation(self.builder, binding.value, self.string_table)?;
            bindings.insert(binding.path, value);
        }
        Ok(())
    }

    fn bind_collection_iteration(
        &mut self,
        loop_bindings: &LoopBindings,
        item_value: ConstValueId,
        source_index: usize,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let item_type = self
            .constants
            .metadata(item_value)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "collection loop item is outside its constant owner",
                )
            })?
            .type_id;
        let item_expression = expression_with_value(
            self.builder,
            item_value,
            item_type,
            None,
            ValueMode::ImmutableOwned,
        )?;
        let built = build_collection_iteration_bindings(
            loop_bindings,
            item_expression,
            source_index,
            &Default::default(),
            self.builder,
            self.constants,
        )
        .map_err(template_error)?;
        for binding in built {
            let value = constant_operation(self.builder, binding.value, self.string_table)?;
            bindings.insert(binding.path, value);
        }
        Ok(())
    }

    fn evaluate_bool(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<bool, ConstantFoldError> {
        let value = self.evaluate_expression(expression, bindings)?;
        if self.state.pending_handler_return.is_some() {
            return Ok(false);
        }
        match self.constants.payload(value) {
            Some(ConstValuePayload::Bool(boolean)) => Ok(*boolean),
            _ => Err(unsupported_error(
                labels::CONDITION,
                self.builder.view(expression)?.span(),
                self.string_table,
            )),
        }
    }

    /// Evaluate one typed expression to a constant value, consuming inherited recovery once.
    pub(super) fn evaluate_expression(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<ConstValueId, ConstantFoldError> {
        if self.state.expression_depth >= MAX_EXPRESSION_DEPTH {
            return Err(hard_limit_error(
                self.builder.view(expression)?.span(),
                ConstBudgetCategory::Work,
                ConstBudgetUnit::ExpressionLevels,
                MAX_EXPRESSION_DEPTH,
            ));
        }
        let depth = self.state.expression_depth;
        let value_count = self.constants.value_count();
        self.state.expression_depth += 1;
        let result = self.evaluate_expression_inner(expression, bindings);
        self.state.expression_depth -= 1;
        // Charge new value rows before the recovery marker is consulted: a budget refusal while
        // charging must escape even when a handler recovered part of this expression. The marker
        // substitutes a recovery value for the evaluation result, but never suppresses this
        // charge.
        if result.is_ok() {
            charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        }
        if let Some(value) = self.state.inherited_handler_value {
            // Recovery belongs to the outermost handled expression. Nested evaluations use the
            // recovery value but leave the marker set so the boundary can return it once.
            if depth == 0 {
                self.state.inherited_handler_recovered = false;
                return Ok(self.state.inherited_handler_value.take().unwrap_or(value));
            }
            return Ok(value);
        }
        if let Some(value) = self
            .state
            .pending_handler_return
            .as_ref()
            .and_then(|values| values.first())
            .copied()
        {
            return Ok(value);
        }
        result
    }

    fn evaluate_expression_inner(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<ConstValueId, ConstantFoldError> {
        if let Some(value) = self.evaluate_collection_read(expression, bindings)? {
            return Ok(value);
        }
        // Resolve the selected catch body before touching the inherited handler: handler lookup
        // reads only the prepared frame tables, and an infrastructure error here must not clear
        // live state.
        let own_handler = self.fallible_handler_instructions(expression)?;
        let own_error_binding = fallible_handler_error_binding(self.builder, expression)?;
        let inherited_handler = std::mem::take(&mut self.state.active_handler);
        self.state.active_handler = ActiveHandler {
            body: own_handler.or_else(|| inherited_handler.body.clone()),
            error_binding: own_error_binding.or(inherited_handler.error_binding),
            boundary: false,
        };
        let release_provenances = rustc_hash::FxHashMap::default();
        let rewritten = self.rewrite_with_bindings(
            expression,
            RewriteMode::InterpretedBody,
            &release_provenances,
            bindings,
        );
        self.state.active_handler = inherited_handler;
        let rewritten = rewritten?;
        if let Some(value) = self.evaluate_collection_read(rewritten, bindings)? {
            return Ok(value);
        }
        if let Some(value) = self.fold_to_constant(rewritten, bindings)? {
            return Ok(value);
        }
        if let ExpressionOperationView::Copy(place) = self.builder.view(rewritten)?.operation() {
            let (root, projections) = self.evaluate_place_projections(place.id(), bindings)?;
            if let Some(root_value) = bindings.get(&root).copied() {
                return project_constant_value(
                    self.constants,
                    root_value,
                    &projections,
                    self.path_fork,
                )
                .ok_or_else(|| {
                    unsupported_error(
                        labels::PROJECTED_READ,
                        self.builder
                            .view(rewritten)
                            .ok()
                            .and_then(|view| view.span()),
                        self.string_table,
                    )
                });
            }
        }
        if let Some(projected) = self.fold_template_with_bindings(rewritten, bindings)? {
            return Ok(projected.value);
        }
        Err(unsupported_error(
            labels::VALUE_EXPRESSION,
            self.builder
                .view(expression)
                .ok()
                .and_then(|view| view.span()),
            self.string_table,
        ))
    }

    /// Find how a collection failure on this expression must be handled.
    pub(super) fn fallible_handler_instructions(
        &self,
        expression: ExprId,
    ) -> Result<Option<Rc<[ConstInstruction]>>, ConstantFoldError> {
        super::places::fallible_handler_instructions(self.builder, self.state, expression)
    }

    /// Attach the active call chain to the first escaping diagnostic of this attempt.
    ///
    /// WHY: call labels belong to the diagnostic, while the chain describes budgeted call nesting.
    ///      Reading both owners here keeps the one-shot latch with the state it labels, and stops a
    ///      later attempt from labelling a diagnostic that already escaped.
    pub(super) fn attach_call_chain(&mut self, error: ConstantFoldError) -> ConstantFoldError {
        if self.state.call_chain_attached {
            return error;
        }
        let chain: Vec<SourceSpan> = self
            .budget
            .call_stack
            .iter()
            .filter_map(|(_, span)| *span)
            .collect();
        match error {
            ConstantFoldError::Diagnostic(diagnostic) => {
                self.state.call_chain_attached = true;
                ConstantFoldError::Diagnostic(attach_call_site_labels(diagnostic, &chain))
            }
            ConstantFoldError::NumericFailure(diagnostic) => {
                self.state.call_chain_attached = true;
                ConstantFoldError::NumericFailure(attach_call_site_labels(diagnostic, &chain))
            }
            ConstantFoldError::Budget(mut exhaustion) => {
                if exhaustion.call_chain.is_empty() {
                    exhaustion.call_chain = chain;
                }
                self.state.call_chain_attached = true;
                ConstantFoldError::Budget(exhaustion)
            }
            ConstantFoldError::Infrastructure(error) => ConstantFoldError::Infrastructure(error),
        }
    }

    /// Folds a template expression exposed after call substitution bound its parameter references.
    ///
    /// WHY: expression parsing folds templates whose pieces are already constant. A template that
    /// references a const-function parameter stays durable until the executor binds that
    /// parameter, so the executor re-enters the same TIR preparation and fold authority rather
    /// than reading template syntax itself.
    pub(super) fn fold_template_with_bindings(
        &mut self,
        expression: ExprId,
        bindings: &FrameBindings,
    ) -> Result<Option<ProjectedConstant>, ConstantFoldError> {
        let (reference, template_span, value_mode) = {
            let view = self.builder.view(expression)?;
            let ExpressionOperationView::Template(template) = view.operation() else {
                return Ok(None);
            };
            (template.tir_reference, view.span(), view.value_mode())
        };
        let template_store = self.scope.template_ir_store.borrow();
        let tir_view = TirView::with_minimum_phase(
            &template_store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )?;
        let mut bound_paths = bindings.keys().copied().collect::<Vec<_>>();
        bound_paths.sort_unstable();
        let preparation = prepare_tir_view_with_source_scope_and_bindings(
            &tir_view,
            self.builder,
            self.constants,
            TemplatePreparationMode::ConstRequired,
            self.scope,
            &bound_paths,
            Some(self.executables),
        )?;
        if !matches!(preparation.outcome, TemplatePreparationOutcome::Foldable)
            || matches!(
                preparation.facts.final_value_kind,
                TemplateConstValueKind::WrapperTemplate
            )
        {
            return Ok(None);
        }
        let mut fold_bindings = Vec::with_capacity(bindings.len());
        for (path, value) in bindings {
            let metadata = self.constants.metadata(*value).ok_or_else(|| {
                unsupported_error(labels::TEMPLATE_VALUE, template_span, self.string_table)
            })?;
            let constant_expression = self.builder.constant(
                *value,
                metadata.type_id,
                metadata.span,
                metadata.value_mode,
            )?;
            let annotated_expression = self.builder.with_synthetic_interface_provenance(
                constant_expression,
                metadata.synthetic_interface_provenance.clone(),
            )?;
            fold_bindings.push(TemplateFoldBinding {
                path: *path,
                value: annotated_expression.id(),
            });
        }
        let builder = &mut *self.builder;
        let constants = &mut *self.constants;
        let string_table = &mut *self.string_table;
        let budget = &mut *self.budget;
        let state = &mut *self.state;
        let template_output_limits = budget.template_output_limits(template_span);
        let template_work = budget.template_work(template_span);
        let value_count = constants.value_count();
        let mut site_rewriter = ExecutorSiteRewriter {
            executables: self.executables,
            types: self.types,
            path_fork: self.path_fork,
            budget,
            state,
        };
        let result = {
            let mut fold_context =
                self.scope
                    .new_tir_fold_context(string_table, builder, constants);
            fold_context.template_output_limits = Some(template_output_limits);
            fold_context.template_work = Some(template_work);
            fold_context.bindings = fold_bindings;
            fold_context.required_call_rewriter = Some(&mut site_rewriter);
            fold_prepared_template(&preparation, tir_view, &mut fold_context)?
        };
        let mut result = result;
        if let Some(diagnostic) = result.take_required_diagnostic() {
            return Err(ConstantFoldError::Diagnostic(diagnostic));
        }
        let string_value = match result.emission {
            TemplateEmission::Output(value) => value,
            TemplateEmission::NoOutput => ConstStringValue::Text(string_table.intern("")),
        };
        let mut metadata = {
            let view = builder.view(expression)?;
            ConstValueStore::metadata_from_view(view, ConstValueKind::RenderableTemplate)?
        };
        metadata.synthetic_interface_provenance = metadata
            .synthetic_interface_provenance
            .union(&result.provenance);
        let type_id = metadata.type_id;
        let value = constants.insert_value(metadata, ConstValuePayload::String(string_value))?;
        charge_new_value_text(constants, string_table, value_count, budget)?;
        Ok(Some(ProjectedConstant {
            value,
            type_id,
            span: template_span,
            value_mode,
        }))
    }
}

/// Executes required const calls at the selected TIR expression site with its active bindings.
struct ExecutorSiteRewriter<'a> {
    executables: &'a FxHashMap<PathId, ConstFunctionExecutable>,
    types: &'a TypeEnvironment,
    path_fork: &'a PathInternerFork,
    budget: &'a mut ExecutionBudget,
    state: &'a mut ExecutionState,
}

impl RequiredCallSiteRewriter for ExecutorSiteRewriter<'_> {
    fn rewrite_site(
        &mut self,
        expression: ExprId,
        expression_builder: &mut AstExpressionBuilder,
        const_values: &mut ConstValueStore,
        string_table: &mut StringTable,
        source_scope: Option<&ScopeContext>,
        bindings: &[TemplateFoldBinding],
    ) -> Result<ExprId, ConstantFoldError> {
        let Some(scope) = source_scope else {
            return Ok(expression);
        };
        let mut value_bindings = FrameBindings::default();
        for binding in bindings {
            if let ExpressionOperationView::Constant(value) =
                expression_builder.view(binding.value)?.operation()
            {
                value_bindings.insert(binding.path, value);
            }
        }
        let release_provenances = rustc_hash::FxHashMap::default();
        let mut services = ConstExecutionServices {
            builder: expression_builder,
            constants: const_values,
            executables: self.executables,
            scope,
            string_table,
            types: self.types,
            path_fork: self.path_fork,
        };
        let mut interpreter = services.interpreter(&mut *self.budget, &mut *self.state);
        interpreter.rewrite_with_bindings(
            expression,
            RewriteMode::InterpretedBody,
            &release_provenances,
            &mut value_bindings,
        )
    }
}

/// Truncate the frame stack at the nearest active loop for `break`.
fn execute_break(
    frames: &mut Vec<ExecFrame>,
    string_table: &mut StringTable,
) -> Result<(), ConstantFoldError> {
    if let Some(index) = nearest_loop(frames) {
        frames.truncate(index);
        Ok(())
    } else {
        Err(unsupported_error(labels::LOOP_CONTROL, None, string_table))
    }
}

/// Resume the nearest active loop for `continue`.
fn execute_continue(
    frames: &mut Vec<ExecFrame>,
    string_table: &mut StringTable,
) -> Result<(), ConstantFoldError> {
    if let Some(index) = nearest_loop(frames) {
        frames.truncate(index + 1);
        Ok(())
    } else {
        Err(unsupported_error(labels::LOOP_CONTROL, None, string_table))
    }
}

fn nearest_loop(frames: &[ExecFrame]) -> Option<usize> {
    frames.iter().rposition(|frame| {
        matches!(
            frame,
            ExecFrame::While { .. } | ExecFrame::Range { .. } | ExecFrame::Collection { .. }
        )
    })
}

/// Label one escaping diagnostic with every active const call site.
fn attach_call_site_labels(
    mut diagnostic: DiagnosticDraft,
    chain: &[SourceSpan],
) -> DiagnosticDraft {
    for span in chain.iter().rev() {
        diagnostic = diagnostic.with_label(Some(*span), Some(LabelDraftMessage::ConstCallSite));
    }
    diagnostic
}
