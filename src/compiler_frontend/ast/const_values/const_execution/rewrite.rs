//! Required const-call rewriting over typed expressions.
//!
//! WHAT: replaces required source const calls with folded constants, both in ordinary program
//!       expressions and while a prepared const body executes.
//! WHY: folding runs before expression freezing and HIR lowering, so the final AST must already
//!      contain the replacement.
//! MUST NOT: own executable preparation, the executed-frame machine or eligibility policy. It
//!           invokes those owners and keeps only the AST rewrite transaction.

use super::budget::ExecutionBudget;
use super::labels;
use super::places::unwrap_fallible_expression;
use super::state::{ConstExecutionServices, ConstInterpreter, ExecutionState, FrameBindings};
use super::storage::{LiveValue, Permission};
use super::values::{
    attach_live_provenance, commit_call_result, constant_index, expression_with_value,
    project_constant_value, unsupported_error, validate_call_results,
};
use super::{ConstFunctionExecutable, ProjectedConstant, ResolvedPlaceProjection};
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression_rewrite::rewrite_expression_children;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, LogicalOperator,
    PlaceProjectionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use std::num::NonZeroU32;

struct ConstCallSite {
    path: PathId,
    span: Option<SourceSpan>,
}

/// Rewrite every required source const call inside one expression into its folded value.
pub(crate) fn rewrite_required_const_calls(
    mut services: ConstExecutionServices<'_>,
    root: ExprId,
) -> Result<ExprId, ConstantFoldError> {
    let mut budget = ExecutionBudget::new(services.constants.execution_meter());
    let mut state = ExecutionState::default();
    let mut interpreter = services.interpreter(&mut budget, &mut state);
    interpreter.rewrite_with_bindings(
        root,
        RewriteMode::RequiredExpression,
        &mut FrameBindings::default(),
    )
}

/// Why one typed-expression rewrite is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RewriteMode {
    /// Fold required source calls without completing unrelated runtime expressions.
    RequiredExpression,
    /// Execute a prepared const body: every completed expression may fold.
    InterpretedBody,
}

impl RewriteMode {
    /// Whether collection reads, template completion and fallible unwrapping fold on completion.
    fn folds_auxiliary(self) -> bool {
        self == Self::InterpretedBody
    }
}

/// A rewrite either leaves an expression for its consumer or delivers a root call's checked slots.
pub(super) enum RewriteOutput {
    Expression(ExprId),
    Value(LiveValue),
    CallResults(Vec<LiveValue>),
}

/// One pending rewriter action.
enum RewriteStep {
    Enter {
        expression: ExprId,
    },
    Complete {
        expression: ExprId,
        base: usize,
    },
    CompleteCall {
        expression: ExprId,
        base: usize,
        call: ConstCallSite,
    },
    HoldOperand {
        expression: ExprId,
        access: OperandAccess,
    },
    CompleteLogical {
        expression: ExprId,
        base: usize,
        right: ExprId,
        operation: LogicalOperator,
    },
    DecExponent(bool),
    RestoreSlots {
        base: usize,
        slots: Vec<usize>,
    },
}

#[derive(Clone, Copy)]
enum OperandAccess {
    SharedArgument,
    StoredValue,
}

/// One iterative typed-expression rewrite.
struct RewriteWalk<'walk, 'services> {
    interpreter: &'walk mut ConstInterpreter<'services>,
    bindings: &'walk mut FrameBindings,
    mode: RewriteMode,
    root: ExprId,
    direct_producer: Option<ExprId>,
    steps: Vec<RewriteStep>,
    /// Completed (original, replacement) child pairs in completion order. A completion's
    /// `results[base..]` slice holds exactly its direct children in source order; it is truncated
    /// only after provenance inheritance has read those pairs, and the completion then pushes
    /// exactly one pair, so every parent observes only its own children.
    results: Vec<(ExprId, ExprId)>,
    live_results: Vec<LiveValue>,
    scratch_bytes: usize,
    dec_depth: u32,
}

