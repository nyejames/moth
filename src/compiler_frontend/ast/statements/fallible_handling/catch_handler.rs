//! Catch fallible-handler parsing helpers.
//!
//! WHAT: parses `catch:` no-binding handlers and `catch |err|:` binding handlers, preserving
//! `then` value-production statements inside value-producing handler bodies.
//! WHY: call and expression fallible handling share identical catch-handler syntax and validation,
//! so this module removes duplicated parser paths.

use std::convert::Infallible;

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CatchErrorBinding, FallibleHandling,
};
use crate::compiler_frontend::ast::field_access::retained_terminal_call;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::numeric_origins::GenericErrorReturn;
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::numeric_construction::{
    BodyEntry, ConstructionResult, DeferredBody, DeferredConstruct, DeferredExpressionCompletion,
    DeferredExpressionServices, DeferredNumericExpression, DetachedContinuations, NumericBindingId,
    NumericConstructionState, NumericContinuation, NumericContinuationId, NumericFamily,
    StatementExpression, binding_class, binding_reference_shape, family_receives_recipe,
    item_is_literal_recipe_shape, raw_fragment_result_is_independently_fixed, retained_known_shape,
    single_direct_pending_reference, union_pending_binding_with_generic_slot,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, fragment_shape, register_shape_witness, union_shape_classes,
};
use crate::compiler_frontend::ast::statements::body_dispatch::ParsedBody;
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    analyze_branch_exits, visit_authored_then_values_mut, visit_reachable_then_values,
};
use crate::compiler_frontend::ast::statements::value_production::parse_values::{
    ProducedValuesParseInput, validate_and_coerce_completed_then_group,
};
use crate::compiler_frontend::ast::statements::value_production::types::BranchExitSummary;
use crate::compiler_frontend::ast::statements::value_production::{
    ActiveValueProductionTarget, ProducedValues, ThenValueContinuation, ThenValueSlot,
    ValueReceiverKind, is_missing_produced_value_boundary, parse_retained_produced_values,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::type_coercion::parse_context::{
    ExpectedType, is_numeric_literal_destination_type_id,
};

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

use super::parser::{
    ProducerCatchOutcome, complete_catch_failure_fact, completed_catch_attachment,
    producer_catch_site, wrap_catch_expression,
};
use super::validation::{
    validate_catch_fallible_handler_binding, validate_catch_fallible_handler_conflict,
    validate_catch_fallible_handler_value_requirement,
};

pub(crate) struct CatchFallibleHandler {
    pub(crate) error: Option<CatchErrorBinding>,
    pub(crate) body: Vec<AstNode>,
}

/// One catch handler body as parsed: complete where it was parsed, or retained for the body root.
///
/// WHAT: either the authored statement nodes, or the branch draft plus the value-production
///       contract that only becomes checkable once the statements are concrete.
/// WHY: a body that retained numeric continuations cannot be validated or published at its parse
///      position; the check must run exactly once, where the body resumes.
#[derive(Debug, Clone)]
pub(crate) enum CatchHandlerBody {
    /// The handler body completed without any numeric continuation and was validated in place.
    Ready(Vec<AstNode>),
    /// The handler body is one retained `then` continuation whose fallback values still hold
    /// numeric origins; the resolved slots decide their types at the body root. The continuation
    /// owns the authored span and scope, so the body records only the pending entry.
    PendingThen { continuation: NumericContinuationId },
    /// The handler body retained numeric continuations for the enclosing body root.
    Draft {
        body: DeferredBody,
        value_required: bool,
        value_required_span: Option<SourceSpan>,
    },
}

/// One parsed block-form catch handler whose numeric continuations must resume at the body root.
///
/// WHAT: the authored error binding and handler body, ready or retained.
/// WHY: a handler body shares the enclosing root's numeric domain, so its pending bindings and
///      fallback receivers cannot resolve where the handler was parsed. Only the body root can
///      finish them, and it must rebuild the catch exactly as this site would have.
#[derive(Debug, Clone)]
pub(crate) struct RetainedCatchHandler {
    pub(crate) error: Option<CatchErrorBinding>,
    pub(crate) body: CatchHandlerBody,
}

/// One catch recovery whose handler body retained numeric continuations.
///
/// WHAT: the protected value exactly as this catch site protects it, the site's fallible facts,
///       and the retained handler.
/// WHY: this is the complete input the owning constructor needs to rebuild the catch value block
///      at the body root; retaining it here avoids a second parse, an eager fallback, or a
///      fabricated handled value.
#[derive(Debug, Clone)]
pub(crate) struct DeferredCatchHandler {
    /// The protected value: already wrapped as this site wraps it, or its authored fragment.
    pub(crate) handled: StatementExpression,
    /// The site facts of an already-built protected value.
    ///
    /// A protected value that still has to resolve derives the same facts at completion, exactly
    /// as its completed-expression parse would have.
    pub(crate) ready_site: Option<ReadyCatchSite>,
    /// The numeric shape of a raw literal protected value, opened when the catch was retained.
    ///
    /// WHAT: present only for a provisional raw literal fragment, whose fallbacks join its class at
    ///       retention. A generic call and every other protected value leave it empty.
    /// WHY: body-root defaults must see the fallback witnesses before they choose the literal's
    ///      width, so the class exists at retention and completion reads its merged receiver.
    pub(crate) protected_shape: Option<KnownShapeId>,
    pub(crate) handler: RetainedCatchHandler,
}

/// The fallible site of a protected value that was already built where it was authored.
#[derive(Debug, Clone)]
pub(crate) struct ReadyCatchSite {
    pub(crate) error_return_type_id: TypeId,
    pub(crate) success_result_type_ids: Vec<TypeId>,
}

/// The success contract a receiving site knows for its protected value.
///
/// WHAT: `Known` supplies concrete slot types from the signature or an immediate typed receiver;
///       `Known(&[])` has zero success slots. `Pending` keeps the declared arity of a generic call
///       whose slot types all resolve at completion. `Partial` keeps one entry per declared success
///       slot: `Some` for a slot whose type is already concrete, `None` for a slot whose type
///       resolves later. `Unresolved` has no declared arity to read yet.
/// WHY: a generic call keeps its concrete declared slots as immediate receivers while its generic
///      slots stay unknown, so a handler body parses each slot against what is already known and
///      never against a type the call has not resolved. Only `Known` and `Partial` supply slot
///      types; `Pending` supplies only the arity.
#[derive(Clone, Copy)]
pub(crate) enum FallibleSuccessContract<'a> {
    Known(&'a [TypeId]),
    Pending { slot_count: usize },
    Partial(&'a [Option<TypeId>]),
    Unresolved,
}

impl<'a> FallibleSuccessContract<'a> {
    /// The fully concrete receiving slot types, or `None` while any slot type remains unknown.
    pub(crate) fn known(self) -> Option<&'a [TypeId]> {
        match self {
            Self::Known(types) => Some(types),
            Self::Pending { .. } | Self::Partial(_) | Self::Unresolved => None,
        }
    }

    /// The value-production target a catch handler body parses against for this contract.
    ///
    /// WHAT: a known contract targets its concrete receiving slots; a pending one targets its
    ///       declared arity with every slot left inferred; a partial one targets its declared arity
    ///       with each concrete slot typed and each unknown slot left inferred; an unresolved one
    ///       targets a single inferred slot.
    /// WHY: a generic call keeps its arity, so a multi-value `then` is checked against the declared
    ///      count rather than rejected as too many values, and a concrete slot is typed at parse
    ///      time without resolving a sibling's generic parameter. Completion revalidates the body.
    pub(crate) fn handler_target(self) -> ActiveValueProductionTarget {
        match self {
            Self::Known(types) => {
                ActiveValueProductionTarget::known(types.to_vec(), ValueReceiverKind::CatchHandler)
            }
            Self::Pending { slot_count } => {
                ActiveValueProductionTarget::provisional_catch_handler(slot_count)
            }
            Self::Partial(slots) => {
                ActiveValueProductionTarget::mixed(slots, ValueReceiverKind::CatchHandler)
            }
            Self::Unresolved => ActiveValueProductionTarget::provisional_catch_handler(1),
        }
    }
}

pub(crate) struct CatchFallibleHandlerSite<'a> {
    /// The protected value's success contract. `Known` supplies concrete signature or immediate
    /// receiving types, possibly empty for zero success slots; `Pending` keeps a generic call's
    /// declared arity while every slot type resolves; `Partial` types each concrete slot and leaves
    /// each generic slot unknown until it resolves; `Unresolved` has no declared contract to read
    /// at parse time.
    pub(crate) success_contract: FallibleSuccessContract<'a>,
    /// The protected call's error return: a concrete error, or a numeric-dependent one whose
    /// binding is declared as a pending row over its shape.
    pub(crate) error_return: GenericErrorReturn,
    pub(crate) value_required: bool,
    pub(crate) compilation_stage: &'a str,
    pub(crate) value_required_span: Option<SourceSpan>,
}

