//! Assertion-message escape classification.
//!
//! WHAT: classifies control-flow effects reachable from an assertion message across resolved AST
//!       expressions, canonical store-aware TIR views at the relevant AST phases, and finalized
//!       owned runtime-template handoffs when those representations exist.
//! WHY: assertion messages execute only on the assertion failure edge. An effect that exits the
//!      enclosing function, propagates a carrier, or propagates an option would bypass the
//!      assertion's terminal failure operation and therefore must be rejected at the AST boundary.
//!
//! ## Representation boundary
//!
//! This module reads resolved AST expression payloads and uses the canonical store-aware TIR and
//! owned-handoff walkers for representations present at the current AST phase. It owns the effect
//! classification only; it does not parse syntax, type-check values, normalize templates, lower
//! HIR, or execute runtime behaviour.
//!
//! The traversal is scoped to the current enclosing function. Nested function bodies are opaque,
//! and `break` / `continue` are local to the loop that owns them. Assertion call arguments do not
//! accept value-producing blocks, so a depth-zero loop-control node is an internal AST invariant,
//! not a valid route to an enclosing source loop. Deliberately excluded are plain values, ordinary
//! call control flow, and runtime-template nodes without an expression payload.

use std::collections::HashSet;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, ImplicitFailureContributor, ImplicitFailureSource,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
    walk_owned_runtime_slot_application_handoff, walk_owned_runtime_template_handoff,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplateTirReference, walk_expression_payloads_with_nested_tir_views,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use crate::compiler_frontend::source::SourceSpan;

/// One control-flow effect that can escape an assertion message's value computation.
///
/// WHAT: retains the exact authored source span for the effect rather than collapsing every
///       operation onto the surrounding call/value expression.
/// WHY: diagnostics and internal tests need to distinguish `!`, `?`, return, and error-return
///      sites while ordinary call mapping continues to use the call's own span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnclosingExitEffect {
    ErrorPropagation(Option<SourceSpan>),
    OptionPropagation(Option<SourceSpan>),
    FunctionReturn(Option<SourceSpan>),
    ErrorReturn(Option<SourceSpan>),
    InferredFailure(Option<SourceSpan>),
}

impl EnclosingExitEffect {
    pub(crate) fn span(&self) -> Option<SourceSpan> {
        match self {
            Self::ErrorPropagation(span)
            | Self::OptionPropagation(span)
            | Self::FunctionReturn(span)
            | Self::ErrorReturn(span)
            | Self::InferredFailure(span) => *span,
        }
    }
}

/// Returns whether an assertion condition is the compiler's statically-known true case.
///
/// WHAT: identifies the only assertion form whose message is validated but never evaluated or
///       published into downstream executable/fact representations.
/// WHY: the same frontend-owned fact gates request publication and inactive message facts across
///      AST finalization, so later stages do not rediscover or filter it independently.
pub(crate) fn assertion_condition_is_statically_true(condition: &Expression) -> bool {
    matches!(&condition.kind, ExpressionKind::Bool(true))
}

/// Finds the first enclosing exit effect reachable from an assertion message.
pub(crate) fn classify_assertion_message_effect(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let mut state = TraversalState::default();
    classify_expression(message, template_ir_store, &mut state)
}

/// Collect pending semantic contributors through the same store-aware traversal used for
/// assertion messages. Statement-owned range checks join the expression-owned facts here.
pub(crate) fn pending_body_failure_facts(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let mut state = TraversalState {
        pending_facts: Some(ExpressionFailureFacts::default()),
        ..TraversalState::default()
    };
    classify_nodes(nodes, template_ir_store, &mut state)?;
    Ok(state.pending_facts.take().unwrap_or_default())
}

pub(crate) fn pending_expression_failure_facts(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let mut state = TraversalState {
        pending_facts: Some(ExpressionFailureFacts::default()),
        ..TraversalState::default()
    };
    classify_expression(expression, template_ir_store, &mut state)?;
    Ok(state.pending_facts.take().unwrap_or_default())
}

