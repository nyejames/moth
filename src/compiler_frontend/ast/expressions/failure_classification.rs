//! AST-owned failure classification and projection.
//!
//! Expressions retain origin witnesses and compact summaries. This owner walks evaluated AST,
//! effective TIR payloads and owned runtime handoffs, scopes catch protection and projects each
//! escaping origin once. Assertion-message rejection policy remains in assertion_message_effects.
//!
//! Expression walks reuse heap scratch between roots, so a long operand chain does not consume
//! thread stack. `Early` child results bypass parent exit actions and retain their protection. `Tail`
//! results run those actions before returning. Statement, block and template walkers use
//! recursion bounded by block and template nesting.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionStore, ExpressionView,
    PlaceProjectionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::{
    ExpressionFailureFacts, FailureDisposition, FailureSummary, ImplicitFailureContributor,
    ImplicitFailureContributorSummary, ImplicitFailureSource, WitnessCandidates,
    retain_bounded_witness_hops,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
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
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

/// A classifier binds roots in their issuing phase or frozen AST owner. TIR is only
/// readable in the phase case; frozen payloads have already become neutral handoffs.
#[derive(Clone, Copy)]
pub(crate) enum FailureClassificationExpressions<'a> {
    Building(&'a AstExpressionBuilder),
    Frozen(&'a ExpressionStore),
}

impl<'a> From<&'a AstExpressionBuilder> for FailureClassificationExpressions<'a> {
    fn from(owner: &'a AstExpressionBuilder) -> Self {
        Self::Building(owner)
    }
}

impl<'a> From<&'a ExpressionStore> for FailureClassificationExpressions<'a> {
    fn from(owner: &'a ExpressionStore) -> Self {
        Self::Frozen(owner)
    }
}

impl<'a> FailureClassificationExpressions<'a> {
    fn view(self, id: ExprId) -> Result<ExpressionView<'a>, CompilerError> {
        match self {
            Self::Building(owner) => owner.view(id),
            Self::Frozen(owner) => owner.view(id),
        }
    }

    fn validate_view(self, view: ExpressionView<'_>) -> Result<(), CompilerError> {
        let owner = match self {
            Self::Building(owner) => owner.store(),
            Self::Frozen(owner) => owner,
        };
        if owner.owner_token() != view.owner_token() {
            return Err(CompilerError::compiler_error(
                "Failure classification received an expression from another owner.",
            ));
        }
        Ok(())
    }

    fn builder(self) -> Result<&'a AstExpressionBuilder, CompilerError> {
        match self {
            Self::Building(owner) => Ok(owner),
            Self::Frozen(_) => Err(CompilerError::compiler_error(
                "Build-only Template reached frozen failure classification.",
            )),
        }
    }
}

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

pub(crate) fn classify_enclosing_exit_effect<'a>(
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    classification: ExitClassification,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let expression_builder = expression_builder.into();
    expression_builder.validate_view(expression)?;
    let purpose = match classification {
        ExitClassification::AssertionMessage => TraversalPurpose::AssertionMessage,
        ExitClassification::ExplicitPropagation => TraversalPurpose::ExplicitPropagation,
    };
    classify_expression(
        expression,
        template_ir_store,
        &mut TraversalState::new(purpose, expression_builder),
    )
}

pub(crate) fn pending_body_failure_facts<'a>(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let expression_builder = expression_builder.into();
    let mut state = TraversalState::new(
        TraversalPurpose::PendingSummary(ExpressionFailureFacts::default()),
        expression_builder,
    );
    classify_nodes(nodes, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::PendingSummary(facts) = state.purpose else {
        unreachable!("pending-summary traversal retains its purpose");
    };
    Ok(facts)
}

