//! Const-body eligibility policy.
//!
//! WHAT: classifies typed operations the bounded executor can represent and validates every
//!       expression and catch body of a prepared declaration.
//! WHY: declaration preparation needs a source diagnostic for an unsupported body before any call
//!      runs. A declaration also checks that every root reference names a frame-local binding or
//!      an explicit constant. Every expression variant is classified against executor support.
//! MUST NOT: execute a body, charge execution work or rewrite expressions.

use super::labels;
use super::{
    ConstBodyPreparationError, ConstBodyReferenceScope, ConstBodyRejection,
    ConstFunctionExecutable,
    instructions::{ConstInstruction, ConstInstructionKind, ValueReceiver, compile_handler_body},
};
use crate::compiler_frontend::ast::ast_nodes::MultiBindTargetKind;
use crate::compiler_frontend::ast::const_eval::{
    FoldRequirement, const_cast_policy, registered_external_const_eval_op,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::tir::{TirView, walk_tir_view_expression_payloads};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, types_borrows_targets,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerStage, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::rc::Rc;

/// One operation-level answer for declaration eligibility.
///
/// WHAT: `Unsupported` names a typed expression form the bounded executor cannot evaluate in any
///       context. Root-reference validation stays separate from operation classification.
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
        ExpressionOperationView::Map(_) => ConstExpressionSafety::Unsupported("map construction"),
        ExpressionOperationView::MapBuiltin { .. } => {
            ConstExpressionSafety::Unsupported("map operation")
        }
        ExpressionOperationView::RouteConstruct { .. } => {
            ConstExpressionSafety::Unsupported("constructor route")
        }
        ExpressionOperationView::PropagateOption { .. } => {
            ConstExpressionSafety::Unsupported("option propagation")
        }
        ExpressionOperationView::Constant(_)
        | ExpressionOperationView::Reference(_)
        | ExpressionOperationView::Copy(_)
        | ExpressionOperationView::FieldAccess { .. }
        | ExpressionOperationView::NumericUnary { .. }
        | ExpressionOperationView::NumericBinary { .. }
        | ExpressionOperationView::LogicalNot { .. }
        | ExpressionOperationView::LogicalBinary { .. }
        | ExpressionOperationView::Compare { .. }
        | ExpressionOperationView::Range { .. }
        | ExpressionOperationView::Call(_)
        | ExpressionOperationView::CollectionBuiltin { .. }
        | ExpressionOperationView::Cast { .. }
        | ExpressionOperationView::HandleFallible { .. }
        | ExpressionOperationView::Coerce { .. }
        | ExpressionOperationView::Collection(_)
        | ExpressionOperationView::Struct { .. }
        | ExpressionOperationView::Choice { .. }
        | ExpressionOperationView::ValueBlock(_)
        | ExpressionOperationView::Template(_) => ConstExpressionSafety::Supported,
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
            ConstInstructionKind::Match { arms, default, .. } => {
                for arm in arms.iter() {
                    match &arm.pattern {
                        MatchPattern::ChoiceVariant { captures, .. } => {
                            roots.extend(captures.iter().map(|capture| capture.binding_path));
                        }
                        MatchPattern::OptionPresentCapture { binding_path, .. } => {
                            roots.insert(*binding_path);
                        }
                        MatchPattern::Literal(_)
                        | MatchPattern::OptionNone { .. }
                        | MatchPattern::OptionValue { .. }
                        | MatchPattern::Relational { .. } => {}
                    }
                    collect_local_binding_roots(&arm.body, roots);
                }
                if let Some(default) = default {
                    collect_local_binding_roots(default, roots);
                }
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
            ConstInstructionKind::ValueBlock { body, receiver } => {
                collect_receiver_roots(receiver, roots);
                collect_local_binding_roots(body, roots);
            }
            ConstInstructionKind::MultiBind { receiver, .. } => {
                collect_receiver_roots(receiver, roots);
            }
            ConstInstructionKind::Assign { .. }
            | ConstInstructionKind::Eval(_)
            | ConstInstructionKind::Return(_)
            | ConstInstructionKind::Recover(_)
            | ConstInstructionKind::Then(_)
            | ConstInstructionKind::Break
            | ConstInstructionKind::Continue
            | ConstInstructionKind::Assert { .. } => {}
        }
    }
}

fn collect_receiver_roots(receiver: &ValueReceiver, roots: &mut FxHashSet<PathId>) {
    match receiver {
        ValueReceiver::Declaration { path, .. } => {
            roots.insert(*path);
        }
        ValueReceiver::MultiBind(targets) => roots.extend(
            targets
                .iter()
                .filter(|target| matches!(target.kind, MultiBindTargetKind::Declaration))
                .map(|target| target.id),
        ),
        ValueReceiver::Assignment(_) | ValueReceiver::Return => {}
    }
}