/// Function projection keeps failure-edge messages separate from escaping body work.
pub(crate) struct PendingFunctionFailureFacts {
    pub(crate) body: ExpressionFailureFacts,
    pub(crate) assertion_message_calls: Vec<ImplicitFailureContributor>,
}

pub(crate) fn pending_function_failure_facts(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
) -> Result<PendingFunctionFailureFacts, CompilerError> {
    let mut state = TraversalState {
        pending_facts: Some(ExpressionFailureFacts::default()),
        assertion_message_facts: Some(ExpressionFailureFacts::default()),
        ..TraversalState::default()
    };
    classify_nodes(nodes, template_ir_store, &mut state)?;
    let mut assertion_facts = state.assertion_message_facts.take().unwrap_or_default();
    assertion_facts.implicit.retain(|contributor| {
        matches!(contributor.source, ImplicitFailureSource::PrivateCall(_))
    });
    Ok(PendingFunctionFailureFacts {
        body: state.pending_facts.take().unwrap_or_default(),
        assertion_message_calls: assertion_facts.implicit,
    })
}

/// Builds the user-facing diagnostic for an assertion message that can escape.
pub(crate) fn assert_message_escape_diagnostic(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    Ok(
        classify_assertion_message_effect(message, template_ir_store)?.map(|effect| {
            CompilerDiagnostic::invalid_fallible_handling(
                InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
                effect.span(),
            )
        }),
    )
}

/// An explicit exit inside protected work still targets the function, never the catch.
pub(crate) fn explicit_propagation_catch_diagnostic(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    if expression.failure_facts.postfix_exit_span.is_none() {
        return Ok(None);
    }
    let mut state = TraversalState {
        explicit_exit_only: true,
        ..TraversalState::default()
    };
    let effect = classify_expression(expression, template_ir_store, &mut state)?;
    Ok(effect.map(|effect| {
        let reason = match effect {
            EnclosingExitEffect::OptionPropagation(_) => {
                InvalidFallibleHandlingReason::OptionPropagationCatchConflict
            }
            _ => InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict,
        };
        CompilerDiagnostic::invalid_fallible_handling(reason, effect.span())
    }))
}

/// Reject source recovery shapes before the invariant-only HIR lowering boundary.
/// This is a mode of the existing AST/runtime-handoff traversal, not another expression walker.
pub(crate) fn unsupported_catch_diagnostic(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    string_table: &mut StringTable,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let mut state = TraversalState {
        pending_facts: Some(ExpressionFailureFacts::default()),
        check_catch_support: true,
        ..TraversalState::default()
    };
    classify_nodes(nodes, template_ir_store, &mut state)?;
    Ok(state.unsupported_catch.map(|(name, span)| {
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::UnsupportedCatchExpressionShape {
                expression_name: string_table.intern(name),
            },
            span,
        )
    }))
}