pub(crate) fn pending_expression_failure_facts<'a>(
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<ExpressionFailureFacts, CompilerError> {
    let expression_builder = expression_builder.into();
    expression_builder.validate_view(expression)?;
    let mut state = TraversalState::new(
        TraversalPurpose::PendingSummary(ExpressionFailureFacts::default()),
        expression_builder,
    );
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

pub(crate) fn collect_expression_failure_facts<'a>(
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<PendingFailureFacts, CompilerError> {
    let expression_builder = expression_builder.into();
    expression_builder.validate_view(expression)?;
    let mut state = TraversalState::new(
        TraversalPurpose::ExpressionProjection(PendingFailureFacts::default()),
        expression_builder,
    );
    classify_expression(expression, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::ExpressionProjection(facts) = state.purpose else {
        unreachable!("expression projection retains its purpose");
    };
    Ok(facts)
}

pub(crate) fn pending_function_failure_facts<'a>(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<PendingFunctionFailureFacts, CompilerError> {
    let expression_builder = expression_builder.into();
    let mut state = TraversalState::new(
        TraversalPurpose::FunctionProjection(PendingFunctionFailureFacts::default()),
        expression_builder,
    );
    classify_nodes(nodes, template_ir_store, &mut state)?;

    // Traversal mutates the selected job's result, never replaces its purpose.
    let TraversalPurpose::FunctionProjection(facts) = state.purpose else {
        unreachable!("function projection retains its purpose");
    };
    Ok(facts)
}
fn pending_function_failure_facts_with_call_summaries(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    active_private_calls: &HashSet<PathId>,
    known_function_paths: &HashSet<PathId>,
    summary_dependencies: &RefCell<HashSet<PathId>>,
    expression_builder: FailureClassificationExpressions<'_>,
) -> Result<PendingFunctionFailureFacts, CompilerError> {
    increment_ast_counter(AstCounter::FailureSummaryBodiesClassified);
    add_ast_counter(AstCounter::FailureSummaryBodyNodesClassified, nodes.len());
    let mut state = TraversalState::new(
        TraversalPurpose::FunctionProjection(PendingFunctionFailureFacts::default()),
        expression_builder,
    )
    .with_call_summaries(
        active_private_calls,
        known_function_paths,
        summary_dependencies,
    );
    classify_nodes(nodes, template_ir_store, &mut state)?;
    let TraversalPurpose::FunctionProjection(facts) = state.purpose else {
        unreachable!("function projection retains its purpose");
    };
    Ok(facts)
}

/// Closed AST-local verdicts for retained typed bodies, never public concrete call summaries.
///
/// WHAT: classifies bodies once and closes their stable escaping-call edges with a worklist.
///       Only catch executability consumes provisional callee activity. Those queries retain
///       reverse dependencies and batch affected bodies after stable activation has drained.
/// WHY: known bodies begin at the lattice bottom, so origin-free recursion stays infallible.
///      Unknown callees remain conservative. Unrelated bodies and summary-insensitive call
///      chains never reopen their AST, while dormant handler validation retains its scope.
pub(crate) struct AstBuiltinFailureSummaries {
    functions: HashMap<PathId, PendingFunctionFailureFacts>,
    active: HashSet<PathId>,
    known_functions: HashSet<PathId>,
}

impl AstBuiltinFailureSummaries {
    pub(crate) fn compute<'a>(
        node_groups: &[&[AstNode]],
        template_ir_store: &TemplateIrStore,
        expression_builder: impl Into<FailureClassificationExpressions<'a>>,
    ) -> Result<Self, CompilerError> {
        let expression_builder = expression_builder.into();
        let mut bodies = HashMap::new();
        for nodes in node_groups {
            for node in *nodes {
                if let NodeKind::Function(path, _, body) = &node.kind {
                    bodies.insert(*path, body.as_slice());
                }
            }
        }
        let known_functions: HashSet<_> = bodies.keys().copied().collect();
        let mut functions = HashMap::with_capacity(bodies.len());
        let mut active = HashSet::new();
        let mut activation_queue = Vec::new();
        let mut activation_dependants: HashMap<PathId, HashSet<PathId>> = HashMap::new();
        let mut classification_dependants: HashMap<PathId, HashSet<PathId>> = HashMap::new();
        let mut dirty = known_functions.clone();

        while !dirty.is_empty() {
            // A wave observes all stable activations from the previous wave, not one callee
            // at a time. Within each body the traversal retains authored witness order.
            let mut batch: Vec<_> = dirty.drain().collect();
            batch.sort_unstable();
            for path in batch {
                let dependencies = RefCell::new(HashSet::new());
                let facts = pending_function_failure_facts_with_call_summaries(
                    bodies[&path],
                    template_ir_store,
                    &active,
                    &known_functions,
                    &dependencies,
                    expression_builder,
                )?;
                for dependency in dependencies.into_inner() {
                    classification_dependants
                        .entry(dependency)
                        .or_default()
                        .insert(path);
                }
                for contributor in &facts.body.implicit {
                    if let ImplicitFailureSource::PrivateCall(callee) = contributor.source
                        && known_functions.contains(&callee)
                        && !active.contains(&callee)
                    {
                        activation_dependants
                            .entry(callee)
                            .or_default()
                            .insert(path);
                    }
                }
                if !active.contains(&path)
                    && facts.body.implicit.iter().any(|contributor| {
                        Self::source_is_active(contributor, &active, &known_functions)
                    })
                {
                    active.insert(path);
                    activation_queue.push(path);
                }
                functions.insert(path, facts);
            }

            // Escaping calls need only their retained contributors. Draining this closure
            // first batches all changed catch dependencies before reopening any body.
            while let Some(callee) = activation_queue.pop() {
                if let Some(dependants) = classification_dependants.remove(&callee) {
                    dirty.extend(dependants);
                }
                if let Some(dependants) = activation_dependants.remove(&callee) {
                    for dependant in dependants {
                        if active.insert(dependant) {
                            activation_queue.push(dependant);
                        }
                    }
                }
            }
        }
        Ok(Self {
            functions,
            active,
            known_functions,
        })
    }

    pub(crate) fn function_facts(&self, path: PathId) -> Option<&PendingFunctionFailureFacts> {
        self.functions.get(&path)
    }

    pub(crate) fn is_active(&self, contributor: &ImplicitFailureContributor) -> bool {
        Self::source_is_active(contributor, &self.active, &self.known_functions)
    }

    fn source_is_active(
        contributor: &ImplicitFailureContributor,
        active_private_calls: &HashSet<PathId>,
        known_function_paths: &HashSet<PathId>,
    ) -> bool {
        match contributor.source {
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::CompoundWriteBack { .. } => !contributor.codes.is_empty(),
            ImplicitFailureSource::AuthoredCastConversion => true,
            ImplicitFailureSource::PrivateCall(path) => {
                !known_function_paths.contains(&path) || active_private_calls.contains(&path)
            }
        }
    }

    pub(crate) fn first_active<'a>(
        &self,
        contributors: &'a [ImplicitFailureContributor],
    ) -> Option<&'a ImplicitFailureContributor> {
        // The first policy candidate is the selection: no later contributor displaces a call,
        // and an origin's own write-back is found by the cursor's single lookahead.
        WitnessCandidates::new(contributors)
            .next_candidate(
                |contributor| Ok::<_, std::convert::Infallible>(self.is_active(contributor)),
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
                    ImplicitFailureSource::NumericOperation
                    | ImplicitFailureSource::AuthoredCastConversion => {
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
pub(crate) fn explicit_propagation_catch_diagnostic<'a>(
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    expression_builder: impl Into<FailureClassificationExpressions<'a>>,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let expression_builder = expression_builder.into();
    expression_builder.validate_view(expression)?;
    if expression
        .failure_facts()
        .is_none_or(|facts| facts.postfix_exit_span.is_none())
    {
        return Ok(None);
    }
    let effect = classify_enclosing_exit_effect(
        expression,
        template_ir_store,
        ExitClassification::ExplicitPropagation,
        expression_builder,
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
    CatchFailureCandidates(Vec<ImplicitFailureContributor>),
}

struct TraversalState<'a> {
    expression_builder: FailureClassificationExpressions<'a>,
    purpose: TraversalPurpose,
    loop_depth: usize,
    visited_templates: HashSet<TemplateTirReference>,
    protected_failure_depth: usize,
    active_private_calls: Option<&'a HashSet<PathId>>,
    known_function_paths: Option<&'a HashSet<PathId>>,
    summary_dependencies: Option<&'a RefCell<HashSet<PathId>>>,
    typed_error_seen: bool,
    expression_steps: Vec<ExpressionStep<'a>>,
}

impl<'a> TraversalState<'a> {
    fn new(
        purpose: TraversalPurpose,
        expression_builder: FailureClassificationExpressions<'a>,
    ) -> Self {
        Self {
            purpose,
            expression_builder,
            loop_depth: 0,
            visited_templates: HashSet::new(),
            protected_failure_depth: 0,
            active_private_calls: None,
            known_function_paths: None,
            summary_dependencies: None,
            typed_error_seen: false,
            expression_steps: Vec::new(),
        }
    }

    fn with_call_summaries(
        mut self,
        active_private_calls: &'a HashSet<PathId>,
        known_function_paths: &'a HashSet<PathId>,
        summary_dependencies: &'a RefCell<HashSet<PathId>>,
    ) -> Self {
        self.active_private_calls = Some(active_private_calls);
        self.known_function_paths = Some(known_function_paths);
        self.summary_dependencies = Some(summary_dependencies);
        self
    }

    fn with_purpose(&self, purpose: TraversalPurpose) -> TraversalState<'a> {
        TraversalState {
            purpose,
            expression_builder: self.expression_builder,
            loop_depth: 0,
            visited_templates: HashSet::new(),
            protected_failure_depth: 0,
            active_private_calls: self.active_private_calls,
            known_function_paths: self.known_function_paths,
            summary_dependencies: self.summary_dependencies,
            typed_error_seen: false,
            expression_steps: Vec::new(),
        }
    }

    fn private_call_may_fail(&self, path: PathId) -> bool {
        match (self.active_private_calls, self.known_function_paths) {
            (Some(active), Some(known)) => {
                if known.contains(&path)
                    && !active.contains(&path)
                    && let Some(dependencies) = self.summary_dependencies
                {
                    dependencies.borrow_mut().insert(path);
                }
                !known.contains(&path) || active.contains(&path)
            }
            (Some(active), None) => active.contains(&path),
            _ => true,
        }
    }

    fn record_typed_error(&mut self, facts: &ExpressionFailureFacts) {
        if matches!(self.purpose, TraversalPurpose::CatchFailureCandidates(_))
            && facts.typed_error.is_some()
        {
            self.typed_error_seen = true;
        }
    }

    fn contributor_may_fail(&self, contributor: &ImplicitFailureContributor) -> bool {
        match contributor.source {
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::CompoundWriteBack { .. } => !contributor.codes.is_empty(),
            ImplicitFailureSource::AuthoredCastConversion => true,
            ImplicitFailureSource::PrivateCall(path) => self.private_call_may_fail(path),
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
            TraversalPurpose::CatchFailureCandidates(candidates) => {
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
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<(), CompilerError> {
    if state.projection_mut().is_none() {
        return Ok(());
    }
    let default_facts = ExpressionFailureFacts::default();
    let facts = expression.failure_facts().unwrap_or(&default_facts);
    // Private-call candidates permit parsing, but only an active exact summary makes a
    // private-only catch eligible. Authored numeric checks remain eligible after folding.
    let eligibility_only = matches!(facts.disposition, FailureDisposition::HandledByCatch { .. })
        && !facts.checked_numeric_operation
        && !facts.authored_cast
        && facts.safety_call_count < 2
        && facts.summary.first_numeric.is_none()
        && facts.summary.first_typed.is_none()
        && facts.summary.first_private_call.is_some();
    if facts.deferred_custom_catch.is_none() && !eligibility_only {
        return Ok(());
    }

    // Ignore only this protected root's handling. Nested catches still exclude their origins.
    let mut candidates = state.with_purpose(TraversalPurpose::PrivateCallCandidates(Vec::new()));
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
                    .map_or(expression.span(), |check| check.catch_span),
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
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    classify_expression_with_root_protection(
        expression,
        template_ir_store,
        state,
        RootProtection::Respect,
    )
}

/// How a child's result reaches its parent when the child yields an effect or an error.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildRelation {
    /// The parent returns straight past its remaining children and its exit actions.
    Early,
    /// The parent drops its remaining children, then runs its exit actions before returning.
    Tail,
}

/// Exit actions for one expression, run after all of its operands have reported.
struct ExpressionExit<'a> {
    expression: ExpressionView<'a>,
    /// True when the expression is pending its own disposition, so it raised no protection.
    pending_root: bool,
    owns_payload: bool,
    /// True for a recovered call, handle or cast, which opened an extra catch window.
    recovers: bool,
}

/// One pending item on the expression walk stack.
enum ExpressionStep<'a> {
    /// An operand still to enter.
    Visit {
        expression: ExpressionView<'a>,
        relation: ChildRelation,
    },
    /// The exit actions of an expression whose operands were pushed above it.
    Exit {
        exit: ExpressionExit<'a>,
        /// How this expression's own result reaches its parent.
        relation: ChildRelation,
    },
}

/// Classifies one expression tree and returns its first effect or error.
///
/// Operands are walked on one stack for the whole call in evaluation order.
/// A child that yields an effect or an error stops its siblings and unwinds the frames waiting
/// on it, as described on `unwind_expression_steps`.
fn classify_expression_with_root_protection<'a>(
    expression: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'a>,
    root_protection: RootProtection,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    // Bind the scratch views to their retained owner rather than the caller's short borrow.
    let expression = state.expression_builder.view(expression.id())?;
    // Value blocks and templates can start nested walks while the outer frames are active.
    let mut steps = std::mem::take(&mut state.expression_steps);
    let result = (|| {
        let mut relation = ChildRelation::Tail;
        let mut produced = enter_expression(
            expression,
            root_protection,
            relation,
            template_ir_store,
            state,
            &mut steps,
        );
        loop {
            if !matches!(produced, Ok(None)) {
                return unwind_expression_steps(
                    &mut steps,
                    relation,
                    produced,
                    template_ir_store,
                    state,
                );
            }
            let Some(step) = steps.pop() else {
                return Ok(None);
            };
            match step {
                ExpressionStep::Visit {
                    expression: child,
                    relation: child_relation,
                } => {
                    relation = child_relation;
                    produced = enter_expression(
                        child,
                        RootProtection::Respect,
                        relation,
                        template_ir_store,
                        state,
                        &mut steps,
                    );
                }
                ExpressionStep::Exit {
                    exit,
                    relation: exit_relation,
                } => {
                    relation = exit_relation;
                    produced = finish_expression(exit, template_ir_store, state).map(|()| None);
                }
            }
        }
    })();
    steps.clear();
    state.expression_steps = steps;
    result
}