/// One parsed catch handler: ready where it was parsed, or retained for the body root.
pub(crate) enum ParsedCatchHandler {
    /// The handler body completed without any numeric continuation.
    Ready(CatchFallibleHandler),
    /// The handler body retained numeric continuations that only the body root can resolve.
    Retained(Box<RetainedCatchHandler>),
}

/// Rebuilds one retained catch recovery at the enclosing numeric body root.
///
/// WHAT: resolves the protected value and the retained handler body, re-runs the handler's
///       value-production contract, and reattaches the catch exactly as the catch site attaches it.
/// WHY: the handler body and the protected value share the root's construction state, so this is
///      the only place the authored catch can be finished — never as a placeholder, a defaulted
///      handler, or a second parse of the same source.
pub(crate) fn complete_deferred_catch_handler<'env>(
    handler: DeferredCatchHandler,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredCatchHandler {
        mut handled,
        ready_site,
        protected_shape,
        handler,
    } = handler;

    // A handler body that parsed without an established site was validated against a provisional
    // contract; its fallthrough requirement must be revalidated against the resolved one below.
    let provisional_site = ready_site.is_none();

    // A raw literal protected value opened its numeric shape when its catch was retained, and its
    // fallbacks already joined that class. Its literal leaves take the class's merged receiver
    // here, before any language default types them. A generic call has no such literal leaves.
    if let Some(shape) = protected_shape {
        constrain_raw_protected_expected(&mut handled, shape, completion)?;
    }

    // The protected fragment completes without closing its typed errors: the catch attached
    // below is exactly the recovery that consumes them, and the recovered expression is
    // validated at the enclosing position. Closing the protected value here would report the
    // authored catch as unhandled before it exists.
    let handled_expression = completion.resolve_protected_statement_expression(handled)?;

    // A protected value that was built where it was authored carries that site's facts. A value
    // that resolved here derives the same facts its completed-expression parse derives.
    let (handled_expression, error_return_type_id, success_result_type_ids) = match ready_site {
        Some(site) => (
            handled_expression,
            site.error_return_type_id,
            site.success_result_type_ids,
        ),
        None => {
            let services = completion.session_services();
            resolved_catch_site(
                handled_expression,
                services.context,
                services.type_interner,
                services.expression_builder,
                services.const_values,
                services.string_table,
            )?
        }
    };

    // A body that already completed where it was parsed was validated there; a retained body is
    // validated exactly once here, against the site the protected value resolved to.
    let body = match handler.body {
        CatchHandlerBody::Ready(mut nodes) => {
            // A body parsed before the fragment's contract existed was checked against a
            // provisional site. Under the resolved slots it must satisfy the ordinary receiving
            // boundary: the fallthrough requirement, the exact arity, and a per-slot coercion so
            // an incompatible fallback reports the authored type/shape diagnostic instead of
            // reaching HIR as a mismatched write.
            if provisional_site {
                // The ordinary paths report this rejection at the protected expression's span
                // (`value_required_span`); a retained body has no parse-time site, so the resolved
                // protected expression supplies the same authored location, with the body's first
                // authored statement as a fallback — never a synthetic span.
                let services = completion.session_services();
                let fallthrough_span = services
                    .expression_builder
                    .view_owned(handled_expression)?
                    .span()
                    .or_else(|| nodes.first().and_then(|node| node.span));
                validate_catch_fallible_handler_value_requirement(
                    true,
                    Some(&success_result_type_ids),
                    &nodes,
                    services.expression_builder,
                    services.const_values,
                    fallthrough_span,
                )?;
            }

            // Every reachable `then` group of the completed body — top level and nested inside
            // `if`/`match` — receives the ordinary receiving boundary: exact arity plus a
            // per-slot coercion. A group parsed before the protected slot was known, or under a
            // nested provisional contract, therefore reports the authored diagnostic instead of
            // reaching HIR as a mismatched write.
            let mut services = completion.session_services();
            validate_completed_handler_then_values(
                &mut nodes,
                &success_result_type_ids,
                &mut services,
            )?;
            nodes
        }
        CatchHandlerBody::PendingThen { continuation } => {
            // The fallback values materialise at the resolved slots inside the continuation's own
            // resolution; the authored `then` node is the handler body.
            completion.resolve_continuation_nodes(continuation, &success_result_type_ids)?
        }
        CatchHandlerBody::Draft {
            body,
            value_required,
            value_required_span,
        } => {
            // The retained fallback values parsed against a provisional contract materialise at
            // the resolved success slots, which stay visible to value-production completion for
            // exactly this body.
            let mut nodes = completion
                .resolve_deferred_body_with_expectations(body, Some(&success_result_type_ids))?;
            let services = completion.session_services();
            validate_catch_fallible_handler_value_requirement(
                value_required,
                Some(&success_result_type_ids),
                &nodes,
                services.expression_builder,
                services.const_values,
                value_required_span,
            )?;

            // A retained body completes its own `then` groups here, and the groups that were
            // parsed eagerly before the protected slot was known receive the same receiving
            // boundary as a ready body.
            let mut services = completion.session_services();
            validate_completed_handler_then_values(
                &mut nodes,
                &success_result_type_ids,
                &mut services,
            )?;
            nodes
        }
    };

    let services = completion.session_services();
    let mut recovered = wrap_catch_expression(
        services.expression_builder,
        handled_expression,
        FallibleHandling::Handler {
            error: handler.error,
            body,
        },
        success_result_type_ids,
    )?;

    let services = completion.session_services();
    complete_catch_failure_fact(
        &mut recovered,
        services.expression_builder,
        error_return_type_id,
        services.context,
    )?;

    Ok(recovered)
}

