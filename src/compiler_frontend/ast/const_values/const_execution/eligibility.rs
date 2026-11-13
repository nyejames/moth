//! Const-body and release-candidate eligibility policy.
//!
//! WHAT: classifies typed operations the bounded executor can represent, validates every
//!       expression and catch body of a prepared declaration, and proves which ordinary function
//!       bodies may be folded as release candidates.
//! WHY: declaration preparation needs a source diagnostic for an unsupported body, while release
//!      optimisation must refuse an unproven candidate without changing the runtime call. Both
//!      decisions share the operation classifier and the prepared instruction stack, but they stop
//!      at different boundaries: a declaration also checks that every root reference names a
//!      frame-local binding or an explicit constant, while a release candidate additionally proves
//!      purity, receiver locality and the absence of module-state writes. Unknown operation shapes
//!      remain `Supported` so a new expression variant fails visibly at execution instead of being
//!      silently dropped from a rendered result.
//! MUST NOT: execute a body, charge execution work or rewrite expressions.

use super::labels;
use super::{
    ConstBodyPreparationError, ConstBodyReferenceScope, ConstBodyRejection,
    ConstFunctionExecutable, executable_for_body,
    instructions::{ConstInstruction, ConstInstructionKind, compile_handler_body},
};
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_eval::registered_external_const_eval_op;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::rc::Rc;

/// One operation-level answer shared by declaration and speculative eligibility.
///
/// WHAT: `Unsupported` names a typed expression form the bounded executor cannot evaluate in any
///       context. Call-target purity and binding safety stay separate policy facts, because a
///       release candidate may prove an ordinary function transitive callee const-safe while a
///       declaration may not call it at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstExpressionSafety {
    Supported,
    Unsupported(&'static str),
}

pub(crate) fn classify_const_expression(
    operation: ExpressionOperationView<'_>,
) -> ConstExpressionSafety {
    match operation {
        ExpressionOperationView::MethodCall { .. } => {
            ConstExpressionSafety::Unsupported(labels::RECEIVER_METHOD_CALL)
        }
        ExpressionOperationView::RuntimeTemplate(_) => {
            ConstExpressionSafety::Unsupported(labels::RUNTIME_TEMPLATE)
        }
        ExpressionOperationView::RuntimeSlotApplication(_) => {
            ConstExpressionSafety::Unsupported(labels::RUNTIME_TEMPLATE_SLOT)
        }
        _ => ConstExpressionSafety::Supported,
    }
}

/// Build the typed diagnostic for one refused declaration body.
pub(crate) fn const_body_rejection_diagnostic(
    rejection: &ConstBodyRejection,
    fallback_span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> DiagnosticDraft {
    let operation = string_table.intern(rejection.operation);
    choices_control_flow::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstFunctionExecutionUnsupported,
        Some(operation),
        rejection.span.or(fallback_span),
    )
}

