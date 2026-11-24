//! The bounded typed-AST frame machine.
//!
//! WHAT: executes prepared const instructions and typed expressions with explicit heap frames,
//!       call depth and budget charges. It owns expression evaluation, range/collection loop
//!       bindings and the TIR template-folding adapter.
//! WHY: a const call can execute before HIR exists. One frame machine sequences the interpreter
//!      context and reuses declaration-time instruction compilation for every call.
//! MUST NOT: decide whether a body is eligible or rewrite an unreachable AST.

use super::{
    AliasTarget, ConstFunctionExecutable, ConstValueId, ProjectedConstant, ResolvedPlaceProjection,
    budget::{
        CallDepthRefusal, ExecutionBudget, MAX_CALL_DEPTH, MAX_EXPRESSION_DEPTH, MAX_HANDLER_DEPTH,
        MAX_LOOP_ITERATIONS, charge_new_value_text,
    },
    instructions::{ConstInstruction, ConstInstructionKind, ConstMatchArm, ValueReceiver},
    labels,
    places::{
        direct_alias_target, expand_alias_target, fallible_handler_error_binding,
        unwrap_fallible_expression,
    },
    rewrite::{RewriteMode, RewriteOutput},
    state::{
        ActiveHandler, ConstExecutionServices, ConstInterpreter, ExecutionState, FrameBindings,
        FrameOutcome,
    },
    values::{
        ProjectionWrite, assign_constant_projections, constant_result_block_slots,
        expression_with_value, hard_limit_error, project_constant_value, template_error,
        unsupported_error,
    },
};
use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, MultiBindTargetKind, RangeLoopSpec};
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetExhaustion, ConstBudgetUnit, ConstantFoldError,
    OwnedConstCallSite,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, PlaceId, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, optional_cast_catch_target,
};
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    ConstRangeCursor, TemplateFoldBinding, const_iteration_index,
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
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
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
    /// The receiver of an active value block. The branch's `then` unwinds to it; `return`,
    /// `break` and `continue` discard it with the frames above their own target.
    ValueBlock(Rc<ValueReceiver>),
}