impl ConstInterpreter<'_> {
    /// Rewrite one expression tree, restoring every installed catch handler on any exit.
    pub(super) fn rewrite_with_bindings(
        &mut self,
        root: ExprId,
        mode: RewriteMode,
        bindings: &mut FrameBindings,
    ) -> Result<ExprId, ConstantFoldError> {
        let RewriteOutput::Expression(expression) = self.run_rewrite(root, mode, bindings, None)?
        else {
            unreachable!("an AST rewrite does not request direct call results");
        };
        Ok(expression)
    }

    pub(super) fn rewrite_for_value(
        &mut self,
        root: ExprId,
        bindings: &mut FrameBindings,
    ) -> Result<RewriteOutput, ConstantFoldError> {
        let producer = unwrap_fallible_expression(root, self.builder)?;
        let direct_producer =
            RewriteWalk::const_call_site(&self.builder.view(producer)?).map(|_| producer);
        self.run_rewrite(
            root,
            RewriteMode::InterpretedBody,
            bindings,
            direct_producer,
        )
    }

    fn run_rewrite(
        &mut self,
        root: ExprId,
        mode: RewriteMode,
        bindings: &mut FrameBindings,
        direct_producer: Option<ExprId>,
    ) -> Result<RewriteOutput, ConstantFoldError> {
        self.budget.span = self.builder.view(root)?.span();
        let mut walk = RewriteWalk {
            interpreter: self,
            bindings,
            mode,
            root,
            direct_producer,
            steps: Vec::new(),
            results: Vec::new(),
            live_results: Vec::new(),
            scratch_bytes: 0,
            dec_depth: 0,
        };
        let result = walk
            .push_step(RewriteStep::Enter { expression: root })
            .and_then(|()| walk.run());
        walk.interpreter
            .budget
            .release_memory_bytes(walk.scratch_bytes);
        result
    }
}

impl RewriteWalk<'_, '_> {
    fn push_step(&mut self, step: RewriteStep) -> Result<(), ConstantFoldError> {
        if self.mode.folds_auxiliary() {
            self.interpreter
                .budget
                .reserve_scratch(&mut self.steps, 1, &mut self.scratch_bytes)?;
        }
        self.steps.push(step);
        Ok(())
    }

    fn push_live_result(&mut self, value: LiveValue) -> Result<(), ConstantFoldError> {
        self.interpreter.budget.reserve_scratch(
            &mut self.live_results,
            1,
            &mut self.scratch_bytes,
        )?;
        self.live_results.push(value);
        Ok(())
    }

    fn hold_operand(
        &mut self,
        expression: ExprId,
        access: OperandAccess,
    ) -> Result<(), ConstantFoldError> {
        let span = self.interpreter.builder.view(expression)?.span();
        let value = self
            .live_results
            .last_mut()
            .ok_or_else(live_operand_mismatch)?;
        *value = match access {
            OperandAccess::SharedArgument => self.interpreter.acquire_shared_value(*value, span)?,
            OperandAccess::StoredValue => {
                self.interpreter.budget.span = span;
                let site = self.interpreter.access_site(span);
                self.interpreter.state.storage.hold_operand(
                    *value,
                    site,
                    self.interpreter.budget,
                )?
            }
        };
        Ok(())
    }

    fn annotate_live_value(
        &mut self,
        expression: ExprId,
        value: ConstValueId,
    ) -> Result<ConstValueId, ConstantFoldError> {
        let view = self.interpreter.builder.view(expression)?;
        let Some(facts) = view.occurrence_facts() else {
            return Ok(value);
        };
        if matches!(view.operation(), ExpressionOperationView::Cast { .. }) {
            // The shared cast fold carries only its evaluated source's dependencies.
            return Ok(value);
        }
        self.interpreter.budget.span = view.span();
        attach_live_provenance(
            self.interpreter.constants,
            self.interpreter.string_table,
            self.interpreter.budget,
            value,
            &facts.provenance,
        )
    }

    fn run(&mut self) -> Result<RewriteOutput, ConstantFoldError> {
        while let Some(step) = self.steps.pop() {
            if self.interpreter.state.outcome.is_some() {
                return Ok(RewriteOutput::Expression(self.root));
            }
            match step {
                RewriteStep::Enter { expression } => self.enter(expression)?,
                RewriteStep::Complete { expression, base } => {
                    if self.mode.folds_auxiliary() {
                        self.complete_live(expression, base)?;
                    } else {
                        self.complete(expression, base)?;
                    }
                }
                RewriteStep::CompleteCall {
                    expression,
                    base,
                    call,
                } => {
                    if let Some(slots) = self.complete_call(expression, base, &call)? {
                        return Ok(RewriteOutput::CallResults(slots));
                    }
                }
                RewriteStep::HoldOperand { expression, access } => {
                    self.hold_operand(expression, access)?
                }
                RewriteStep::CompleteLogical {
                    expression,
                    base,
                    right,
                    operation,
                } => self.complete_logical(expression, base, right, operation)?,
                RewriteStep::DecExponent(enter) => {
                    if enter {
                        self.dec_depth += 1;
                    } else {
                        self.dec_depth -= 1;
                    }
                }
                RewriteStep::RestoreSlots { base, mut slots } => {
                    for source in 0..slots.len() {
                        while slots[source] != source {
                            let destination = slots[source];
                            self.live_results.swap(base + source, base + destination);
                            slots.swap(source, destination);
                        }
                    }
                    let bytes = slots.len() * std::mem::size_of::<usize>();
                    drop(slots);
                    self.scratch_bytes -= bytes;
                    self.interpreter.budget.release_memory_bytes(bytes);
                }
            }
        }
        if let Some(value) = self.live_results.pop() {
            return Ok(RewriteOutput::Value(value));
        }
        Ok(RewriteOutput::Expression(
            self.results
                .pop()
                .map(|(_, result)| result)
                .unwrap_or(self.root),
        ))
    }

