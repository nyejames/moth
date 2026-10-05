//! AST-owned failure classification and projection.
//!
//! Expressions retain origin witnesses and compact summaries. This owner walks evaluated AST,
//! effective TIR payloads and owned runtime handoffs, scopes catch protection, and projects each
//! escaping origin once. Assertion-message rejection policy remains in assertion_message_effects.

use std::collections::{HashMap, HashSet};

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, FailureSummary, ImplicitFailureContributor,
    ImplicitFailureContributorSummary, ImplicitFailureSource, WitnessCandidates,
    retain_bounded_witness_hops, select_failure_witness,
};
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
    TemplateIrStore, TemplateTirReference, TirView, walk_expression_payloads_with_nested_tir_views,
    walk_tir_view_expression_payloads,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, CompilerDiagnostic,
    InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// A control-flow effect targeting the current enclosing function.
///
/// The operator's authored span remains separate from the call/value span. Catch conflicts and
/// assertion policy consume these effects without reconstructing their source location.
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

/// Selects the enclosing exits relevant to the caller's semantic rule.
#[derive(Clone, Copy)]
pub(crate) enum ExitClassification {
    AssertionMessage,
    ExplicitPropagation,
}

pub(crate) fn classify_enclosing_exit_effect(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    classification: ExitClassification,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let purpose = match classification {
        ExitClassification::AssertionMessage => TraversalPurpose::AssertionMessage,
        ExitClassification::ExplicitPropagation => TraversalPurpose::ExplicitPropagation,
    };
    classify_expression(
        expression,
        template_ir_store,
        &mut TraversalState::new(purpose),
    )
}