fn catch_operand_has_unsupported_failure(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<bool, CompilerError> {
    let mut state = TraversalState {
        pending_facts: Some(ExpressionFailureFacts::default()),
        check_pending_failure: true,
        check_unsupported_failure: true,
        ..TraversalState::default()
    };
    classify_expression(expression, template_ir_store, &mut state)?;
    Ok(state.has_pending_failure)
}

fn protected_operands_have_unsupported_failure(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<bool, CompilerError> {
    match &expression.kind {
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
            for argument in args {
                if catch_operand_has_unsupported_failure(&argument.value, template_ir_store)? {
                    return Ok(true);
                }
            }
        }
        ExpressionKind::HandledFallibleExpression { value, .. } => {
            return protected_operands_have_unsupported_failure(value, template_ir_store);
        }
        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            if catch_operand_has_unsupported_failure(receiver, template_ir_store)? {
                return Ok(true);
            }
            for argument in args {
                if catch_operand_has_unsupported_failure(&argument.value, template_ir_store)? {
                    return Ok(true);
                }
            }
        }
        ExpressionKind::Cast(cast) => {
            return catch_operand_has_unsupported_failure(&cast.source, template_ir_store);
        }
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item
                    && catch_operand_has_unsupported_failure(operand, template_ir_store)?
                {
                    return Ok(true);
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

#[derive(Default)]
struct TraversalState {
    loop_depth: usize,
    visited_templates: HashSet<TemplateTirReference>,
    pending_facts: Option<ExpressionFailureFacts>,
    assertion_message_facts: Option<ExpressionFailureFacts>,
    protected_failure_depth: usize,
    check_catch_support: bool,
    unsupported_catch: Option<(&'static str, Option<SourceSpan>)>,
    check_pending_failure: bool,
    check_unsupported_failure: bool,
    has_pending_failure: bool,
    explicit_exit_only: bool,
}

fn classify_expression(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    if state.unsupported_catch.is_some() || state.has_pending_failure {
        return Ok(None);
    }
    let traverses_owned_payload = matches!(
        expression.kind,
        ExpressionKind::ValueBlock { .. }
            | ExpressionKind::Template(_)
            | ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_)
    );
    if let Some(facts) = &mut state.pending_facts {
        if !traverses_owned_payload && state.protected_failure_depth == 0 {
            if state.check_pending_failure {
                let facts = &expression.failure_facts;
                let pending_implicit = if state.check_unsupported_failure {
                    facts.disposition == FailureDisposition::Pending
                        && facts.implicit.iter().any(|contributor| {
                            matches!(contributor.source, ImplicitFailureSource::PrivateCall(_))
                        })
                } else {
                    facts.has_pending_implicit()
                };
                state.has_pending_failure = pending_implicit
                    || (facts.disposition == FailureDisposition::Pending
                        && !facts.typed_errors.is_empty());
            } else if !state.check_catch_support {
                facts.merge_pending_from(&expression.failure_facts);
            }
        }
    } else if !state.explicit_exit_only
        && state.protected_failure_depth == 0
        && expression.failure_facts.disposition == FailureDisposition::Pending
    {
        // Private-call candidates are resolved by summary convergence, not guessed here.
        let implicit = expression.failure_facts.implicit.iter().find(|contributor| {
            contributor.source == ImplicitFailureSource::NumericOperation
        });
        let span = implicit.map(|contributor| contributor.span).or_else(|| {
            expression.failure_facts.typed_errors.first().map(|producer| producer.span)
        });
        if let Some(span) = span {
            return Ok(Some(EnclosingExitEffect::InferredFailure(span)));
        }
    }

    let protects_failure = expression.failure_facts.disposition != FailureDisposition::Pending;
    if protects_failure {
        state.protected_failure_depth += 1;
    }
    let effect = match &expression.kind {
        ExpressionKind::HandledFallibleFunctionCall { args, handling, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, handling, .. } => {
            if state.pending_facts.is_none()
                && matches!(handling, FallibleExpressionHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers = matches!(handling, FallibleExpressionHandling::Recover);
            state.protected_failure_depth += usize::from(recovers);
            let effect = classify_call_arguments(args, template_ir_store, state);
            state.protected_failure_depth -= usize::from(recovers);
            effect
        }
        ExpressionKind::HandledFallibleExpression {
            value, handling, ..
        } => {
            if state.pending_facts.is_none()
                && matches!(handling, FallibleExpressionHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers = matches!(handling, FallibleExpressionHandling::Recover);
            state.protected_failure_depth += usize::from(recovers);
            let effect = classify_expression(value, template_ir_store, state);
            state.protected_failure_depth -= usize::from(recovers);
            effect
        }
        ExpressionKind::OptionPropagation { value } => {
            if state.pending_facts.is_some() {
                classify_expression(value, template_ir_store, state)
            } else {
                Ok(Some(EnclosingExitEffect::OptionPropagation(expression.span)))
            }
        }
        ExpressionKind::Cast(cast) => {
            // Only user-authored `cast!` is an expression-level error-propagation effect.
            // `StoreConversion` belongs to a statement assignment; its HIR error edge is not part
            // of an assertion-message expression.
            if state.pending_facts.is_none() && matches!(cast.handling, CastHandling::Propagate) {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(cast.span)));
            }
            let recovers = matches!(cast.handling, CastHandling::Recover);
            state.protected_failure_depth += usize::from(recovers);
            let effect = classify_expression(&cast.source, template_ir_store, state);
            state.protected_failure_depth -= usize::from(recovers);
            effect
        }
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item
                    && let Some(effect) = classify_expression(operand, template_ir_store, state)?
                {
                    return Ok(Some(effect));
                }
            }
            Ok(None)
        }
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => {
            classify_call_arguments(args, template_ir_store, state)
        }
        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            if let Some(effect) = classify_expression(receiver, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            classify_call_arguments(args, template_ir_store, state)
        }
        ExpressionKind::FieldAccess { base, .. } => {
            classify_expression(base, template_ir_store, state)
        }
        ExpressionKind::Copy(place) => classify_place(place),
        ExpressionKind::Collection(items) => classify_expressions(items, template_ir_store, state),
        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                if let Some(effect) = classify_expression(&entry.key, template_ir_store, state)? {
                    return Ok(Some(effect));
                }
                if let Some(effect) = classify_expression(&entry.value, template_ir_store, state)? {
                    return Ok(Some(effect));
                }
            }
            Ok(None)
        }
        ExpressionKind::StructInstance(fields)
        | ExpressionKind::AnonymousConstRecord { fields }
        | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                if let Some(effect) = classify_expression(&field.value, template_ir_store, state)? {
                    return Ok(Some(effect));
                }
            }
            Ok(None)
        }
        ExpressionKind::Range(start, end) => {
            if let Some(effect) = classify_expression(start, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            classify_expression(end, template_ir_store, state)
        }
        ExpressionKind::Coerced { value, .. } => {
            classify_expression(value, template_ir_store, state)
        }
        ExpressionKind::ValueBlock { block } => {
            classify_value_block(block, template_ir_store, state)
        }
        ExpressionKind::Template(template) => {
            if !state.visited_templates.insert(template.tir_reference) {
                return Ok(None);
            }

            let mut effect = None;
            walk_expression_payloads_with_nested_tir_views(
                expression,
                template_ir_store,
                &mut |nested_expression| {
                    if effect.is_none() {
                        effect = classify_expression(nested_expression, template_ir_store, state)?;
                    }
                    Ok(())
                },
            )?;
            Ok(effect)
        }
        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            classify_runtime_template_handoff(handoff, template_ir_store, state)
        }
        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            classify_runtime_slot_application_handoff(handoff, template_ir_store, state)
        }
        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::FixedScalar(_)
        | ExpressionKind::Number(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_)
        | ExpressionKind::Function(_)
        | ExpressionKind::StructDefinition(_) => Ok(None),
        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            classify_expression(value, template_ir_store, state)
        }
    };
    if protects_failure {
        state.protected_failure_depth -= 1;
    }
    effect
}

