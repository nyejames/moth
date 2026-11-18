//! Required and release call rewriting over typed expressions.
//!
//! WHAT: replaces required source const calls with folded constants, discovers ordinary call
//!       targets for release preparation, and rewrites release-eligible calls in executable
//!       bodies while preserving synthetic-interface provenance.
//! WHY: folding runs before expression freezing and HIR lowering, so the final AST must already
//!      contain the replacement. A release attempt that cannot prove a fold leaves the exact
//!      runtime call and its folded children in place.
//! MUST NOT: own executable preparation, the executed-frame machine or eligibility policy. It
//!           invokes those owners and keeps only the AST rewrite transaction.

use super::budget::ExecutionBudget;
use super::eligibility::{
    ReleaseCandidate, RetainedReleaseCandidates, prepare_release_executable,
    retain_acyclic_release_candidates,
};
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
use super::{ConstBodyReferenceScope, ProjectedConstant, ResolvedPlaceProjection};
use crate::compiler_frontend::ast::AstReleaseErasedCallSite;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_rewrite::rewrite_expression_children;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, LogicalOperator,
    PlaceProjectionView, ResolvedCallableTarget, SuccessShape,
};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::control_flow_exits::collect_authored_dead_expression_roots;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

struct ConstCallSite {
    path: PathId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    result_type: TypeId,
    /// Whether an enclosing catch owns this call's recoverable failure.
    failure_caught: bool,
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
    let release_provenances = FxHashMap::default();
    interpreter.rewrite_with_bindings(
        root,
        RewriteMode::RequiredExpression,
        &release_provenances,
        &mut FrameBindings::default(),
    )
}