pub(crate) fn pending_body_failure_facts(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let mut state = TraversalState::new(TraversalPurpose::PendingSummary(
        ExpressionFailureFacts::default(),
    ));
    classify_nodes(nodes, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::PendingSummary(facts) = state.purpose else {
        unreachable!("pending-summary traversal retains its purpose");
    };
    Ok(facts)
}

pub(crate) fn pending_expression_failure_facts(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let mut state = TraversalState::new(TraversalPurpose::PendingSummary(
        ExpressionFailureFacts::default(),
    ));
    classify_expression(expression, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::PendingSummary(facts) = state.purpose else {
        unreachable!("pending-summary traversal retains its purpose");
    };
    Ok(facts)
}

/// Flat owning projection: no expression summary contains descendant witness lists.
#[derive(Default)]
pub(crate) struct PendingFailureFacts {
    pub(crate) summary: ExpressionFailureFacts,
    pub(crate) implicit: Vec<ImplicitFailureContributor>,
    pub(crate) deferred_custom_catches: Vec<CollectedDeferredCustomCatchCheck>,
}

pub(crate) struct CollectedDeferredCustomCatchCheck {
    pub(crate) catch_span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    pub(crate) typed_producer_span: Option<SourceSpan>,
    pub(crate) candidates: Vec<ImplicitFailureContributor>,
    pub(crate) eligibility_only: bool,
}

#[derive(Default)]
pub(crate) struct PendingFunctionFailureFacts {
    pub(crate) body: PendingFailureFacts,
    pub(crate) assertion_message_calls: Vec<ImplicitFailureContributor>,
}

pub(crate) fn collect_expression_failure_facts(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<PendingFailureFacts, CompilerError> {
    let mut state = TraversalState::new(TraversalPurpose::ExpressionProjection(
        PendingFailureFacts::default(),
    ));
    classify_expression(expression, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::ExpressionProjection(facts) = state.purpose else {
        unreachable!("expression projection retains its purpose");
    };
    Ok(facts)
}

pub(crate) fn pending_function_failure_facts(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
) -> Result<PendingFunctionFailureFacts, CompilerError> {
    let mut state = TraversalState::new(TraversalPurpose::FunctionProjection(
        PendingFunctionFailureFacts::default(),
    ));
    classify_nodes(nodes, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::FunctionProjection(facts) = state.purpose else {
        unreachable!("function projection retains its purpose");
    };
    Ok(facts)
}

/// Closed AST-local verdicts for retained typed bodies, never public concrete call summaries.
///
/// All known bodies start at the bottom of the failure lattice. Numeric origins and unavailable
/// callees activate edges monotonically, so declaration order and origin-free recursion do not
/// manufacture failure. Dormant templates resolve catch eligibility and compatibility here.
pub(crate) struct AstBuiltinFailureSummaries {
    functions: HashMap<PathId, PendingFunctionFailureFacts>,
    active: HashSet<PathId>,
}

impl AstBuiltinFailureSummaries {
    pub(crate) fn compute(
        node_groups: &[&[AstNode]],
        template_ir_store: &TemplateIrStore,
    ) -> Result<Self, CompilerError> {
        let mut functions = HashMap::new();
        for nodes in node_groups {
            for node in *nodes {
                if let NodeKind::Function(path, _, body) = &node.kind {
                    functions.insert(
                        *path,
                        pending_function_failure_facts(body, template_ir_store)?,
                    );
                }
            }
        }
        let mut summaries = Self {
            functions,
            active: HashSet::new(),
        };
        loop {
            let mut changed = false;
            for (path, facts) in &summaries.functions {
                if !summaries.active.contains(path)
                    && facts
                        .body
                        .implicit
                        .iter()
                        .any(|contributor| summaries.is_active(contributor))
                {
                    summaries.active.insert(*path);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Ok(summaries)
    }

    pub(crate) fn function_facts(&self, path: PathId) -> Option<&PendingFunctionFailureFacts> {
        self.functions.get(&path)
    }

    pub(crate) fn is_active(&self, contributor: &ImplicitFailureContributor) -> bool {
        match contributor.source {
            // Both numeric shapes carry their shared code set; an empty set is an exact,
            // provably infallible operation that never activates a boundary or a witness.
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::CompoundWriteBack { .. } => !contributor.codes.is_empty(),
            ImplicitFailureSource::PrivateCall(path) => self.call_may_fail(path),
        }
    }

    fn call_may_fail(&self, path: PathId) -> bool {
        !self.functions.contains_key(&path) || self.active.contains(&path)
    }

    pub(crate) fn first_active<'a>(
        &self,
        contributors: &'a [ImplicitFailureContributor],
    ) -> Option<&'a ImplicitFailureContributor> {
        select_failure_witness(
            contributors
                .iter()
                .filter(|contributor| self.is_active(contributor))
                .map(Ok::<_, std::convert::Infallible>),
            ImplicitFailureContributor::witness_site,
        )
        .unwrap_or_else(|never| match never {})
    }

    pub(crate) fn witness(
        &self,
        contributor: &ImplicitFailureContributor,
    ) -> BuiltinFailureWitness {
        self.search_witness(contributor, &mut HashSet::new())
    }

    /// Depth-first witness search with an explicit stack of entered bodies.
    ///
    /// WHAT: follows each entered body's candidates in policy order. A numeric or write-back
    ///       candidate is the origin. A callee without retained facts is an unavailable leaf:
    ///       its hop stays the visible boundary and the search stops without an origin beyond
    ///       it. A body that runs out of candidates is popped with its hop.
    /// WHY: a recursive call can precede the real producer in source order, so a call into an
    ///      already-entered body is skipped rather than ending the search. An entered body is
    ///      either on the current path (a back-edge) or already exhausted. An exhausted body has
    ///      no origin or leaf reachable without passing through bodies still on the path, so
    ///      re-entering it can never change the witness. Each body is therefore expanded at
    ///      most once per search, recorded in the caller's empty `entered` set.
    fn search_witness(
        &self,
        root: &ImplicitFailureContributor,
        entered: &mut HashSet<PathId>,
    ) -> BuiltinFailureWitness {
        let mut witness = BuiltinFailureWitness {
            codes: Vec::new(),
            call_spans: Vec::new(),
            origin_span: None,
            origin: BuiltinFailureOriginKind::Operation,
            elided_call_hops: 0,
        };
        let mut stack: Vec<WitnessCandidates<'_, ImplicitFailureContributor>> = Vec::new();
        let mut hops: Vec<Option<SourceSpan>> = Vec::new();
        let mut next = Some(root);
        loop {
            if let Some(contributor) = next.take() {
                match contributor.source {
                    ImplicitFailureSource::NumericOperation => {
                        witness.codes.extend_from_slice(contributor.codes);
                        witness.origin_span = contributor.span;
                        witness.origin = BuiltinFailureOriginKind::Operation;
                        break;
                    }
                    ImplicitFailureSource::CompoundWriteBack { target, .. } => {
                        witness.codes.extend_from_slice(contributor.codes);
                        witness.origin_span = contributor.span;
                        witness.origin = BuiltinFailureOriginKind::CompoundWriteBack { target };
                        break;
                    }
                    ImplicitFailureSource::PrivateCall(callee) => {
                        let Some(facts) = self.functions.get(&callee) else {
                            hops.push(contributor.span);
                            break;
                        };
                        if entered.insert(callee) {
                            hops.push(contributor.span);
                            stack.push(WitnessCandidates::new(&facts.body.implicit));
                        }
                    }
                }
            }

            let Some(candidates) = stack.last_mut() else {
                break;
            };
            next = candidates
                .next_candidate(
                    |candidate| Ok::<_, std::convert::Infallible>(self.is_active(candidate)),
                    ImplicitFailureContributor::witness_site,
                )
                .unwrap_or_else(|never| match never {});
            if next.is_none() {
                stack.pop();
                hops.pop();
            }
        }

        retain_bounded_witness_hops(&mut witness, hops);
        witness
    }

    /// Witness search that also reports how many bodies it expanded.
    #[cfg(test)]
    pub(crate) fn witness_with_expanded_bodies(
        &self,
        contributor: &ImplicitFailureContributor,
    ) -> (BuiltinFailureWitness, usize) {
        let mut entered = HashSet::new();
        let witness = self.search_witness(contributor, &mut entered);
        (witness, entered.len())
    }
}

/// An explicit exit inside protected work still targets the function, never the catch.
pub(crate) fn explicit_propagation_catch_diagnostic(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    if expression.failure_facts.postfix_exit_span.is_none() {
        return Ok(None);
    }
    let effect = classify_enclosing_exit_effect(
        expression,
        template_ir_store,
        ExitClassification::ExplicitPropagation,
    )?;
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

enum TraversalPurpose {
    AssertionMessage,
    ExplicitPropagation,
    PendingSummary(ExpressionFailureFacts),
    ExpressionProjection(PendingFailureFacts),
    FunctionProjection(PendingFunctionFailureFacts),
    PrivateCallCandidates(Vec<ImplicitFailureContributor>),
}

struct TraversalState {
    purpose: TraversalPurpose,
    loop_depth: usize,
    visited_templates: HashSet<TemplateTirReference>,
    protected_failure_depth: usize,
}

impl TraversalState {
    fn new(purpose: TraversalPurpose) -> Self {
        Self {
            purpose,
            loop_depth: 0,
            visited_templates: HashSet::new(),
            protected_failure_depth: 0,
        }
    }

    fn classifies_exits(&self) -> bool {
        matches!(
            self.purpose,
            TraversalPurpose::AssertionMessage | TraversalPurpose::ExplicitPropagation
        )
    }

    fn summary_mut(&mut self) -> Option<&mut ExpressionFailureFacts> {
        match &mut self.purpose {
            TraversalPurpose::PendingSummary(facts) => Some(facts),
            TraversalPurpose::ExpressionProjection(facts) => Some(&mut facts.summary),
            TraversalPurpose::FunctionProjection(facts) => Some(&mut facts.body.summary),
            _ => None,
        }
    }

    fn projection_mut(&mut self) -> Option<&mut PendingFailureFacts> {
        match &mut self.purpose {
            TraversalPurpose::ExpressionProjection(facts) => Some(facts),
            TraversalPurpose::FunctionProjection(facts) => Some(&mut facts.body),
            _ => None,
        }
    }

    fn record_origin(&mut self, contributor: &ImplicitFailureContributor) {
        match &mut self.purpose {
            TraversalPurpose::ExpressionProjection(facts) => facts.implicit.push(*contributor),
            TraversalPurpose::FunctionProjection(facts) => facts.body.implicit.push(*contributor),
            TraversalPurpose::PrivateCallCandidates(candidates)
                if matches!(contributor.source, ImplicitFailureSource::PrivateCall(_)) =>
            {
                candidates.push(*contributor);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum RootProtection {
    Respect,
    Ignore,
}

fn collect_deferred_check(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<(), CompilerError> {
    if state.projection_mut().is_none() {
        return Ok(());
    }
    let facts = &expression.failure_facts;
    // Private-call candidates permit parsing, but only an active exact summary makes a
    // private-only catch eligible. Authored numeric checks remain eligible after folding.
    let eligibility_only = matches!(facts.disposition, FailureDisposition::HandledByCatch { .. })
        && !facts.checked_numeric_operation
        && facts.summary.first_numeric.is_none()
        && facts.summary.first_typed.is_none()
        && facts.summary.first_private_call.is_some();
    if facts.deferred_custom_catch.is_none() && !eligibility_only {
        return Ok(());
    }

    // Ignore only this protected root's handling. Nested catches still exclude their origins.
    let mut candidates = TraversalState::new(TraversalPurpose::PrivateCallCandidates(Vec::new()));
    classify_expression_with_root_protection(
        expression,
        template_ir_store,
        &mut candidates,
        RootProtection::Ignore,
    )?;

    // The scoped candidate job cannot turn into a projection or collect another deferred check.
    let TraversalPurpose::PrivateCallCandidates(candidates) = candidates.purpose else {
        unreachable!("candidate traversal retains its purpose");
    };
    if let Some(projection) = state.projection_mut() {
        projection
            .deferred_custom_catches
            .push(CollectedDeferredCustomCatchCheck {
                catch_span: facts
                    .deferred_custom_catch
                    .map_or(expression.span, |check| check.catch_span),
                error_type_id: facts.deferred_custom_catch.map_or_else(
                    || match facts.disposition {
                        FailureDisposition::HandledByCatch { error_type_id } => error_type_id,
                        FailureDisposition::Pending => {
                            unreachable!("eligibility check owns a catch")
                        }
                    },
                    |check| check.error_type_id,
                ),
                typed_producer_span: facts
                    .deferred_custom_catch
                    .and_then(|check| check.typed_producer_span),
                eligibility_only,
                candidates,
            });
    }
    Ok(())
}

fn classify_expression(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    classify_expression_with_root_protection(
        expression,
        template_ir_store,
        state,
        RootProtection::Respect,
    )
}

fn classify_expression_with_root_protection(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
    root_protection: RootProtection,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let ignores_root = matches!(root_protection, RootProtection::Ignore);
    let pending_root =
        ignores_root || expression.failure_facts.disposition == FailureDisposition::Pending;
    let owns_payload = matches!(
        expression.kind,
        ExpressionKind::ValueBlock { .. }
            | ExpressionKind::Template(_)
            | ExpressionKind::RuntimeTemplateHandoff(_)
            | ExpressionKind::RuntimeSlotApplicationHandoff(_)
    );
    if state.protected_failure_depth == 0 && pending_root {
        if let TraversalPurpose::AssertionMessage = state.purpose {
            // Private candidates wait for exact callee convergence. Numeric failure takes
            // precedence over typed failure, matching the retained aggregate diagnostics.
            let facts = &expression.failure_facts.summary;
            let span = facts
                .first_numeric
                .as_ref()
                .map(|witness| witness.span)
                .or_else(|| facts.first_typed.as_ref().map(|producer| producer.span));
            if let Some(span) = span {
                return Ok(Some(EnclosingExitEffect::InferredFailure(span)));
            }
        }
        if !owns_payload && let Some(facts) = state.summary_mut() {
            facts.merge_pending_from(&expression.failure_facts);
        }
    }

    let protects_failure = !pending_root;
    if protects_failure {
        state.protected_failure_depth += 1;
    }
    let effect = match &expression.kind {
        ExpressionKind::HandledFallibleFunctionCall { args, handling, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, handling, .. } => {
            if state.classifies_exits() && matches!(handling, FallibleExpressionHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers = !ignores_root && matches!(handling, FallibleExpressionHandling::Recover);
            state.protected_failure_depth += usize::from(recovers);
            let effect = classify_call_arguments(args, template_ir_store, state);
            state.protected_failure_depth -= usize::from(recovers);
            effect
        }
        ExpressionKind::HandledFallibleExpression {
            value, handling, ..
        } => {
            if state.classifies_exits() && matches!(handling, FallibleExpressionHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers = !ignores_root && matches!(handling, FallibleExpressionHandling::Recover);
            state.protected_failure_depth += usize::from(recovers);
            let effect = classify_expression(value, template_ir_store, state);
            state.protected_failure_depth -= usize::from(recovers);
            effect
        }
        ExpressionKind::OptionPropagation { value } => {
            if !state.classifies_exits() {
                classify_expression(value, template_ir_store, state)
            } else {
                Ok(Some(EnclosingExitEffect::OptionPropagation(
                    expression.span,
                )))
            }
        }
        ExpressionKind::Cast(cast) => {
            // Only user-authored `cast!` is an expression-level error-propagation effect.
            // `StoreConversion` belongs to a statement assignment; its HIR error edge is not part
            // of an assertion-message expression.
            if state.classifies_exits() && matches!(cast.handling, CastHandling::Propagate) {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(cast.span)));
            }
            let recovers = !ignores_root && matches!(cast.handling, CastHandling::Recover);
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
            let classifies_exits = state.classifies_exits();
            let mut visit = |nested_expression: &Expression| {
                if effect.is_none() {
                    effect = classify_expression(nested_expression, template_ir_store, state)?;
                }
                Ok(())
            };
            if classifies_exits {
                walk_expression_payloads_with_nested_tir_views(
                    expression,
                    template_ir_store,
                    &mut visit,
                )?;
            } else {
                // Projection needs the wrapper origins that the predicate-oriented nested walker
                // intentionally unwraps. Structural recursion still belongs to canonical TIR.
                let reference = template.tir_reference;
                let view = TirView::new(
                    template_ir_store,
                    reference.root,
                    reference.phase,
                    reference.context,
                )?;
                walk_tir_view_expression_payloads(&view, &mut visit)?;
            }
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

    // Children precede their call/operator origin. This is the old aggregate's witness order,
    // without copying a descendant into each of its ancestors.
    if state.protected_failure_depth == 0 && pending_root {
        for contributor in &expression.failure_facts.implicit {
            state.record_origin(contributor);
        }
    }
    collect_deferred_check(expression, template_ir_store, state)?;
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
            if matches!(
                state.purpose,
                TraversalPurpose::ExpressionProjection(_) | TraversalPurpose::FunctionProjection(_)
            ) && let FallibleHandling::Handler { body, .. } = &value_catch.handler
            {
                // Catch completion historically published handler checks before protected checks.
                // Protected work contributes no escaping origins, so visiting it second preserves
                // that diagnostic order without changing the escaping contributor order.
                classify_catch_handler(body, &value_catch.handled_value, template_ir_store, state)?;

                state.protected_failure_depth += 1;
                let effect =
                    classify_expression(&value_catch.handled_value, template_ir_store, state);
                state.protected_failure_depth -= 1;
                return effect;
            }

            if state.classifies_exits()
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
            } else {
                state.protected_failure_depth += 1;
                let effect =
                    classify_expression(&value_catch.handled_value, template_ir_store, state);
                state.protected_failure_depth -= 1;
                if let Some(effect) = effect? {
                    return Ok(Some(effect));
                }
            }
            if let FallibleHandling::Handler { body, .. } = &value_catch.handler {
                return classify_catch_handler(
                    body,
                    &value_catch.handled_value,
                    template_ir_store,
                    state,
                );
            }
            Ok(None)
        }
    }
}

fn classify_catch_handler(
    body: &[AstNode],
    protected: &Expression,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let dead_handler =
        !state.classifies_exits() && protected.failure_facts.is_folded_numeric_catch_success();
    state.protected_failure_depth += usize::from(dead_handler);
    let effect = classify_nodes(body, template_ir_store, state);
    state.protected_failure_depth -= usize::from(dead_handler);
    effect
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
        let effect = match &node.kind {
            NodeKind::Return(values) if !state.classifies_exits() => {
                classify_expressions(values, template_ir_store, state)?
            }
            NodeKind::ReturnError(value) if !state.classifies_exits() => {
                classify_expression(value, template_ir_store, state)?
            }
            NodeKind::Break | NodeKind::Continue if !state.classifies_exits() => None,
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
                if let Some(effect) = classify_range_failure(range.start.type_id, node.span, state)
                {
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
                match &mut state.purpose {
                    TraversalPurpose::FunctionProjection(facts) => {
                        let message_facts =
                            collect_expression_failure_facts(message, template_ir_store)?;
                        facts.assertion_message_calls.extend(
                            message_facts.implicit.into_iter().filter(|contributor| {
                                matches!(contributor.source, ImplicitFailureSource::PrivateCall(_))
                            }),
                        );
                    }
                    TraversalPurpose::AssertionMessage | TraversalPurpose::ExplicitPropagation => {
                        return Ok(effect.or(classify_expression(
                            message,
                            template_ir_store,
                            state,
                        )?));
                    }
                    _ => {}
                }
                effect
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
            let span = range
                .step
                .as_ref()
                .and_then(|step| step.span)
                .or(range.start.span);
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
    if state.protected_failure_depth > 0 {
        return None;
    }
    match &mut state.purpose {
        TraversalPurpose::AssertionMessage => {
            return Some(EnclosingExitEffect::InferredFailure(span));
        }
        TraversalPurpose::ExplicitPropagation | TraversalPurpose::PrivateCallCandidates(_) => {
            return None;
        }
        _ => {}
    }

    let binary_float = type_id == builtin_type_ids::FLOAT
        || [FixedScalar::F16, FixedScalar::F32, FixedScalar::F64]
            .into_iter()
            .any(|scalar| type_id == builtin_type_ids::fixed_scalar(scalar));
    // Range loops fail on overflow plus their step guards: zero step (306) in every domain and
    // no progress (307) when floating rounding leaves the candidate unchanged.
    let codes: &'static [BuiltinErrorCode] = if binary_float {
        &[
            BuiltinErrorCode::FloatNonFinite,
            BuiltinErrorCode::InvalidRangeStep,
            BuiltinErrorCode::RangeStepNoProgress,
        ]
    } else {
        &[
            BuiltinErrorCode::IntOverflow,
            BuiltinErrorCode::InvalidRangeStep,
        ]
    };
    let contributor = ImplicitFailureContributor {
        span,
        codes,
        source: ImplicitFailureSource::NumericOperation,
    };
    if let Some(facts) = state.summary_mut() {
        let witness = Some(ImplicitFailureContributorSummary { span });
        facts.summary.merge_from(&FailureSummary {
            first_implicit: witness,
            first_numeric: witness,
            ..FailureSummary::default()
        });
    }
    state.record_origin(&contributor);
    None
}