fn expression_propagation_span(expression: &Expression) -> Option<SourceSpan> {
    expression.propagation_span().or(expression.span)
}

fn classify_call_arguments(
    arguments: &[CallArgument],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for argument in arguments {
        if let Some(effect) = classify_expression(&argument.value, template_ir_store, state)? {
            return Ok(Some(effect));
        }
    }
    Ok(None)
}

fn classify_expressions(
    expressions: &[Expression],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for expression in expressions {
        if let Some(effect) = classify_expression(expression, template_ir_store, state)? {
            return Ok(Some(effect));
        }
    }
    Ok(None)
}

fn classify_place(place: &PlaceExpression) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match &place.kind {
        PlaceExpressionKind::Local(_) => Ok(None),
        PlaceExpressionKind::Field { base, .. } => classify_place(base),
    }
}

fn classify_value_block(
    block: &ValueBlock,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match block {
        ValueBlock::If(value_if) => {
            if let Some(effect) =
                classify_expression(&value_if.condition, template_ir_store, state)?
            {
                return Ok(Some(effect));
            }
            if let Some(effect) = classify_nodes(&value_if.then_body, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            classify_nodes(&value_if.else_body, template_ir_store, state)
        }
        ValueBlock::LexicalScope(value_lexical_scope) => {
            classify_nodes(&value_lexical_scope.body, template_ir_store, state)
        }
        ValueBlock::Match(value_match) => {
            if let Some(effect) =
                classify_expression(&value_match.scrutinee, template_ir_store, state)?
            {
                return Ok(Some(effect));
            }
            for arm in &value_match.arms {
                if let Some(effect) = classify_match_arm(arm, template_ir_store, state)? {
                    return Ok(Some(effect));
                }
            }
            match value_match.default.as_deref() {
                Some(default) => classify_nodes(default, template_ir_store, state),
                None => Ok(None),
            }
        }
        ValueBlock::Catch(value_catch) => {
            if state.check_catch_support {
                let protected = &value_catch.handled_value;
                let unsupported_name = match &protected.kind {
                    ExpressionKind::HandledFallibleFunctionCall { .. }
                    | ExpressionKind::HandledFallibleHostFunctionCall { .. }
                    | ExpressionKind::HandledFallibleExpression { .. }
                    | ExpressionKind::Cast(_)
                    | ExpressionKind::Runtime(_) => {
                        protected_operands_have_unsupported_failure(protected, template_ir_store)?
                            .then_some("an expression with unsupported fallible operands or arguments")
                    }
                    ExpressionKind::FunctionCall { .. } => Some("an inferred-failure private call"),
                    _ => Some("this protected expression"),
                };
                if let Some(name) = unsupported_name {
                    state.unsupported_catch.get_or_insert((name, protected.span));
                    return Ok(None);
                }
            }
            if state.pending_facts.is_none()
                && matches!(value_catch.handler, FallibleHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(&value_catch.handled_value),
                )));
            }
            if matches!(value_catch.handler, FallibleHandling::Propagate) {
                if let Some(effect) =
                    classify_expression(&value_catch.handled_value, template_ir_store, state)?
                {
                    return Ok(Some(effect));
                }
            } else if state.pending_facts.is_none() || state.check_catch_support {
                state.protected_failure_depth += 1;
                let effect = classify_expression(
                    &value_catch.handled_value, template_ir_store, state,
                );
                state.protected_failure_depth -= 1;
                if let Some(effect) = effect? {
                    return Ok(Some(effect));
                }
            }
            if let FallibleHandling::Handler { body, .. } = &value_catch.handler {
                return classify_nodes(body, template_ir_store, state);
            }
            Ok(None)
        }
    }
}

