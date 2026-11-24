//! Const-body eligibility policy.
//!
//! WHAT: classifies typed operations the bounded executor can represent and validates every
//!       expression and catch body of a prepared declaration.
//! WHY: declaration preparation needs a source diagnostic for an unsupported body before any call
//!      runs. A declaration also checks that every root reference names a frame-local binding or
//!      an explicit constant. Unknown operation shapes remain `Supported` so a new expression
//!      variant fails visibly at execution instead of being silently dropped from a rendered
//!      result.
//! MUST NOT: execute a body, charge execution work or rewrite expressions.

use super::labels;
use super::{
    ConstBodyPreparationError, ConstBodyReferenceScope, ConstBodyRejection,
    ConstFunctionExecutable,
    instructions::{ConstInstruction, ConstInstructionKind, compile_handler_body},
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
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

/// Validate every typed expression in a const body and every expression-owned catch handler.
///
/// WHAT: one conservative operation classification plus a reference check that every root is
///       frame-local or an explicit constant. A rejection is a source diagnostic.
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