/// One active collection-loop frame.
struct CollectionFrame {
    iterable: ConstValueId,
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
    ///
    /// `body` is the executable's identity in the executables table. Call sites inside it resolve
    /// their authored owner through that identity; `None` covers direct executor entry that has no
    /// table row.
    pub(super) fn execute_body(
        &mut self,
        body: Option<PathId>,
        executable: &ConstFunctionExecutable,
        mut parameter_values: FrameBindings,
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let saved_aliases = std::mem::take(&mut self.state.aliases);
        let saved_handler_bodies = self.state.handler_bodies.take();
        let saved_body = std::mem::replace(&mut self.state.current_body, body);
        // The budget span names the source that spends the allowance. A callee leaves its last
        // instruction there, and its first frame charges before reading its own instruction span,
        // so enter the callee's source now and restore the caller's span on unwind. Otherwise a
        // refusal on entry or after return pairs one body's span with the other body's donor.
        let saved_span = self.budget.span;
        self.budget.span = executable
            .instructions
            .first()
            .and_then(|instruction| instruction.span);
        self.state.handler_bodies = Some(Rc::clone(&executable.handler_bodies));
        let result = self.execute_frames(
            &executable.instructions,
            FrameMode::FunctionBody,
            &mut parameter_values,
        );
        self.state.aliases = saved_aliases;
        self.state.handler_bodies = saved_handler_bodies;
        self.state.current_body = saved_body;
        self.budget.span = saved_span;
        let result = result
            .map_err(|error| attach_donor_identity(error, executable.donor_identity.as_ref()))?;
        match result {
            FrameOutcome::Return(slots) => Ok(slots),
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const control transfer escaped its function",
            )
            .into()),
        }
    }

    /// Run a selected catch in isolation, with transfers reported to the enclosing frame.
    pub(super) fn execute_handler_body(
        &mut self,
        instructions: &Rc<[ConstInstruction]>,
        bindings: &mut FrameBindings,
    ) -> Result<FrameOutcome, ConstantFoldError> {
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

    /// Run one instruction stream with its own expression-recovery context.
    ///
    /// Callers receive any frame outcome explicitly; their in-flight handler state is restored
    /// even on failure. Depths remain shared to bound native recursion.
    fn execute_frames(
        &mut self,
        instructions: &Rc<[ConstInstruction]>,
        mode: FrameMode,
        bindings: &mut FrameBindings,
    ) -> Result<FrameOutcome, ConstantFoldError> {
        let saved_handler = std::mem::take(&mut self.state.active_handler);
        let saved_outcome = self.state.outcome.take();
        let result = self.run_frames(instructions, mode, bindings);
        self.state.active_handler = saved_handler;
        self.state.outcome = saved_outcome;
        result
    }

    /// Run the frame machine until it returns, falls through, or hits a refusal.
    fn run_frames(
        &mut self,
        instructions: &Rc<[ConstInstruction]>,
        mode: FrameMode,
        bindings: &mut FrameBindings,
    ) -> Result<FrameOutcome, ConstantFoldError> {
        let mut frames = vec![ExecFrame::Program {
            instructions: Rc::clone(instructions),
            ip: 0,
        }];

        while let Some(frame) = frames.pop() {
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
                    self.execute_instruction(&instruction.kind, &mut frames, bindings)?;
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
                ExecFrame::ValueBlock(_) => {
                    // The AST rejects a producing branch that can fall through.
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "const value block branch finished without producing its values",
                    )
                    .into());
                }
            }
            if let Some(outcome) = self.state.outcome.take() {
                let keep_loop = match outcome {
                    FrameOutcome::Return(_) | FrameOutcome::Recovered(_) => return Ok(outcome),
                    FrameOutcome::Break => false,
                    FrameOutcome::Continue => true,
                };
                if let Some(index) = nearest_loop(&frames) {
                    frames.truncate(index + usize::from(keep_loop));
                } else {
                    // A handler can target a loop in its enclosing frame stream.
                    return Ok(outcome);
                }
            }
        }

        match mode {
            FrameMode::HandlerBody => Ok(FrameOutcome::Recovered(Vec::new())),
            FrameMode::FunctionBody => Err(unsupported_error(
                labels::WITHOUT_FINAL_RETURN,
                None,
                self.string_table,
            )),
        }
    }

    /// Run one instruction.
    fn execute_instruction(
        &mut self,
        instruction: &ConstInstructionKind,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
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
                // A collection mutation has at most one slot, so a several-slot statement is a
                // discarded const call.
                if self.has_multiple_success_slots(*expression)? {
                    self.evaluate_result_block(*expression, bindings)?;
                } else if !self.execute_collection_mutation(*expression, bindings)? {
                    self.evaluate_expression(*expression, bindings)?;
                }
            }
            ConstInstructionKind::If {
                condition,
                then_body,
                else_body,
            } => self.execute_if(*condition, then_body, else_body, frames, bindings)?,
            ConstInstructionKind::Match {
                scrutinee,
                arms,
                default,
            } => self.execute_match(*scrutinee, arms, default.as_ref(), frames, bindings)?,
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
            ConstInstructionKind::Return(values) | ConstInstructionKind::Recover(values) => {
                if let Some(slots) = self.evaluate_value_slots(values, bindings)? {
                    self.state.outcome = Some(match instruction {
                        ConstInstructionKind::Return(_) => FrameOutcome::Return(slots),
                        _ => FrameOutcome::Recovered(slots),
                    });
                }
            }
            ConstInstructionKind::MultiBind {
                expression,
                receiver,
            } => {
                if let Some(slots) =
                    self.evaluate_value_slots(std::slice::from_ref(expression), bindings)?
                {
                    self.receive_values(receiver, slots, bindings)?;
                }
            }
            ConstInstructionKind::ValueBlock { body, receiver } => {
                frames.push(ExecFrame::ValueBlock(Rc::clone(receiver)));
                frames.push(ExecFrame::Program {
                    instructions: Rc::clone(body),
                    ip: 0,
                });
            }
            ConstInstructionKind::Then(values) => {
                self.execute_then(values, frames, bindings)?;
            }
            ConstInstructionKind::Break => self.state.outcome = Some(FrameOutcome::Break),
            ConstInstructionKind::Continue => self.state.outcome = Some(FrameOutcome::Continue),
            ConstInstructionKind::Assert { condition, message } => {
                self.execute_assert(*condition, *message, bindings)?;
            }
        }
        Ok(())
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
        let Some(value) = self.evaluate_expression(expression, bindings)? else {
            return Ok(());
        };
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
        let Some(value) = self.evaluate_expression(expression, bindings)? else {
            return Ok(());
        };
        self.assign_value(place, value, bindings)
    }

    fn assign_value(
        &mut self,
        place: PlaceId,
        value: ConstValueId,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some((root, projections)) = self.evaluate_place_projections(place, bindings)? else {
            return Ok(());
        };
        let span = self
            .builder
            .place(place)
            .ok()
            .and_then(|view| view.metadata().span);
        self.write_through(root, projections, value, span, bindings)
    }

    /// Write through an existing root and its projections, following mutable aliases to their
    /// referent and refreshing every alias of the written root.
    fn write_through(
        &mut self,
        root: PathId,
        projections: Vec<ResolvedPlaceProjection>,
        value: ConstValueId,
        span: Option<SourceSpan>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let (root, projections) = expand_alias_target(root, projections, &self.state.aliases);
        if projections.is_empty() {
            // A whole-root write needs no projected constant. A declaration lowering can also
            // assign its result to a fresh temporary place before the body has bound that place.
            return self.write_binding_value(root, value, bindings);
        }
        let root_value = bindings
            .get(&root)
            .copied()
            .ok_or_else(|| unsupported_error(labels::ASSIGNMENT_TARGET, span, self.string_table))?;
        let mut projected_write = ProjectionWrite {
            constants: &mut *self.constants,
            path_fork: self.path_fork,
            budget: &mut *self.budget,
            string_table: &mut *self.string_table,
        };
        let updated =
            assign_constant_projections(root_value, &projections, value, &mut projected_write)?;
        self.write_binding_value(root, updated, bindings)
    }

    /// Complete the innermost value block. Every slot is read before the receiver writes any
    /// target, so a later slot never observes an earlier target's update.
    fn execute_then(
        &mut self,
        values: &[ExprId],
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some(slots) = self.evaluate_value_slots(values, bindings)? else {
            return Ok(());
        };
        let receiver = loop {
            match frames.pop() {
                Some(ExecFrame::ValueBlock(receiver)) => break receiver,
                Some(_) => {}
                None => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "const `then` ran outside its value block",
                    )
                    .into());
                }
            }
        };
        self.receive_values(&receiver, slots, bindings)
    }

    /// Deliver a value block's slots to its receiving site. A return receiver ends the frame
    /// stack; every other receiver writes its targets and keeps executing.
    fn receive_values(
        &mut self,
        receiver: &ValueReceiver,
        slots: Vec<ConstValueId>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        match receiver {
            ValueReceiver::Declaration(path) => {
                let [value] = slots[..] else {
                    return Err(receiver_slot_mismatch());
                };
                // A produced value is never a place, so the declaration aliases nothing.
                self.state.aliases.remove(path);
                bindings.insert(*path, value);
            }
            ValueReceiver::Assignment(place) => {
                let [value] = slots[..] else {
                    return Err(receiver_slot_mismatch());
                };
                self.assign_value(*place, value, bindings)?;
            }
            ValueReceiver::MultiBind(targets) => {
                if targets.len() != slots.len() {
                    return Err(receiver_slot_mismatch());
                }
                for (target, &value) in targets.iter().zip(&slots) {
                    match target.kind {
                        MultiBindTargetKind::Declaration => {
                            self.state.aliases.remove(&target.id);
                            bindings.insert(target.id, value);
                        }
                        MultiBindTargetKind::Assignment => {
                            self.write_through(
                                target.id,
                                Vec::new(),
                                value,
                                target.span,
                                bindings,
                            )?;
                        }
                    }
                }
            }
            ValueReceiver::Return => self.state.outcome = Some(FrameOutcome::Return(slots)),
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
        let Some(condition) = self.evaluate_bool(condition, bindings)? else {
            return Ok(());
        };
        let body = if condition { then_body } else { else_body };
        if !body.is_empty() {
            frames.push(ExecFrame::Program {
                instructions: Rc::clone(body),
                ip: 0,
            });
        }
        Ok(())
    }

    fn execute_match(
        &mut self,
        scrutinee: ExprId,
        arms: &[ConstMatchArm],
        default: Option<&Rc<[ConstInstruction]>>,
        frames: &mut Vec<ExecFrame>,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some(body) = self.select_match_arm(scrutinee, arms, default, bindings)? else {
            return Ok(());
        };
        if !body.is_empty() {
            frames.push(ExecFrame::Program {
                instructions: body,
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
        if self.evaluate_bool(condition, bindings)? == Some(true) {
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
        if self.state.outcome.is_some() {
            return Ok(());
        }
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
        let Some(iterable) = self.evaluate_expression(iterable, bindings)? else {
            return Ok(());
        };
        let Some(ConstValuePayload::Collection(values)) = self.constants.payload(iterable) else {
            return Err(unsupported_error(
                labels::COLLECTION_LOOP,
                span,
                self.string_table,
            ));
        };
        if !values.is_empty() {
            frames.push(ExecFrame::Collection(CollectionFrame {
                iterable,
                next: 0,
                loop_bindings: Box::new(loop_bindings.clone()),
                body: Rc::clone(body),
            }));
        }
        Ok(())
    }

    /// Evaluate `return` or `then` values in order, expanding each multi-result producer into its
    /// slots. A handler transfer interrupts the whole receiving site.
    fn evaluate_value_slots(
        &mut self,
        values: &[ExprId],
        bindings: &mut FrameBindings,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        if let [expression] = values
            && self.has_multiple_success_slots(*expression)?
        {
            return self.evaluate_result_block(*expression, bindings);
        }
        let mut slots = Vec::with_capacity(values.len());
        for &expression in values {
            if self.has_multiple_success_slots(expression)? {
                let Some(produced) = self.evaluate_result_block(expression, bindings)? else {
                    return Ok(None);
                };
                slots.extend(produced);
            } else {
                let Some(value) = self.evaluate_expression(expression, bindings)? else {
                    return Ok(None);
                };
                slots.push(value);
            }
        }
        Ok(Some(slots))
    }

    /// Whether `expression` produces several ordered success slots.
    fn has_multiple_success_slots(&self, expression: ExprId) -> Result<bool, ConstantFoldError> {
        Ok(matches!(
            self.builder.view(expression)?.success_shape(),
            SuccessShape::Multiple(_)
        ))
    }

    fn execute_assert(
        &mut self,
        condition: ExprId,
        message: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let Some(condition_value) = self.evaluate_bool(condition, bindings)? else {
            return Ok(());
        };
        if !condition_value {
            // Runtime assertion lowering evaluates the message only on the failure branch, so a
            // passing assertion must not execute it.
            if self.evaluate_expression(message, bindings)?.is_none() {
                return Ok(());
            }
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
        if self.evaluate_bool(condition, bindings)? == Some(true) {
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
        let Some(ConstValuePayload::Collection(values)) = self.constants.payload(frame.iterable)
        else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const collection loop lost its iterable value",
            )
            .into());
        };
        let Some(&item_value) = values.get(frame.next) else {
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
            fast_math: range.fast_math,
        })
    }

    fn evaluate_range_bound(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<ExprId, ConstantFoldError> {
        if self.state.outcome.is_some() {
            return Ok(expression);
        }
        let (type_id, span, value_mode) = {
            let view = self.builder.view(expression)?;
            if matches!(view.operation(), ExpressionOperationView::Constant(_)) {
                return Ok(expression);
            }
            let type_id = view.success_shape().single().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "range bound used before its single numeric slot was available",
                )
            })?;
            (type_id, view.span(), view.value_mode())
        };
        let Some(value) = self.evaluate_expression(expression, bindings)? else {
            return Ok(expression);
        };
        expression_with_value(self.builder, value, type_id, span, value_mode)
    }

    fn bind_range_iteration(
        &mut self,
        loop_bindings: &LoopBindings,
        counter: moth_lexical::numeric::fixed_scalar::FixedScalarValue,
        zero_based_index: usize,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let count = self.constants.value_count();
        if let Some(item) = &loop_bindings.item {
            let value = self.constants.insert_resolved(
                item.type_id,
                ConstValuePayload::FixedScalar(counter),
                item.binding_span,
            )?;
            bindings.insert(item.id, value);
        }
        self.bind_iteration_index(loop_bindings, zero_based_index, bindings)?;
        charge_new_value_text(self.constants, self.string_table, count, self.budget)
    }

    fn bind_collection_iteration(
        &mut self,
        loop_bindings: &LoopBindings,
        item_value: ConstValueId,
        source_index: usize,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        let count = self.constants.value_count();
        if let Some(item) = &loop_bindings.item {
            bindings.insert(item.id, item_value);
        }
        self.bind_iteration_index(loop_bindings, source_index, bindings)?;
        charge_new_value_text(self.constants, self.string_table, count, self.budget)
    }

    fn bind_iteration_index(
        &mut self,
        loop_bindings: &LoopBindings,
        position: usize,
        bindings: &mut FrameBindings,
    ) -> Result<(), ConstantFoldError> {
        if let Some(index) = &loop_bindings.index {
            let value =
                const_iteration_index(index, position, self.constants).map_err(template_error)?;
            bindings.insert(index.id, value);
        }
        Ok(())
    }

    pub(super) fn evaluate_bool(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<bool>, ConstantFoldError> {
        let Some(value) = self.evaluate_expression(expression, bindings)? else {
            return Ok(None);
        };
        match self.constants.payload(value) {
            Some(ConstValuePayload::Bool(boolean)) => Ok(Some(*boolean)),
            _ => Err(unsupported_error(
                labels::CONDITION,
                self.builder.view(expression)?.span(),
                self.string_table,
            )),
        }
    }

    /// Refuse an expression nested beyond the interpreter's expression-depth bound.
    fn check_expression_depth(&self, expression: ExprId) -> Result<(), ConstantFoldError> {
        if self.state.expression_depth >= MAX_EXPRESSION_DEPTH {
            return Err(hard_limit_error(
                self.builder.view(expression)?.span(),
                ConstBudgetCategory::Work,
                ConstBudgetUnit::ExpressionLevels,
                MAX_EXPRESSION_DEPTH,
            ));
        }
        Ok(())
    }

    /// A recovery belongs to its authored catch, never to a nested operand or a mutation argument.
    fn take_recovery(
        &mut self,
        expression: ExprId,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        if matches!(self.state.outcome, Some(FrameOutcome::Recovered(_)))
            && self.fallible_handler_instructions(expression)?.is_some()
            && let Some(FrameOutcome::Recovered(slots)) = self.state.outcome.take()
        {
            return Ok(Some(slots));
        }
        Ok(None)
    }

    /// Evaluate one multi-result producer to its ordered slots.
    fn evaluate_result_block(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        self.check_expression_depth(expression)?;
        let value_count = self.constants.value_count();
        self.state.expression_depth += 1;
        let rewritten = self.with_expression_handler(expression, |interpreter| {
            interpreter.rewrite_for_value(expression, bindings)
        });
        self.state.expression_depth -= 1;
        let rewritten = rewritten?;
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        if let Some(slots) = self.take_recovery(expression)? {
            return Ok(Some(slots));
        }
        if self.state.outcome.is_some() {
            return Ok(None);
        }
        let rewritten = match rewritten {
            RewriteOutput::CallResults(slots) => return Ok(Some(slots)),
            RewriteOutput::Expression(expression) => expression,
        };
        let produced = unwrap_fallible_expression(rewritten, self.builder)?;
        let Some(slots) = constant_result_block_slots(self.builder, produced)? else {
            return Err(unsupported_error(
                labels::VALUE_EXPRESSION,
                self.builder
                    .view(expression)
                    .ok()
                    .and_then(|view| view.span()),
                self.string_table,
            ));
        };
        Ok(Some(slots))
    }

    /// Evaluate one typed expression to a constant value, consuming inherited recovery once.
    pub(super) fn evaluate_expression(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        self.check_expression_depth(expression)?;
        let value_count = self.constants.value_count();
        self.state.expression_depth += 1;
        let result = self.with_expression_handler(expression, |interpreter| {
            interpreter.evaluate_expression_inner(expression, bindings)
        });
        self.state.expression_depth -= 1;
        let result = result?;
        charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
        if let Some(slots) = self.take_recovery(expression)? {
            let [value] = slots[..] else {
                return Err(receiver_slot_mismatch());
            };
            let value = self.complete_recovered_value(expression, value)?;
            charge_new_value_text(self.constants, self.string_table, value_count, self.budget)?;
            return Ok(Some(value));
        }
        Ok(if self.state.outcome.is_some() {
            None
        } else {
            result
        })
    }

    fn complete_recovered_value(
        &mut self,
        expression: ExprId,
        value: ConstValueId,
    ) -> Result<ConstValueId, ConstantFoldError> {
        let view = self.builder.view(expression)?;
        let ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) = view.operation() else {
            return Ok(value);
        };
        let protected = self.builder.view(block.handled_value)?;
        let Some(target) = optional_cast_catch_target(protected) else {
            return Ok(value);
        };
        let recovered = self
            .constants
            .metadata(value)
            .filter(|value| value.type_id == target)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "const catch recovery disagrees with the cast's inner receiving type",
                )
            })?;
        let mut metadata = ConstValueStore::metadata_from_view(view, ConstValueKind::Composite)?;
        metadata.type_id = protected.value_type_id()?;
        metadata
            .synthetic_interface_provenance
            .merge(&recovered.synthetic_interface_provenance);
        Ok(self
            .constants
            .insert_value(metadata, ConstValuePayload::OptionSome(value))?)
    }

    /// Keep an authored catch active across expression rewriting and evaluation, including
    /// completion of rebuilt expressions. A nested catch installs its own handler.
    pub(super) fn with_expression_handler<T>(
        &mut self,
        expression: ExprId,
        evaluate: impl FnOnce(&mut Self) -> Result<T, ConstantFoldError>,
    ) -> Result<T, ConstantFoldError> {
        let Some(body) = self.fallible_handler_instructions(expression)? else {
            return evaluate(self);
        };
        let error_binding = fallible_handler_error_binding(self.builder, expression)?;
        let inherited_handler = ActiveHandler::replace(self.state, Some(body), error_binding);
        let result = evaluate(self);
        self.state.active_handler = inherited_handler;
        result
    }

    fn evaluate_expression_inner(
        &mut self,
        expression: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        if let Some(value) = self.evaluate_collection_read(expression, bindings)? {
            return Ok(Some(value));
        }
        if self.state.outcome.is_some() {
            return Ok(None);
        }
        let rewritten = match self.rewrite_for_value(expression, bindings)? {
            RewriteOutput::CallResults(slots) => {
                let [value] = slots[..] else {
                    return Err(receiver_slot_mismatch());
                };
                return Ok(Some(value));
            }
            RewriteOutput::Expression(expression) => expression,
        };
        if self.state.outcome.is_some() {
            return Ok(None);
        }
        if let Some(value) = self.fold_to_constant(rewritten, bindings)? {
            return Ok(Some(value));
        }
        if let ExpressionOperationView::Copy(place) = self.builder.view(rewritten)?.operation() {
            let Some((root, projections)) =
                self.evaluate_place_projections(place.id(), bindings)?
            else {
                return Ok(None);
            };
            if let Some(root_value) = bindings.get(&root).copied() {
                return project_constant_value(
                    self.constants,
                    root_value,
                    &projections,
                    self.path_fork,
                )
                .map(Some)
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
            return Ok(Some(projected.value));
        }
        if self.state.outcome.is_some() {
            return Ok(None);
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

    /// Enter one interpreted call, converting a depth refusal into its budget diagnostic.
    ///
    /// WHY: the budget owns the frame stack and its hard bound, while this interpreter owns the
    ///      executables table that resolves each call site's declaring-domain owner.
    pub(super) fn enter_call(&mut self, span: Option<SourceSpan>) -> Result<(), ConstantFoldError> {
        self.budget
            .enter_call(span, self.state.current_body)
            .map_err(|refusal| ConstantFoldError::Budget(self.call_depth_exhaustion(refusal)))
    }

    /// Resolve one body identity's declaring-domain owner.
    ///
    /// Donor handles stay in the executables table; a diagnostic clones one only when it must own
    /// it. An identity without a table row cannot name a donor boundary.
    pub(super) fn body_donor_identity(&self, body: Option<PathId>) -> Option<FrozenIdentityHandle> {
        self.executables
            .get(&body?)
            .and_then(|executable| executable.donor_identity.clone())
    }

    /// Build the owned chain for the active calls, outermost first.
    ///
    /// Only a diagnostic construction path calls this, so a successful call never clones a donor
    /// handle.
    fn active_call_chain(&self) -> Vec<OwnedConstCallSite> {
        let mut chain = Vec::with_capacity(self.budget.call_stack.len());
        for frame in &self.budget.call_stack {
            if let Some(span) = frame.span {
                chain.push(OwnedConstCallSite {
                    span,
                    owner: self.body_donor_identity(frame.owner_body),
                });
            }
        }
        chain
    }

    /// Convert a call-depth refusal into the budget diagnostic the refused call owns.
    fn call_depth_exhaustion(&self, refusal: CallDepthRefusal) -> ConstBudgetExhaustion {
        let owner = self.body_donor_identity(refusal.owner_body);
        let mut call_chain = self.active_call_chain();
        if let Some(span) = refusal.span {
            call_chain.push(OwnedConstCallSite {
                span,
                owner: owner.clone(),
            });
        }
        ConstBudgetExhaustion {
            category: ConstBudgetCategory::Work,
            unit: ConstBudgetUnit::CallFrames,
            limit: MAX_CALL_DEPTH as u64,
            setting: None,
            span: refusal.span,
            span_owner: owner,
            call_chain,
        }
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
        let chain = self.active_call_chain();
        match error {
            ConstantFoldError::Diagnostic(diagnostic) => {
                self.state.call_chain_attached = true;
                ConstantFoldError::Diagnostic(attach_call_site_labels(diagnostic, chain))
            }
            ConstantFoldError::NumericFailure(diagnostic) => {
                self.state.call_chain_attached = true;
                ConstantFoldError::NumericFailure(attach_call_site_labels(diagnostic, chain))
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
        // TIR still receives expression IDs for its bindings. These rows live in the source
        // arena, so bound them before adapting a frame, even when the template emits no text.
        self.budget.retain_values(bindings.len(), template_span)?;
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
            &mut value_bindings,
        )
    }
}

/// The AST checks every receiving site's arity, so a mismatch here is a compiler bug.
fn receiver_slot_mismatch() -> ConstantFoldError {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "const value block produced a slot count its receiver does not accept",
    )
    .into()
}

fn nearest_loop(frames: &[ExecFrame]) -> Option<usize> {
    frames.iter().rposition(|frame| {
        matches!(
            frame,
            ExecFrame::While { .. } | ExecFrame::Range { .. } | ExecFrame::Collection { .. }
        )
    })
}

/// Resolve a foreign body's diagnostic spans against its declaring source domain.
fn attach_donor_identity(
    error: ConstantFoldError,
    donor_identity: Option<&FrozenIdentityHandle>,
) -> ConstantFoldError {
    let Some(donor_identity) = donor_identity else {
        return error;
    };
    match error {
        ConstantFoldError::Diagnostic(diagnostic) => {
            ConstantFoldError::Diagnostic(attach_donor_to_diagnostic(diagnostic, donor_identity))
        }
        ConstantFoldError::NumericFailure(diagnostic) => ConstantFoldError::NumericFailure(
            attach_donor_to_diagnostic(diagnostic, donor_identity),
        ),
        ConstantFoldError::Budget(mut exhaustion) => {
            if exhaustion.span_owner.is_none() {
                exhaustion.span_owner = Some(donor_identity.clone());
            }
            ConstantFoldError::Budget(exhaustion)
        }
        other => other,
    }
}

/// Attach a provider body's declaring domain to the diagnostic that escaped it.
///
/// The innermost provider body owns its diagnostic before any caller adds call-site labels. Once a
/// primary owner exists, a later provider unwind must leave every label alone: consumer-local call
/// sites deliberately carry no owner and must stay local.
fn attach_donor_to_diagnostic(
    mut diagnostic: DiagnosticDraft,
    donor_identity: &FrozenIdentityHandle,
) -> DiagnosticDraft {
    if diagnostic.primary_frozen_identity_handle().is_none() {
        diagnostic.attach_frozen_identity_handle_if_missing(donor_identity.clone());
    }
    diagnostic
}

/// Label one escaping diagnostic with every active const call site.
fn attach_call_site_labels(
    mut diagnostic: DiagnosticDraft,
    chain: Vec<OwnedConstCallSite>,
) -> DiagnosticDraft {
    for site in chain.into_iter().rev() {
        let index = diagnostic.label_count();
        diagnostic = diagnostic.with_label(Some(site.span), Some(LabelDraftMessage::ConstCallSite));
        if let Some(owner) = site.owner {
            diagnostic = diagnostic.with_label_frozen_identity_handle(index, owner);
        }
    }
    diagnostic
}