/// Rewrite eligible ordinary calls in the final AST for a release build.
///
/// WHAT: prepares each reachable ordinary body once, then replaces only calls whose arguments
///       fold to constants and whose execution succeeds. Any refusal, impurity, nonconstant
///       argument, recursion/depth stop or folder diagnostic leaves the exact call in place.
/// WHY: release optimisation must remain an invisible rewrite of the final AST. It runs before
///      expression freezing and HIR lowering, so no later analysis can observe a removed call.
///
/// Returns deduplicated budget Notes and the selected call sites the folding erased. Call counts
/// are not part of the release contract; the finalizer stores the erased rows on
/// [`Ast`](crate::compiler_frontend::ast::Ast) for HIR constraint projection.
pub(crate) fn rewrite_release_calls(
    ast: &mut [AstNode],
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    scope: &ScopeContext,
    string_table: &mut StringTable,
    types: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<(Vec<DiagnosticDraft>, Vec<AstReleaseErasedCallSite>), Box<InfrastructureErrors>> {
    const MAX_RELEASE_BUDGET_NOTES: usize = 3;
    // Prepare only function bodies reachable from an actual ordinary call site. Uncalled bodies
    // cost nothing in a release build, while the worklist still closes over callees named by a
    // prepared body.
    let mut bodies = FxHashMap::default();
    for node in ast.iter() {
        if let NodeKind::Function(path, signature, body) = &node.kind {
            bodies.insert(*path, (signature, body.as_slice()));
        }
    }
    let mut pending = release_call_targets(ast, builder)?;
    let mut considered = FxHashSet::default();
    let mut candidates: FxHashMap<PathId, ReleaseCandidate> = FxHashMap::default();
    while let Some(path) = pending.pop() {
        if !considered.insert(path) {
            continue;
        }
        let Some((signature, body)) = bodies.get(&path) else {
            continue;
        };
        match prepare_release_executable(
            signature,
            body,
            builder,
            types,
            scope.external_package_registry.as_ref(),
            ConstBodyReferenceScope {
                scope,
                path_fork,
                constants: &*constants,
            },
        ) {
            Ok(Some((executable, proof))) => {
                pending.extend(proof.ordinary_calls.iter().copied());
                candidates.insert(path, ReleaseCandidate { executable, proof });
            }
            Ok(None) => {}
            Err(super::ConstBodyPreparationError::Unsupported(_)) => {}
            Err(super::ConstBodyPreparationError::Infrastructure(error)) => return Err(error),
        }
    }
    let retained = retain_acyclic_release_candidates(candidates);
    let RetainedReleaseCandidates {
        executables,
        provenances: release_provenances,
    } = retained;
    if executables.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Authored-dead statements keep their runtime expressions. HIR lowering omits exactly those
    // statements and records their selected evidence, so a debug build never executes them either.
    // A per-statement decision also keeps a const template's shared definition spans from hiding
    // a live call: the same authored call can be dead in one use and live in another.
    let mut authored_dead_expression_roots = FxHashSet::default();
    collect_authored_dead_expression_roots(
        ast,
        builder.store(),
        constants,
        &mut authored_dead_expression_roots,
    );

    let mut services = ConstExecutionServices {
        builder,
        constants,
        executables: &executables,
        scope,
        string_table,
        types,
        path_fork,
    };
    let meter = services.constants.execution_meter();
    let mut notes = Vec::new();
    let mut note_spans = FxHashSet::default();
    let mut erased_call_sites = Vec::new();
    let mut infrastructure_error = None;
    for node in ast.iter_mut() {
        // Only a callable body can own executable selected code. A fold inside a non-callable
        // top-level declaration has no HIR function to evidence, just as in a debug build.
        let owner = node.function_identity();
        node.visit_expression_ids_mut(&mut |expression| {
            if infrastructure_error.is_some() {
                return;
            }
            // Discovery over an ordinary expression is not const-call work. Each expression gets
            // its own attempt allowance, while the store keeps charging every attempt against the
            // one aggregate module allowance.
            let mut budget = ExecutionBudget::new(Rc::clone(&meter));
            let mut state = ExecutionState::default();
            let mut value_bindings = FrameBindings::default();
            let outcome = {
                let mut interpreter = services.interpreter(&mut budget, &mut state);
                interpreter.rewrite_release_with_bindings(
                    *expression,
                    &release_provenances,
                    &mut value_bindings,
                    &authored_dead_expression_roots,
                )
            };
            let committed = match outcome {
                Ok(rewritten) => {
                    let changed = rewritten != *expression;
                    if changed {
                        *expression = rewritten;
                    }
                    changed
                }
                Err(ConstantFoldError::Infrastructure(error)) => {
                    infrastructure_error = Some(error);
                    false
                }
                Err(ConstantFoldError::Budget(exhaustion)) => {
                    let span = exhaustion.call_span.or(exhaustion.span);
                    if notes.len() < MAX_RELEASE_BUDGET_NOTES && note_spans.insert(span) {
                        let scope = if exhaustion.aggregate {
                            choices_control_flow::ReleaseBudgetScope::ModuleAggregate
                        } else {
                            choices_control_flow::ReleaseBudgetScope::Call
                        };
                        let setting = exhaustion
                            .setting
                            .map(|setting| services.string_table.intern(setting));
                        // Every producer charges its allowance as a `usize`, so the recorded
                        // `u64` limit always fits back into one.
                        notes.push(choices_control_flow::release_folding_budget_stopped(
                            exhaustion.unit,
                            usize::try_from(exhaustion.limit)
                                .expect("const execution limits are charged as `usize`"),
                            setting,
                            scope,
                            span,
                        ));
                    }
                    false
                }
                Err(ConstantFoldError::Diagnostic(_) | ConstantFoldError::NumericFailure(_)) => {
                    false
                }
            };
            if committed && let Some(owner) = owner {
                // A fresh state owns exactly this root rewrite, so every recorded row belongs to
                // the node's callable. Nested candidate execution uses `InterpretedBody` walks,
                // which record nothing.
                erased_call_sites.extend(state.release_erased_calls.drain(..).map(
                    |(target, span, failure_caught)| AstReleaseErasedCallSite {
                        function: owner,
                        target,
                        span,
                        failure_caught,
                    },
                ));
            }
        });
    }
    match infrastructure_error {
        Some(error) => Err(error),
        None => Ok((notes, erased_call_sites)),
    }
}

/// Collect ordinary function paths named by any call expression in the final AST.
///
/// WHAT: one immutable typed-expression walk before candidate preparation.
/// WHY:  release preparation compiles a body into an executable and validates every expression.
///      A body that no source call can reach must not pay that cost. Ordinary callees discovered
///      inside a prepared body extend the worklist until the callable closure is closed.
fn release_call_targets(
    ast: &[AstNode],
    builder: &AstExpressionBuilder,
) -> Result<Vec<PathId>, Box<InfrastructureErrors>> {
    let mut roots = Vec::new();
    for node in ast {
        node.visit_expression_ids(&mut |expression| roots.push(expression));
    }
    let mut targets = Vec::new();
    let mut visited = FxHashSet::default();
    while let Some(expression) = roots.pop() {
        if !visited.insert(expression) {
            continue;
        }
        let view = builder.view(expression)?;
        if let ExpressionOperationView::Call(call) = view.operation()
            && call.metadata().evaluation != CallableEvaluation::Const
            && let ResolvedCallableTarget::Function(path) = call.metadata().target
        {
            targets.push(path);
        }
        view.visit_children(|child| roots.push(child.id()));
    }
    Ok(targets)
}

/// Add the candidate's proven transitive provenance to a release replacement.
///
/// WHAT: merges the retained candidate set for one ordinary call path into the inherited
///       provenance used by the replacement expression.
/// WHY: execution may return a value selected from a branch or projection whose own metadata does
///      not carry a dependency present in the authored body. The candidate set is authoritative
///      for the dependencies erased with the call.
pub(super) fn merge_release_candidate_provenance(
    inherited: &mut SyntheticInterfaceProvenance,
    path: PathId,
    release_provenances: &FxHashMap<PathId, SyntheticInterfaceProvenance>,
) {
    if let Some(provenance) = release_provenances.get(&path) {
        inherited.merge(provenance);
    }
}

/// Why one typed-expression rewrite is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RewriteMode {
    /// Fold required source calls without completing unrelated runtime expressions.
    RequiredExpression,
    /// Execute a prepared const body: every completed expression may fold.
    InterpretedBody,
    /// Release speculation: auxiliary folding stays inside candidate-call arguments.
    Release,
}