/// Runs the pre-step for one expression, then either finishes a leaf or pushes its operands.
///
/// A leaf runs its exit actions before returning. Operand arms that return before scheduling an
/// exit retain their entry protection. `Early` children bypass parent exits while `Tail` children
/// run them. A template's first visit owns its payload traversal and exit actions.
fn enter_expression<'a>(
    expression: ExpressionView<'a>,
    root_protection: RootProtection,
    relation: ChildRelation,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
    steps: &mut Vec<ExpressionStep<'a>>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let default_facts = ExpressionFailureFacts::default();
    let expression_facts = expression.failure_facts().unwrap_or(&default_facts);
    let ignores_root = matches!(root_protection, RootProtection::Ignore);
    let pending_root = ignores_root || expression_facts.disposition == FailureDisposition::Pending;
    let operation = expression.operation();
    let owns_payload = matches!(
        &operation,
        ExpressionOperationView::ValueBlock(_)
            | ExpressionOperationView::Template(_)
            | ExpressionOperationView::RuntimeTemplate(_)
            | ExpressionOperationView::RuntimeSlotApplication(_)
    );
    if state.protected_failure_depth == 0 && pending_root {
        state.record_typed_error(expression_facts);
        if let TraversalPurpose::AssertionMessage = state.purpose {
            // Private candidates wait for exact callee convergence. Numeric failure takes
            // precedence over typed failure, matching the retained aggregate diagnostics.
            let facts = &expression_facts.summary;
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
            facts.merge_pending_from(expression_facts);
        }
    }

    let protects_failure = !pending_root;
    if protects_failure {
        state.protected_failure_depth += 1;
    }
    let exit = |recovers: bool| ExpressionExit {
        expression,
        pending_root,
        owns_payload,
        recovers,
    };
    match operation {
        ExpressionOperationView::Call(call) => {
            let handling = call.handling();
            if state.classifies_exits()
                && handling.is_some_and(|handling| {
                    matches!(handling.handling, FallibleExpressionHandling::Propagate)
                })
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers = !ignores_root
                && handling.is_some_and(|handling| {
                    matches!(handling.handling, FallibleExpressionHandling::Recover)
                });
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(recovers),
                relation,
                call.arguments()
                    .iter()
                    .map(|argument| visit_child(argument.value(), ChildRelation::Tail)),
            )?;
            Ok(None)
        }
        ExpressionOperationView::HandleFallible { value, handling } => {
            if state.classifies_exits()
                && matches!(handling.handling, FallibleExpressionHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(expression),
                )));
            }
            let recovers =
                !ignores_root && matches!(handling.handling, FallibleExpressionHandling::Recover);
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(recovers),
                relation,
                [visit_child(value, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::PropagateOption { value } => {
            if state.classifies_exits() {
                finish_expression(exit(false), template_ir_store, state)?;
                return Ok(Some(EnclosingExitEffect::OptionPropagation(
                    expression.span(),
                )));
            }
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [visit_child(value, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::Cast { source, facts } => {
            let recovers = !ignores_root && matches!(facts.handling, CastHandling::Recover);
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(recovers),
                relation,
                [visit_child(source, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::NumericBinary { left, right, .. }
        | ExpressionOperationView::LogicalBinary { left, right, .. }
        | ExpressionOperationView::Compare { left, right, .. } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [
                    visit_child(left, ChildRelation::Early),
                    visit_child(right, ChildRelation::Tail),
                ],
            )?;
            Ok(None)
        }
        ExpressionOperationView::NumericUnary { operand, .. }
        | ExpressionOperationView::LogicalNot { operand } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [visit_child(operand, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::MethodCall { receiver, call } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                std::iter::once(visit_child(receiver, ChildRelation::Early)).chain(
                    call.arguments()
                        .iter()
                        .map(|argument| visit_child(argument.value(), ChildRelation::Tail)),
                ),
            )?;
            Ok(None)
        }
        ExpressionOperationView::CollectionBuiltin {
            receiver,
            arguments,
            ..
        }
        | ExpressionOperationView::MapBuiltin {
            receiver,
            arguments,
            ..
        } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                std::iter::once(visit_child(receiver, ChildRelation::Early)).chain(
                    arguments
                        .iter()
                        .map(|argument| visit_child(argument.value(), ChildRelation::Tail)),
                ),
            )?;
            Ok(None)
        }
        ExpressionOperationView::FieldAccess { base, .. } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [visit_child(base, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::Copy(place) => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                place
                    .projections()
                    .filter_map(|projection| match projection {
                        PlaceProjectionView::Index { index, .. } => {
                            Some(visit_child(index, ChildRelation::Tail))
                        }
                        _ => None,
                    }),
            )?;
            Ok(None)
        }
        ExpressionOperationView::Collection(items) => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                items
                    .iter()
                    .map(|item| visit_child(item, ChildRelation::Early)),
            )?;
            Ok(None)
        }
        ExpressionOperationView::Map(entries) => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                entries.iter().flat_map(|entry| {
                    [
                        visit_child(entry.key(), ChildRelation::Early),
                        visit_child(entry.value(), ChildRelation::Early),
                    ]
                }),
            )?;
            Ok(None)
        }
        ExpressionOperationView::Struct { fields }
        | ExpressionOperationView::Choice { fields, .. }
        | ExpressionOperationView::RouteConstruct { fields, .. } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                fields
                    .iter()
                    .map(|field| visit_child(field.value(), ChildRelation::Early)),
            )?;
            Ok(None)
        }
        ExpressionOperationView::Range { start, end, .. } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [
                    visit_child(start, ChildRelation::Early),
                    visit_child(end, ChildRelation::Tail),
                ],
            )?;
            Ok(None)
        }
        ExpressionOperationView::Coerce { value, .. } => {
            open_expression_frame(
                template_ir_store,
                state,
                steps,
                exit(false),
                relation,
                [visit_child(value, ChildRelation::Tail)],
            )?;
            Ok(None)
        }
        ExpressionOperationView::ValueBlock(block) => {
            let effect = classify_value_block(block, template_ir_store, state);
            finish_expression(exit(false), template_ir_store, state)?;
            effect
        }
        ExpressionOperationView::Template(template) => {
            if !state.visited_templates.insert(template.tir_reference) {
                // Only the first template visit owns payload traversal and exit actions.
                // A repeated visit bypasses the exit and retains its entry protection.
                return Ok(None);
            }
            let mut effect = None;
            let classifies_exits = state.classifies_exits();
            let expression_builder = state.expression_builder.builder()?;
            let mut visit = |nested_expression: ExpressionView<'_>| {
                if effect.is_none() {
                    effect = classify_expression(nested_expression, template_ir_store, state)?;
                }
                Ok(())
            };
            if classifies_exits {
                walk_expression_payloads_with_nested_tir_views(
                    expression,
                    template_ir_store,
                    expression_builder,
                    &mut visit,
                )?;
            } else {
                // Projection keeps wrapper origins, unlike the predicate-oriented nested walker.
                let reference = template.tir_reference;
                let view = TirView::new(
                    template_ir_store,
                    reference.root,
                    reference.phase,
                    reference.context,
                )?;
                walk_tir_view_expression_payloads(&view, expression_builder, &mut visit)?;
            }
            finish_expression(exit(false), template_ir_store, state)?;
            Ok(effect)
        }
        ExpressionOperationView::RuntimeTemplate(handoff) => {
            let effect = classify_runtime_template_handoff(handoff, template_ir_store, state);
            finish_expression(exit(false), template_ir_store, state)?;
            effect
        }
        ExpressionOperationView::RuntimeSlotApplication(handoff) => {
            let effect =
                classify_runtime_slot_application_handoff(handoff, template_ir_store, state);
            finish_expression(exit(false), template_ir_store, state)?;
            effect
        }
        ExpressionOperationView::Constant(_) | ExpressionOperationView::Reference(_) => {
            finish_expression(exit(false), template_ir_store, state)?;
            Ok(None)
        }
    }
}