fn classify_match_arm(
    arm: &MatchArm,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    if let Some(guard) = &arm.guard
        && let Some(effect) = classify_expression(guard, template_ir_store, state)?
    {
        return Ok(Some(effect));
    }
    classify_nodes(&arm.body, template_ir_store, state)
}

fn classify_declarations(
    declarations: &[Declaration],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for declaration in declarations {
        if let Some(effect) = classify_expression(&declaration.value, template_ir_store, state)? {
            return Ok(Some(effect));
        }
    }
    Ok(None)
}

fn classify_nodes(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for node in nodes {
        if state.unsupported_catch.is_some() || state.has_pending_failure {
            return Ok(None);
        }
        let effect = match &node.kind {
            NodeKind::Return(values) if state.pending_facts.is_some() => {
                classify_expressions(values, template_ir_store, state)?
            }
            NodeKind::ReturnError(value) if state.pending_facts.is_some() => {
                classify_expression(value, template_ir_store, state)?
            }
            NodeKind::Break | NodeKind::Continue if state.pending_facts.is_some() => None,
            NodeKind::Return(_) => Some(EnclosingExitEffect::FunctionReturn(node.span)),
            NodeKind::ReturnError(_) => Some(EnclosingExitEffect::ErrorReturn(node.span)),
            // A loop-local control transfer cannot escape the assertion message's enclosing
            // function. Valid ASTs only contain these nodes under an owning loop.
            NodeKind::Break if state.loop_depth > 0 => None,
            NodeKind::Continue if state.loop_depth > 0 => None,
            // Assertion call arguments reject value-producing blocks, so a depth-zero loop
            // control node would indicate a broken AST boundary rather than source syntax
            // that can target an ordinary loop surrounding the assertion.
            NodeKind::Break => {
                return Err(CompilerError::compiler_error(
                    "Assertion-message AST invariant: depth-zero `break` cannot appear because assertion call arguments reject value-producing blocks.",
                ));
            }
            NodeKind::Continue => {
                return Err(CompilerError::compiler_error(
                    "Assertion-message AST invariant: depth-zero `continue` cannot appear because assertion call arguments reject value-producing blocks.",
                ));
            }
            NodeKind::If(condition, then_body, else_body, _) => {
                classify_expression(condition, template_ir_store, state)?
                    .or(classify_nodes(then_body, template_ir_store, state)?)
                    .or(match else_body.as_deref() {
                        Some(body) => classify_nodes(body, template_ir_store, state)?,
                        None => None,
                    })
            }
            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                let mut effect = classify_expression(scrutinee, template_ir_store, state)?;
                if effect.is_none() {
                    for arm in arms {
                        effect = classify_match_arm(arm, template_ir_store, state)?;
                        if effect.is_some() {
                            break;
                        }
                    }
                }
                if effect.is_none()
                    && let Some(default) = default.as_deref()
                {
                    effect = classify_nodes(default, template_ir_store, state)?;
                }
                effect
            }
            NodeKind::LexicalScope { body } => classify_nodes(body, template_ir_store, state)?,
            NodeKind::RangeLoop { range, body, .. } => {
                if let Some(effect) = classify_range_failure(
                    range.start.type_id, node.span, state,
                ) {
                    return Ok(Some(effect));
                }
                let mut effect = classify_expression(&range.start, template_ir_store, state)?;
                if effect.is_none() {
                    effect = classify_expression(&range.end, template_ir_store, state)?;
                }
                if effect.is_none()
                    && let Some(step) = &range.step
                {
                    effect = classify_expression(step, template_ir_store, state)?;
                }
                if effect.is_none() {
                    effect = classify_loop_body(body, template_ir_store, state)?;
                }
                effect
            }
            NodeKind::CollectionLoop { iterable, body, .. } => classify_expression(
                iterable,
                template_ir_store,
                state,
            )?
            .or(classify_loop_body(body, template_ir_store, state)?),
            NodeKind::WhileLoop(condition, body) => classify_expression(
                condition,
                template_ir_store,
                state,
            )?
            .or(classify_loop_body(body, template_ir_store, state)?),
            NodeKind::VariableDeclaration(Declaration { value, .. })
            | NodeKind::ExpressionStatement(value)
            | NodeKind::PushStartRuntimeFragment(value) => {
                classify_expression(value, template_ir_store, state)?
            }
            NodeKind::Assignment { value, .. } | NodeKind::MultiBind { value, .. } => {
                classify_expression(value, template_ir_store, state)?
            }
            NodeKind::Assert { condition, message } => {
                let effect = classify_expression(condition, template_ir_store, state)?;
                if state.check_catch_support {
                    classify_expression(message, template_ir_store, state)?;
                }
                if state.pending_facts.is_some() {
                    if state.assertion_message_facts.is_some() {
                        let message_facts =
                            pending_expression_failure_facts(message, template_ir_store)?;
                        if let Some(facts) = &mut state.assertion_message_facts {
                            facts.merge_pending_from(&message_facts);
                        }
                    }
                    effect
                } else {
                    effect.or(classify_expression(message, template_ir_store, state)?)
                }
            }
            // Nested function returns belong to that function and cannot escape this message.
            NodeKind::Function(_, _, body) if state.check_catch_support => {
                classify_nodes(body, template_ir_store, state)?
            }
            NodeKind::Function(_, _, _) => None,
            NodeKind::StructDefinition(_, fields) => {
                classify_declarations(fields, template_ir_store, state)?
            }
            NodeKind::ThenValue(values) => {
                classify_expressions(&values.expressions, template_ir_store, state)?
            }
        };
        if effect.is_some() {
            return Ok(effect);
        }
    }
    Ok(None)
}