    /// Resolve a root that already names one constant value.
    ///
    /// WHAT: a bare reference to an explicit `#` constant, or a `copy` whose root is a frame
    ///       binding or an explicit constant, projected through constant field/index steps.
    /// WHY: expression construction rewrites explicit-constant operands, but a bare return value is
    ///      never an operand. One substitution here covers bare references, `copy` forms and
    ///      module-constant projections before the generic folder sees a non-constant reference.
    fn root_substitution(
        &self,
        view: &ExpressionView<'_>,
    ) -> Result<Option<ProjectedConstant>, ConstantFoldError> {
        let (root_value, projections) = match view.operation() {
            ExpressionOperationView::Reference(path) => {
                let Some(value) = self.interpreter.scope.explicit_constant_value(
                    &path,
                    self.interpreter.path_fork,
                    self.interpreter.constants,
                ) else {
                    return Ok(None);
                };
                (value, Vec::new())
            }
            ExpressionOperationView::Copy(place_view) => {
                let root_value = self.interpreter.scope.explicit_constant_value(
                    &place_view.metadata().root,
                    self.interpreter.path_fork,
                    self.interpreter.constants,
                );
                let Some(root_value) = root_value else {
                    return Ok(None);
                };
                let mut projections = Vec::new();
                for projection in place_view.projections() {
                    match projection {
                        PlaceProjectionView::Field { field, .. } => {
                            projections.push(ResolvedPlaceProjection::Field(field));
                        }
                        PlaceProjectionView::Index { index, .. } => {
                            if let ExpressionOperationView::Constant(value) = index.operation()
                                && let Some(index) =
                                    constant_index(self.interpreter.constants, value)
                            {
                                projections.push(ResolvedPlaceProjection::Index(index));
                            } else {
                                return Ok(None);
                            }
                        }
                    }
                }
                (root_value, projections)
            }
            _ => return Ok(None),
        };
        let value = if projections.is_empty() {
            Some(root_value)
        } else {
            project_constant_value(
                self.interpreter.constants,
                root_value,
                &projections,
                self.interpreter.path_fork,
            )
        };
        let Some(value) = value else {
            return Ok(None);
        };
        let type_id = view.success_shape().single().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "constant reference used before its single success slot was available",
            )
        })?;
        Ok(Some(ProjectedConstant {
            value,
            type_id,
            span: view.span(),
            value_mode: view.value_mode(),
        }))
    }

    /// Recognise a call site whose callee is a required source const function.
    fn const_call_site(view: &ExpressionView<'_>) -> Option<ConstCallSite> {
        let ExpressionOperationView::Call(call) = view.operation() else {
            return None;
        };
        if call.metadata().evaluation != CallableEvaluation::Const {
            return None;
        }
        let ResolvedCallableTarget::Function(path) = call.metadata().target else {
            return None;
        };
        Some(ConstCallSite {
            path,
            span: view.span(),
        })
    }

    fn enter_live(&mut self, expression: ExprId) -> Result<bool, ConstantFoldError> {
        let view = self.interpreter.builder.view(expression)?;
        let span = view.span();
        let value = match view.operation() {
            ExpressionOperationView::Reference(path) => {
                if let Some(binding) = self.bindings.get(&path).copied() {
                    Some(self.interpreter.read_place(binding.place(), span)?)
                } else if let Some(projected) = self.root_substitution(&view)? {
                    Some(
                        self.interpreter
                            .materialize_value(projected.value, Permission::ReadOnly)?,
                    )
                } else {
                    return Ok(false);
                }
            }
            ExpressionOperationView::Copy(place) => {
                let place_id = place.id();
                if let Some((root, projections)) = self
                    .interpreter
                    .evaluate_place_projections(place_id, self.bindings)?
                {
                    if let Some(binding) = self.bindings.get(&root).copied() {
                        let target = self
                            .interpreter
                            .project_live_place(binding.place(), &projections, span)?
                            .ok_or_else(|| {
                                unsupported_error(
                                    super::labels::PROJECTED_READ,
                                    span,
                                    self.interpreter.string_table,
                                )
                            })?;
                        let value = self.interpreter.read_place(target, span)?;
                        Some(self.interpreter.copy_live_value(value)?)
                    } else {
                        let view = self.interpreter.builder.view(expression)?;
                        let Some(projected) = self.root_substitution(&view)? else {
                            return Ok(false);
                        };
                        let value = self
                            .interpreter
                            .materialize_value(projected.value, Permission::ReadOnly)?;
                        Some(self.interpreter.copy_live_value(value)?)
                    }
                } else {
                    None
                }
            }
            ExpressionOperationView::CollectionBuiltin { .. } => self
                .interpreter
                .evaluate_collection_read(expression, self.bindings)?,
            ExpressionOperationView::Template(_) => self
                .interpreter
                .fold_template_with_bindings(expression, self.bindings)?,
            ExpressionOperationView::ValueBlock(ValueBlock::Catch(_))
                if expression != self.root
                    && self
                        .interpreter
                        .fallible_handler_instructions(expression)?
                        .is_some() =>
            {
                self.interpreter
                    .evaluate_expression(expression, self.bindings)?
            }
            _ => return Ok(false),
        };
        if let Some(mut value) = value {
            value.shape = self.annotate_live_value(expression, value.shape)?;
            self.push_live_result(value)?;
        } else if self.interpreter.state.outcome.is_none() {
            return Err(unsupported_error(
                super::labels::VALUE_EXPRESSION,
                span,
                self.interpreter.string_table,
            ));
        }
        Ok(true)
    }

    fn complete_live(&mut self, expression: ExprId, base: usize) -> Result<(), ConstantFoldError> {
        let view = self.interpreter.builder.view(expression)?;
        let span = view.span();
        self.interpreter.budget.span = span;
        let children = &self.live_results[base..];
        let value = match view.operation() {
            ExpressionOperationView::FieldAccess { field, .. } => {
                let [base] = children else {
                    return Err(live_operand_mismatch());
                };
                let place = self
                    .interpreter
                    .project_live_value(*base, ResolvedPlaceProjection::Field(field), span)?
                    .ok_or_else(|| {
                        unsupported_error(
                            super::labels::PROJECTED_READ,
                            span,
                            self.interpreter.string_table,
                        )
                    })?;
                let mut value = self.interpreter.read_place(place, span)?;
                if let Some(metadata) = self.interpreter.constants.metadata(base.shape)
                    && !metadata.synthetic_interface_provenance.is_empty()
                {
                    let provenance = metadata.synthetic_interface_provenance.clone();
                    value.shape = attach_live_provenance(
                        self.interpreter.constants,
                        self.interpreter.string_table,
                        self.interpreter.budget,
                        value.shape,
                        &provenance,
                    )?;
                }
                Some(value)
            }
            ExpressionOperationView::HandleFallible { .. }
            | ExpressionOperationView::ValueBlock(ValueBlock::Catch(_)) => {
                let [value] = children else {
                    return Err(live_operand_mismatch());
                };
                Some(*value)
            }
            _ => self.interpreter.fold_live_expression(
                expression,
                children,
                self.dec_depth,
                self.bindings,
            )?,
        };
        self.live_results.truncate(base);
        if let Some(mut value) = value {
            value.shape = self.annotate_live_value(expression, value.shape)?;
            self.push_live_result(value)?;
        } else if self.interpreter.state.outcome.is_none() {
            return Err(unsupported_error(
                super::labels::VALUE_EXPRESSION,
                span,
                self.interpreter.string_table,
            ));
        }
        Ok(())
    }

    fn enter(&mut self, expression: ExprId) -> Result<(), ConstantFoldError> {
        self.interpreter.budget.span = self.interpreter.builder.view(expression)?.span();
        if self.mode.folds_auxiliary() && self.enter_live(expression)? {
            return Ok(());
        }
        let view = self.interpreter.builder.view(expression)?;
        if !self.mode.folds_auxiliary()
            && let Some(projected) = self.root_substitution(&view)?
        {
            let replacement = expression_with_value(
                self.interpreter.builder,
                projected.value,
                projected.type_id,
                projected.span,
                projected.value_mode,
            )?;
            self.results.push((expression, replacement));
            return Ok(());
        }
        let const_call = Self::const_call_site(&view);
        let base = if self.mode.folds_auxiliary() {
            self.live_results.len()
        } else {
            self.results.len()
        };
        if self.mode.folds_auxiliary() {
            self.interpreter
                .budget
                .reserve_scratch(&mut self.steps, 2, &mut self.scratch_bytes)?;
        }
        // The right operand of a logical operator must not be rewritten until the left operand
        // has failed to short-circuit, so a required call on the inactive side never executes.
        if let ExpressionOperationView::LogicalBinary {
            left,
            right,
            operation,
        } = view.operation()
        {
            self.steps.push(RewriteStep::CompleteLogical {
                expression,
                base,
                right: right.id(),
                operation,
            });
            self.steps.push(RewriteStep::Enter {
                expression: left.id(),
            });
            return Ok(());
        }
        let argument_access = const_call
            .is_some()
            .then_some(OperandAccess::SharedArgument);
        match const_call {
            Some(call) => self.steps.push(RewriteStep::CompleteCall {
                expression,
                base,
                call,
            }),
            None => self.steps.push(RewriteStep::Complete { expression, base }),
        }
        if self.mode.folds_auxiliary() {
            let operands = match view.operation() {
                ExpressionOperationView::Call(call) => {
                    schedule_authored_operands(
                        &mut self.steps,
                        self.interpreter.budget,
                        &mut self.scratch_bytes,
                        base,
                        argument_access,
                        || {
                            call.arguments().iter().map(|argument| {
                                (argument.value().id(), argument.metadata().authored_ordinal)
                            })
                        },
                    )?;
                    true
                }
                ExpressionOperationView::Struct { fields }
                | ExpressionOperationView::Choice { fields, .. } => {
                    schedule_authored_operands(
                        &mut self.steps,
                        self.interpreter.budget,
                        &mut self.scratch_bytes,
                        base,
                        Some(OperandAccess::StoredValue),
                        || {
                            fields
                                .iter()
                                .map(|field| (field.value().id(), field.authored_ordinal()))
                        },
                    )?;
                    true
                }
                ExpressionOperationView::Collection(items) => {
                    schedule_authored_operands(
                        &mut self.steps,
                        self.interpreter.budget,
                        &mut self.scratch_bytes,
                        base,
                        Some(OperandAccess::StoredValue),
                        || items.iter().map(|item| (item.id(), None)),
                    )?;
                    true
                }
                _ => false,
            };
            if operands {
                return Ok(());
            }
        }
        // Children complete from the LIFO stack, so push them in reverse source order to
        // keep the collected replacement rows aligned with the parent's child order.
        if self.mode.folds_auxiliary()
            && let ExpressionOperationView::NumericBinary { left, right, facts } = view.operation()
            && facts.is_dec_power()
        {
            self.interpreter
                .budget
                .reserve_scratch(&mut self.steps, 4, &mut self.scratch_bytes)?;
            self.steps.push(RewriteStep::DecExponent(false));
            self.steps.push(RewriteStep::Enter {
                expression: right.id(),
            });
            self.steps.push(RewriteStep::DecExponent(true));
            self.steps.push(RewriteStep::Enter {
                expression: left.id(),
            });
            return Ok(());
        }
        let child_start = self.steps.len();
        if self.mode.folds_auxiliary()
            && let ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) = view.operation()
        {
            // The selected handler's prepared instructions own its body. Rewriting it on the
            // success path would execute required calls in an inactive handler.
            self.steps.push(RewriteStep::Enter {
                expression: block.handled_value,
            });
        } else {
            let mut reservation = Ok(());
            let budget = &mut *self.interpreter.budget;
            let bounded = self.mode.folds_auxiliary();
            view.visit_children(|child| {
                if bounded && reservation.is_ok() {
                    reservation =
                        budget.reserve_scratch(&mut self.steps, 1, &mut self.scratch_bytes);
                }
                if reservation.is_err() {
                    return;
                }
                self.steps.push(RewriteStep::Enter {
                    expression: child.id(),
                });
            });
            reservation?;
        }
        self.steps[child_start..].reverse();
        Ok(())
    }

    /// Fold every completed call argument.
    ///
    /// A required call with a non-constant argument is a source diagnostic; budget and
    /// infrastructure failures escape.
    fn fold_call_arguments(
        interpreter: &mut ConstInterpreter<'_>,
        bindings: &mut FrameBindings,
        call: &ConstCallSite,
        arguments: &[(ExprId, ExprId)],
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        let mut values = Vec::with_capacity(arguments.len());
        for (_, argument) in arguments {
            let Some(value) = interpreter.fold_to_constant(*argument, bindings)? else {
                if interpreter.state.outcome.is_some() {
                    return Ok(None);
                }
                return Err(ConstantFoldError::Diagnostic(
                    choices_control_flow::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::ConstCallRequiresConstantArguments,
                        None,
                        call.span,
                    ),
                ));
            };
            values.push(value);
        }
        Ok(Some(values))
    }

    fn complete(&mut self, expression: ExprId, base: usize) -> Result<(), ConstantFoldError> {
        // The stack does not mutate `results` below until it truncates to `base`, so the
        // parent completion can borrow its direct child pairs instead of copying them.
        let replacements = &self.results[base..];
        let rewritten = if replacements
            .iter()
            .all(|(original, result)| original == result)
        {
            expression
        } else {
            let replacement = self.interpreter.builder.own_slice(replacements);
            rewrite_expression_children(
                self.interpreter.builder,
                self.interpreter.builder.own(expression),
                replacement,
            )?
            .id()
        };
        let rewritten =
            if let Some(projected) = self.interpreter.constant_field_projection(rewritten)? {
                expression_with_value(
                    self.interpreter.builder,
                    projected.value,
                    projected.type_id,
                    projected.span,
                    projected.value_mode,
                )?
            } else {
                rewritten
            };
        let rewritten = inherit_replaced_provenance(
            self.interpreter.builder,
            expression,
            rewritten,
            replacements,
        )?;
        self.results.truncate(base);
        self.results.push((expression, rewritten));
        Ok(())
    }

    fn complete_call(
        &mut self,
        expression: ExprId,
        base: usize,
        call: &ConstCallSite,
    ) -> Result<Option<Vec<LiveValue>>, ConstantFoldError> {
        self.interpreter.budget.span = call.span;
        let Some(executable) = self.interpreter.executables.get(&call.path) else {
            return Err(required_call_error(call, self.interpreter.string_table));
        };
        let mut provenance = SyntheticInterfaceProvenance::default();
        let mut parameter_values = FrameBindings::default();
        if self.mode.folds_auxiliary() {
            let arguments = &self.live_results[base..];
            if arguments.len() != executable.parameters.len() {
                return Err(required_call_error(call, self.interpreter.string_table));
            }
            for (&value, &(path, _)) in arguments.iter().zip(&executable.parameters) {
                if let Some(metadata) = self.interpreter.constants.metadata(value.shape) {
                    provenance.merge(&metadata.synthetic_interface_provenance);
                }
                self.interpreter.bind_slot(
                    &mut parameter_values,
                    path,
                    value,
                    Permission::ReadOnly,
                )?;
            }
        } else if !self.bind_folded_call_arguments(
            call,
            base,
            executable,
            &mut parameter_values,
            &mut provenance,
        )? {
            return Ok(None);
        }
        self.interpreter.enter_call(call.span)?;
        let returns = self
            .interpreter
            .execute_body(Some(call.path), executable, parameter_values)
            .map_err(|error| self.interpreter.attach_call_chain(error));
        self.interpreter.budget.leave_call();
        self.finish_call(expression, base, executable, returns?, provenance)
    }

    fn bind_folded_call_arguments(
        &mut self,
        call: &ConstCallSite,
        base: usize,
        executable: &ConstFunctionExecutable,
        parameter_values: &mut FrameBindings,
        provenance: &mut SyntheticInterfaceProvenance,
    ) -> Result<bool, ConstantFoldError> {
        let arguments = &self.results[base..];
        let Some(values) =
            Self::fold_call_arguments(self.interpreter, self.bindings, call, arguments)?
        else {
            return Ok(false);
        };
        if values.len() != executable.parameters.len() {
            return Err(required_call_error(call, self.interpreter.string_table));
        }
        for (((original, replacement), value), &(path, _)) in
            arguments.iter().zip(values).zip(&executable.parameters)
        {
            for argument in [original, replacement] {
                if let Some(facts) = self.interpreter.builder.view(*argument)?.occurrence_facts() {
                    provenance.merge(&facts.provenance);
                }
            }
            if let Some(metadata) = self.interpreter.constants.metadata(value) {
                provenance.merge(&metadata.synthetic_interface_provenance);
            }
            let value = self
                .interpreter
                .materialize_value(value, Permission::ReadOnly)?;
            self.interpreter
                .bind_slot(parameter_values, path, value, Permission::ReadOnly)?;
        }
        Ok(true)
    }

    /// Do not keep publication scratch on the native stack across a nested call.
    fn finish_call(
        &mut self,
        expression: ExprId,
        base: usize,
        executable: &ConstFunctionExecutable,
        mut returns: Vec<LiveValue>,
        mut provenance: SyntheticInterfaceProvenance,
    ) -> Result<Option<Vec<LiveValue>>, ConstantFoldError> {
        let view = self.interpreter.builder.view(expression)?;
        self.interpreter.budget.span = view.span();
        validate_call_results(
            self.interpreter.constants,
            view.success_types(),
            &executable.return_slots,
            &returns,
        )?;
        if let Some(facts) = view.occurrence_facts() {
            provenance.merge(&facts.provenance);
        }
        for &slot in &returns {
            if let Some(metadata) = self.interpreter.constants.metadata(slot.shape) {
                provenance.merge(&metadata.synthetic_interface_provenance);
            }
        }
        if self.direct_producer == Some(expression) {
            // The only parents bypassed here are fallible wrappers around this successful call.
            // Their handlers are inactive, but their authored occurrence facts still survive.
            let mut wrapper = self.root;
            while wrapper != expression {
                let view = self.interpreter.builder.view(wrapper)?;
                if let Some(facts) = view.occurrence_facts() {
                    provenance.merge(&facts.provenance);
                }
                wrapper = match view.operation() {
                    ExpressionOperationView::HandleFallible { value, .. } => value.id(),
                    ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) => {
                        block.handled_value
                    }
                    _ => unreachable!("direct producer must be the root or its wrapped call"),
                };
            }
        }
        if self.mode.folds_auxiliary() {
            for slot in &mut returns {
                slot.shape = attach_live_provenance(
                    self.interpreter.constants,
                    self.interpreter.string_table,
                    self.interpreter.budget,
                    slot.shape,
                    &provenance,
                )?;
            }
            if self.direct_producer == Some(expression) {
                return Ok(Some(returns));
            }
            let [value] = returns[..] else {
                return Err(live_operand_mismatch());
            };
            self.live_results.truncate(base);
            self.push_live_result(value)?;
            return Ok(None);
        }
        let frozen = self.interpreter.freeze_live_values(&returns)?;
        let replacement = commit_call_result(
            self.interpreter.builder,
            expression,
            &frozen,
            self.interpreter.scope.scope,
            provenance,
        )?;
        self.results.truncate(base);
        self.results.push((expression, replacement));
        Ok(None)
    }

    fn complete_logical(
        &mut self,
        expression: ExprId,
        base: usize,
        right: ExprId,
        operation: LogicalOperator,
    ) -> Result<(), ConstantFoldError> {
        let left_constant = if self.mode.folds_auxiliary() {
            Some(self.live_results[base].shape)
        } else {
            let left = self.results[base].1;
            let left_view = self.interpreter.builder.view(left)?;
            match left_view.operation() {
                ExpressionOperationView::Constant(value) => Some(value),
                ExpressionOperationView::Call(call)
                    if call.metadata().evaluation == CallableEvaluation::Const =>
                {
                    self.interpreter.fold_to_constant(left, self.bindings)?
                }
                _ => None,
            }
        };
        let left_bool =
            left_constant.and_then(|value| match self.interpreter.constants.payload(value) {
                Some(ConstValuePayload::Bool(value)) => Some(*value),
                _ => None,
            });
        let short_circuits = matches!(
            (operation, left_bool),
            (LogicalOperator::And, Some(false)) | (LogicalOperator::Or, Some(true))
        );
        if short_circuits {
            if self.mode.folds_auxiliary() {
                let value = self.annotate_live_value(expression, self.live_results[base].shape)?;
                self.live_results[base] = LiveValue::leaf(value, Permission::Writable);
                return Ok(());
            }
            let left = self.results[base].1;
            let view = self.interpreter.builder.view(expression)?;
            let result_type = view.success().single().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "logical expression folded before its single success slot was available",
                )
            })?;
            let value = left_constant.ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "logical short-circuit lost its folded boolean",
                )
            })?;
            let replacement = expression_with_value(
                self.interpreter.builder,
                value,
                result_type,
                view.span(),
                view.value_mode(),
            )?;
            let replacement = inherit_replaced_provenance(
                self.interpreter.builder,
                expression,
                replacement,
                &[(left, left)],
            )?;
            self.results.truncate(base);
            self.results.push((expression, replacement));
            return Ok(());
        }
        self.push_step(RewriteStep::Complete { expression, base })?;
        self.push_step(RewriteStep::Enter { expression: right })
    }
}