/// Fixes a raw protected literal's destination from its numeric class's merged receiver.
///
/// WHAT: a retained raw literal protected value without a destination of its own takes its class's
///       merged receiving type as its `NumericLiteral` destination, when the literal rule accepts
///       that type.
/// WHY: the class already carries every fallback and use that reached the protected value, so the
///      literal's leaves take that width before the language default types them as I32/F64. A class
///      with no receiver keeps the default.
fn constrain_raw_protected_expected<'env>(
    handled: &mut StatementExpression,
    shape: KnownShapeId,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<(), ExpressionParseError> {
    let StatementExpression::Retained(deferred) = handled else {
        return Ok(());
    };
    if deferred.expected != ExpectedType::Infer {
        return Ok(());
    }
    let Some(receiver) = completion.shape_receiver(shape)? else {
        return Ok(());
    };
    let services = completion.session_services();
    let environment = services.type_interner.environment();
    let family = NumericFamily::of_items(&deferred.items);
    if raw_literal_receives_recipe(family, receiver, environment) {
        deferred.expected = ExpectedType::NumericLiteral(receiver);
    }
    Ok(())
}

/// Whether a raw protected literal of this family may materialise at a receiving type.
///
/// WHAT: a whole literal takes every destination the literal rule accepts: fixed integers, binary
///       floats, `Dec` scales, and `Byte`. A decimal literal takes binary floats and `Dec` scales.
/// WHY: an immediate receiver materialises the literal's leaves before operator selection, so the
///      fallback's width reaches the literal. Named-origin family barriers are not repeated here:
///      witness registration already refuses a receiver that a pinned class family rejects.
fn raw_literal_receives_recipe(
    family: NumericFamily,
    receiver: TypeId,
    environment: &TypeEnvironment,
) -> bool {
    match family {
        NumericFamily::Whole => is_numeric_literal_destination_type_id(receiver, environment),
        NumericFamily::Decimal => {
            family_receives_recipe(Some(NumericFamily::Decimal), receiver, environment)
                || environment.number_scale(receiver).is_some()
        }
    }
}

/// A fallback's exact receiving type and the source that supplied it.
#[derive(Clone, Copy)]
struct ExactFallbackType {
    type_id: TypeId,
    span: Option<SourceSpan>,
}

/// A fallback supplies a concrete receiver or links the protected slot to a shared origin.
#[derive(Clone, Copy)]
enum FallbackWitness {
    Exact(ExactFallbackType),
    LinkedBinding(NumericBindingId),
    /// A retained result with a numeric or structured shape, joined by structural union.
    LinkedShape(KnownShapeId),
}