/// Validate every typed expression in a const body and every expression-owned catch handler.
///
/// WHAT: checks typed operations and body-local references, including effective TIR sites.
///       TIR owns its loop and capture bindings, so its references stay under TIR preparation.
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
    let mut tir_roots = VecDeque::new();
    collect_instruction_expression_roots(&executable.instructions, expressions, &mut roots)?;
    let mut visited = FxHashSet::default();
    while let Some((root, in_tir)) = roots
        .pop_front()
        .map(|root| (root, false))
        .or_else(|| tir_roots.pop_front().map(|root| (root, true)))
    {
        if !visited.insert((root, in_tir)) {
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
        if let ExpressionOperationView::Call(call) = view.operation()
            && !(call.metadata().evaluation == CallableEvaluation::Const
                && matches!(call.metadata().target, ResolvedCallableTarget::Function(_)))
            && registered_external_const_eval_op(
                view,
                Some(references.scope.external_package_registry.as_ref()),
            )
            .is_none()
        {
            return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
                operation: "runtime call",
                span: view.span(),
            }));
        }
        if let ExpressionOperationView::Cast { facts, .. } = view.operation()
            && let Err(diagnostic) =
                const_cast_policy(facts, view.span(), FoldRequirement::Required)
        {
            return Err(ConstBodyPreparationError::Diagnostic(diagnostic));
        }
        if !in_tir
            && let Some(operation) = unsupported_root_reference(&view, &local_roots, &references)
        {
            return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
                operation,
                span: view.span(),
            }));
        }
        if let ExpressionOperationView::Template(template) = view.operation() {
            let store = references.scope.template_ir_store.borrow();
            let reference = template.tir_reference;
            let tir_view = TirView::new(&store, reference.root, reference.phase, reference.context)
                .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
            walk_tir_view_expression_payloads(&tir_view, expressions, &mut |site| {
                tir_roots.push_back(site.id());
                Ok(())
            })
            .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
        }
        if let ExpressionOperationView::ValueBlock(ValueBlock::Catch(block)) = view.operation()
            && let FallibleHandling::Handler { error, body } = &block.handler
        {
            // The handler contract introduces the error binding; no body statement declares it.
            if let Some(binding) = error {
                local_roots.insert(binding.error_binding);
            }
            let instructions = compile_handler_body(body, expressions)
                .map_err(ConstBodyPreparationError::Unsupported)?;
            collect_local_binding_roots(&instructions, &mut local_roots);
            handler_bodies.insert(root, Rc::clone(&instructions));
            let queue = if in_tir { &mut tir_roots } else { &mut roots };
            collect_instruction_expression_roots(&instructions, expressions, queue)?;
        }
        let queue = if in_tir { &mut tir_roots } else { &mut roots };
        view.visit_children(|child| queue.push_back(child.id()));
    }
    executable.handler_bodies = Rc::new(handler_bodies);
    Ok(())
}

/// Why one root reference cannot produce an interpretable value.
///
/// WHAT: a `Reference` or `Copy` must name a local binding or an explicit `#` constant.
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
        ExpressionOperationView::Copy(place) => place.metadata().root,
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
) -> Result<(), ConstBodyPreparationError> {
    for instruction in instructions {
        match &instruction.kind {
            ConstInstructionKind::Bind {
                expression,
                value_mode,
                ..
            } => {
                let view = expressions
                    .view(*expression)
                    .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
                // Follow only a direct authored field chain. A copy or computed base is fresh
                // or dynamic, and must not be treated as an alias of its operands.
                let mut place = view;
                while let ExpressionOperationView::FieldAccess { base, .. } = place.operation() {
                    place = base;
                }
                if value_mode.is_mutable()
                    && !place.value_mode().is_mutable()
                    && let ExpressionOperationView::Reference(path) = place.operation()
                {
                    return Err(ConstBodyPreparationError::Diagnostic(
                        types_borrows_targets::invalid_mutable_access(
                            DiagnosticPlace::Path(path),
                            InvalidMutableAccessReason::ImmutablePlace,
                            None,
                            None,
                            view.span().or(instruction.span),
                        ),
                    ));
                }
                roots.push_back(*expression);
            }
            ConstInstructionKind::MultiBind { expression, .. }
            | ConstInstructionKind::Eval(expression) => roots.push_back(*expression),
            ConstInstructionKind::Assign { place, expression } => {
                roots.push_back(*expression);
                expressions
                    .store()
                    .visit_place_index_expressions(*place, &mut |index| roots.push_back(index))
                    .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
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
            ConstInstructionKind::Match {
                scrutinee,
                arms,
                default,
            } => {
                roots.push_back(*scrutinee);
                for arm in arms.iter() {
                    arm.pattern
                        .visit_expression_ids(&mut |id| roots.push_back(id));
                    roots.extend(arm.guard);
                    collect_instruction_expression_roots(&arm.body, expressions, roots)?;
                }
                if let Some(default) = default {
                    collect_instruction_expression_roots(default, expressions, roots)?;
                }
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
            | ConstInstructionKind::Recover(values)
            | ConstInstructionKind::Then(values) => {
                roots.extend(values.iter().copied());
            }
            ConstInstructionKind::ValueBlock { body, receiver } => {
                if let ValueReceiver::Assignment(place) = receiver.as_ref() {
                    expressions
                        .store()
                        .visit_place_index_expressions(*place, &mut |index| roots.push_back(index))
                        .map_err(|error| ConstBodyPreparationError::Infrastructure(error.into()))?;
                }
                collect_instruction_expression_roots(body, expressions, roots)?;
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