/// Prepare one ordinary function body as a conservative release candidate.
///
/// WHAT: returns `None` when the signature, parameters or body cannot prove purity; returns an
///       infrastructure error only for internal failures. The same prepared instruction stack and
///       expression classifier used by required calls execute the candidate.
/// WHY:  speculative folding must never change a runtime call it cannot prove. A refused candidate
///       stays an ordinary HIR call, so an imprecise proof is safe but an over-permissive one is not.
pub(crate) fn prepare_release_executable(
    signature: &FunctionSignature,
    body: &[AstNode],
    expressions: &AstExpressionBuilder,
    types: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    references: ConstBodyReferenceScope<'_>,
) -> Result<Option<(ConstFunctionExecutable, ReleaseCandidateProof)>, ConstBodyPreparationError> {
    if !signature.parameters.iter().all(|parameter| {
        parameter.value_mode == ValueMode::ImmutableOwned
            && release_type_is_independent_scalar(parameter.type_id, types)
    }) {
        return Ok(None);
    }
    let Some(return_shape) = signature.return_contract.ordinary() else {
        return Ok(None);
    };
    // Expression constants carry exactly one typed value. A zero- or multiple-slot ordinary call
    // has no native constant carrier here, so keeping it at runtime preserves its ordered success
    // shape instead of inventing a tuple-like replacement.
    if return_shape.success().len() != 1
        || return_shape.is_fallible()
        || !release_type_is_independent_scalar(return_shape.success()[0], types)
    {
        return Ok(None);
    }
    let mut executable = match executable_for_body(signature, body) {
        Ok(executable) => executable,
        Err(_) => {
            return Ok(None);
        }
    };
    match validate_const_body_expressions(&mut executable, expressions, references) {
        Ok(()) => {}
        Err(ConstBodyPreparationError::Unsupported(_)) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    let Some(proof) = release_body_is_pure(&executable, expressions, external_package_registry)?
    else {
        return Ok(None);
    };
    Ok(Some((executable, proof)))
}

/// Whether a release candidate may carry this result/parameter type.
///
/// WHAT: admits only fixed-width scalars, Bool and Char.
/// WHY:  constants preserve scalar values exactly. Aggregate, collection and String constants
///       duplicate shared child topology when lowered, and resource-bearing values need a
///       provenance contract the release pass does not yet own. Conservative refusal leaves the
///       ordinary runtime call untouched.
fn release_type_is_independent_scalar(type_id: TypeId, types: &TypeEnvironment) -> bool {
    type_id == builtin_type_ids::BOOL
        || type_id == builtin_type_ids::CHAR
        || types.fixed_scalar(type_id).is_some()
}

/// Reject runtime calls and host effects that a speculative fold must not erase.
///
/// WHAT: returns the ordinary callees this body can fold transitively, or `None` when the body has
///       an effect, a possible module-state write or an unsupported operation.
/// WHY:  a const declaration already refuses a runtime call at parse time. An ordinary body does
///       not, so the release classifier has to prove every call is an existing const call, a
///       registered pure Core compile-time operation or another candidate. Recording ordinary
///       callees lets the caller reject dependency cycles before any speculative execution.
fn release_body_is_pure(
    executable: &ConstFunctionExecutable,
    expressions: &AstExpressionBuilder,
    external_package_registry: &ExternalPackageRegistry,
) -> Result<Option<ReleaseCandidateProof>, ConstBodyPreparationError> {
    let mut local_roots: FxHashSet<PathId> = executable
        .parameters
        .iter()
        .map(|(path, _)| *path)
        .collect();
    collect_local_binding_roots(&executable.instructions, &mut local_roots);
    for handler in executable.handler_bodies.values() {
        collect_local_binding_roots(handler, &mut local_roots);
    }

    let mut roots = VecDeque::new();
    collect_instruction_expression_roots(&executable.instructions, expressions, &mut roots)
        .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
    for handler in executable.handler_bodies.values() {
        collect_instruction_expression_roots(handler, expressions, &mut roots)
            .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
    }

    let mut proof = ReleasePurityProof::default();
    let mut visited = FxHashSet::default();
    while let Some(root) = roots.pop_front() {
        if !visited.insert(root) {
            continue;
        }
        let view = expressions.view(root).map_err(|error| {
            ConstBodyPreparationError::Infrastructure(Box::new(
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    format!("release eligibility could not read a completed expression: {error:?}"),
                )
                .into(),
            ))
        })?;
        if matches!(
            classify_const_expression(view.operation()),
            ConstExpressionSafety::Unsupported(_)
        ) {
            return Ok(None);
        }
        if matches!(view.operation(), ExpressionOperationView::Template(_)) {
            return Ok(None);
        }
        if let Some(facts) = view.occurrence_facts() {
            proof.body_provenance.merge(&facts.provenance);
        }
        match view.operation() {
            ExpressionOperationView::Call(call) => {
                if call.handling().is_some() || call.metadata().error_type.is_some() {
                    return Ok(None);
                }
                match call.metadata().target {
                    ResolvedCallableTarget::Function(path) => {
                        if call.metadata().evaluation != CallableEvaluation::Const {
                            proof.ordinary_calls.insert(path);
                        }
                    }
                    ResolvedCallableTarget::Host(_)
                        if registered_external_const_eval_op(
                            view,
                            Some(external_package_registry),
                        )
                        .is_some() => {}
                    _ => return Ok(None),
                }
            }
            // Builtins that mutate a receiver must only mutate a body-local binding. A
            // module-level collection can otherwise be changed by a call the compiler erases.
            ExpressionOperationView::CollectionBuiltin { receiver, .. }
            | ExpressionOperationView::MapBuiltin { receiver, .. }
                if !projected_place_is_local(receiver, &local_roots) =>
            {
                return Ok(None);
            }
            _ => {}
        }
        view.visit_children(|child| roots.push_back(child.id()));
    }

    if instruction_writes_module_state(&executable.instructions, expressions, &local_roots)? {
        return Ok(None);
    }
    for handler in executable.handler_bodies.values() {
        if instruction_writes_module_state(handler, expressions, &local_roots)? {
            return Ok(None);
        }
    }
    // The release replacement merges this candidate's complete effective provenance, including
    // transitive ordinary callees. That attachment is authoritative for every selected branch,
    // index or projection, so no per-return subset gate is needed here.
    Ok(Some(ReleaseCandidateProof {
        ordinary_calls: proof.ordinary_calls,
        body_provenance: proof.body_provenance,
    }))
}