/// The numeric shape a raw literal protected value opens when its catch is retained.
///
/// WHAT: a protected value that is one literal recipe with no destination of its own, and whose
///       result is not independently fixed, opens its numeric shape now through the fragment-shape
///       owner. Every other protected value opens none.
/// WHY: the catch's fallbacks must reach the protected literal's class before body-root defaults
///      choose its width, so the class has to exist when the catch is retained, where those
///      fallbacks are read.
pub(crate) fn raw_protected_result_shape(
    items: &[ExpressionRpnItem],
    expected: ExpectedType,
    scope_context: &ScopeContext,
    environment: &mut TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
) -> ConstructionResult<Option<KnownShapeId>> {
    if expected != ExpectedType::Infer || !items.iter().all(item_is_literal_recipe_shape) {
        return Ok(None);
    }
    if raw_fragment_result_is_independently_fixed(items, NumericFamily::of_items(items)) {
        return Ok(None);
    }
    fragment_shape(scope_context, environment, items, expression_builder).map(Some)
}

/// Applies a retained catch's fallback witnesses to the slots its protected value opens.
///
/// WHAT: a raw literal protected value that opened a shape at retention protects that one slot; a
///       retained single generic call protects its declared result slots. Any other protected value
///       protects no slot and supplies nothing here.
/// WHY: a fallback's exact type or shared origin must join the protected slot before body-root
///      defaults choose the origin's width. The slot list is the only difference between the two
///      protected forms, so one walk serves both.
pub(crate) fn constrain_protected_by_fallbacks(
    payload: &DeferredCatchHandler,
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) {
    if let Some(protected_shape) = payload.protected_shape {
        visit_fallback_witnesses(
            &payload.handler.body,
            std::slice::from_ref(&protected_shape),
            state,
            environment,
            expression_builder,
            const_values,
        );
        return;
    }
    let StatementExpression::Retained(deferred) = &payload.handled else {
        return;
    };
    if deferred.expected != ExpectedType::Infer {
        return;
    }
    let Some(item) = single_operand(&deferred.items) else {
        return;
    };
    let Some(terminal) = retained_terminal_call(item) else {
        return;
    };
    let Some(slot_shapes) = terminal.generic_result_shapes() else {
        return;
    };
    visit_fallback_witnesses(
        &payload.handler.body,
        slot_shapes,
        state,
        environment,
        expression_builder,
        const_values,
    );
}

/// Applies every reachable fallback witness of a catch body to the protected slots, in authored order.
///
/// WHAT: visits each reachable `then` group once; each group contributes its witness to every slot it
///       holds, so a later group never hides an earlier one. The continuation table is detached for
///       the walk, so the walk reads its entries while the witness sinks mutate the state.
/// WHY: the earlier first-group-decides rule linked only one branch, leaving a later branch's exact
///      type unjoined before defaults; a single shared visitor reaches every branch.
fn visit_fallback_witnesses(
    body: &CatchHandlerBody,
    slot_shapes: &[KnownShapeId],
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) {
    state.with_detached_continuations(|continuations, state| {
        FallbackWalk {
            continuations,
            slot_shapes,
            environment,
            expression_builder,
            const_values,
        }
        .visit_body(body, state);
    });
}

/// The inputs of one fallback walk over a catch body.
///
/// WHAT: the detached continuation table, the protected slots' shapes in authored order, and the
///       environment every witness reads.
struct FallbackWalk<'walk> {
    continuations: &'walk DetachedContinuations,
    slot_shapes: &'walk [KnownShapeId],
    environment: &'walk TypeEnvironment,
    expression_builder: &'walk AstExpressionBuilder,
    const_values: &'walk ConstValueStore,
}