fn classify_loop_body(
    body: &[AstNode],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    state.loop_depth += 1;
    let result = classify_nodes(body, template_ir_store, state);
    state.loop_depth -= 1;
    result
}

fn classify_runtime_template_handoff(
    handoff: &OwnedRuntimeTemplateHandoff,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let mut effect = None;
    let mut visit = |node: &OwnedRuntimeTemplateNode| {
        if effect.is_none() {
            effect = classify_owned_runtime_node(node, template_ir_store, state)?;
        }
        Ok(())
    };
    walk_owned_runtime_template_handoff(handoff, &mut visit)?;
    Ok(effect)
}

fn classify_runtime_slot_application_handoff(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let mut effect = None;
    let mut visit = |node: &OwnedRuntimeTemplateNode| {
        if effect.is_none() {
            effect = classify_owned_runtime_node(node, template_ir_store, state)?;
        }
        Ok(())
    };
    walk_owned_runtime_slot_application_handoff(handoff, &mut visit)?;
    Ok(effect)
}

fn classify_owned_runtime_node(
    node: &OwnedRuntimeTemplateNode,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            Ok(classify_expression(expression, template_ir_store, state)?)
        }
        OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
            let mut branch_effect = None;
            for branch in branches {
                branch_effect =
                    classify_branch_selector(&branch.selector, template_ir_store, state)?;
                if branch_effect.is_some() {
                    break;
                }
            }
            Ok(branch_effect)
        }
        OwnedRuntimeTemplateNode::Loop { header, .. } => {
            Ok(classify_loop_header(header, template_ir_store, state)?)
        }

        // Structural nodes are traversed recursively by the canonical owned-handoff walker.
        // Naming them here keeps this classifier exhaustive without duplicating that recursion.
        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => Ok(None),
    }
}