#[derive(Default)]
struct ReleasePurityProof {
    ordinary_calls: FxHashSet<PathId>,
    body_provenance: SyntheticInterfaceProvenance,
}

fn collect_local_binding_roots(instructions: &[ConstInstruction], roots: &mut FxHashSet<PathId>) {
    for instruction in instructions {
        match &instruction.kind {
            ConstInstructionKind::Bind { path, .. }
            | ConstInstructionKind::BindValue { path, .. } => {
                roots.insert(*path);
            }
            ConstInstructionKind::If {
                then_body,
                else_body,
                ..
            } => {
                collect_local_binding_roots(then_body, roots);
                collect_local_binding_roots(else_body, roots);
            }
            ConstInstructionKind::While { body, .. } => {
                collect_local_binding_roots(body, roots);
            }
            ConstInstructionKind::Range { bindings, body, .. }
            | ConstInstructionKind::Collection { bindings, body, .. } => {
                if let Some(item) = &bindings.item {
                    roots.insert(item.id);
                }
                if let Some(index) = &bindings.index {
                    roots.insert(index.id);
                }
                collect_local_binding_roots(body, roots);
            }
            ConstInstructionKind::Assign { .. }
            | ConstInstructionKind::Eval(_)
            | ConstInstructionKind::Return(_)
            | ConstInstructionKind::ReturnFromFunction(_)
            | ConstInstructionKind::Break
            | ConstInstructionKind::Continue
            | ConstInstructionKind::Assert { .. } => {}
        }
    }
}

fn instruction_writes_module_state(
    instructions: &[ConstInstruction],
    expressions: &AstExpressionBuilder,
    local_roots: &FxHashSet<PathId>,
) -> Result<bool, ConstBodyPreparationError> {
    for instruction in instructions {
        match &instruction.kind {
            ConstInstructionKind::Assign { place, .. } => {
                let root = expressions
                    .place(*place)
                    .map_err(|error| {
                        ConstBodyPreparationError::Infrastructure(Box::new(
                            InfrastructureFailure::internal(
                                CompilerStage::ConstantEvaluation,
                                format!(
                                    "release eligibility could not read an assignment target: {error:?}"
                                ),
                            )
                            .into(),
                        ))
                    })?
                    .metadata()
                    .root;
                if !local_roots.contains(&root) {
                    return Ok(true);
                }
            }
            ConstInstructionKind::If {
                then_body,
                else_body,
                ..
            } => {
                if instruction_writes_module_state(then_body, expressions, local_roots)?
                    || instruction_writes_module_state(else_body, expressions, local_roots)?
                {
                    return Ok(true);
                }
            }
            ConstInstructionKind::While { body, .. } => {
                if instruction_writes_module_state(body, expressions, local_roots)? {
                    return Ok(true);
                }
            }
            ConstInstructionKind::Range { body, .. }
            | ConstInstructionKind::Collection { body, .. } => {
                if instruction_writes_module_state(body, expressions, local_roots)? {
                    return Ok(true);
                }
            }
            ConstInstructionKind::Bind { .. }
            | ConstInstructionKind::BindValue { .. }
            | ConstInstructionKind::Eval(_)
            | ConstInstructionKind::Return(_)
            | ConstInstructionKind::ReturnFromFunction(_)
            | ConstInstructionKind::Break
            | ConstInstructionKind::Continue
            | ConstInstructionKind::Assert { .. } => {}
        }
    }
    Ok(false)
}