impl FallbackWalk<'_> {
    /// Applies a catch body's reachable fallback groups and reports how the body exits.
    fn visit_body(
        &self,
        body: &CatchHandlerBody,
        state: &mut NumericConstructionState,
    ) -> BranchExitSummary {
        match body {
            CatchHandlerBody::Ready(nodes) => self.visit_built_nodes(nodes, state),
            CatchHandlerBody::Draft { body: deferred, .. } => {
                self.visit_deferred_body(deferred, state)
            }
            CatchHandlerBody::PendingThen { continuation } => {
                self.visit_pending_entry(*continuation, state)
            }
        }
    }

    /// Applies every reachable built `then` group, then reports how the node list exits.
    fn visit_built_nodes(
        &self,
        nodes: &[AstNode],
        state: &mut NumericConstructionState,
    ) -> BranchExitSummary {
        let visited: Result<(), Infallible> = visit_reachable_then_values(
            nodes,
            &mut |produced: &ProducedValues| {
                self.apply_built_group(produced.expressions.as_slice(), state);
                Ok(())
            },
            self.expression_builder,
            self.const_values,
        );
        match visited {
            Ok(()) => {}
            Err(never) => match never {},
        }
        analyze_branch_exits(nodes, self.expression_builder, self.const_values)
    }

    /// Applies a retained body's entries in authored order, stopping once no path falls through.
    fn visit_deferred_body(
        &self,
        body: &DeferredBody,
        state: &mut NumericConstructionState,
    ) -> BranchExitSummary {
        let mut exits = BranchExitSummary::FALLS_THROUGH;
        for entry in &body.draft.entries {
            let entry_exits = match entry {
                BodyEntry::Node(node) => self.visit_built_nodes(std::slice::from_ref(node), state),
                BodyEntry::Pending(id) => self.visit_pending_entry(*id, state),
            };
            exits = exits.then_sequence(entry_exits);
            if !exits.can_fall_through {
                break;
            }
        }
        exits
    }

    /// Applies one pending continuation's fallback witnesses and reports how it exits.
    ///
    /// WHY: a `then` produces and a `return` terminates. A pending `assert` falls through, following
    ///      the owner's convention. A missing continuation is a broken invariant that body
    ///      completion reports, so the walk stops here without supplying a constraint.
    fn visit_pending_entry(
        &self,
        id: NumericContinuationId,
        state: &mut NumericConstructionState,
    ) -> BranchExitSummary {
        let Some(continuation) = self.continuations.get(id) else {
            return BranchExitSummary::TERMINATES;
        };
        match continuation {
            NumericContinuation::Binding(_)
            | NumericContinuation::Assignment(_)
            | NumericContinuation::CompoundAssignment(_)
            | NumericContinuation::ExpressionStatement(_)
            | NumericContinuation::MultiBind(_)
            | NumericContinuation::Assertion(_) => BranchExitSummary::FALLS_THROUGH,
            NumericContinuation::Return(_) => BranchExitSummary::TERMINATES,
            NumericContinuation::ThenValue(values) => {
                self.apply_known_group(values, state);
                BranchExitSummary::PRODUCES
            }
            NumericContinuation::Construct(construct) => {
                self.visit_pending_construct(construct, state)
            }
        }
    }

    /// Applies a retained `if` or `match`'s every branch and unions their exits.
    ///
    /// WHY: every branch is applied, so a later branch's exact type reaches its slot too. A loop
    ///      falls through as the owner's exit facts assume and supplies no constraint.
    fn visit_pending_construct(
        &self,
        construct: &DeferredConstruct,
        state: &mut NumericConstructionState,
    ) -> BranchExitSummary {
        match construct {
            DeferredConstruct::If(deferred_if) => {
                let then_exits = self.visit_deferred_body(&deferred_if.then_body, state);
                let else_exits = match &deferred_if.else_body {
                    Some(else_body) => self.visit_deferred_body(else_body, state),
                    None => BranchExitSummary::FALLS_THROUGH,
                };
                then_exits.union(else_exits)
            }
            DeferredConstruct::Match(deferred_match) => {
                let bodies = deferred_match
                    .arms
                    .iter()
                    .map(|arm| &arm.body)
                    .chain(deferred_match.default.as_ref());
                let mut exits: Option<BranchExitSummary> = None;
                for body in bodies {
                    let body_exits = self.visit_deferred_body(body, state);
                    exits = Some(match exits {
                        Some(joined) => joined.union(body_exits),
                        None => body_exits,
                    });
                }
                exits.unwrap_or(BranchExitSummary::FALLS_THROUGH)
            }
            DeferredConstruct::Loop(_) => BranchExitSummary::FALLS_THROUGH,
        }
    }

    /// Applies one built `then` group: each slot takes the exact type of the value it holds.
    ///
    /// WHY: a group of the call's declared arity supplies one value per slot; any other arity
    ///      supplies no value at any slot.
    fn apply_built_group(&self, values: &[ExprId], state: &mut NumericConstructionState) {
        if values.len() != self.slot_shapes.len() {
            return;
        }
        for (shape, fallback) in self.slot_shapes.iter().copied().zip(values) {
            let fallback = self
                .expression_builder
                .view(*fallback)
                .expect("completed fallback belongs to expression owner");
            if let Ok(type_id) = fallback.value_type_id() {
                register_shape_witness(state, self.environment, shape, type_id, fallback.span());
            }
        }
    }

    /// Applies one pending `then` group: each slot takes the witness its value supplies.
    fn apply_known_group(
        &self,
        values: &ThenValueContinuation,
        state: &mut NumericConstructionState,
    ) {
        if values.values.len() != self.slot_shapes.len() {
            return;
        }
        for (index, shape) in self.slot_shapes.iter().copied().enumerate() {
            if let Some(witness) =
                known_group_witness(values, index, state, self.expression_builder)
            {
                self.apply_witness(witness, shape, state);
            }
        }
    }

    /// Applies one slot's witness: an exact type registers, and a linked origin or shape joins.
    fn apply_witness(
        &self,
        witness: FallbackWitness,
        shape: KnownShapeId,
        state: &mut NumericConstructionState,
    ) {
        match witness {
            FallbackWitness::Exact(found) => {
                register_shape_witness(state, self.environment, shape, found.type_id, found.span);
            }
            FallbackWitness::LinkedBinding(binding) => {
                union_pending_binding_with_generic_slot(state, self.environment, binding, shape);
            }
            FallbackWitness::LinkedShape(fallback_shape) => {
                // A shape pair that does not join stays separate, as for any refused numeric pair.
                union_shape_classes(state, self.environment, fallback_shape, shape);
            }
        }
    }
}

/// The witness a pending `then` group gives one slot of the protected value, when one is known.
///
/// WHAT: a built or completed value has its own exact type. A retained value takes the receiving
///       slot's known type, else the concrete type its own result shape or single call declares,
///       else the exact type of the one binding it names directly, else a link to its numeric or
///       structured result shape, else a link to that binding. A taken value supplies only the
///       receiving slot's known type.
/// WHY: the protected value takes its type from a receiving use or a declared concrete result that
///      already exists in the body, never from a callee body or from a default chosen for an
///      unknown fallback. A binding with no type yet is linked instead, so the protected value and
///      that binding share one class before body-root defaults.
fn known_group_witness(
    values: &ThenValueContinuation,
    index: usize,
    state: &NumericConstructionState,
    expression_builder: &AstExpressionBuilder,
) -> Option<FallbackWitness> {
    let value = values.values.get(index)?;
    let expected = values.slot_expected_types.get(index).copied().flatten();
    let expected_group = |type_id: TypeId| ExactFallbackType {
        type_id,
        span: None,
    };
    match value {
        ThenValueSlot::Pending(StatementExpression::Built(fallback))
        | ThenValueSlot::Completed(fallback) => {
            let fallback = expression_builder
                .view(*fallback)
                .expect("completed fallback belongs to expression owner");
            fallback.value_type_id().ok().map(|type_id| {
                FallbackWitness::Exact(ExactFallbackType {
                    type_id,
                    span: fallback.span(),
                })
            })
        }
        ThenValueSlot::Pending(StatementExpression::Retained(deferred)) => expected
            .map(expected_group)
            .or_else(|| retained_concrete_witness(deferred, state))
            .or_else(|| direct_binding_witness(deferred, state))
            .map(FallbackWitness::Exact)
            .or_else(|| retained_shape_witness(deferred))
            .or_else(|| linked_binding_witness(deferred)),
        ThenValueSlot::Taken => expected.map(expected_group).map(FallbackWitness::Exact),
    }
}