/// Evaluate named operands in authored order, then deliver them in resolved
/// slot order. The common in-order path needs no additional scratch.
fn schedule_authored_operands<I: ExactSizeIterator<Item = (ExprId, Option<NonZeroU32>)>>(
    steps: &mut Vec<RewriteStep>,
    budget: &mut ExecutionBudget,
    scratch_bytes: &mut usize,
    base: usize,
    access: Option<OperandAccess>,
    operands: impl Fn() -> I,
) -> Result<(), ConstantFoldError> {
    let ordinal = |value: Option<NonZeroU32>| value.map_or(u32::MAX, NonZeroU32::get);
    let count = operands().len();
    let push_operand = |steps: &mut Vec<RewriteStep>, expression, position| {
        steps.push(RewriteStep::Enter { expression });
        if let Some(access) = access
            && (matches!(access, OperandAccess::SharedArgument) || position + 1 < count)
        {
            steps.push(RewriteStep::HoldOperand { expression, access });
        }
    };
    let child_steps = count
        + match access {
            Some(OperandAccess::SharedArgument) => count,
            Some(OperandAccess::StoredValue) => count.saturating_sub(1),
            None => 0,
        };
    if operands().map(|(_, order)| ordinal(order)).is_sorted() {
        budget.reserve_scratch(steps, child_steps, scratch_bytes)?;
        let start = steps.len();
        for (position, (expression, _)) in operands().enumerate() {
            push_operand(steps, expression, position);
        }
        steps[start..].reverse();
        return Ok(());
    }
    budget.reserve_scratch(steps, child_steps + 1, scratch_bytes)?;
    let sort_bytes = count * std::mem::size_of::<(u32, usize, ExprId)>();
    let permutation_bytes = count * std::mem::size_of::<usize>();
    budget.charge_scratch(scratch_bytes, sort_bytes + permutation_bytes)?;
    let mut scheduled = operands()
        .enumerate()
        .map(|(slot, (expression, order))| (ordinal(order), slot, expression))
        .collect::<Vec<_>>();
    scheduled.sort_unstable_by_key(|&(order, slot, _)| (order, slot));
    let slots = scheduled.iter().map(|&(_, slot, _)| slot).collect();
    steps.push(RewriteStep::RestoreSlots { base, slots });
    let start = steps.len();
    for (position, (_, _, expression)) in scheduled.into_iter().enumerate() {
        push_operand(steps, expression, position);
    }
    steps[start..].reverse();
    *scratch_bytes -= sort_bytes;
    budget.release_memory_bytes(sort_bytes);
    Ok(())
}