/// A collection builtin may mutate its receiver only when the receiver is a local binding.
fn projected_place_is_local(
    expression: ExpressionView<'_>,
    local_roots: &FxHashSet<PathId>,
) -> bool {
    let mut current = expression;
    loop {
        match current.operation() {
            ExpressionOperationView::Copy(place) => {
                return local_roots.contains(&place.metadata().root);
            }
            ExpressionOperationView::FieldAccess { base, .. } => current = base,
            _ => return false,
        }
    }
}

/// Validate every typed expression in a const body and every expression-owned catch handler.
///
/// WHAT: one conservative operation classification shared by declaration preparation and the
///       release candidate classifier, plus a reference check that every root is frame-local or an
///       explicit constant. A rejection is a source diagnostic on a declaration and an ineligible
///       candidate during speculation.
/// WHY: expression handlers are prepared here, then executed only when their operation fails.
///      Declaration preparation must still prove the whole authored body is representable, or an
///      uncalled const function can carry unsupported statements that only fail after a call.
pub(crate) fn validate_const_body_expressions(
    executable: &mut ConstFunctionExecutable,
    expressions: &AstExpressionBuilder,
    references: ConstBodyReferenceScope<'_>,
) -> Result<(), ConstBodyPreparationError> {
    let mut local_roots: FxHashSet<PathId> = executable
        .parameters
        .iter()
        .map(|(path, _)| *path)
        .collect();
    collect_local_binding_roots(&executable.instructions, &mut local_roots);
    let mut handler_bodies = FxHashMap::default();
    let mut roots = VecDeque::new();
    collect_instruction_expression_roots(&executable.instructions, expressions, &mut roots)
        .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
    let mut visited = FxHashSet::default();
    while let Some(root) = roots.pop_front() {
        if !visited.insert(root) {
            continue;
        }
        let view = expressions.view(root).map_err(|error| {
            ConstBodyPreparationError::Infrastructure(Box::new(
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    format!(
                        "const body eligibility could not read a completed expression: {error:?}"
                    ),
                )
                .into(),
            ))
        })?;
        if let ConstExpressionSafety::Unsupported(operation) =
            classify_const_expression(view.operation())
        {
            return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
                operation,
                span: view.span(),
            }));
        }
        if let Some(operation) = unsupported_root_reference(&view, &local_roots, &references) {
            return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
                operation,
                span: view.span(),
            }));
        }
        if let ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) = view.operation()
            && let FallibleHandling::Handler { error, body } = &block.handler
        {
            // The handler contract introduces the error binding; no body statement declares it.
            if let Some(binding) = error {
                local_roots.insert(binding.error_binding);
            }
            let instructions =
                compile_handler_body(body).map_err(ConstBodyPreparationError::Unsupported)?;
            collect_local_binding_roots(&instructions, &mut local_roots);
            handler_bodies.insert(root, Rc::clone(&instructions));
            collect_instruction_expression_roots(&instructions, expressions, &mut roots)
                .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
        }
        view.visit_children(|child| roots.push_back(child.id()));
    }
    executable.handler_bodies = Rc::new(handler_bodies);
    Ok(())
}