/// The linked witness of the one binding a retained fallback names directly.
///
/// WHAT: a direct reference to a binding that has no exact type yet. A computed or literal fallback
///       names no binding and links nothing.
/// WHY: a catch fallback that reads a shared origin ties the protected value to that origin, so the
///      origin must join the protected shape before body-root defaults choose its type.
fn linked_binding_witness(deferred: &DeferredNumericExpression) -> Option<FallbackWitness> {
    single_direct_pending_reference(&deferred.items)
        .map(|reference| FallbackWitness::LinkedBinding(reference.binding))
}

/// The linked witness of a retained fallback whose result shape is numeric or structured.
///
/// WHAT: a retained call or literal with a non-concrete result shape, such as a generic call over a
///       numeric origin, joins its shape with the protected slot through the structural union.
/// WHY: the fallback's type is the origin's type, so both must share one class before body-root
///      defaults. A shape pair that does not join stays separate, so no default is invented.
fn retained_shape_witness(deferred: &DeferredNumericExpression) -> Option<FallbackWitness> {
    retained_known_shape(&deferred.items).map(FallbackWitness::LinkedShape)
}

/// The exact receiving witness of the one binding a retained fallback names directly.
///
/// WHAT: follows the fragment's one direct reference through its groups. A binding whose known
///       shape is concrete supplies that type; otherwise the numeric class of that binding, or of
///       its numeric known shape, supplies its first receiving type. Other shapes supply nothing.
/// WHY: a direct receiving use of a named origin has already fixed its exact type, so the
///      protected raw fragment takes that type before the literal default. A computed fallback
///      is not a direct use, so it supplies nothing.
fn direct_binding_witness(
    deferred: &DeferredNumericExpression,
    state: &NumericConstructionState,
) -> Option<ExactFallbackType> {
    let reference = single_direct_pending_reference(&deferred.items)?;
    let class = match binding_reference_shape(state, reference.binding) {
        None => binding_class(state, reference.binding),
        Some(shape) => match state.shape(shape) {
            KnownShape::Numeric(class) => class,
            KnownShape::Concrete { type_id, .. } => {
                return Some(ExactFallbackType {
                    type_id,
                    span: reference.span,
                });
            }
            _ => return None,
        },
    };
    let type_id = state.first_receiver_type(class)?;
    Some(ExactFallbackType {
        type_id,
        span: reference.span,
    })
}

/// The exact type a retained fallback's own result already fixes, when that result is concrete.
///
/// WHAT: reads the shared retained result shape first: a literal's shape or a generic call's one
///       declared result, where only a concrete leaf answers. A retained single call with no shared
///       shape answers with its one concrete success type. Numeric, collection, map, and optional
///       shapes supply nothing.
/// WHY: a declared concrete result is exact before the call's generic parameters resolve, so the
///      protected raw fallback takes that type without solving the callee or reading its body.
fn retained_concrete_witness(
    deferred: &DeferredNumericExpression,
    state: &NumericConstructionState,
) -> Option<ExactFallbackType> {
    match retained_known_shape(&deferred.items) {
        Some(shape) => match state.shape(shape) {
            KnownShape::Concrete { type_id, span } => Some(ExactFallbackType { type_id, span }),
            _ => None,
        },
        None => retained_single_call_result(&deferred.items),
    }
}

/// The concrete success type of a retained single call, read from its declared signature.
///
/// WHAT: a call whose declared result is one concrete type, reached through parenthesised groups.
/// WHY: the declared return type is exact before the pending arguments complete. Reading it does
///      not complete the call, read the callee body, or allocate a shape.
fn retained_single_call_result(items: &[ExpressionRpnItem]) -> Option<ExactFallbackType> {
    match single_operand(items) {
        Some(ExpressionRpnItem::PendingNumericCall { call, .. }) => Some(ExactFallbackType {
            type_id: call.single_success_return_type_id()?,
            span: call.call_span(),
        }),
        _ => None,
    }
}

/// Applies the ordinary receiving boundary to every authored `then` group of a completed body.
///
/// WHAT: walks every value-producing group — top level and nested inside `if`/`match` or a lexical
///       scope, including groups after a terminal statement — and validates exact arity plus a
///       per-slot coercion against the resolved catch slots.
/// WHY: a concrete fallback parsed before the protected slot was known, or while another statement
///      made the handler a draft, must still receive the authored diagnostic instead of reaching
///      HIR as a mismatched write. Statically inactive and dead groups are erased only at module
///      finalization, so they cannot skip this check. Value blocks with their own receivers live
///      inside expressions, so the walk never enters an unrelated nested receiver.
fn validate_completed_handler_then_values(
    nodes: &mut [AstNode],
    success_result_type_ids: &[TypeId],
    services: &mut DeferredExpressionServices<'_, '_>,
) -> Result<(), ExpressionParseError> {
    visit_authored_then_values_mut(nodes, &mut |produced| {
        validate_and_coerce_completed_then_group(
            produced,
            success_result_type_ids,
            ValueReceiverKind::CatchHandler,
            services,
        )
    })
}