fn classify_branch_selector(
    selector: &TemplateBranchSelector,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            classify_expression(condition, template_ir_store, state)
        }
        TemplateBranchSelector::OptionPresentCapture {
            scrutinee, pattern, ..
        } => {
            if let Some(effect) = classify_expression(scrutinee, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            match pattern.as_ref() {
                MatchPattern::Literal(expression)
                | MatchPattern::OptionValue {
                    value: expression, ..
                }
                | MatchPattern::Relational {
                    value: expression, ..
                } => classify_expression(expression, template_ir_store, state),
                MatchPattern::OptionNone { .. }
                | MatchPattern::OptionPresentCapture { .. }
                | MatchPattern::ChoiceVariant { .. } => Ok(None),
            }
        }
    }
}

fn classify_loop_header(
    header: &TemplateLoopHeader,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            classify_expression(condition, template_ir_store, state)
        }
        TemplateLoopHeader::Range { range, .. } => {
            let span = range.step.as_ref().and_then(|step| step.span).or(range.start.span);
            if let Some(effect) = classify_range_failure(range.start.type_id, span, state) {
                return Ok(Some(effect));
            }
            if let Some(effect) = classify_expression(&range.start, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            if let Some(effect) = classify_expression(&range.end, template_ir_store, state)? {
                return Ok(Some(effect));
            }
            match &range.step {
                Some(step) => classify_expression(step, template_ir_store, state),
                None => Ok(None),
            }
        }
        TemplateLoopHeader::Collection { iterable, .. } => {
            classify_expression(iterable, template_ir_store, state)
        }
    }
}

fn classify_range_failure(
    type_id: TypeId,
    span: Option<SourceSpan>,
    state: &mut TraversalState,
) -> Option<EnclosingExitEffect> {
    if state.explicit_exit_only || state.check_catch_support || state.protected_failure_depth > 0 {
        return None;
    }
    if state.check_pending_failure {
        state.has_pending_failure = true;
    } else if let Some(facts) = &mut state.pending_facts {
        let binary_float = type_id == builtin_type_ids::FLOAT
            || [FixedScalar::F16, FixedScalar::F32, FixedScalar::F64]
                .into_iter()
                .any(|scalar| type_id == builtin_type_ids::fixed_scalar(scalar));
        let codes = if binary_float {
            vec![BuiltinErrorCode::FloatNonFinite]
        } else {
            vec![BuiltinErrorCode::IntOverflow]
        };
        facts.implicit.push(ImplicitFailureContributor {
            span,
            codes,
            source: ImplicitFailureSource::NumericOperation,
        });
    } else {
        return Some(EnclosingExitEffect::InferredFailure(span));
    }
    None
}