/// Why one root reference cannot produce an interpretable value.
///
/// WHAT: a `Reference`, or a `Copy` with no projections, is acceptable only when its root names a
///       frame-local binding or an explicit `#` constant.
/// WHY: a template content reference is not a constant the bounded executor can resolve. Rejecting
///      it during preparation stops an uncalled body carrying work that only fails at its first
///      call site.
fn unsupported_root_reference(
    view: &ExpressionView<'_>,
    local_roots: &FxHashSet<PathId>,
    references: &ConstBodyReferenceScope<'_>,
) -> Option<&'static str> {
    let root = match view.operation() {
        ExpressionOperationView::Reference(path) => path,
        ExpressionOperationView::Copy(place) if place.projections().len() == 0 => {
            place.metadata().root
        }
        _ => return None,
    };
    if local_roots.contains(&root) {
        return None;
    }
    references
        .scope
        .explicit_constant_value(&root, references.path_fork, references.constants)
        .is_none()
        .then_some(labels::CONTENT_REFERENCE)
}

fn collect_instruction_expression_roots(
    instructions: &[ConstInstruction],
    expressions: &AstExpressionBuilder,
    roots: &mut VecDeque<ExprId>,
) -> Result<(), InfrastructureFailure> {
    for instruction in instructions {
        match &instruction.kind {
            ConstInstructionKind::Bind { expression, .. }
            | ConstInstructionKind::Eval(expression) => roots.push_back(*expression),
            ConstInstructionKind::Assign { place, expression } => {
                roots.push_back(*expression);
                expressions
                    .store()
                    .visit_place_index_expressions(*place, &mut |index| roots.push_back(index))?;
            }
            ConstInstructionKind::If {
                condition,
                then_body,
                else_body,
            } => {
                roots.push_back(*condition);
                collect_instruction_expression_roots(then_body, expressions, roots)?;
                collect_instruction_expression_roots(else_body, expressions, roots)?;
            }
            ConstInstructionKind::While { condition, body } => {
                roots.push_back(*condition);
                collect_instruction_expression_roots(body, expressions, roots)?;
            }
            ConstInstructionKind::Range { range, body, .. } => {
                roots.push_back(range.start);
                roots.push_back(range.end);
                if let Some(step) = range.step {
                    roots.push_back(step);
                }
                collect_instruction_expression_roots(body, expressions, roots)?;
            }
            ConstInstructionKind::Collection { iterable, body, .. } => {
                roots.push_back(*iterable);
                collect_instruction_expression_roots(body, expressions, roots)?;
            }
            ConstInstructionKind::Return(values)
            | ConstInstructionKind::ReturnFromFunction(values) => {
                roots.extend(values.iter().copied());
            }
            ConstInstructionKind::Assert { condition, message } => {
                roots.push_back(*condition);
                roots.push_back(*message);
            }
            ConstInstructionKind::BindValue { .. }
            | ConstInstructionKind::Break
            | ConstInstructionKind::Continue => {}
        }
    }
    Ok(())
}

/// Facts that stay attached to a prepared release candidate.
///
/// WHY: transitive callees contribute dependencies that are absent from the caller's own
///      expression occurrences. Keeping each candidate's body-wide provenance and ordinary
///      callees lets the retention pass propagate those dependencies before any call is erased.
pub(super) struct ReleaseCandidateProof {
    pub(super) ordinary_calls: FxHashSet<PathId>,
    pub(super) body_provenance: SyntheticInterfaceProvenance,
}

pub(super) struct ReleaseCandidate {
    pub(super) executable: ConstFunctionExecutable,
    pub(super) proof: ReleaseCandidateProof,
}

/// The release candidates that survived the dependency graph.
///
/// WHAT: one executable row and one merged provenance set per retained candidate path.
/// WHY: the rewrite needs both maps keyed identically; returning a named value keeps the pairing
///      explicit and stops a caller from zipping two anonymous maps by iteration order.
pub(super) struct RetainedReleaseCandidates {
    pub(super) executables: FxHashMap<PathId, ConstFunctionExecutable>,
    pub(super) provenances: FxHashMap<PathId, SyntheticInterfaceProvenance>,
}