impl RewriteMode {
    fn is_release(self) -> bool {
        self == Self::Release
    }

    fn folds_auxiliary(self) -> bool {
        self != Self::RequiredExpression
    }
}

/// One pending rewriter action.
enum RewriteStep {
    Enter {
        expression: ExprId,
        /// Release speculation may fold auxiliary template, collection-read and
        /// fallible-unwrap work for this subtree. It is set only below the arguments of an
        /// eligible release call, so an unrelated runtime expression stays untouched.
        fold_auxiliary: bool,
        /// Whether this subtree is the handled value of an enclosing catch.
        failure_caught: bool,
    },
    Complete {
        expression: ExprId,
        base: usize,
        /// The entry step installed a catch handler; this completion restores it.
        restores_handler: bool,
        fold_auxiliary: bool,
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
        fold_auxiliary: bool,
        failure_caught: bool,
    },
}

/// One iterative typed-expression rewrite.
///
/// WHY a named walk rather than a closure: the walk owns a step stack, a completed-child stack and
/// installed catch handlers. A method keeps the early-exit unwind explicit and readable.
struct RewriteWalk<'walk, 'services> {
    interpreter: &'walk mut ConstInterpreter<'services>,
    bindings: &'walk mut FrameBindings,
    release_provenances: &'walk FxHashMap<PathId, SyntheticInterfaceProvenance>,
    /// Authored-dead statement roots a release rewrite must leave untouched.
    ///
    /// WHY: a dead statement's expression graph skips folding at its root, keeping the runtime
    ///      call that a debug build would have lowered (and that HIR lowering omits with its
    ///      selected evidence) instead of recording a live erased call.
    release_dead_roots: Option<&'walk FxHashSet<ExprId>>,
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
        release_provenances: &FxHashMap<PathId, SyntheticInterfaceProvenance>,
        bindings: &mut FrameBindings,
    ) -> Result<ExprId, ConstantFoldError> {
        self.rewrite_inner(root, mode, release_provenances, bindings, None)
    }

    /// Rewrite one release expression tree without folding authored-dead statement roots.
    pub(super) fn rewrite_release_with_bindings(
        &mut self,
        root: ExprId,
        release_provenances: &FxHashMap<PathId, SyntheticInterfaceProvenance>,
        bindings: &mut FrameBindings,
        release_dead_roots: &FxHashSet<ExprId>,
    ) -> Result<ExprId, ConstantFoldError> {
        self.rewrite_inner(
            root,
            RewriteMode::Release,
            release_provenances,
            bindings,
            Some(release_dead_roots),
        )
    }

    fn rewrite_inner(
        &mut self,
        root: ExprId,
        mode: RewriteMode,
        release_provenances: &FxHashMap<PathId, SyntheticInterfaceProvenance>,
        bindings: &mut FrameBindings,
        release_dead_roots: Option<&FxHashSet<ExprId>>,
    ) -> Result<ExprId, ConstantFoldError> {
        let mut walk = RewriteWalk {
            interpreter: self,
            bindings,
            release_provenances,
            release_dead_roots,
            mode,
            root,
            steps: vec![RewriteStep::Enter {
                expression: root,
                fold_auxiliary: false,
                failure_caught: false,
            }],
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
            if let RewriteStep::Enter { expression, .. } = &step {
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
                RewriteStep::Enter {
                    expression,
                    fold_auxiliary,
                    failure_caught,
                } => self.enter(expression, fold_auxiliary, failure_caught)?,
                RewriteStep::Complete {
                    expression,
                    base,
                    restores_handler,
                    fold_auxiliary,
                } => self.complete(expression, base, restores_handler, fold_auxiliary)?,
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
                    fold_auxiliary,
                    failure_caught,
                } => self.complete_logical(
                    expression,
                    base,
                    right,
                    operation,
                    fold_auxiliary,
                    failure_caught,
                )?,
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

    /// Recognise a call site whose callee is a prepared const function.
    fn const_call_site(
        &self,
        view: &ExpressionView<'_>,
        failure_caught: bool,
    ) -> Result<Option<ConstCallSite>, ConstantFoldError> {
        let ExpressionOperationView::Call(call) = view.operation() else {
            return Ok(None);
        };
        let span = view.span();
        // Every ledger row carries its authored call site, and HIR's pruned-handler liveness split
        // matches a row's span. A spanless row could not be attributed, which would let release
        // accept or reject `$infallible` source differently from debug, so release speculation
        // declines a spanless call and keeps the runtime call in both profiles. No legal source or
        // materialised callable produces one today; this owns the invariant permanently at the
        // ledger's producer.
        if self.mode.is_release() && span.is_none() {
            return Ok(None);
        }
        // A direct catch subject owns the recovery facts its enclosing catch validates. Replacing
        // it with a constant would erase that recovery channel and leave an invalid catch block, so
        // release speculation leaves the call at runtime. Calls merely nested inside a handled
        // subject still fold and carry their containment into the erased-call ledger.
        if self.mode.is_release()
            && view.failure_facts().is_some_and(|facts| {
                matches!(facts.disposition, FailureDisposition::HandledByCatch { .. })
            })
        {
            return Ok(None);
        }
        let is_const_target = call.metadata().evaluation == CallableEvaluation::Const
            || matches!(
                call.metadata().target,
                ResolvedCallableTarget::Function(path)
                    if self.interpreter.executables.contains_key(&path)
            );
        if !is_const_target {
            return Ok(None);
        }
        let ResolvedCallableTarget::Function(path) = call.metadata().target else {
            return Ok(None);
        };
        if call.metadata().evaluation != CallableEvaluation::Const
            && view.success_shape().single().is_none()
        {
            return Ok(None);
        }
        let result_type = view.success_shape().single().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function call used before its single success slot was available",
            )
        })?;
        Ok(Some(ConstCallSite {
            path,
            span,
            value_mode: view.value_mode(),
            result_type,
            failure_caught,
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

    fn enter(
        &mut self,
        expression: ExprId,
        fold_auxiliary: bool,
        failure_caught: bool,
    ) -> Result<(), ConstantFoldError> {
        // Authored-dead source keeps its runtime expressions byte-for-byte. HIR lowering omits
        // exactly those statements and preserves their selected evidence, so folding here would
        // only invent a live erased call from code no profile executes.
        if self
            .release_dead_roots
            .is_some_and(|dead_roots| dead_roots.contains(&expression))
        {
            self.results.push((expression, expression));
            return Ok(());
        }
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
        let const_call = self.const_call_site(&view, failure_caught)?;
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
        if (!self.mode.is_release() || !self.interpreter.budget.call_stack.is_empty())
            && let ExpressionOperationView::LogicalBinary {
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
                fold_auxiliary,
                failure_caught,
            });
            self.steps.push(RewriteStep::Enter {
                expression: left.id(),
                fold_auxiliary,
                failure_caught,
            });
            return Ok(());
        }
        // A release candidate's argument expressions may need the auxiliary folders to
        // become constants (for example a scalar read from a constant collection). Every
        // other release subtree keeps its runtime expression untouched.
        let candidate_arguments = self.mode.is_release()
            && matches!(
                &const_call,
                Some(call) if self.interpreter.executables.contains_key(&call.path)
            );
        let child_fold_auxiliary = fold_auxiliary || candidate_arguments;
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
                fold_auxiliary,
            }),
        }
        // Children complete from the LIFO stack, so push them in reverse source order to
        // keep the collected replacement rows aligned with the parent's child order.
        let handled_value = match view.operation() {
            ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) => {
                Some(block.handled_value)
            }
            _ => None,
        };
        let mut children = Vec::new();
        view.visit_children(|child| children.push(child.id()));
        for child in children.into_iter().rev() {
            // Every failure inside a catch subject is recovered by that catch. A handler body is
            // not protected by its own catch, but inherits an enclosing subject's containment.
            let child_failure_caught = failure_caught || handled_value == Some(child);
            self.steps.push(RewriteStep::Enter {
                expression: child,
                fold_auxiliary: child_fold_auxiliary,
                failure_caught: child_failure_caught,
            });
        }
        Ok(())
    }

    /// Fold every completed call argument.
    ///
    /// Returns `None` when release speculation must keep the runtime call. A required call with a
    /// non-constant argument is a source diagnostic; budget and infrastructure failures escape.
    fn fold_call_arguments(
        &mut self,
        call: &ConstCallSite,
        arguments: &[(ExprId, ExprId)],
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        let mut values = Vec::with_capacity(arguments.len());
        for (_, argument) in arguments {
            match self.interpreter.fold_to_constant(*argument, self.bindings) {
                Ok(Some(value)) => values.push(value),
                Ok(None) if self.mode.is_release() => return Ok(None),
                Ok(None) => {
                    return Err(ConstantFoldError::Diagnostic(
                        choices_control_flow::compile_time_evaluation_error(
                            CompileTimeEvaluationErrorReason::ConstCallRequiresConstantArguments,
                            None,
                            call.span,
                        ),
                    ));
                }
                Err(ConstantFoldError::Budget(mut exhaustion)) => {
                    if exhaustion.call_span.is_none() {
                        exhaustion.call_span = call.span;
                    }
                    return Err(ConstantFoldError::Budget(exhaustion));
                }
                Err(error) if self.mode.is_release() && release_can_preserve_runtime(&error) => {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Some(values))
    }

    fn complete(
        &mut self,
        expression: ExprId,
        base: usize,
        restores_handler: bool,
        fold_auxiliary: bool,
    ) -> Result<(), ConstantFoldError> {
        // Required and interpreted rewriting fold every completed expression. Release
        // speculation folds auxiliary work only inside a candidate call's arguments.
        let auxiliary_folding =
            self.mode.folds_auxiliary() && (!self.mode.is_release() || fold_auxiliary);
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
        let projection_enabled = !self.mode.is_release() || fold_auxiliary;
        let rewritten = self.interpreter.fold_completed_value(
            rewritten,
            &mut *self.bindings,
            auxiliary_folding,
            projection_enabled,
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
        // Move the completed argument pairs off the scratch stack: every path below either
        // replaces them with one completed call pair or leaves the runtime call's pair in their
        // place.
        let arguments = self.results.split_off(base);
        let Some(executable) = self.interpreter.executables.get(&call.path) else {
            if self.mode.is_release() {
                self.results.push((
                    expression,
                    fallback_with_rewritten_children(
                        expression,
                        &arguments,
                        self.interpreter.builder,
                    )?,
                ));
                return Ok(());
            }
            return Err(required_call_error(call, self.interpreter.string_table));
        };
        let Some(argument_values) = self.fold_call_arguments(call, &arguments)? else {
            self.results.push((
                expression,
                fallback_with_rewritten_children(expression, &arguments, self.interpreter.builder)?,
            ));
            return Ok(());
        };
        if argument_values.len() != executable.parameters.len() {
            if self.mode.is_release() {
                self.results.push((
                    expression,
                    fallback_with_rewritten_children(
                        expression,
                        &arguments,
                        self.interpreter.builder,
                    )?,
                ));
                return Ok(());
            }
            return Err(required_call_error(call, self.interpreter.string_table));
        }
        let parameter_values = executable
            .parameters
            .iter()
            .map(|(path, _)| *path)
            .zip(argument_values.iter().copied())
            .collect::<FrameBindings>();
        self.interpreter.budget.enter_call(call.path, call.span)?;
        let returns = self
            .interpreter
            .execute_body(executable, parameter_values)
            .map_err(|error| self.interpreter.attach_call_chain(error));
        self.interpreter.budget.leave_call();
        let returns = match returns {
            Ok(returns) => returns,
            Err(ConstantFoldError::Budget(mut exhaustion)) => {
                // The innermost refused call owns the actionable site; an outer caller
                // must not replace it with a less specific call site.
                if exhaustion.call_span.is_none() {
                    exhaustion.call_span = call.span;
                }
                return Err(ConstantFoldError::Budget(exhaustion));
            }
            Err(error) if self.mode.is_release() && release_can_preserve_runtime(&error) => {
                self.results.push((
                    expression,
                    fallback_with_rewritten_children(
                        expression,
                        &arguments,
                        self.interpreter.builder,
                    )?,
                ));
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if returns.len() != 1 || executable.return_slots.len() != 1 {
            if self.mode.is_release() {
                self.results.push((
                    expression,
                    fallback_with_rewritten_children(
                        expression,
                        &arguments,
                        self.interpreter.builder,
                    )?,
                ));
                return Ok(());
            }
            return Err(required_call_error(call, self.interpreter.string_table));
        }
        if executable.return_slots[0] != call.result_type {
            if self.mode.is_release() {
                self.results.push((
                    expression,
                    fallback_with_rewritten_children(
                        expression,
                        &arguments,
                        self.interpreter.builder,
                    )?,
                ));
                return Ok(());
            }
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "const function result type disagreed with its call result slot",
            )
            .into());
        }
        // The call is about to be erased. Release rewriting must leave the selected call site in
        // the AST ledger so HIR constraint evidence still sees this callee, exactly as a debug
        // build that keeps the runtime call does. Nested candidate execution runs interpreted
        // walks, so a callee's internal folds are never recorded against this caller.
        if self.mode.is_release() {
            // `const_call_site` only recognises a release call whose expression carries an
            // authored span, and the record's span is that same view span.
            let Some(span) = call.span else {
                crate::compiler_bug!(
                    CompilerStage::ConstantEvaluation,
                    "release erased a call site without an authored span"
                );
            };
            self.interpreter.state.release_erased_calls.push((
                call.path,
                span,
                call.failure_caught,
            ));
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
        // The executed value can come from a branch or projection whose own metadata
        // omits a dependency that the authored return expression carried. The candidate's
        // proven transitive set is the authoritative replacement provenance.
        merge_release_candidate_provenance(
            &mut inherited_provenance,
            call.path,
            self.release_provenances,
        );
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
        fold_auxiliary: bool,
        failure_caught: bool,
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
            fold_auxiliary,
        });
        self.steps.push(RewriteStep::Enter {
            expression: right,
            fold_auxiliary,
            failure_caught,
        });
        Ok(())
    }
}

impl ConstInterpreter<'_> {
    /// Apply the auxiliary folders to one completed expression.
    ///
    /// Field projection, collection reads, template completion and fallible unwrapping are the
    /// distinct post-completion folds. Release speculation scopes them to candidate arguments.
    fn fold_completed_value(
        &mut self,
        mut rewritten: ExprId,
        bindings: &mut FrameBindings,
        auxiliary_folding: bool,
        projection_enabled: bool,
    ) -> Result<ExprId, ConstantFoldError> {
        // Field projection after a folded const call stays available in every mode; release
        // speculation scopes it to candidate-call arguments.
        if projection_enabled
            && let Some(projected) = self.constant_field_projection(rewritten, bindings)?
        {
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

/// A runtime call a release candidate could not fold, with any folded children retained.
fn fallback_with_rewritten_children(
    expression: ExprId,
    replacements: &[(ExprId, ExprId)],
    builder: &mut AstExpressionBuilder,
) -> Result<ExprId, ConstantFoldError> {
    // The fallback only exists to install rewritten children. With no rewritten child, rebuilding
    // the parent would allocate an equivalent expression and inherit no new provenance.
    if replacements
        .iter()
        .all(|(original, replacement)| original == replacement)
    {
        return Ok(expression);
    }
    let rewritten = rewrite_expression_children(
        builder,
        builder.own(expression),
        builder.own_slice(replacements),
    )?
    .id();
    inherit_replaced_provenance(builder, expression, rewritten, replacements)
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

/// A speculative attempt may keep these failures as runtime calls; infrastructure must still fail.
fn release_can_preserve_runtime(error: &ConstantFoldError) -> bool {
    matches!(
        error,
        ConstantFoldError::Diagnostic(_) | ConstantFoldError::NumericFailure(_)
    )
}

/// A required call whose declaration body the current executor cannot represent.
fn required_call_error(call: &ConstCallSite, string_table: &mut StringTable) -> ConstantFoldError {
    unsupported_error(labels::REQUIRED_CALL, call.span, string_table)
}