/// Derives the catch site of a protected value that only resolved at the body root.
///
/// WHAT: a carried value keeps its recovery representation; any other completed value derives the
///       compatible producer and success slots its completed-expression parse derives.
/// WHY: a retained protected value skips the parse-time site derivation, so completion must run
///      exactly the derivation the authored site would have run.
fn resolved_catch_site(
    expression: OwnedExpr,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<(OwnedExpr, TypeId, Vec<TypeId>), ExpressionParseError> {
    match producer_catch_site(expression, expression_builder)? {
        ProducerCatchOutcome::Producer(producer) => producer.recovery(expression_builder),
        ProducerCatchOutcome::NoErrorChannel(expression) => {
            let span = expression_builder.view_owned(expression)?.span();
            completed_catch_attachment(
                expression,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                span,
            )
        }
    }
}

/// The authored `|name|` of a catch handler, before the handler scope exists.
struct ParsedCatchErrorBinding {
    name: StringId,
    span: Option<SourceSpan>,
}

/// Parses a `catch |err|:` handler with an explicit error binding.
///
/// WHAT: consumes the `|identifier|` binding syntax and the subsequent handler body,
/// validates the binding name, and produces a typed handler outcome.
/// WHY: expression and call fallible handling both use this entrypoint when the user
/// supplies an error variable, and a handler body that retained numeric continuations must
/// travel to the body root rather than resolve here.
#[allow(
    clippy::too_many_arguments,
    reason = "catch handler keeps stream, scope, site, warnings and mutable parse resources apart"
)]
pub(crate) fn parse_catch_fallible_handler_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };

    let error_binding =
        parse_catch_error_binding(token_stream, context, &site, warnings, string_table)?;

    parse_catch_fallible_handler_body(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        site,
        Some(error_binding),
        warnings,
        string_table,
        path_fork,
    )
}

/// Parses a `catch:` handler without an error binding.
#[allow(
    clippy::too_many_arguments,
    reason = "catch without binding keeps stream, scope, site, warnings and mutable parse resources apart"
)]
pub(crate) fn parse_catch_without_error_binding_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };

    parse_catch_fallible_handler_body(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        site,
        None,
        warnings,
        string_table,
        path_fork,
    )
}

/// Parses the `|identifier|` error binding inside a catch handler.
///
/// WHAT: validates bracket tokens, extracts the handler identifier, runs naming and
/// scope-conflict checks, and returns the binding name for the handler scope to bind.
fn parse_catch_error_binding(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    site: &CatchFallibleHandlerSite<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<ParsedCatchErrorBinding, ExpressionParseError> {
    if token_stream.current_tag() != TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerOpeningPipe,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();

    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::EmptyCatchHandlerBinding,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() != TokenTag::SYMBOL {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let Some(handler_name) = token_stream.current_string_id_in(string_table)? else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier,
            Some(token_stream.current_span()),
        )
        .into());
    };

    let handler_name_span = Some(token_stream.current_span());
    validate_catch_fallible_handler_binding(
        handler_name,
        handler_name_span,
        site.compilation_stage,
        warnings,
        string_table,
    )?;

    validate_catch_fallible_handler_conflict(
        context,
        handler_name,
        handler_name_span,
        string_table,
    )?;

    token_stream.advance();

    if token_stream.current_tag() == TokenTag::COMMA {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::MultipleCatchHandlerBindings,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() != TokenTag::TYPE_PARAMETER_BRACKET {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerClosingPipe,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();

    Ok(ParsedCatchErrorBinding {
        name: handler_name,
        span: handler_name_span,
    })
}

/// Declares the caught error as a local of the handler scope.
///
/// WHY: the binding path is interned under the handler's own child scope, not the scope
/// holding the handled expression. The binding is visible only inside its handler, so
/// sibling handlers may reuse a name, and HIR keys function locals by this path.
fn bind_catch_error(
    handler_context: &mut ScopeContext,
    error: &ParsedCatchErrorBinding,
    error_return: GenericErrorReturn,
    path_fork: &mut PathInternerFork,
) -> Result<CatchErrorBinding, ExpressionParseError> {
    let error_binding = path_fork
        .try_intern_child(handler_context.scope, error.name)
        .expect("path table exhausted while creating catch binding");

    match error_return {
        GenericErrorReturn::Concrete(error_type_id) => {
            handler_context.add_var(
                Declaration {
                    id: error_binding,
                    type_id: error_type_id,
                    type_syntax: None,
                    value_mode: ValueMode::ImmutableOwned,
                    value: DeclarationValue::Absent,
                    binding_span: error.span,
                    config_qualifier: None,
                },
                error.span,
                path_fork,
            );
        }
        // A numeric-dependent error has no type until completion, so the binding is a pending row
        // over the producer's error shape. Its type materialises when the handler body resolves
        // that shape; no builtin `Error` and no parse-time default is involved.
        GenericErrorReturn::Retained(shape) => {
            handler_context.declare_pending_caught_error(
                AuthoredBindingFacts {
                    name: error.name,
                    path: error_binding,
                    span: error.span,
                    binding_span: error.span,
                    value_mode: ValueMode::ImmutableOwned,
                    mutable: false,
                },
                shape,
            )?;
        }
    }

    Ok(CatchErrorBinding { error_binding })
}

/// Parses the colon and statement body of a catch handler.
///
/// WHAT: validates the leading colon, sets up a child control-flow context with the
/// error variable bound, parses the handler body, and validates value-production requirements.
/// WHY: this is the shared backend for both binding and no-binding catch handlers.
#[allow(
    clippy::too_many_arguments,
    reason = "catch body parsing keeps the token stream, scope, mutable interner/string/path state, handler site, error binding, and warning sink as separate borrows"
)]
fn parse_catch_fallible_handler_body(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    error: Option<ParsedCatchErrorBinding>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    if token_stream.current_tag() != TokenTag::COLON {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerColon,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut handler_context =
        context.new_child_control_flow(ContextKind::CatchHandler, string_table, path_fork);
    // Even an error-only catch owns its zero success slots. A `then` must diagnose that
    // contract rather than target an enclosing value block or report a crossed boundary. A
    // pending contract parses against its declared arity of inferred slots, and an unresolved
    // one against one inferred slot; completion revalidates the body against the resolved site.
    handler_context.active_value_target = Some(site.success_contract.handler_target());

    let error = error
        .map(|parsed| bind_catch_error(&mut handler_context, &parsed, site.error_return, path_fork))
        .transpose()?;

    let handler_scope = handler_context.scope;

    token_stream.advance();

    let parsed_body = function_body_to_ast(
        token_stream,
        handler_context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
    )?;

    // A handler body shares the enclosing body root's numeric domain, so its parse returns the
    // authored-order draft. A draft that retained no numeric continuation converts straight to
    // nodes; one that did keeps every retained statement (never fabricating, defaulting or
    // dropping an entry) for the enclosing root to rebuild at this catch's own site.
    let handler_outcome = match parsed_body {
        ParsedBody::Complete(nodes) => {
            ParsedCatchHandler::Ready(CatchFallibleHandler { error, body: nodes })
        }
        ParsedBody::Draft(draft) => {
            if draft.has_pending() {
                ParsedCatchHandler::Retained(Box::new(RetainedCatchHandler {
                    error,
                    body: CatchHandlerBody::Draft {
                        body: DeferredBody {
                            draft,
                            scope: handler_scope,
                        },
                        value_required: site.value_required,
                        value_required_span: site.value_required_span,
                    },
                }))
            } else {
                ParsedCatchHandler::Ready(CatchFallibleHandler {
                    error,
                    body: draft.into_nodes_for_host("a catch-handler body")?,
                })
            }
        }
    };

    if let ParsedCatchHandler::Ready(handler) = &handler_outcome {
        validate_catch_fallible_handler_value_requirement(
            site.value_required,
            site.success_contract.known(),
            &handler.body,
            expression_builder,
            const_values,
            site.value_required_span,
        )?;
    }

    Ok(handler_outcome)
}

/// Parses inline `catch then ...` recovery without an error binding.
///
/// WHAT: builds the same one-node handler body as block-form `catch: then ... ;`.
/// WHY: inline catch is only sugar at receiving sites; keeping the body as `ThenValue`
/// lets AST/HIR reuse the value-block catch path without catch-specific fallback fields.
#[allow(
    clippy::too_many_arguments,
    reason = "inline catch keeps stream, scope, site and mutable parse resources apart"
)]
pub(super) fn parse_inline_catch_without_error_binding_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    parse_inline_catch_handler_body(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        site,
        None,
        string_table,
        path_fork,
    )
}

