//! Required const-call rewriting over typed expressions.
//!
//! WHAT: replaces required source const calls with folded constants, both in ordinary program
//!       expressions and while a prepared const body executes.
//! WHY: folding runs before expression freezing and HIR lowering, so the final AST must already
//!      contain the replacement.
//! MUST NOT: own executable preparation, the executed-frame machine or eligibility policy. It
//!           invokes those owners and keeps only the AST rewrite transaction.

use super::budget::ExecutionBudget;
use super::instructions::ConstInstruction;
use super::labels;
use super::places::{fallible_handler_error_binding, unwrap_fallible_expression};
use super::state::{
    ActiveHandler, ConstExecutionServices, ConstInterpreter, ExecutionState, FrameBindings,
};
use super::values::{
    constant_index, expression_with_value, expression_with_value_inheriting_provenance,
    project_constant_value, unsupported_error,
};
use super::{ProjectedConstant, ResolvedPlaceProjection};
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression_rewrite::rewrite_expression_children;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, LogicalOperator,
    PlaceProjectionView, ResolvedCallableTarget, SuccessShape,
};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;

struct ConstCallSite {
    path: PathId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    result_type: TypeId,
}

/// A prepared catch body selected for one catch expression.
struct CatchHandler {
    body: Rc<[ConstInstruction]>,
    error_binding: Option<PathId>,
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

/// One pending rewriter action.
enum RewriteStep {
    Enter {
        expression: ExprId,
    },
    Complete {
        expression: ExprId,
        base: usize,
        /// The entry step installed a catch handler; this completion restores it.
        restores_handler: bool,
    },
    CompleteCall {
        expression: ExprId,
        base: usize,
        call: ConstCallSite,
    },
    CompleteLogical {
        expression: ExprId,
        base: usize,
        right: ExprId,
        operation: LogicalOperator,
    },
}

/// One iterative typed-expression rewrite.
///
/// WHY a named walk rather than a closure: the walk owns a step stack, a completed-child stack and
/// installed catch handlers. A method keeps the early-exit unwind explicit and readable.
struct RewriteWalk<'walk, 'services> {
    interpreter: &'walk mut ConstInterpreter<'services>,
    bindings: &'walk mut FrameBindings,
    mode: RewriteMode,
    root: ExprId,
    steps: Vec<RewriteStep>,
    /// Completed (original, replacement) child pairs in completion order. A completion's
    /// `results[base..]` slice holds exactly its direct children in source order; it is truncated
    /// only after provenance inheritance has read those pairs, and the completion then pushes
    /// exactly one pair, so every parent observes only its own children.
    results: Vec<(ExprId, ExprId)>,
    /// Saved handler states for catch handlers this walk currently has installed. Entries are
    /// pushed when a catch is entered and popped by its completion; anything left when the walk
    /// exits early or fails is restored so the caller never keeps a leaked handler.
    installed_handlers: Vec<ActiveHandler>,
}

impl ConstInterpreter<'_> {
    /// Rewrite one expression tree, restoring every installed catch handler on any exit.
    pub(super) fn rewrite_with_bindings(
        &mut self,
        root: ExprId,
        mode: RewriteMode,
        bindings: &mut FrameBindings,
    ) -> Result<ExprId, ConstantFoldError> {
        let mut walk = RewriteWalk {
            interpreter: self,
            bindings,
            mode,
            root,
            steps: vec![RewriteStep::Enter { expression: root }],
            results: Vec::new(),
            installed_handlers: Vec::new(),
        };
        let outcome = walk.run();
        walk.unwind_handlers();
        outcome
    }
}

impl RewriteWalk<'_, '_> {
    fn run(&mut self) -> Result<ExprId, ConstantFoldError> {
        while let Some(step) = self.steps.pop() {
            if let RewriteStep::Enter { expression } = &step {
                self.interpreter.budget.span = self.interpreter.builder.view(*expression)?.span();
            }
            // Discovery over an ordinary expression is not const-call work. Only execution and the
            // shared folders below consume the aggregate const-execution allowance.
            if self.interpreter.state.inherited_handler_value.is_some()
                || self.interpreter.state.pending_handler_return.is_some()
            {
                return Ok(self.root);
            }
            match step {
                RewriteStep::Enter { expression } => self.enter(expression)?,
                RewriteStep::Complete {
                    expression,
                    base,
                    restores_handler,
                } => self.complete(expression, base, restores_handler)?,
                RewriteStep::CompleteCall {
                    expression,
                    base,
                    call,
                } => self.complete_call(expression, base, &call)?,
                RewriteStep::CompleteLogical {
                    expression,
                    base,
                    right,
                    operation,
                } => self.complete_logical(expression, base, right, operation)?,
            }
        }
        Ok(self
            .results
            .pop()
            .map(|(_, result)| result)
            .unwrap_or(self.root))
    }