/// Finishes an empty operand directly, or pushes its exit and children in source order.
///
/// Children are pushed in order and then reversed, so the first child pops first. A recovered
/// catch window protects every child from entry onwards. The exit owns its release, so `Tail`
/// results release it while `Early` results bypass the exit and retain it.
fn open_expression_frame<'a>(
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
    steps: &mut Vec<ExpressionStep<'a>>,
    exit: ExpressionExit<'a>,
    relation: ChildRelation,
    children: impl IntoIterator<Item = ExpressionStep<'a>>,
) -> Result<(), CompilerError> {
    state.protected_failure_depth += usize::from(exit.recovers);
    let mut children = children.into_iter().peekable();
    if children.peek().is_none() {
        return finish_expression(exit, template_ir_store, state);
    }
    steps.push(ExpressionStep::Exit { exit, relation });
    let first_child = steps.len();
    steps.extend(children);
    steps[first_child..].reverse();
    Ok(())
}

fn visit_child<'a>(expression: ExpressionView<'a>, relation: ChildRelation) -> ExpressionStep<'a> {
    ExpressionStep::Visit {
        expression,
        relation,
    }
}

/// Lowers the protection raised on entry, then runs the post-step for one expression.
fn finish_expression(
    exit: ExpressionExit<'_>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<(), CompilerError> {
    if exit.recovers {
        state.protected_failure_depth -= 1;
    }
    if !exit.pending_root {
        state.protected_failure_depth -= 1;
    }

    let default_facts = ExpressionFailureFacts::default();
    let expression_facts = exit.expression.failure_facts().unwrap_or(&default_facts);
    // Children precede their call/operator origin. Numeric constructors put the resolved
    // operation's contributor on this row, so reading its side facts again would double-count
    // the origin. Ancestor summaries retain eligibility, never descendant origin lists.
    if state.protected_failure_depth == 0 && exit.pending_root && !exit.owns_payload {
        for contributor in &expression_facts.implicit {
            state.record_origin(contributor);
        }
    }
    collect_deferred_check(exit.expression, template_ir_store, state)
}