/// Parses inline `catch |err| then ...` recovery with an error binding.
///
/// WHAT: validates and binds `err`, then parses the single inline produced-value list.
/// WHY: the inline shorthand must preserve the same scoping and conflict rules as
/// block-form catch handlers.
#[allow(
    clippy::too_many_arguments,
    reason = "inline catch keeps stream, scope, site, warnings and mutable parse resources apart"
)]
pub(super) fn parse_inline_catch_fallible_handler_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };
    let error_binding =
        parse_catch_error_binding(token_stream, context, &site, warnings, string_table)?;

    parse_inline_catch_handler_body(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        site,
        Some(error_binding),
        string_table,
        path_fork,
    )
}
#[allow(
    clippy::too_many_arguments,
    reason = "inline catch body keeps stream, scope, site, error binding and mutable parse resources apart"
)]
fn parse_inline_catch_handler_body(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    site: CatchFallibleHandlerSite<'_>,
    error: Option<ParsedCatchErrorBinding>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCatchHandler, ExpressionParseError> {
    if token_stream.current_tag() == TokenTag::NEWLINE {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::InlineCatchMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_tag() != TokenTag::THEN {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // A known-empty contract keeps the ordinary error-only fallback diagnostic; an unresolved
    // contract defers that decision to completion, where the resolved site decides it.
    if !site.value_required
        || matches!(
            site.success_contract.known(),
            Some(success_result_type_ids) if success_result_type_ids.is_empty()
        )
    {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let then_span = Some(token_stream.current_span());
    token_stream.advance();

    if token_stream.current_tag() == TokenTag::NEWLINE {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::InlineCatchMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if is_missing_produced_value_boundary(token_stream.current_tag()) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ThenRequiresValues,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut handler_context =
        context.new_child_control_flow(ContextKind::CatchHandler, string_table, path_fork);
    let active_target = site.success_contract.handler_target();
    handler_context.active_value_target = Some(active_target.clone());

    let error = error
        .map(|parsed| bind_catch_error(&mut handler_context, &parsed, site.error_return, path_fork))
        .transpose()?;

    // The target owns the receiving contract, one entry per produced value (a known site's resolved
    // slots, a provisional site's single `None`); every fallback parses through the owning producer,
    // so a numeric-dependent one raises its deferral and stays retained while concrete values
    // materialise eagerly at the contract's slots.
    let slot_expected_types = active_target.slot_expected_types();
    let values = parse_retained_produced_values(ProducedValuesParseInput {
        token_stream,
        context: &handler_context,
        type_interner,
        expression_builder,
        const_values,
        target: &active_target,
        string_table,
        path_fork,
    })?;

    if token_stream.current_tag() == TokenTag::CATCH {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if values
        .iter()
        .all(|value| matches!(value, StatementExpression::Built(_)))
    {
        // Every fallback materialised against the receiving contract, so the authored `then` node
        // is the whole handler body. The eager check above makes the retained arm unreachable.
        let mut expressions = Vec::with_capacity(values.len());
        for value in values {
            if let StatementExpression::Built(expression) = value {
                expressions.push(expression);
            }
        }
        let body = vec![AstNode {
            kind: NodeKind::ThenValue(ProducedValues {
                expressions,
                span: then_span,
            }),
            span: then_span,
            scope: handler_context.scope,
        }];

        return Ok(ParsedCatchHandler::Ready(CatchFallibleHandler {
            error,
            body,
        }));
    }

    // A numeric-dependent fallback stays owned by this producer and resumes at the stored
    // contract: the handler travels to the body root with the protected value.
    let continuation = handler_context.register_numeric_continuation(
        NumericContinuation::ThenValue(ThenValueContinuation {
            values: values.into_iter().map(ThenValueSlot::Pending).collect(),
            slot_expected_types,
            receiver_kind: active_target.receiver_kind,
            span: then_span,
            scope: handler_context.scope,
        }),
    )?;

    Ok(ParsedCatchHandler::Retained(Box::new(
        RetainedCatchHandler {
            error,
            body: CatchHandlerBody::PendingThen { continuation },
        },
    )))
}