    fn unwind_handlers(&mut self) {
        while let Some(saved_handler) = self.installed_handlers.pop() {
            self.interpreter.state.active_handler = saved_handler;
        }
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
                let root_value = self
                    .bindings
                    .get(&place_view.metadata().root)
                    .copied()
                    .or_else(|| {
                        self.interpreter.scope.explicit_constant_value(
                            &place_view.metadata().root,
                            self.interpreter.path_fork,
                            self.interpreter.constants,
                        )
                    });
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
    fn const_call_site(
        &self,
        view: &ExpressionView<'_>,
    ) -> Result<Option<ConstCallSite>, ConstantFoldError> {
        let ExpressionOperationView::Call(call) = view.operation() else {
            return Ok(None);
        };
        if call.metadata().evaluation != CallableEvaluation::Const {
            return Ok(None);
        }
        let ResolvedCallableTarget::Function(path) = call.metadata().target else {
            return Ok(None);
        };
        let result_type = view.success_shape().single().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function call used before its single success slot was available",
            )
        })?;
        Ok(Some(ConstCallSite {
            path,
            span: view.span(),
            value_mode: view.value_mode(),
            result_type,
        }))
    }

    /// Resolve the prepared catch body for a catch expression.
    fn catch_handler_for(
        &self,
        expression: ExprId,
    ) -> Result<Option<CatchHandler>, ConstantFoldError> {
        let Some(body) = self.interpreter.fallible_handler_instructions(expression)? else {
            return Ok(None);
        };
        let error_binding = fallible_handler_error_binding(self.interpreter.builder, expression)?;
        Ok(Some(CatchHandler {
            body,
            error_binding,
        }))
    }

    fn enter(&mut self, expression: ExprId) -> Result<(), ConstantFoldError> {
        let view = self.interpreter.builder.view(expression)?;
        self.interpreter.budget.span = view.span();
        if let Some(projected) = self.root_substitution(&view)? {
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
        let const_call = self.const_call_site(&view)?;
        let restores_handler = if expression != self.root
            && matches!(
                view.operation(),
                ExpressionOperationView::ValueBlock(ValueBlock::Catch(_))
            ) {
            match self.catch_handler_for(expression)? {
                Some(handler) => {
                    let saved_handler = std::mem::replace(
                        &mut self.interpreter.state.active_handler,
                        ActiveHandler {
                            body: Some(handler.body),
                            error_binding: handler.error_binding,
                            boundary: true,
                        },
                    );
                    self.installed_handlers.push(saved_handler);
                    true
                }
                None => false,
            }
        } else {
            false
        };
        let base = self.results.len();
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
        match const_call {
            Some(call) => self.steps.push(RewriteStep::CompleteCall {
                expression,
                base,
                call,
            }),
            None => self.steps.push(RewriteStep::Complete {
                expression,
                base,
                restores_handler,
            }),
        }
        // Children complete from the LIFO stack, so push them in reverse source order to
        // keep the collected replacement rows aligned with the parent's child order.
        let mut children = Vec::new();
        view.visit_children(|child| children.push(child.id()));
        for child in children.into_iter().rev() {
            self.steps.push(RewriteStep::Enter { expression: child });
        }
        Ok(())
    }

    /// Fold every completed call argument.
    ///
    /// A required call with a non-constant argument is a source diagnostic; budget and
    /// infrastructure failures escape.
    fn fold_call_arguments(
        &mut self,
        call: &ConstCallSite,
        arguments: &[(ExprId, ExprId)],
    ) -> Result<Vec<ConstValueId>, ConstantFoldError> {
        let mut values = Vec::with_capacity(arguments.len());
        for (_, argument) in arguments {
            let Some(value) = self
                .interpreter
                .fold_to_constant(*argument, self.bindings)?
            else {
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
        Ok(values)
    }

    fn complete(
        &mut self,
        expression: ExprId,
        base: usize,
        restores_handler: bool,
    ) -> Result<(), ConstantFoldError> {
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
        // A rebuilt catch keeps the handler compiled for its authored expression. Completion can
        // evaluate the rebuilt catch immediately (a successful collection read under an arithmetic
        // catch is enough), so the prepared body must follow the fresh id.
        if rewritten != expression
            && matches!(
                self.interpreter.builder.view(expression)?.operation(),
                ExpressionOperationView::ValueBlock(ValueBlock::Catch(_))
            )
            && let Some(body) = self.interpreter.fallible_handler_instructions(expression)?
        {
            self.interpreter
                .state
                .rewritten_handlers
                .insert(rewritten, body);
        }
        let rewritten = self.interpreter.fold_completed_value(
            rewritten,
            &mut *self.bindings,
            self.mode.folds_auxiliary(),
        )?;
        if restores_handler {
            let saved_handler = self
                .installed_handlers
                .pop()
                .expect("a catch completion owns the handler its entry installed");
            self.interpreter.state.active_handler = saved_handler;
        }
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
    ) -> Result<(), ConstantFoldError> {
        // Move the completed argument pairs off the scratch stack: the completed call pushes one
        // replacement pair in their place.
        let arguments = self.results.split_off(base);
        let Some(executable) = self.interpreter.executables.get(&call.path) else {
            return Err(required_call_error(call, self.interpreter.string_table));
        };
        let argument_values = self.fold_call_arguments(call, &arguments)?;
        if argument_values.len() != executable.parameters.len() {
            return Err(required_call_error(call, self.interpreter.string_table));
        }
        let parameter_values = executable
            .parameters
            .iter()
            .map(|(path, _)| *path)
            .zip(argument_values)
            .collect::<FrameBindings>();
        self.interpreter.enter_call(call.span)?;
        let returns = self
            .interpreter
            .execute_body(Some(call.path), executable, parameter_values)
            .map_err(|error| self.interpreter.attach_call_chain(error));
        self.interpreter.budget.leave_call();
        let returns = returns?;
        if returns.len() != 1 || executable.return_slots.len() != 1 {
            return Err(required_call_error(call, self.interpreter.string_table));
        }
        if executable.return_slots[0] != call.result_type {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function result type disagreed with its call result slot",
            )
            .into());
        }
        let mut inherited_provenance = self
            .interpreter
            .builder
            .view(expression)?
            .occurrence_facts()
            .map(|facts| facts.provenance.clone())
            .unwrap_or_default();
        for (_, argument) in &arguments {
            if let Some(facts) = self.interpreter.builder.view(*argument)?.occurrence_facts() {
                inherited_provenance.merge(&facts.provenance);
            }
        }
        let replacement = expression_with_value_inheriting_provenance(
            self.interpreter.builder,
            self.interpreter.constants,
            returns[0],
            call.result_type,
            call.span,
            call.value_mode,
            inherited_provenance,
        )?;
        self.results.push((expression, replacement));
        Ok(())
    }

    fn complete_logical(
        &mut self,
        expression: ExprId,
        base: usize,
        right: ExprId,
        operation: LogicalOperator,
    ) -> Result<(), ConstantFoldError> {
        let left = self.results[base].1;
        let left_view = self.interpreter.builder.view(left)?;
        let left_constant = match left_view.operation() {
            ExpressionOperationView::Constant(value) => Some(value),
            ExpressionOperationView::Reference(path) => self.bindings.get(&path).copied(),
            ExpressionOperationView::Call(call)
                if call.metadata().evaluation == CallableEvaluation::Const =>
            {
                self.interpreter.fold_to_constant(left, self.bindings)?
            }
            _ if !self.interpreter.budget.call_stack.is_empty() => {
                self.interpreter.fold_to_constant(left, self.bindings)?
            }
            _ => None,
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
        self.steps.push(RewriteStep::Complete {
            expression,
            base,
            restores_handler: false,
        });
        self.steps.push(RewriteStep::Enter { expression: right });
        Ok(())
    }
}

impl ConstInterpreter<'_> {
    /// Apply the post-completion folders to one completed expression.
    ///
    /// Field projection after a folded const call applies in every mode. Collection reads,
    /// template completion and fallible unwrapping are the auxiliary folds an executing const body
    /// needs; a required program expression keeps those operations at runtime.
    fn fold_completed_value(
        &mut self,
        mut rewritten: ExprId,
        bindings: &mut FrameBindings,
        auxiliary_folding: bool,
    ) -> Result<ExprId, ConstantFoldError> {
        if let Some(projected) = self.constant_field_projection(rewritten, bindings)? {
            rewritten = expression_with_value(
                self.builder,
                projected.value,
                projected.type_id,
                projected.span,
                projected.value_mode,
            )?;
        } else if auxiliary_folding
            && let Some(value) = self.evaluate_collection_read(rewritten, bindings)?
        {
            let view = self.builder.view(rewritten)?;
            let result_type = view.success().single().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "collection read folded before its single success slot was available",
                )
            })?;
            let span = view.span();
            let value_mode = view.value_mode();
            rewritten = expression_with_value(self.builder, value, result_type, span, value_mode)?;
        } else if auxiliary_folding
            && let Some(projected) = self.fold_template_with_bindings(rewritten, bindings)?
        {
            rewritten = expression_with_value(
                self.builder,
                projected.value,
                projected.type_id,
                projected.span,
                projected.value_mode,
            )?;
        }
        if auxiliary_folding {
            let inner = unwrap_fallible_expression(rewritten, self.builder)?;
            if inner != rewritten {
                match self.fold_to_constant(inner, bindings) {
                    Ok(Some(value)) => {
                        let view = self.builder.view(inner)?;
                        if let SuccessShape::One(result_type) = view.success() {
                            let span = view.span();
                            let value_mode = view.value_mode();
                            rewritten = expression_with_value(
                                self.builder,
                                value,
                                result_type,
                                span,
                                value_mode,
                            )?;
                        }
                    }
                    Ok(None) => {}
                    Err(ConstantFoldError::Budget(exhaustion)) => {
                        return Err(ConstantFoldError::Budget(exhaustion));
                    }
                    Err(
                        ConstantFoldError::Diagnostic(_) | ConstantFoldError::NumericFailure(_),
                    ) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(rewritten)
    }
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