/// Applies the frames that wait on a child's effect or error.
///
/// Remaining siblings are dropped. A parent whose child is `Tail` runs its exit actions with the
/// escaping result and a failing exit replaces it. An `Early` child bypasses the parent's exit
/// actions, retaining its protection. Each exit records its own relation to the next parent.
fn unwind_expression_steps(
    steps: &mut Vec<ExpressionStep<'_>>,
    mut relation: ChildRelation,
    mut produced: Result<Option<EnclosingExitEffect>, CompilerError>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    while let Some(step) = steps.pop() {
        if let ExpressionStep::Exit {
            exit,
            relation: exit_relation,
        } = step
        {
            if relation == ChildRelation::Tail
                && let Err(error) = finish_expression(exit, template_ir_store, state)
            {
                produced = Err(error);
            }
            relation = exit_relation;
        }
    }
    produced
}

fn expression_propagation_span(expression: ExpressionView<'_>) -> Option<SourceSpan> {
    expression.propagation_span().or(expression.span())
}

fn classify_expressions(
    expressions: &[ExprId],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for expression in expressions {
        if let Some(effect) = classify_expression(
            state.expression_builder.view(*expression)?,
            template_ir_store,
            state,
        )? {
            return Ok(Some(effect));
        }
    }
    Ok(None)
}