/// Keep only candidates whose whole ordinary-call closure is foldable and acyclic.
///
/// WHY: the executor can follow an ordinary callee only when that callee was prepared too. A
///      missing dependency or a cycle would either abort required evaluation or fall back inside
///      an outer speculative call after partial work, so reject the whole candidate before rewrite.
pub(super) fn retain_acyclic_release_candidates(
    candidates: FxHashMap<PathId, ReleaseCandidate>,
) -> RetainedReleaseCandidates {
    let graph: FxHashMap<PathId, FxHashSet<PathId>> = candidates
        .iter()
        .map(|(path, candidate)| (*path, candidate.proof.ordinary_calls.clone()))
        .collect();
    let reverse = reverse_call_graph(&graph);
    let eligible = eligible_acyclic_paths(&graph, &reverse);

    // A callee's dependency is also reachable through the caller's erased call, even when the
    // returned value comes from another branch. Propagate the transitive set before accepting a
    // candidate; the rewrite merges the complete set onto every successful replacement.
    let order = candidate_dependency_order(&graph, &eligible);
    let mut effective_provenance: FxHashMap<PathId, SyntheticInterfaceProvenance> =
        FxHashMap::default();
    let mut incomplete = FxHashSet::default();
    for path in order {
        let Some(candidate) = candidates.get(&path) else {
            continue;
        };
        let mut provenance = candidate.proof.body_provenance.clone();
        for callee in &candidate.proof.ordinary_calls {
            match effective_provenance.get(callee) {
                Some(callee_provenance) => provenance.merge(callee_provenance),
                None => {
                    // Every eligible callee was ordered first. Reaching this branch means the
                    // graph proof and traversal disagreed, so refuse rather than silently
                    // dropping transitive provenance.
                    incomplete.insert(path);
                }
            }
        }
        effective_provenance.insert(path, provenance);
    }
    let failed: FxHashSet<PathId> = eligible
        .iter()
        .filter(|path| incomplete.contains(path) || !effective_provenance.contains_key(path))
        .copied()
        .collect();
    let removed = dependent_closure(&graph, &reverse, &failed);
    let retained: FxHashSet<PathId> = eligible.difference(&removed).copied().collect();
    let provenances = effective_provenance
        .into_iter()
        .filter(|(path, _)| retained.contains(path))
        .collect();
    let executables = candidates
        .into_iter()
        .filter(|(path, _)| retained.contains(path))
        .map(|(path, candidate)| (path, candidate.executable))
        .collect();
    RetainedReleaseCandidates {
        executables,
        provenances,
    }
}

/// Order eligible candidates so every callee is visited before its callers.
pub(super) fn candidate_dependency_order(
    graph: &FxHashMap<PathId, FxHashSet<PathId>>,
    eligible: &FxHashSet<PathId>,
) -> Vec<PathId> {
    let adjacency: FxHashMap<PathId, Vec<PathId>> = eligible
        .iter()
        .map(|path| {
            let callees = graph
                .get(path)
                .into_iter()
                .flatten()
                .filter(|callee| eligible.contains(callee))
                .copied()
                .collect();
            (*path, callees)
        })
        .collect();
    let mut visited = FxHashSet::default();
    let mut order = Vec::with_capacity(eligible.len());
    for root in eligible {
        if !visited.insert(*root) {
            continue;
        }
        let mut stack = vec![(*root, 0usize)];
        while let Some((path, next)) = stack.last_mut() {
            let callees = adjacency.get(path).map(Vec::as_slice).unwrap_or_default();
            if *next < callees.len() {
                let callee = callees[*next];
                *next += 1;
                if visited.insert(callee) {
                    stack.push((callee, 0));
                }
            } else {
                order.push(*path);
                stack.pop();
            }
        }
    }
    order
}

/// Every candidate that reaches one of `roots` through an ordinary-call edge.
fn dependent_closure(
    graph: &FxHashMap<PathId, FxHashSet<PathId>>,
    reverse: &FxHashMap<PathId, Vec<PathId>>,
    roots: &FxHashSet<PathId>,
) -> FxHashSet<PathId> {
    if roots.is_empty() {
        return FxHashSet::default();
    }
    debug_assert_eq!(reverse.len(), graph.len(), "one reverse row per candidate");
    let mut removed = roots.clone();
    let mut queue: Vec<PathId> = roots.iter().copied().collect();
    while let Some(path) = queue.pop() {
        if let Some(dependents) = reverse.get(&path) {
            for dependent in dependents {
                if removed.insert(*dependent) {
                    queue.push(*dependent);
                }
            }
        }
    }
    removed
}