fn live_operand_mismatch() -> ConstantFoldError {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "prepared const expression lost its live operand slots",
    )
    .into()
}

/// Keep synthetic-interface provenance when an expression replaces its children.
///
/// WHAT: unions the original expression, its completed replacement and every rewritten child onto
///       the surviving expression.
/// WHY: folding a child into a constant removes the call or projection that originally carried the
///      dependency. Later link facts inspect the surviving expression tree, so an intermediate
///      binary, field access or fallback call must carry that provenance onward.
pub(super) fn inherit_replaced_provenance(
    builder: &mut AstExpressionBuilder,
    original: ExprId,
    rewritten: ExprId,
    replacements: &[(ExprId, ExprId)],
) -> Result<ExprId, ConstantFoldError> {
    if rewritten == original
        && replacements
            .iter()
            .all(|(previous, replacement)| previous == replacement)
    {
        return Ok(rewritten);
    }
    let mut provenance = builder
        .view(original)?
        .occurrence_facts()
        .map(|facts| facts.provenance.clone())
        .unwrap_or_default();
    if rewritten != original
        && let Some(facts) = builder.view(rewritten)?.occurrence_facts()
    {
        provenance.merge(&facts.provenance);
    }
    for (_, replacement) in replacements {
        if let Some(facts) = builder.view(*replacement)?.occurrence_facts() {
            provenance.merge(&facts.provenance);
        }
    }
    let rewritten =
        builder.with_synthetic_interface_provenance(builder.own(rewritten), provenance)?;
    Ok(rewritten.id())
}

/// A required call whose declaration body the current executor cannot represent.
fn required_call_error(call: &ConstCallSite, string_table: &mut StringTable) -> ConstantFoldError {
    unsupported_error(labels::REQUIRED_CALL, call.span, string_table)
}