fn classify_value_block(
    block: &ValueBlock,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match block {
        ValueBlock::If(value_if) => {
            if let Some(effect) = classify_expression(
                state.expression_builder.view(value_if.condition)?,
                template_ir_store,
                state,
            )? {
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
            if let Some(effect) = classify_expression(
                state.expression_builder.view(value_match.scrutinee)?,
                template_ir_store,
                state,
            )? {
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
                classify_catch_handler(
                    body,
                    state.expression_builder.view(value_catch.handled_value)?,
                    template_ir_store,
                    state,
                )?;

                state.protected_failure_depth += 1;
                let effect = classify_expression(
                    state.expression_builder.view(value_catch.handled_value)?,
                    template_ir_store,
                    state,
                );
                state.protected_failure_depth -= 1;
                return effect;
            }

            if state.classifies_exits()
                && matches!(value_catch.handler, FallibleHandling::Propagate)
            {
                return Ok(Some(EnclosingExitEffect::ErrorPropagation(
                    expression_propagation_span(
                        state.expression_builder.view(value_catch.handled_value)?,
                    ),
                )));
            }
            if matches!(value_catch.handler, FallibleHandling::Propagate) {
                if let Some(effect) = classify_expression(
                    state.expression_builder.view(value_catch.handled_value)?,
                    template_ir_store,
                    state,
                )? {
                    return Ok(Some(effect));
                }
            } else {
                state.protected_failure_depth += 1;
                let effect = classify_expression(
                    state.expression_builder.view(value_catch.handled_value)?,
                    template_ir_store,
                    state,
                );
                state.protected_failure_depth -= 1;
                if let Some(effect) = effect? {
                    return Ok(Some(effect));
                }
            }
            if let FallibleHandling::Handler { body, .. } = &value_catch.handler {
                return classify_catch_handler(
                    body,
                    state.expression_builder.view(value_catch.handled_value)?,
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
    protected: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    let dead_handler = if state.classifies_exits() {
        false
    } else {
        !catch_protected_expression_can_fail(protected, template_ir_store, state)?
    };
    state.protected_failure_depth += usize::from(dead_handler);
    let effect = classify_nodes(body, template_ir_store, state);
    state.protected_failure_depth -= usize::from(dead_handler);
    effect
}

fn catch_protected_expression_can_fail(
    protected: ExpressionView<'_>,
    template_ir_store: &TemplateIrStore,
    state: &TraversalState<'_>,
) -> Result<bool, CompilerError> {
    let mut candidates = state.with_purpose(TraversalPurpose::CatchFailureCandidates(Vec::new()));
    classify_expression_with_root_protection(
        protected,
        template_ir_store,
        &mut candidates,
        RootProtection::Ignore,
    )?;
    let typed_error_seen = candidates.typed_error_seen;
    let TraversalPurpose::CatchFailureCandidates(contributors) = candidates.purpose else {
        unreachable!("catch candidate traversal retains its purpose");
    };
    Ok(typed_error_seen
        || contributors
            .iter()
            .any(|contributor| state.contributor_may_fail(contributor)))
}

fn classify_match_arm(
    arm: &MatchArm,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    if let Some(guard) = &arm.guard
        && let Some(effect) = classify_expression(
            state.expression_builder.view(*guard)?,
            template_ir_store,
            state,
        )?
    {
        return Ok(Some(effect));
    }
    classify_nodes(&arm.body, template_ir_store, state)
}

fn classify_declarations(
    declarations: &[Declaration],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for declaration in declarations {
        if let Some(initializer) = declaration.value.initializer()
            && let Some(effect) = classify_expression(
                state.expression_builder.view(initializer)?,
                template_ir_store,
                state,
            )?
        {
            return Ok(Some(effect));
        }
    }
    Ok(None)
}

fn classify_nodes(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    for node in nodes {
        let effect = match &node.kind {
            NodeKind::Return(values) if !state.classifies_exits() => {
                classify_expressions(values, template_ir_store, state)?
            }
            NodeKind::ReturnError(value) if !state.classifies_exits() => classify_expression(
                state.expression_builder.view(*value)?,
                template_ir_store,
                state,
            )?,
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
            NodeKind::If(condition, then_body, else_body, _) => classify_expression(
                state.expression_builder.view(*condition)?,
                template_ir_store,
                state,
            )?
            .or(classify_nodes(then_body, template_ir_store, state)?)
            .or(match else_body.as_deref() {
                Some(body) => classify_nodes(body, template_ir_store, state)?,
                None => None,
            }),
            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                let mut effect = classify_expression(
                    state.expression_builder.view(*scrutinee)?,
                    template_ir_store,
                    state,
                )?;
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
                if let Some(effect) = classify_range_failure(range.domain, node.span, state) {
                    return Ok(Some(effect));
                }
                let mut effect = classify_expression(
                    state.expression_builder.view(range.start)?,
                    template_ir_store,
                    state,
                )?;
                if effect.is_none() {
                    effect = classify_expression(
                        state.expression_builder.view(range.end)?,
                        template_ir_store,
                        state,
                    )?;
                }
                if effect.is_none()
                    && let Some(step) = &range.step
                {
                    effect = classify_expression(
                        state.expression_builder.view(*step)?,
                        template_ir_store,
                        state,
                    )?;
                }
                if effect.is_none() {
                    effect = classify_loop_body(body, template_ir_store, state)?;
                }
                effect
            }
            NodeKind::CollectionLoop { iterable, body, .. } => classify_expression(
                state.expression_builder.view(*iterable)?,
                template_ir_store,
                state,
            )?
            .or(classify_loop_body(body, template_ir_store, state)?),
            NodeKind::WhileLoop(condition, body) => classify_expression(
                state.expression_builder.view(*condition)?,
                template_ir_store,
                state,
            )?
            .or(classify_loop_body(body, template_ir_store, state)?),
            NodeKind::VariableDeclaration(declaration) => match declaration.value.initializer() {
                Some(initializer) => classify_expression(
                    state.expression_builder.view(initializer)?,
                    template_ir_store,
                    state,
                )?,
                None => None,
            },
            NodeKind::ExpressionStatement(value) | NodeKind::PushStartRuntimeFragment(value) => {
                classify_expression(
                    state.expression_builder.view(*value)?,
                    template_ir_store,
                    state,
                )?
            }
            NodeKind::Assignment { value, .. } | NodeKind::MultiBind { value, .. } => {
                classify_expression(
                    state.expression_builder.view(*value)?,
                    template_ir_store,
                    state,
                )?
            }
            NodeKind::Assert { condition, message } => {
                let effect = classify_expression(
                    state.expression_builder.view(*condition)?,
                    template_ir_store,
                    state,
                )?;
                match &mut state.purpose {
                    TraversalPurpose::FunctionProjection(facts) => {
                        let message_facts = collect_expression_failure_facts(
                            state.expression_builder.view(*message)?,
                            template_ir_store,
                            state.expression_builder,
                        )?;
                        facts.assertion_message_calls.extend(
                            message_facts.implicit.into_iter().filter(|contributor| {
                                matches!(contributor.source, ImplicitFailureSource::PrivateCall(_))
                            }),
                        );
                    }
                    TraversalPurpose::AssertionMessage | TraversalPurpose::ExplicitPropagation => {
                        return Ok(effect.or(classify_expression(
                            state.expression_builder.view(*message)?,
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
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    state.loop_depth += 1;
    let result = classify_nodes(body, template_ir_store, state);
    state.loop_depth -= 1;
    result
}

fn classify_runtime_template_handoff(
    handoff: &OwnedRuntimeTemplateHandoff,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
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
    state: &mut TraversalState<'_>,
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
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => Ok(classify_expression(
            state.expression_builder.view(*expression)?,
            template_ir_store,
            state,
        )?),
        OwnedRuntimeTemplateNode::Conditional { selector, .. } => {
            classify_branch_selector(selector, template_ir_store, state)
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
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => Ok(None),
    }
}

fn classify_branch_selector(
    selector: &TemplateBranchSelector,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => classify_expression(
            state.expression_builder.view(*condition)?,
            template_ir_store,
            state,
        ),
        TemplateBranchSelector::OptionPresentCapture { scrutinee, .. } => {
            if let Some(effect) = classify_expression(
                state.expression_builder.view(*scrutinee)?,
                template_ir_store,
                state,
            )? {
                return Ok(Some(effect));
            }
            Ok(None)
        }
    }
}

fn classify_loop_header(
    header: &TemplateLoopHeader,
    template_ir_store: &TemplateIrStore,
    state: &mut TraversalState<'_>,
) -> Result<Option<EnclosingExitEffect>, CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => classify_expression(
            state.expression_builder.view(*condition)?,
            template_ir_store,
            state,
        ),
        TemplateLoopHeader::Range { range, .. } => {
            let span = match range.step {
                Some(step) => state.expression_builder.view(step)?.span(),
                None => None,
            }
            .or(state.expression_builder.view(range.start)?.span());
            if let Some(effect) = classify_range_failure(range.domain, span, state) {
                return Ok(Some(effect));
            }
            if let Some(effect) = classify_expression(
                state.expression_builder.view(range.start)?,
                template_ir_store,
                state,
            )? {
                return Ok(Some(effect));
            }
            if let Some(effect) = classify_expression(
                state.expression_builder.view(range.end)?,
                template_ir_store,
                state,
            )? {
                return Ok(Some(effect));
            }
            match &range.step {
                Some(step) => classify_expression(
                    state.expression_builder.view(*step)?,
                    template_ir_store,
                    state,
                ),
                None => Ok(None),
            }
        }
        TemplateLoopHeader::Collection { iterable, .. } => classify_expression(
            state.expression_builder.view(*iterable)?,
            template_ir_store,
            state,
        ),
    }
}

fn classify_range_failure(
    domain: NumericScalar,
    span: Option<SourceSpan>,
    state: &mut TraversalState<'_>,
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

    // Range loops fail on overflow plus their step guards: zero step (306) in every domain and
    // no progress (307) when floating rounding leaves the candidate unchanged.
    let codes: &'static [BuiltinErrorCode] = if domain.is_binary_float() {
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