/// Reverse every ordinary-call edge once for the graph passes that need dependents.
pub(super) fn reverse_call_graph(
    graph: &FxHashMap<PathId, FxHashSet<PathId>>,
) -> FxHashMap<PathId, Vec<PathId>> {
    let mut reverse: FxHashMap<PathId, Vec<PathId>> =
        graph.keys().map(|path| (*path, Vec::new())).collect();
    for (caller, callees) in graph {
        for callee in callees {
            if let Some(dependents) = reverse.get_mut(callee) {
                dependents.push(*caller);
            }
        }
    }
    reverse
}

/// Resolve missing dependencies and cycles in one linear graph walk.
///
/// WHAT: Kosaraju SCC detection removes every cycle, then reverse edges propagate removal to all
///       dependents. A dependency outside the candidate set also removes its caller.
/// WHY: SCC detection and reverse-edge propagation each visit every edge once, so a long chain
///      ending in one ineligible callee costs linear rather than per-removal rescanning work.
pub(super) fn eligible_acyclic_paths(
    graph: &FxHashMap<PathId, FxHashSet<PathId>>,
    reverse: &FxHashMap<PathId, Vec<PathId>>,
) -> FxHashSet<PathId> {
    let adjacency: FxHashMap<PathId, Vec<PathId>> = graph
        .iter()
        .map(|(path, callees)| (*path, callees.iter().copied().collect()))
        .collect();
    let cyclic = cyclic_paths(&adjacency, reverse);
    let mut ineligible = cyclic;
    for (path, callees) in graph {
        if callees.iter().any(|callee| !graph.contains_key(callee)) {
            ineligible.insert(*path);
        }
    }
    let mut queue: Vec<PathId> = ineligible.iter().copied().collect();
    while let Some(path) = queue.pop() {
        if let Some(dependents) = reverse.get(&path) {
            for dependent in dependents {
                if ineligible.insert(*dependent) {
                    queue.push(*dependent);
                }
            }
        }
    }
    graph
        .keys()
        .filter(|path| !ineligible.contains(path))
        .copied()
        .collect()
}

/// Identify every path in a dependency cycle using iterative Kosaraju SCC passes.
fn cyclic_paths(
    graph: &FxHashMap<PathId, Vec<PathId>>,
    reverse: &FxHashMap<PathId, Vec<PathId>>,
) -> FxHashSet<PathId> {
    let mut visited = FxHashSet::default();
    let mut finish_order = Vec::with_capacity(graph.len());
    for root in graph.keys() {
        if !visited.insert(*root) {
            continue;
        }
        let mut stack = vec![(*root, 0usize)];
        while let Some((path, next)) = stack.last_mut() {
            let Some(callees) = graph.get(path) else {
                finish_order.push(*path);
                stack.pop();
                continue;
            };
            if *next < callees.len() {
                let callee = callees[*next];
                *next += 1;
                if visited.insert(callee) {
                    stack.push((callee, 0));
                }
            } else {
                finish_order.push(*path);
                stack.pop();
            }
        }
    }

    let mut assigned = FxHashSet::default();
    let mut cyclic = FxHashSet::default();
    for root in finish_order.into_iter().rev() {
        if !assigned.insert(root) {
            continue;
        }
        let mut component = vec![root];
        let mut stack = vec![root];
        while let Some(path) = stack.pop() {
            if let Some(dependents) = reverse.get(&path) {
                for dependent in dependents {
                    if assigned.insert(*dependent) {
                        component.push(*dependent);
                        stack.push(*dependent);
                    }
                }
            }
        }
        let self_cycle = graph
            .get(&root)
            .is_some_and(|callees| callees.contains(&root));
        if component.len() > 1 || self_cycle {
            cyclic.extend(component);
        }
    }
    cyclic
}
