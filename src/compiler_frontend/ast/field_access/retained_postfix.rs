//! Retained postfix continuations for numeric-dependent operands.
//!
//! WHAT: retains the authored member/method steps that follow a pending operand whose result type
//!       is already known, and the authored `!`/`?` handling suffix authored at a pending
//!       operand's own position; body completion then resumes the ordinary field/method
//!       constructors and the shared handling-suffix applier once the operand's value exists.
//! WHY: a retained call or binding has no value yet, but its declared result type already fixes
//!      every member lookup on it (`next -> Node` is known before `count` resolves), and an
//!      authored suffix cannot apply before that value exists while the enclosing expression must
//!      keep collecting its authored operators. Keeping steps and suffix at the operand position
//!      replays one constructor per authored step instead of fabricating a receiver value or type,
//!      and never ends the fragment early or drops authored arithmetic.
//!
//! A rejected member ends the chain as one deferred occurrence. Completion issues its ordinary
//! diagnostic on the receiver it has by then. An unhandled fallible step is recovered before the
//! next step consumes only its success shape.

use super::PostfixChainAccess;
use super::builtin_call_args::{
    BuiltinCallSite, builtin_argument_expectations, builtin_argument_slots,
    builtin_result_type_ids, builtin_success_type_ids, concrete_shape_type, intern_concrete_shape,
    materialise_builtin_shape, resolve_builtin_member_arguments,
};
use super::collection_builtin::{
    CollectionBuiltinReceiver, collection_builtin_op_source_name, resolve_collection_builtin_member,
};
use super::field_member::{
    ResolvedFieldMember, parse_member_name_typed, resolve_field_member_for_known_receiver,
};
use super::map_builtin::{MapBuiltinReceiver, resolve_map_builtin_member};
use super::parse_chain::{
    collect_known_member_names, pending_numeric_member_access_error, previous_token_span,
    type_id_is_external,
};
use super::receiver_access::{
    ReceiverAccessDiagnostic, reject_temporary_receiver_access, validate_place_receiver_access,
};
use super::receiver_calls::{
    DeferredCallReceiver, DeferredReceiverCall, DeferredReceiverCallee, ReservedGenericRequest,
    complete_deferred_receiver_call, generic_parameter_receiver_type, lookup_receiver_method,
    recover_pending_producer,
};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, expected_type_for_resolved_parameter,
    parse_call_arguments_typed_with_expectations,
    parse_call_arguments_typed_with_numeric_retention, parse_call_arguments_without_expectations,
    parse_generic_call_arguments_typed,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    ExpectedParameterType, expectations_from_receiver_method_signature,
    generic_template_expectations,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand, single_operand_mut, trailing_operand_start,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::parse_expression::validate_completed_typed_errors;
use crate::compiler_frontend::ast::generic_functions::numeric_origins::{
    GenericErrorReturn, GenericNumericSignatureInput, declared_field_shape,
    link_generic_numeric_signature,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericReceiverMethodInstantiationInput,
    GenericReceiverMethodReceiver, instantiate_generic_receiver_method,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredCallHandling, DeferredExpression,
    DeferredExpressionCompletion, DeferredExpressionSuffix, NumericRecipe, StatementExpression,
    binding_reference_shape, retained_known_shape,
};
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShape, KnownShapeId};
use crate::compiler_frontend::ast::statements::body_symbol;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    DeferredCatchHandler, FUNCTION_CALL_STAGE, FallibleSuccessContract, ParsedFallibleSuffix,
    apply_completed_handling_suffix, parse_fragment_handling_suffix,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::{
    InvalidAssignmentTargetReason, InvalidBuiltinCallReason, InvalidFieldAccessReason,
    InvalidReceiverCallReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// One authored postfix step retained for body completion.
///
/// WHAT: a resolved field projection or a resolved source receiver method call, each carrying the
///       exact facts its ordinary constructor needs.
/// WHY: the receiver value does not exist at parse time, so the step keeps its resolved identity
///      and authored spans; completion builds the same node the completed chain would have built.
#[derive(Debug, Clone)]
// The field step stays inline: boxing it would add a heap allocation to every retained field
// projection, while the method and builtin steps are already boxed.
#[allow(clippy::large_enum_variant)]
pub(crate) enum RetainedPostfixStep {
    Field(RetainedFieldStep),
    /// The call facts are the same record the completed chain call would build; only its receiver
    /// is the preceding step's completed value (`DeferredCallReceiver::Chained`).
    Method(Box<DeferredReceiverCall>),
    /// One resolved collection builtin member whose receiver value does not exist yet.
    Builtin(Box<RetainedBuiltinStep>),
    /// One declared envelope field projection, resolved on the concrete owner at completion.
    DeclaredField(RetainedDeclaredFieldStep),
}

/// One resolved collection builtin member step retained for completion.
///
/// WHAT: retains the resolved operation, authored member span, parsed arguments, and the
///       construction shapes its parameters and result derive from.
/// WHY: the member resolves against the receiver's declared shape, which the pending operand
///      already fixes, and its construction facts come from the same collection builtin owner the
///      completed chain uses — no second member grammar and no numeric candidate search.
#[derive(Debug, Clone)]
pub(crate) struct RetainedBuiltinStep {
    pub(crate) kind: RetainedBuiltinKind,
    pub(crate) member_name: StringId,
    pub(crate) member_span: Option<SourceSpan>,
    pub(crate) arguments: ParsedCallArguments,
    /// The operation's fixed parameter shapes; completion materialises them into the same
    /// expectations the completed lane validates against once every leaf is concrete.
    pub(crate) expected_argument_shapes: Vec<KnownShapeId>,
    /// The success slot as a construction shape; `None` when the operation yields no value.
    /// Its canonical success type materialises at body-root completion.
    pub(crate) success_shape: Option<KnownShapeId>,
    /// The declared error an authored `!` unwraps; `None` for an infallible operation.
    pub(crate) error_return_type_id: Option<TypeId>,
    /// The authored handling consumed at the operand position (`!` only; a `catch` stays with the
    /// enclosing fragment owner).
    pub(crate) authored_handling: DeferredCallHandling,
}

/// The compiler-owned builtin family one retained step resumes through.
///
/// WHAT: the resolved operation of the collection or map builtin family, exactly as the completed
///       chain resolved it from the receiver's canonical shape.
/// WHY: both families resolve their member once and then share the retained-step machinery, so the
///      step carries the family's own operation instead of a second dispatch decision at
///      completion.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RetainedBuiltinKind {
    Collection(CollectionBuiltinOp),
    Map(MapBuiltinOp),
}

impl RetainedBuiltinKind {
    /// The canonical source name of the resolved operation.
    ///
    /// WHAT: the recognized builtin spelling (no aliases), taken from the operation owner.
    /// WHY: the builtin-member diagnostic surface at completion needs the operation's name, and the
    ///      resolved operation already fixes it — no authored-name resolution, allocation or second
    ///      name table is required.
    pub(crate) fn source_name(self) -> &'static str {
        match self {
            RetainedBuiltinKind::Collection(op) => collection_builtin_op_source_name(op),
            RetainedBuiltinKind::Map(op) => op.source_name(),
        }
    }
}

/// Known place facts of the receiver at one retained postfix step.
///
/// WHAT: whether the current receiver is an existing binding place or a field projection of one,
///       and, when it is, the root binding's declaration mutability.
/// WHY: ordinary non-inlined field access preserves the root place; calls, inlined fields and
///      computed values do not. Retained access must follow that same projection boundary.
#[derive(Clone, Copy)]
struct RetainedReceiverAccess {
    is_place: bool,
    is_mutable: bool,
}

impl RetainedReceiverAccess {
    fn temporary() -> Self {
        Self {
            is_place: false,
            is_mutable: false,
        }
    }
}

/// The receiver a retained member chain resumes from.
///
/// WHAT: the completed receiver shape together with the place access the chain root grants.
/// WHY: both facts describe one receiver, so they travel together into the step walk.
struct RetainedChainRoot {
    shape: KnownShapeId,
    access: RetainedReceiverAccess,
}

/// Classify one retained chain's root receiver item.
fn retained_chain_root(item: &ExpressionRpnItem) -> RetainedReceiverAccess {
    match item {
        ExpressionRpnItem::PendingNumericBinding {
            value_mode,
            is_copy: false,
            ..
        } => RetainedReceiverAccess {
            is_place: true,
            is_mutable: value_mode.is_mutable(),
        },
        _ => RetainedReceiverAccess::temporary(),
    }
}

/// Validate one authored step's receiver access against the receiver it actually has.
///
/// WHAT: an existing place or non-inlined projection uses the place policy; calls, inlined fields
///       and other computed values use the temporary policy.
/// WHY: the retained state follows the same field-projection boundary as ordinary place access,
///      while only the root binding's declaration fixes mutability.
fn validate_retained_step_access(
    receiver: RetainedReceiverAccess,
    chain_access: &PostfixChainAccess,
    requires_mutable: bool,
    diagnostic: &ReceiverAccessDiagnostic,
    member_span: Option<SourceSpan>,
) -> Result<(), DiagnosticDraft> {
    if receiver.is_place {
        return validate_place_receiver_access(
            chain_access.mode,
            chain_access.authored_marker_span,
            requires_mutable,
            receiver.is_mutable,
            None,
            diagnostic,
            member_span,
        );
    }

    reject_temporary_receiver_access(
        chain_access.mode,
        chain_access.authored_marker_span,
        requires_mutable,
        diagnostic,
        member_span,
    )
}

/// One resolved field projection retained on a pending receiver.
#[derive(Debug, Clone)]
pub(crate) struct RetainedFieldStep {
    pub(crate) field_name: StringId,
    pub(crate) type_id: TypeId,
    pub(crate) value_mode: ValueMode,
    pub(crate) const_record_state: ConstRecordState,
    /// A declaration-level const field inlines its compile-time value exactly as the completed
    /// chain does; the pending receiver has no const-record projection of its own.
    pub(crate) const_inline_value: Option<ExprId>,
    pub(crate) span: Option<SourceSpan>,
}

/// One declared-envelope field projection retained on a pending receiver.
///
/// WHAT: the authored field name, the projected field's known shape, and its authored span.
/// WHY: a declared envelope's owner is generic, so the field's concrete type exists only once
///      the receiver completes. The shape lets the retained chain continue, and completion
///      resolves the field on the concrete owner through the ordinary field owner.
#[derive(Debug, Clone)]
pub(crate) struct RetainedDeclaredFieldStep {
    pub(crate) field_name: StringId,
    pub(crate) result_shape: KnownShapeId,
    pub(crate) span: Option<SourceSpan>,
}

/// The retained steps one authored member chain produced, and the rejected member it ended on.
///
/// WHAT: `steps` is the chain resolved before the first member no receiver type resolves.
///       `invalid_member` carries that member so completion reports the ordinary diagnostic on the
///       receiver's independently resolved type.
/// WHY: a later rejected member must not discard the resolved steps before it, because the
///      receiver's type is known only at completion and an authored suffix still follows the chain.
struct RetainedPostfixSteps {
    steps: Vec<RetainedPostfixStep>,
    invalid_member: Option<body_symbol::PendingNumericMemberAccess>,
}

/// One retained operand whose authored postfix continuation applies before the enclosing
/// expression's remaining operators evaluate.
///
/// WHAT: the pending operand items, its authored member steps, its authored handling suffix and the
///       evaluation context the operand's own fragment completes with.
/// WHY: fragment completion replaces this item with one operand value, so the following
///      operators keep their authored RPN positions while the suffix applies to exactly this
///      operand. The payload lives with the postfix owner. The shared enum only carries it.
#[derive(Debug, Clone)]
pub(crate) struct RetainedOperandContinuation {
    pub(crate) operand: Vec<ExpressionRpnItem>,
    pub(crate) steps: Vec<RetainedPostfixStep>,
    pub(crate) suffix: DeferredExpressionSuffix,
    pub(crate) expected: ExpectedType,
    pub(crate) value_mode: ValueMode,
    /// Set when this operand stands for a member access already recorded for its delayed ordinary
    /// diagnostic.
    ///
    /// WHAT: the occurrence's own facts (receiver binding, authored member name and span).
    /// WHY: the member exists at no numeric type, so the operand has no value and no result shape —
    ///      no direct-reference witness, alias classification or statement value may read it, and
    ///      completion reports the occurrence's diagnostic on the independently resolved receiver
    ///      instead of fabricating one. `None` is an ordinary retained operand.
    pub(crate) invalid_member: Option<body_symbol::PendingNumericMemberAccess>,
}

/// The assignable member place one retained operand denotes.
///
/// WHAT: the member's field name, which an immutable-root diagnostic names, and the declared
///       envelope shape the written member occupies when the member is declared through a generic
///       parameter; `None` for a concrete member's shape.
/// WHY: a member write shares one static numeric shape with the value written into it, so a
///      declared member contributes its shape, and a concrete member already fixes its exact type.
///      An invalid member has no place and no name, so its ordinary diagnostic completes at the
///      target instead.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetainedMemberTarget {
    pub(crate) field_name: Option<StringId>,
    pub(crate) declared_shape: Option<KnownShapeId>,
    /// The exact type of a concrete member, which fixes the written value's receiving contract.
    pub(crate) concrete_type: Option<TypeId>,
}

/// Classify one retained operand as an assignable member place of a pending numeric root.
///
/// WHAT: accepts an operand whose root is a non-copy pending binding, whose steps are all field
///       projections that keep the root place, and which carries no authored handling suffix.
/// WHY: the authored `root.field...` chain completes as an ordinary field place, which an
///      assignment can write; a method, builtin, inlined constant or computed receiver has no place.
pub(crate) fn retained_member_target(item: &ExpressionRpnItem) -> Option<RetainedMemberTarget> {
    let ExpressionRpnItem::PendingNumericExpression { expression, .. } = item else {
        return None;
    };
    let DeferredExpression::OperandContinuation(payload) = expression.as_ref() else {
        return None;
    };
    let root_is_place = single_operand(&payload.operand)
        .is_some_and(|operand| retained_chain_root(operand).is_place);
    if !matches!(payload.suffix, DeferredExpressionSuffix::None) || !root_is_place {
        return None;
    }
    if payload.invalid_member.is_some() {
        return Some(RetainedMemberTarget {
            field_name: None,
            declared_shape: None,
            concrete_type: None,
        });
    }
    let (last, earlier) = payload.steps.split_last()?;
    if !earlier
        .iter()
        .chain(std::iter::once(last))
        .all(step_keeps_root_place)
    {
        return None;
    }
    let (field_name, declared_shape, concrete_type) = match last {
        RetainedPostfixStep::Field(field) => (field.field_name, None, Some(field.type_id)),
        RetainedPostfixStep::DeclaredField(field) => {
            (field.field_name, Some(field.result_shape), None)
        }
        RetainedPostfixStep::Method(_) | RetainedPostfixStep::Builtin(_) => return None,
    };
    Some(RetainedMemberTarget {
        field_name: Some(field_name),
        declared_shape,
        concrete_type,
    })
}

/// Whether one retained member step keeps the pending root's place.
///
/// WHAT: a field projection keeps the place unless it is an inlined constant; a declared field
///       always keeps it; a method or builtin step produces a computed value.
/// WHY: this mirrors the completed chain's place rule, so the classifier and completion agree.
fn step_keeps_root_place(step: &RetainedPostfixStep) -> bool {
    match step {
        RetainedPostfixStep::Field(field) => field.const_inline_value.is_none(),
        RetainedPostfixStep::DeclaredField(_) => true,
        RetainedPostfixStep::Method(_) | RetainedPostfixStep::Builtin(_) => false,
    }
}

/// The known result one retained operand's authored continuation already fixes.
///
/// WHAT: a retained call's single declared success slot, an authored group of one, a pending
///       binding whose known shape or live recipe is exactly one such item, and — for an operand
///       continuation — the last retained step's own result projected through its authored `!`/`?`
///       suffix. A result is either a concrete type or a construction-only shape.
/// WHY: member resolution and consumers of the produced value (a loop's collection source) may use
///      known structure, so one derivation serves both. Shapes stay construction-only until body
///      completion materialises them. Numeric origins report no shape, so the scalar diagnostic
///      path keeps owning them; no width, trait or member search selects a type here.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RetainedOperandResult {
    Type(TypeId),
    Shape(KnownShapeId),
}

/// Keeps a known result as a member receiver only when it names a non-scalar structure.
///
/// WHY: a numeric class is a scalar origin whose receiver diagnostics stay on the scalar path. The
///      exclusion is receiver-only: a known result is otherwise returned unfiltered, so directed
///      recipes and consumers can read a numeric shape. A declared envelope is a receiver: its
///      fields project through the declared owner's struct.
fn receiver_shape(scope_context: &ScopeContext, shape: KnownShapeId) -> Option<KnownShapeId> {
    match scope_context.numeric_state().shape(shape) {
        KnownShape::Numeric(_) => None,
        KnownShape::Optional { .. } => None,
        KnownShape::Concrete { .. }
        | KnownShape::Collection { .. }
        | KnownShape::Map { .. }
        | KnownShape::DeclaredSubstitution { .. } => Some(shape),
    }
}

/// One retained terminal call a fragment or multi-bind value reads its success and generic shape
/// from, whether it is a bare call or the terminal unhandled method of a retained chain.
///
/// WHAT: names the one call-like producer a consumer checks, without the operand chain around it.
/// WHY: a chained method and a bare call share one result contract, so the fragment, catch, and
///      multi-bind consumers read one view instead of matching each chain shape themselves.
#[derive(Clone, Copy)]
pub(crate) enum RetainedTerminalCall<'a> {
    Call(&'a DeferredCall),
    Method(&'a DeferredReceiverCall),
}

impl<'a> RetainedTerminalCall<'a> {
    /// The success slot types the call's signature declares, when that signature is concrete.
    pub(crate) fn success_return_type_ids(self) -> Option<Vec<TypeId>> {
        match self {
            Self::Call(call) => call.success_return_type_ids(),
            Self::Method(method) => method
                .concrete_signature()
                .map(|signature| signature.success_return_type_ids()),
        }
    }

    /// The declared success slot count, known even while the slot types are still generic.
    pub(crate) fn declared_success_slot_count(self) -> usize {
        match self {
            Self::Call(call) => call.declared_success_slot_count(),
            Self::Method(method) => method.declared_signature().success_return_slot_count(),
        }
    }

    /// The generic result shapes whose origins resolve at completion, when the call is generic.
    pub(crate) fn generic_result_shapes(self) -> Option<&'a [KnownShapeId]> {
        match self {
            Self::Call(call) => call.generic_result_shapes(),
            Self::Method(method) => method.generic_result_shapes(),
        }
    }

    /// The success slots of a generic call, classified by which declared slots are concrete.
    ///
    /// WHAT: reads only `KnownShape::Concrete` facts from the call's generic result shapes, so no
    ///       generic parameter is resolved here. A call without generic shapes keeps its declared
    ///       arity with every slot unknown; a call with zero declared slots is `Concrete` (empty).
    /// WHY: a concrete declared slot is a fixed receiving type, so a handler can parse against it
    ///      while a sibling generic slot is still pending. A single slot stays inline, and an
    ///      all-unknown arity needs no storage, so only a mixed or multi-slot list owns a `Vec`.
    pub(crate) fn generic_success_slots(self, scope_context: &ScopeContext) -> GenericSuccessSlots {
        let shapes = match self.generic_result_shapes() {
            Some(shapes) if !shapes.is_empty() => shapes,
            _ => {
                return match self.declared_success_slot_count() {
                    0 => GenericSuccessSlots::Concrete(Vec::new()),
                    slot_count => GenericSuccessSlots::Unknown(slot_count),
                };
            }
        };
        let known = |shape: KnownShapeId| concrete_shape_type(scope_context, shape);
        if let [shape] = shapes {
            return match known(*shape) {
                Some(type_id) => GenericSuccessSlots::Single(type_id),
                None => GenericSuccessSlots::Unknown(1),
            };
        }
        let known_count = shapes
            .iter()
            .filter(|&&shape| known(shape).is_some())
            .count();
        match known_count {
            0 => GenericSuccessSlots::Unknown(shapes.len()),
            count if count == shapes.len() => GenericSuccessSlots::Concrete(
                shapes.iter().filter_map(|&shape| known(shape)).collect(),
            ),
            _ => GenericSuccessSlots::Mixed(shapes.iter().map(|&shape| known(shape)).collect()),
        }
    }
}

/// The success slots of a generic call, classified by which declared slots are already concrete.
pub(crate) enum GenericSuccessSlots {
    /// One declared slot, already concrete; kept inline.
    Single(TypeId),
    /// Every declared slot is concrete, or the call has zero success slots.
    Concrete(Vec<TypeId>),
    /// Every declared slot is still generic, at the declared arity.
    Unknown(usize),
    /// Concrete and generic slots mixed; `None` marks each slot whose type resolves at completion.
    Mixed(Vec<Option<TypeId>>),
}

/// Reads the terminal call a fragment or multi-bind value produces, if the item is one.
///
/// WHAT: returns a bare pending call, or the terminal unhandled method of a retained chain whose
///       operand and suffix are valid.
/// WHY: a chained method is the same call-like producer as a bare call once its operand resolves,
///      so every consumer reads one query. A handled method already consumed its own carrier
///      inside the chain, so it is not a terminal call and returns `None`.
pub(crate) fn retained_terminal_call(item: &ExpressionRpnItem) -> Option<RetainedTerminalCall<'_>> {
    match item {
        ExpressionRpnItem::PendingNumericCall { call, .. } => {
            Some(RetainedTerminalCall::Call(call.as_ref()))
        }
        ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
            let DeferredExpression::OperandContinuation(payload) = expression.as_ref() else {
                return None;
            };
            if payload.invalid_member.is_some()
                || !matches!(payload.suffix, DeferredExpressionSuffix::None)
            {
                return None;
            }
            match payload.steps.last()? {
                RetainedPostfixStep::Method(method)
                    if matches!(method.authored_handling, DeferredCallHandling::Unhandled) =>
                {
                    Some(RetainedTerminalCall::Method(method.as_ref()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Derives one retained operand's known result, following its authored continuations.
///
/// `carrier_handled` records that an enclosing catch handles this operand's error channel, so an
/// unhandled builtin step contributes its success result rather than an unresolved handling need.
pub(crate) fn retained_operand_result(
    item: &ExpressionRpnItem,
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    carrier_handled: bool,
    expression_builder: &AstExpressionBuilder,
) -> Option<RetainedOperandResult> {
    match item {
        ExpressionRpnItem::PendingNumericCall { call, .. } => {
            match call.single_success_result_shape() {
                Some(shape) => Some(RetainedOperandResult::Shape(shape)),
                None => call
                    .single_success_return_type_id()
                    .map(RetainedOperandResult::Type),
            }
        }
        ExpressionRpnItem::PendingNumericBinding { binding, .. } => {
            if let Some(shape) = binding_reference_shape(&scope_context.numeric_state(), *binding) {
                return Some(RetainedOperandResult::Shape(shape));
            }
            let state = scope_context.numeric_state();
            let row = state.binding(*binding);
            match row.recipe.as_ref() {
                // The retained initializer is exactly one authored operand at the value position
                // (an authored group follows the same position), so the binding's value is that
                // operand's derived result; a multi-operand recipe keeps no known result. The
                // initializer's carrier is its own handling, never the use site's catch.
                Some(NumericRecipe::Directed { items, .. }) => {
                    single_operand(items).and_then(|single| {
                        retained_operand_result(
                            single,
                            scope_context,
                            environment,
                            false,
                            expression_builder,
                        )
                    })
                }
                _ => None,
            }
        }
        ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
            match expression.as_ref() {
                DeferredExpression::OperandContinuation(payload) => retained_continuation_result(
                    payload,
                    scope_context,
                    environment,
                    carrier_handled,
                    expression_builder,
                ),
                DeferredExpression::CatchHandler(handler) => {
                    retained_catch_result(handler, scope_context, environment, expression_builder)
                }
                _ => retained_known_shape(std::slice::from_ref(item))
                    .map(RetainedOperandResult::Shape),
            }
        }
        _ => None,
    }
}

/// The known result one retained operand continuation fixes from its last step and its suffix.
///
/// WHY: an unhandled fallible builtin step yields its carrier, which fixes no receiver, unless an
/// enclosing catch recovers that carrier.
fn retained_continuation_result(
    payload: &RetainedOperandContinuation,
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    carrier_handled: bool,
    expression_builder: &AstExpressionBuilder,
) -> Option<RetainedOperandResult> {
    if payload.invalid_member.is_some() {
        // The rejected member's result exists at no type, so it fixes no shape for a further step
        // and never becomes a receiver for one.
        return None;
    }
    let completed = match payload.steps.last() {
        Some(RetainedPostfixStep::Field(field)) => Some(RetainedOperandResult::Type(field.type_id)),
        Some(RetainedPostfixStep::DeclaredField(field)) => {
            Some(RetainedOperandResult::Shape(field.result_shape))
        }
        Some(RetainedPostfixStep::Method(call)) => match call.single_success_result_shape() {
            Some(shape) => Some(RetainedOperandResult::Shape(shape)),
            None => call
                .single_success_return_type_id()
                .map(RetainedOperandResult::Type),
        },
        Some(RetainedPostfixStep::Builtin(step)) => {
            match (step.error_return_type_id, step.authored_handling) {
                (Some(_), DeferredCallHandling::Unhandled) if !carrier_handled => None,
                _ => step.success_shape.map(RetainedOperandResult::Shape),
            }
        }
        None => single_operand(&payload.operand).and_then(|operand| {
            retained_operand_result(
                operand,
                scope_context,
                environment,
                carrier_handled,
                expression_builder,
            )
        }),
    }?;
    match payload.suffix {
        DeferredExpressionSuffix::None | DeferredExpressionSuffix::Propagate { .. } => {
            Some(completed)
        }
        DeferredExpressionSuffix::OptionalPropagate { .. } => {
            let inner = match completed {
                RetainedOperandResult::Type(type_id) => type_id,
                // Only a concrete result has an option projection; a construction-only shape with no
                // concrete type fixes no option result here.
                RetainedOperandResult::Shape(shape) => concrete_shape_type(scope_context, shape)?,
            };
            environment
                .option_inner_type(inner)
                .map(RetainedOperandResult::Type)
        }
    }
}

/// The known result of one catch-protected operand, whose handler recovers the protected carrier.
///
/// WHAT: a built protected value yields its single ready success type; a retained one yields its
///       single item's result with the carrier recovered by the catch.
/// WHY: the catch replaces the error lane with its handler, so the protected success value flows on
///      as the operand's value.
fn retained_catch_result(
    handler: &DeferredCatchHandler,
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
) -> Option<RetainedOperandResult> {
    match &handler.handled {
        StatementExpression::Built(_) => {
            let site = handler.ready_site.as_ref()?;
            match site.success_result_type_ids.as_slice() {
                [single] => Some(RetainedOperandResult::Type(*single)),
                _ => None,
            }
        }
        StatementExpression::Retained(deferred) => {
            single_operand(&deferred.items).and_then(|single| {
                retained_operand_result(
                    single,
                    scope_context,
                    environment,
                    true,
                    expression_builder,
                )
            })
        }
    }
}

/// The construction shape one retained operand's known result is carried as.
///
/// WHAT: a concrete result interns as a concrete shape leaf; a shape result is returned as is. An
///       unhandled fallible builtin step contributes its success shape only when
///       `include_unhandled_success` is set, and otherwise contributes no result.
/// WHY: a retained receiver chain carries shapes, so member steps resume from the same identity
///      whether the origin was a concrete call result or a construction-only collection.
///      `include_unhandled_success` selects success metadata only. It never makes a typed producer
///      lawfully handled: ordinary typed validation still owns that legality.
pub(crate) fn retained_operand_result_shape(
    item: &ExpressionRpnItem,
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    include_unhandled_success: bool,
    expression_builder: &AstExpressionBuilder,
) -> Result<Option<KnownShapeId>, ExpressionParseError> {
    match retained_operand_result(
        item,
        scope_context,
        environment,
        include_unhandled_success,
        expression_builder,
    ) {
        Some(RetainedOperandResult::Type(type_id)) => {
            intern_concrete_shape(scope_context, type_id).map(Some)
        }
        Some(RetainedOperandResult::Shape(shape)) => Ok(Some(shape)),
        None => Ok(None),
    }
}

/// The numeric origin shape one retained operand's known result names for a numeric receiver.
///
/// WHAT: a numeric shape result is returned. An optional numeric result is returned only for an
///       optional numeric receiver, whose single optional layer it pairs with. A concrete type,
///       collection or map result names no numeric origin and yields `None` without interning
///       anything.
/// WHY: a numeric receiver's member step resolves its projection from the origin class, and a field
///      declared `T?` projects as an optional numeric origin. A non-optional receiver cannot take
///      that layer, so it keeps the ordinary diagnostic path.
pub(crate) fn retained_operand_numeric_origin_shape(
    item: &ExpressionRpnItem,
    scope_context: &ScopeContext,
    environment: &TypeEnvironment,
    receiver_type_id: TypeId,
    include_unhandled_success: bool,
    expression_builder: &AstExpressionBuilder,
) -> Option<KnownShapeId> {
    let shape = match retained_operand_result(
        item,
        scope_context,
        environment,
        include_unhandled_success,
        expression_builder,
    )? {
        RetainedOperandResult::Type(_) => return None,
        RetainedOperandResult::Shape(shape) => shape,
    };
    let state = scope_context.numeric_state();
    match state.shape(shape) {
        KnownShape::Numeric(_) => Some(shape),
        KnownShape::Optional { inner }
            if environment.option_inner_type(receiver_type_id).is_some()
                && matches!(state.shape(inner), KnownShape::Numeric(_)) =>
        {
            Some(shape)
        }
        KnownShape::Concrete { .. }
        | KnownShape::Collection { .. }
        | KnownShape::Map { .. }
        | KnownShape::Optional { .. }
        | KnownShape::DeclaredSubstitution { .. } => None,
    }
}

/// Whether one unresolved retained origin is a pending numeric call.
///
/// WHY: only a call's member waits for the call's own result. Every other unresolved origin keeps
///      its member as a retained occurrence, so its ordinary diagnostic comes from completion.
fn retained_pending_numeric_call(item: &ExpressionRpnItem) -> bool {
    matches!(item, ExpressionRpnItem::PendingNumericCall { .. })
}

/// The authored access and receiving facts of one retained operand member chain.
///
/// WHAT: the chain's own access (`.` or an authored `~` marker), plus the expected type and value
///       mode the operand position receives.
/// WHY: those facts describe one operand site and are read together when the retained steps and
///      their invalid-member occurrence are built, so they travel as one record.
pub(crate) struct RetainedOperandInput<'a> {
    pub(crate) chain_access: PostfixChainAccess,
    pub(crate) expected: ExpectedType,
    pub(crate) value_mode: &'a ValueMode,
}

/// Retain the member access authored after one pending numeric operand.
///
/// WHAT: with a known receiver result shape, resolves and retains every authored step; with an
///       unresolved scalar origin, records the invalid-member occurrence for its delayed ordinary
///       diagnostic and consumes the authored postfix syntax so the body keeps parsing.
/// WHY: the operand position is the only place the authored member tokens and the enclosing
///      expression exist together, so both the retention and the delayed diagnostic decision live
///      here instead of in a second member-name convention.
///
/// PRECONDITION: the cursor is on the authored dot and `expression`'s last item is a retained
///               numeric operand.
#[allow(
    clippy::too_many_arguments,
    reason = "retained operand member access keeps the token stream, expression output, operand input, scope, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
pub(crate) fn retain_pending_operand_member_access(
    token_stream: &mut AstCursor<'_>,
    expression: &mut Vec<ExpressionRpnItem>,
    input: RetainedOperandInput<'_>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let RetainedOperandInput {
        chain_access,
        expected: operand_expected,
        value_mode: operand_value_mode,
    } = input;
    let Some(operand_start) = trailing_operand_start(expression)? else {
        crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "a retained operand member step was recorded without an authored operand",
        );
    };
    let operand = expression.split_off(operand_start);
    let item = single_operand(&operand);

    // A grouped operand with several items has no single receiver item, so it has no receiver. It
    // records an occurrence, like any other origin without a receiver, rather than failing here.
    let receiver = match item {
        Some(receiver_item) => retained_operand_result_shape(
            receiver_item,
            scope_context,
            type_interner.environment(),
            false,
            expression_builder,
        )?
        .and_then(|shape| receiver_shape(scope_context, shape))
        .map(|receiver| (receiver_item, receiver)),
        None => None,
    };
    let Some((receiver_item, receiver)) = receiver else {
        // An origin that fixes no receiver type cannot resolve a member here. A pending call's member
        // waits for its own result, while every other origin, including a scalar awaiting its width,
        // keeps the member as an occurrence so later eligible uses still parse and the ordinary
        // diagnostic issues at the body root.
        if item.is_some_and(retained_pending_numeric_call) {
            return Err(pending_numeric_member_access_error(
                token_stream,
                string_table,
            ));
        }
        // The rejected member's authored syntax goes through the shared member-name and
        // call-argument owners, so a nested call's own parameter expectations still witness the
        // origins they constrain while the member itself has no signature to validate against.
        let (member_name, member_span) = consume_invalid_member_access(
            token_stream,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?;
        // The occurrence is diagnostic metadata only; the original operand retains the binding
        // identity and cannot contribute a result value or receiver constraint.
        expression.push(invalid_member_operand(
            operand,
            Vec::new(),
            body_symbol::PendingNumericMemberAccess {
                member_name,
                member_span,
            },
            operand_expected,
            operand_value_mode,
            expression_builder,
        )?);
        return Ok(());
    };

    let RetainedPostfixSteps {
        steps,
        invalid_member,
    } = parse_retained_postfix_steps(
        token_stream,
        RetainedChainRoot {
            shape: receiver,
            access: retained_chain_root(receiver_item),
        },
        &chain_access,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;
    expression.push(match invalid_member {
        Some(occurrence) => invalid_member_operand(
            operand,
            steps,
            occurrence,
            operand_expected,
            operand_value_mode,
            expression_builder,
        )?,
        None => retained_operand_item(
            operand,
            steps,
            DeferredExpressionSuffix::None,
            operand_expected,
            operand_value_mode,
            expression_builder,
        )?,
    });
    Ok(())
}

/// Attach the authored operand `!` to the retained operation this operand ends with.
///
/// WHAT: when the operand's last item is one retained call (directly, or through authored groups of
///       one), or the terminal step of a suffix-free operand continuation that is a receiver method
///       or a compiler builtin member, consumes and validates the authored `!` through the shared
///       fallible-handling fragment entry using that operation's declared facts, and records the
///       propagation on that operation so its completion builds the propagated value exactly once.
/// WHY: a call's declared typed failure lives in its retained signature, not in the value type that
///      arrives at completion, so applying the operand `!` to the value would need a carrier-typed
///      value no producer builds. Keeping the authored decision on the terminal operation keeps one
///      handling owner, one propagation and the following authored operators in place. Earlier chain
///      steps keep their own handling, and a field tail has no fallible handling to propagate.
///
/// Returns false when the operand does not end in such an operation, or that operation already
/// carries an authored handling; the caller then keeps the operand-position suffix path.
#[allow(
    clippy::too_many_arguments,
    reason = "call propagation retention keeps the token stream, expression output, scope, type interner, expression builder, const store, string table, path fork, and catch policy as separate borrows"
)]
pub(crate) fn retain_operand_call_propagation(
    token_stream: &mut AstCursor<'_>,
    expression: &mut [ExpressionRpnItem],
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    allow_boundary_catch: bool,
) -> Result<bool, ExpressionParseError> {
    let Some(operand_start) = trailing_operand_start(expression)? else {
        return Ok(false);
    };
    if let Some(call) =
        single_operand_mut(&mut expression[operand_start..]).and_then(single_retained_call_mut)
    {
        return retain_deferred_call_propagation(
            token_stream,
            call,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
            allow_boundary_catch,
        );
    }
    let Some(step) =
        single_operand_mut(&mut expression[operand_start..]).and_then(terminal_retained_step_mut)
    else {
        return Ok(false);
    };
    match step {
        RetainedPostfixStep::Method(method) => retain_receiver_method_propagation(
            token_stream,
            method,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
            allow_boundary_catch,
        ),
        RetainedPostfixStep::Builtin(builtin) => retain_builtin_step_propagation(
            token_stream,
            builtin,
            scope_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
            allow_boundary_catch,
        ),
        RetainedPostfixStep::Field(_) | RetainedPostfixStep::DeclaredField(_) => Ok(false),
    }
}

/// Retain an authored operand `!` on one retained call, reading its declared facts per variant.
#[allow(
    clippy::too_many_arguments,
    reason = "deferred call propagation keeps the token stream, call, scope, type interner, expression builder, const store, string table, path fork, and catch policy as separate borrows"
)]
fn retain_deferred_call_propagation(
    token_stream: &mut AstCursor<'_>,
    call: &mut DeferredCall,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    allow_boundary_catch: bool,
) -> Result<bool, ExpressionParseError> {
    if !matches!(
        retained_call_authored_handling(call),
        DeferredCallHandling::Unhandled
    ) {
        return Ok(false);
    }

    // Only a single success slot can make the operand optional, so the `!` decision reads that one
    // declared type. Any other contract keeps its resolved slots, or classifies a generic call's
    // slots only when neither the declared single nor the resolved slots answer.
    let resolved_success_types = call.success_return_type_ids();
    let declared_single_success_type_id = call.declared_single_success_return_type_id();
    let declared_slot_count = call.declared_success_slot_count();
    let generic_slots =
        if resolved_success_types.is_none() && declared_single_success_type_id.is_none() {
            Some(RetainedTerminalCall::Call(&*call).generic_success_slots(scope_context))
        } else {
            None
        };
    let handling = parse_retained_operation_propagation(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        allow_boundary_catch,
        RetainedPropagationFacts {
            value_required: declared_slot_count > 0,
            success_contract: retained_success_contract(
                declared_single_success_type_id.as_ref(),
                resolved_success_types.as_deref(),
                generic_slots.as_ref(),
            ),
            error_return: call.error_return(),
            authored_failure_lane: retained_call_has_private_failure_lane(call, scope_context),
            call_span: call.call_span(),
        },
    )?;
    set_retained_call_authored_handling(call, handling);
    Ok(true)
}

/// Retain an authored operand `!` on the terminal receiver-method step of a retained chain.
///
/// WHAT: validates the `!` against the method's declared success and error facts and records the
///       propagation on that step, so completion applies the one propagation the completed method
///       call builds.
/// WHY: the method step that ends the operand is the chain's fallible producer; its handling stays
///      on that step, so earlier steps and the continuation suffix are left unchanged.
#[allow(
    clippy::too_many_arguments,
    reason = "receiver method propagation keeps the token stream, method, scope, type interner, expression builder, const store, string table, path fork, and catch policy as separate borrows"
)]
fn retain_receiver_method_propagation(
    token_stream: &mut AstCursor<'_>,
    method: &mut DeferredReceiverCall,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    allow_boundary_catch: bool,
) -> Result<bool, ExpressionParseError> {
    if !matches!(method.authored_handling, DeferredCallHandling::Unhandled) {
        return Ok(false);
    }

    let resolved_success_types = method
        .concrete_signature()
        .map(|signature| signature.success_return_type_ids());
    let declared_single_success_type_id =
        method.declared_signature().single_success_return_type_id();
    let declared_slot_count = method.declared_signature().success_return_slot_count();
    let generic_slots =
        if resolved_success_types.is_none() && declared_single_success_type_id.is_none() {
            Some(RetainedTerminalCall::Method(&*method).generic_success_slots(scope_context))
        } else {
            None
        };
    let handling = parse_retained_operation_propagation(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        allow_boundary_catch,
        RetainedPropagationFacts {
            value_required: declared_slot_count > 0,
            success_contract: retained_success_contract(
                declared_single_success_type_id.as_ref(),
                resolved_success_types.as_deref(),
                generic_slots.as_ref(),
            ),
            error_return: method.error_return(),
            authored_failure_lane: scope_context
                .source_call_has_private_failure_lane(method.method_path),
            call_span: method.call_span,
        },
    )?;
    method.authored_handling = handling;
    Ok(true)
}

/// Retain an authored operand `!` on the terminal builtin step of a retained chain.
///
/// WHAT: validates the authored `!` against the operation's declared success and error slots and
///       records it on the step, so completion applies the one propagation the completed call path
///       would apply.
/// WHY: a retained builtin member already resolved its operation and result slots, so an authored
///      operand suffix must travel with the step instead of ending the fragment; a `catch` stays
///      with the enclosing fragment owner exactly as it does for a retained receiver method.
#[allow(
    clippy::too_many_arguments,
    reason = "builtin step propagation keeps the token stream, step, scope, type interner, expression builder, const store, string table, path fork, and catch policy as separate borrows"
)]
fn retain_builtin_step_propagation(
    token_stream: &mut AstCursor<'_>,
    step: &mut RetainedBuiltinStep,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    allow_boundary_catch: bool,
) -> Result<bool, ExpressionParseError> {
    if !matches!(step.authored_handling, DeferredCallHandling::Unhandled) {
        return Ok(false);
    }

    let success_types = builtin_success_type_ids(scope_context, step.success_shape);
    let handling = parse_retained_operation_propagation(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        allow_boundary_catch,
        RetainedPropagationFacts {
            value_required: step.success_shape.is_some(),
            success_contract: success_types.as_deref().map_or(
                FallibleSuccessContract::Unresolved,
                FallibleSuccessContract::Known,
            ),
            error_return: step.error_return_type_id.map(GenericErrorReturn::Concrete),
            authored_failure_lane: false,
            call_span: step.member_span,
        },
    )?;
    step.authored_handling = handling;
    Ok(true)
}

/// The success contract one retained operation's authored `!` and its handler read.
///
/// WHAT: a declared single slot keeps its own type, which the optional decision reads. Otherwise a
///       resolved slot list is known. A generic call's classified slots are read as they are: a
///       single or fully concrete list is known, an all-unknown arity stays pending at its declared
///       count, and a mixed list keeps each concrete position typed.
/// WHY: a generic call keeps its arity while its origins resolve, so its handler is not sized as
///      one provisional slot. A concrete slot is a fixed receiving type read now; a generic slot
///      stays unknown until completion, so no parameter is resolved early.
fn retained_success_contract<'a>(
    declared_single: Option<&'a TypeId>,
    resolved_slots: Option<&'a [TypeId]>,
    generic_slots: Option<&'a GenericSuccessSlots>,
) -> FallibleSuccessContract<'a> {
    if let Some(single) = declared_single {
        return FallibleSuccessContract::Known(std::slice::from_ref(single));
    }
    if let Some(slots) = resolved_slots {
        return FallibleSuccessContract::Known(slots);
    }
    // Both callers collect generic facts exactly when the two concrete contracts are absent.
    // Missing facts violate that handoff; they cannot stand in for zero success slots.
    let Some(generic_slots) = generic_slots else {
        crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "retained call is missing its declared success-slot facts",
        );
    };
    match generic_slots {
        GenericSuccessSlots::Single(type_id) => {
            FallibleSuccessContract::Known(std::slice::from_ref(type_id))
        }
        GenericSuccessSlots::Concrete(types) => FallibleSuccessContract::Known(types.as_slice()),
        GenericSuccessSlots::Unknown(slot_count) => FallibleSuccessContract::Pending {
            slot_count: *slot_count,
        },
        GenericSuccessSlots::Mixed(slots) => FallibleSuccessContract::Partial(slots.as_slice()),
    }
}

/// The declared facts one retained operation supplies to the shared authored `!` decision.
struct RetainedPropagationFacts<'a> {
    value_required: bool,
    /// The operation's success contract, which the operand's handler receives. `Known` carries the
    /// resolved or declared slots, `Pending` keeps a generic operation's declared arity, `Partial`
    /// types its concrete slots while the rest resolve, and `Unresolved` is left for a builtin whose
    /// slot types are not known.
    success_contract: FallibleSuccessContract<'a>,
    error_return: Option<GenericErrorReturn>,
    authored_failure_lane: bool,
    call_span: Option<SourceSpan>,
}

/// Consume one authored operand `!` against a retained operation's declared facts.
///
/// WHAT: reads the propagation span at the `!`, runs the shared fallible-handling fragment entry
///       once, and returns the propagation handling the operation records for its completion.
/// WHY: every retained operation shares one `!` legality check and one token consumption, so the
///      operand position and the operation's receiving site cannot disagree on the decision.
#[allow(
    clippy::too_many_arguments,
    reason = "retained operation propagation keeps the token stream, scope, type interner, expression builder, const store, string table, path fork, catch policy, and propagation facts as separate borrows"
)]
fn parse_retained_operation_propagation(
    token_stream: &mut AstCursor<'_>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    allow_boundary_catch: bool,
    facts: RetainedPropagationFacts<'_>,
) -> Result<DeferredCallHandling, ExpressionParseError> {
    let propagation_span = Some(token_stream.current_span());
    let authored_suffix = parse_fragment_handling_suffix(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        facts.success_contract,
        facts.error_return,
        facts.authored_failure_lane,
        facts.value_required,
        facts.call_span,
        FUNCTION_CALL_STAGE,
        allow_boundary_catch,
        None,
        string_table,
        path_fork,
    )?;
    let Some(ParsedFallibleSuffix::Ready(FallibleHandling::Propagate)) = authored_suffix else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "an authored operand `!` was not parsed as typed propagation",
        )
        .into());
    };
    Ok(DeferredCallHandling::Propagate {
        span: propagation_span,
    })
}

/// The authored handling already recorded on one retained call, read per variant.
fn retained_call_authored_handling(call: &DeferredCall) -> DeferredCallHandling {
    match call {
        DeferredCall::Source(deferred) => deferred.authored_handling,
        DeferredCall::Receiver(deferred) => deferred.authored_handling,
        DeferredCall::External(deferred) => deferred.authored_handling,
        DeferredCall::Generic(deferred) => deferred.authored_handling,
        // A constructor call has no authored handling slot; the shared entry still reports the
        // ordinary non-fallible `!` diagnostic for it.
        _ => DeferredCallHandling::Unhandled,
    }
}

/// Record the authored handling on one retained call, per variant.
fn set_retained_call_authored_handling(call: &mut DeferredCall, handling: DeferredCallHandling) {
    match call {
        DeferredCall::Source(deferred) => deferred.authored_handling = handling,
        DeferredCall::Receiver(deferred) => deferred.authored_handling = handling,
        DeferredCall::External(deferred) => deferred.authored_handling = handling,
        DeferredCall::Generic(deferred) => deferred.authored_handling = handling,
        _ => {}
    }
}

/// The terminal step of one suffix-free operand continuation.
///
/// WHY: the chain's last step carries its authored `!`, while earlier steps keep their own handling.
///      A rejected member is the position's whole outcome, so a following bang must not validate
///      against its predecessor. A recorded suffix likewise keeps the operand-position handling.
fn terminal_retained_step_mut(item: &mut ExpressionRpnItem) -> Option<&mut RetainedPostfixStep> {
    match item {
        ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
            let DeferredExpression::OperandContinuation(payload) = expression.as_mut() else {
                return None;
            };
            if payload.invalid_member.is_some()
                || !matches!(payload.suffix, DeferredExpressionSuffix::None)
            {
                return None;
            }
            payload.steps.last_mut()
        }
        _ => None,
    }
}

fn single_retained_call_mut(item: &mut ExpressionRpnItem) -> Option<&mut DeferredCall> {
    match item {
        ExpressionRpnItem::PendingNumericCall { call, .. } => Some(call.as_mut()),
        _ => None,
    }
}

/// The authored failure lane of one retained call, read per variant through its declared facts.
fn retained_call_has_private_failure_lane(call: &DeferredCall, context: &ScopeContext) -> bool {
    match call {
        DeferredCall::Source(deferred) => {
            context.source_call_has_private_failure_lane(deferred.function_path)
        }
        DeferredCall::Receiver(deferred) => {
            context.source_call_has_private_failure_lane(deferred.method_path)
        }
        DeferredCall::Generic(deferred) => {
            context.source_call_has_private_failure_lane(deferred.template.function_path)
        }
        // Host calls and constructors keep their own failure rules.
        _ => false,
    }
}

/// Record one authored postfix handling suffix on the operand it follows.
///
/// WHAT: stores the suffix with the preceding retained operand, merging into an existing
///       step-bearing wrapper instead of nesting a second one.
/// WHY: `!`/`?` need the resolved operand value, but consuming them at the operand position keeps
///      every authored operator after the suffix in the same fragment; completion applies the
///      suffix before those operators evaluate.
///
/// PRECONDITION: `expression`'s last item is a retained numeric operand and the cursor has already
///               consumed the authored suffix token.
pub(crate) fn retain_operand_handling_suffix(
    expression: &mut Vec<ExpressionRpnItem>,
    suffix: DeferredExpressionSuffix,
    expected_type: ExpectedType,
    value_mode: &ValueMode,
    expression_builder: &AstExpressionBuilder,
) -> Result<(), ExpressionParseError> {
    let Some(operand_start) = trailing_operand_start(expression)? else {
        crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "an authored operand suffix was recorded without an authored operand",
        );
    };
    let mut operand = expression.split_off(operand_start);

    // A plain continuation that has not recorded its own suffix yet takes this one in place, so one
    // operand keeps one wrapper. A group, or a continuation that already has a suffix, wraps as one
    // operand, so a second authored suffix applies to the first one's result.
    if let [
        ExpressionRpnItem::PendingNumericExpression {
            expression: inner, ..
        },
    ] = operand.as_mut_slice()
        && let DeferredExpression::OperandContinuation(payload) = inner.as_mut()
        && matches!(payload.suffix, DeferredExpressionSuffix::None)
    {
        payload.suffix = suffix;
        expression.extend(operand);
        return Ok(());
    }

    expression.push(retained_operand_item(
        operand,
        Vec::new(),
        suffix,
        expected_type,
        value_mode,
        expression_builder,
    )?);
    Ok(())
}

/// Build the retained operand item for one resolved builtin member step on a completed receiver.
///
/// WHAT: the receiver value exists (an ordinary known-shape receiver), but the step's own arguments
///       still hold numeric origins, so the member resumes through the same constructor once they
///       complete.
/// WHY: the builtin handler already validated receiver access and resolved the operation's
///      parameter and result slots; only the argument values are missing, so no second member
///      grammar or receiver value is needed.
pub(crate) fn retained_builtin_operand(
    receiver_expression: ExprId,
    step: RetainedBuiltinStep,
    expression_builder: &AstExpressionBuilder,
) -> Result<ExpressionRpnItem, ExpressionParseError> {
    retained_operand_item(
        vec![ExpressionRpnItem::Operand(receiver_expression)],
        vec![RetainedPostfixStep::Builtin(Box::new(step))],
        DeferredExpressionSuffix::None,
        ExpectedType::Infer,
        &ValueMode::ImmutableOwned,
        expression_builder,
    )
}

/// Build the wrapper item for one retained operand continuation.
fn retained_operand_item(
    operand: Vec<ExpressionRpnItem>,
    steps: Vec<RetainedPostfixStep>,
    suffix: DeferredExpressionSuffix,
    expected: ExpectedType,
    value_mode: &ValueMode,
    expression_builder: &AstExpressionBuilder,
) -> Result<ExpressionRpnItem, ExpressionParseError> {
    let span = operand_leading_item(&operand)?.source_span(expression_builder)?;
    Ok(ExpressionRpnItem::PendingNumericExpression {
        expression: Box::new(DeferredExpression::OperandContinuation(Box::new(
            RetainedOperandContinuation {
                // A group carries the authored receiving context its own fragment completes with;
                // every other operand completes with its own value so the fragment boundary still
                // owns the coercion of the suffix result.
                expected: operand_evaluation_expected(&operand, expected),
                operand,
                steps,
                suffix,
                value_mode: value_mode.to_owned(),
                invalid_member: None,
            },
        ))),
        span,
    })
}

/// Build the distinct operand for one already-rejected member occurrence.
///
/// WHAT: an operand continuation that carries the occurrence's facts and the chain completed before
///       the rejected member, with no suffix or result shape of its own.
/// WHY: the occurrence is the position's outcome, not a value: keeping it out of the direct
///      reference / alias / value shapes is what stops the rejected member's result position from
///      selecting the scalar receiver's type.
fn invalid_member_operand(
    operand: Vec<ExpressionRpnItem>,
    steps: Vec<RetainedPostfixStep>,
    occurrence: body_symbol::PendingNumericMemberAccess,
    expected: ExpectedType,
    value_mode: &ValueMode,
    expression_builder: &AstExpressionBuilder,
) -> Result<ExpressionRpnItem, ExpressionParseError> {
    let span = operand_leading_item(&operand)?.source_span(expression_builder)?;
    Ok(ExpressionRpnItem::PendingNumericExpression {
        expression: Box::new(DeferredExpression::OperandContinuation(Box::new(
            RetainedOperandContinuation {
                expected: operand_evaluation_expected(&operand, expected),
                operand,
                steps,
                suffix: DeferredExpressionSuffix::None,
                value_mode: value_mode.to_owned(),
                invalid_member: Some(occurrence),
            },
        ))),
        span,
    })
}

/// The receiving context one grouped fragment completes with.
///
/// WHAT: a group is not a receiving boundary, so a numeric literal expectation becomes `Infer` and
///       every other expectation is inherited.
/// WHY: this is the one owner of the group expected-type rule. Dispatch uses it at group open, and
///      retained operands reach it through `operand_evaluation_expected`.
pub(crate) fn grouped_evaluation_expected(outer_expected: ExpectedType) -> ExpectedType {
    match outer_expected {
        ExpectedType::NumericLiteral(_) => ExpectedType::Infer,
        other => other,
    }
}

/// The receiving context one retained operand's own fragment completes with.
///
/// WHY: only a grouped operand inherits the authored receiving context. Every other operand completes
///      with its own value, so the fragment boundary still owns the coercion of the suffix result.
fn operand_evaluation_expected(
    operand: &[ExpressionRpnItem],
    outer_expected: ExpectedType,
) -> ExpectedType {
    match operand.first() {
        Some(ExpressionRpnItem::GroupOpen { .. }) => grouped_evaluation_expected(outer_expected),
        _ => ExpectedType::Infer,
    }
}

/// The leading item of one retained operand, which carries the operand's authored span.
///
/// WHY: a group's open marker carries its first operand's span, so the leading item owns the span
///      of a grouped operand as well as a plain one.
fn operand_leading_item(
    operand: &[ExpressionRpnItem],
) -> Result<&ExpressionRpnItem, InfrastructureFailure> {
    match operand.first() {
        Some(item) => Ok(item),
        None => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "a retained operand was recorded without authored items",
        ),
    }
}

/// Complete one retained operand continuation: its operand value, its authored steps, then its
/// authored suffix.
///
/// WHAT: the operand's own fragment completes first (a call resumes its ordinary constructor, a
///       binding substitutes its canonical reference), then each retained step rebuilds the field
///       or method node the completed chain would have built, then the shared suffix applier
///       applies the authored `!`/`?`.
/// WHY: this runs at the body root, after the origin's single static type is selected, and it is
///      the one place the retained steps and suffix were kept for.
pub(crate) fn complete_retained_operand_continuation<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    payload: RetainedOperandContinuation,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let RetainedOperandContinuation {
        operand,
        steps,
        suffix,
        expected,
        value_mode,
        invalid_member,
    } = payload;
    if let Some(occurrence) = invalid_member {
        // The occurrence's ordinary diagnostic is this position's whole outcome, issued on the
        // receiver completed through the steps before the rejected member. Like any member access,
        // the receiver is a closed receiving site unless a catch protects its pending error.
        let receiver = session.resolve_fragment(operand, ExpectedType::Infer, &value_mode)?;
        let receiver = complete_retained_postfix_steps(receiver, steps, session)?;
        if !session.typed_errors_are_protected() {
            let services = session.session_services();
            validate_completed_typed_errors(services.expression_builder.view_owned(receiver)?)?;
        }
        let receiver_type_id = {
            let services = session.session_services();
            services
                .expression_builder
                .view_owned(receiver)?
                .require_single(ValueReceiverContext::ReceiverBase)?
        };
        return Err(
            body_symbol::pending_member_access_diagnostic(&occurrence, receiver_type_id).into(),
        );
    }

    let value = session.resolve_fragment(operand, expected, &value_mode)?;
    let value = complete_retained_postfix_steps(value, steps, session)?;
    if matches!(suffix, DeferredExpressionSuffix::None) {
        return Ok(value);
    }
    let services = session.session_services();
    apply_completed_handling_suffix(
        value,
        suffix,
        services.context,
        services.type_interner,
        services.expression_builder,
    )
}

/// Complete every retained postfix step on one already-completed receiver value.
pub(crate) fn complete_retained_postfix_steps<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    mut value: OwnedExpr,
    steps: Vec<RetainedPostfixStep>,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    for step in steps {
        // A value still carrying an unhandled typed producer cannot receive a further step. The
        // ready lane rejects it at this closed receiving site, on the producer's own span. A catch
        // that protects this subtree owns that producer, so the step receives it open instead.
        if !session.typed_errors_are_protected() {
            let services = session.session_services();
            validate_completed_typed_errors(services.expression_builder.view_owned(value)?)?;
        }
        {
            let services = session.session_services();
            services
                .expression_builder
                .view_owned(value)?
                .require_single(ValueReceiverContext::ReceiverBase)?;
        }
        value = match step {
            RetainedPostfixStep::Field(field) => {
                let services = session.session_services();
                complete_retained_field_step(value, field, services.expression_builder)?
            }
            RetainedPostfixStep::DeclaredField(field) => {
                complete_retained_declared_field_step(value, field, session)?
            }
            RetainedPostfixStep::Builtin(step) => {
                complete_retained_builtin_step(value, *step, session)?
            }
            RetainedPostfixStep::Method(call) => {
                let DeferredCallReceiver::Chained = call.receiver else {
                    crate::compiler_bug!(
                        CompilerStage::AstConstruction,
                        "a retained chained method step carried a site-materialised receiver",
                    );
                };
                // The step's receiver is the value just completed; injecting it resumes the one
                // shared receiver-call constructor the completed chain also uses.
                let mut call = *call;
                call.receiver = DeferredCallReceiver::Ready(value.id());
                complete_deferred_receiver_call(call, session)?
            }
        };
    }
    Ok(value)
}

/// Rebuild one retained collection builtin member on its completed receiver value.
///
/// WHAT: completes each retained argument through the session's shared argument completion,
///       validates every argument against the materialised parameter slots, and builds the same
///       builtin call node the completed chain builds.
/// WHY: the operation, member span and result slots were resolved against the receiver's declared
///      shape at the authored site, so completion resumes the existing constructor once instead of
///      searching members or re-parsing arguments.
fn complete_retained_builtin_step<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    value: OwnedExpr,
    step: RetainedBuiltinStep,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let RetainedBuiltinStep {
        kind,
        member_name,
        member_span,
        arguments,
        expected_argument_shapes,
        success_shape,
        error_return_type_id,
        authored_handling,
    } = step;

    // Parameter slots materialise before retained arguments finish. Each slot first completes the
    // pending forward producers connected to its class, so its type is final. A retained literal
    // then completes against it instead of defaulting early.
    let slots = {
        let mut slots = Vec::with_capacity(expected_argument_shapes.len());
        for shape in expected_argument_shapes.iter() {
            session.settle_forward_producers(*shape)?;
            let services = session.session_services();
            slots.push(ExpectedParameterType::Known(materialise_builtin_shape(
                services.context,
                services.type_interner,
                *shape,
            )?));
        }
        slots
    };

    // Retained arguments finish against their routed slot, so validation sees concrete leaves.
    // Ready arguments re-enter the issuing expression owner.
    let arguments = match arguments {
        ParsedCallArguments::Ready(arguments) => {
            let services = session.session_services();
            arguments
                .into_iter()
                .map(|argument| argument.map_value(|id| services.expression_builder.own(id)))
                .collect()
        }
        ParsedCallArguments::Retained(arguments) => {
            let mut completed_arguments = Vec::with_capacity(arguments.len());
            for mut argument in arguments {
                if let DeferredCallArgument::Retained { shape, expression } = &mut argument {
                    // Excess arguments have no slot; arity validation reports them.
                    if let Some(ExpectedParameterType::Known(type_id)) = shape
                        .parameter_slot
                        .and_then(|slot| slots.get(slot.index()))
                    {
                        expression.expected = expected_type_for_resolved_parameter(
                            *type_id,
                            session.session_services().type_interner.environment(),
                        );
                    }
                }
                completed_arguments.push(session.complete_call_argument(argument)?);
            }
            completed_arguments
        }
    };

    // The success slot materialises only after its connected producers settle, as the parameter
    // slots do, so a forward result fixes its class before any default applies.
    if let Some(success_shape) = success_shape {
        session.settle_forward_producers(success_shape)?;
    }

    let expression = {
        let services = session.session_services();
        // Ready and retained arguments validate through the same owner, so no argument skips the
        // shared rules.
        let completed_arguments = resolve_builtin_member_arguments(
            BuiltinCallSite {
                member_name: kind.source_name(),
                member_span,
            },
            arguments,
            &builtin_argument_expectations(slots),
            services.context,
            services.type_interner,
            services.expression_builder,
            services.const_values,
            services.string_table,
            services.path_fork,
        )?;
        let result_type_ids =
            builtin_result_type_ids(success_shape, services.context, services.type_interner)?;
        let expression = match kind {
            RetainedBuiltinKind::Collection(op) => services
                .expression_builder
                .collection_builtin_call_with_typed_arguments(
                    value,
                    op,
                    completed_arguments,
                    result_type_ids,
                    member_span,
                )?,
            RetainedBuiltinKind::Map(op) => services
                .expression_builder
                .map_builtin_call_with_typed_arguments(
                    value,
                    op,
                    op.requires_mutable_receiver(),
                    completed_arguments,
                    result_type_ids,
                    member_span,
                )?,
        };
        match error_return_type_id {
            Some(error_type) => services.expression_builder.with_typed_error_producer(
                expression,
                error_type,
                Some(member_name),
            )?,
            None => expression,
        }
    };
    match authored_handling {
        // Recovery preserves the pending typed producer exactly as the ready lane does.
        DeferredCallHandling::Unhandled => {
            let services = session.session_services();
            recover_pending_producer(
                expression,
                member_name,
                services.type_interner,
                services.expression_builder,
            )
        }
        // The authored `!` was consumed and validated at the operand position against this
        // operation's declared slots, so completion applies the one propagation the completed call
        // path would apply, with the operator span the authored site recorded.
        DeferredCallHandling::Propagate { span } => {
            let services = session.session_services();
            apply_completed_handling_suffix(
                expression,
                DeferredExpressionSuffix::Propagate { span },
                services.context,
                services.type_interner,
                services.expression_builder,
            )
        }
    }
}

/// Rebuild one retained field projection on its completed receiver.
///
/// WHAT: returns the declaration-level const field value when the field inlines, otherwise the
///       same `FieldAccess` node the completed chain builds, with the member span.
/// WHY: the receiver value only exists at completion, so the resolved field facts were recorded at
///      the authored site and must not be searched again.
fn complete_retained_field_step(
    value: OwnedExpr,
    field: RetainedFieldStep,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);
    if let Some(inlined) = field.const_inline_value {
        let mut input = expression_builder
            .view(inlined)?
            .to_completed_input(expression_builder)?;
        input.span = field.span.or(input.span);
        return Ok(expression_builder.commit_replacement(input)?);
    }
    Ok(expression_builder.field_access(
        value,
        field.field_name,
        field.type_id,
        field.span,
        field.value_mode,
        field.const_record_state,
    )?)
}

/// Complete one declared-envelope field projection on the completed receiver's concrete owner.
///
/// WHAT: resolves the field on the receiver value's concrete type through the known-receiver field
///       owner, then builds the same field node the completed chain builds.
/// WHY: a declared envelope's owner is generic, so its concrete instance exists only here. The
///      ordinary finisher owns visibility, member, and access diagnostics on that concrete owner.
fn complete_retained_declared_field_step<'env, S: DeferredExpressionCompletion<'env> + ?Sized>(
    value: OwnedExpr,
    field: RetainedDeclaredFieldStep,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let receiver_type_id = {
        let services = session.session_services();
        services
            .expression_builder
            .view_owned(value)?
            .require_single(ValueReceiverContext::ReceiverBase)?
    };
    let field_name = field.field_name;
    let field_span = field.span;
    let resolved = {
        let services = session.session_services();
        resolve_field_member_for_known_receiver(
            receiver_type_id,
            field_name,
            services.context,
            services.type_interner,
            services.expression_builder,
            services.const_values,
            services.path_fork,
        )?
    };
    let Some(member) = resolved else {
        let services = session.session_services();
        return Err(unknown_member_diagnostic(
            receiver_type_id,
            field_name,
            field_span,
            None,
            services.type_interner,
            services.path_fork,
        ));
    };
    let services = session.session_services();
    complete_retained_field_step(
        value,
        retained_field_step(member, field_span),
        services.expression_builder,
    )
}

/// Parse and retain the authored steps of one member chain on a known receiver type.
///
/// WHAT: resolves each authored step against the current receiver type — a field projection or a
///       declared source receiver method — and carries the type forward; the same ordered lookup
///       and diagnostics as the completed chain apply.
/// WHY: member lookup may use known receiver structure, and each retained step keeps exactly the
///      facts its ordinary constructor needs so completion never re-parses tokens.
///
/// PRECONDITION: the cursor is on the first authored dot of the step chain.
#[allow(
    clippy::too_many_arguments,
    reason = "retained postfix parsing keeps the token stream, root, chain access, scope, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
fn parse_retained_postfix_steps(
    token_stream: &mut AstCursor<'_>,
    root: RetainedChainRoot,
    chain_access: &PostfixChainAccess,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<RetainedPostfixSteps, ExpressionParseError> {
    let RetainedChainRoot {
        shape: receiver_shape,
        access: mut receiver_access,
    } = root;
    let mut steps = Vec::new();
    // `None` once a step yields no single value: a member after it has no receiver to resolve.
    let mut current_shape = Some(receiver_shape);

    while token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::DOT
    {
        token_stream.advance();

        // An access that ends immediately after the authored dot has no member name; the dot is
        // the owning boundary, exactly like the completed chain's missing-member diagnostic.
        if token_stream.position() >= token_stream.length()
            || token_stream.current_tag() == TokenTag::EOF
        {
            let dot_span = previous_token_span(token_stream);
            return Err(calls_data::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                dot_span,
            )
            .into());
        }

        let member_name = parse_member_name_typed(token_stream, string_table)?;
        let member_span = Some(token_stream.current_span());
        // A step with no single value leaves no receiver for this member. The member is recorded
        // here and reported by the completed chain once its receiver's type is known.
        let Some(resolved_shape) = current_shape else {
            return rejected_chain_member(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                steps,
                body_symbol::PendingNumericMemberAccess {
                    member_name,
                    member_span,
                },
            );
        };

        // A declared generic envelope has no canonical receiver type. Its field projects through
        // the declared owner's struct definition, and the concrete owner resolves at completion.
        let declared_substitution = match scope_context.numeric_state().shape(resolved_shape) {
            KnownShape::DeclaredSubstitution {
                declared_type_id,
                bindings,
            } => Some((declared_type_id, bindings)),
            _ => None,
        };
        // The declared owner stays addressable after the substitution moves into its field lookup,
        // so a generic method on the same envelope resolves from it as well.
        let declared_owner = declared_substitution
            .as_ref()
            .map(|(declared_type_id, _)| *declared_type_id);
        let declared_field = match declared_substitution {
            Some((declared_type_id, bindings)) => {
                let mut numeric_state = scope_context.numeric_state_mut();
                declared_field_shape(
                    &mut numeric_state,
                    type_interner.environment_mut_for_derived_types(),
                    path_fork,
                    declared_type_id,
                    member_name,
                    bindings,
                    member_span,
                )?
                .map(|field_shape| (declared_type_id, field_shape))
            }
            None => None,
        };
        if let Some((declared_type_id, field_shape)) = declared_field {
            token_stream.advance();
            if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
                return Err(calls_data::invalid_field_access(
                    InvalidFieldAccessReason::FieldNotMethod,
                    Some(member_name),
                    Some(declared_type_id),
                    Vec::new(),
                    member_span,
                )
                .into());
            }
            // A declared field never inlines a const value, so it keeps the receiver's place.
            current_shape = Some(field_shape);
            steps.push(RetainedPostfixStep::DeclaredField(
                RetainedDeclaredFieldStep {
                    field_name: member_name,
                    result_shape: field_shape,
                    span: member_span,
                },
            ));
            continue;
        }
        // A generic method on a declared generic envelope resolves from the declared owner. The
        // receiver has no canonical type yet, so the concrete instance completes with the receiver.
        if let Some(declared_owner_id) = declared_owner
            && let Some(method_entry) = lookup_receiver_method(
                scope_context,
                declared_owner_id,
                member_name,
                type_interner.environment(),
            )
            && let Some(template) =
                scope_context.lookup_generic_function_template(&method_entry.function_path)
        {
            if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
                return Err(calls_data::invalid_receiver_call(
                    InvalidReceiverCallReason::MustUseParentheses,
                    None,
                    Some(member_name),
                    None,
                    None,
                    member_span,
                )
                .into());
            }
            token_stream.advance();

            let access_diagnostic = ReceiverAccessDiagnostic::ReceiverMethod {
                method_name: member_name,
            };
            validate_retained_step_access(
                receiver_access,
                chain_access,
                method_entry.receiver_mutable,
                &access_diagnostic,
                member_span,
            )?;

            // The template's receiver is its first parameter; the authored arguments follow it.
            let expectations = generic_template_expectations(
                &template.signature.parameters[1..],
                path_fork,
                type_interner.environment(),
            );
            let producer_start = scope_context.numeric_state().forward_producer_checkpoint();
            let arguments: Vec<DeferredCallArgument> = match parse_generic_call_arguments_typed(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                Some(member_name),
                &expectations,
                path_fork,
            )? {
                ParsedCallArguments::Ready(arguments) => arguments
                    .into_iter()
                    .map(|argument| DeferredCallArgument::Ready(argument.map_value(OwnedExpr::id)))
                    .collect(),
                ParsedCallArguments::Retained(arguments) => arguments,
            };
            let owned_producers =
                producer_start..scope_context.numeric_state().forward_producer_checkpoint();
            let linked = link_generic_numeric_signature(GenericNumericSignatureInput {
                template,
                arguments: &arguments,
                expected_context: GenericCallExpectedContext::None,
                context: scope_context,
                type_interner,
                expression_builder,
                path_fork,
                call_span: member_span,
                owned_producers,
                known_receiver_shape: Some(resolved_shape),
            })?;

            let private_failure_lane =
                scope_context.source_call_has_private_failure_lane(method_entry.function_path);
            let private_failure_candidate = (private_failure_lane && linked.error_return.is_none())
                .then_some(method_entry.function_path);
            let step_result_shape = match linked.result_shapes.as_slice() {
                [shape] => Some(*shape),
                _ => None,
            };
            // The ticket fixes this call's ledger position now. Completion publishes into it, and a
            // failed completion abandons it. A validation lane records no instance request, so it
            // reserves none.
            let ticket = (!scope_context.generic_template_validation)
                .then(|| scope_context.reserve_pending_generic_request());
            steps.push(RetainedPostfixStep::Method(Box::new(
                DeferredReceiverCall::chained_generic_method(
                    template.clone(),
                    linked,
                    ticket,
                    member_name,
                    method_entry.receiver_mutable,
                    chain_access.authored_marker_span,
                    arguments,
                    member_span,
                    private_failure_candidate,
                ),
            )));
            current_shape = step_result_shape;
            receiver_access = RetainedReceiverAccess::temporary();
            continue;
        }
        let current_type_id = concrete_shape_type(scope_context, resolved_shape);

        if let Some(type_id) = current_type_id
            && let Some(field) = resolve_field_member_for_known_receiver(
                type_id,
                member_name,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                path_fork,
            )?
        {
            token_stream.advance();
            if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
                return Err(calls_data::invalid_field_access(
                    InvalidFieldAccessReason::FieldNotMethod,
                    Some(member_name),
                    Some(type_id),
                    Vec::new(),
                    member_span,
                )
                .into());
            }
            let field_preserves_place =
                receiver_access.is_place && field.const_inline_value.is_none();
            current_shape = Some(intern_concrete_shape(scope_context, field.type_id)?);
            steps.push(RetainedPostfixStep::Field(retained_field_step(
                field,
                member_span,
            )));
            if !field_preserves_place {
                receiver_access = RetainedReceiverAccess::temporary();
            }
            continue;
        }

        // A collection builtin member resolves against the receiver's canonical shape, exactly as
        // the completed chain's builtin dispatch does; its arguments parse through the shared
        // argument owner with the operation's own expectations, so a pending numeric argument is
        // retained instead of ending the fragment.
        if let Some(member) = resolve_collection_builtin_member(
            CollectionBuiltinReceiver::Shape(resolved_shape),
            member_name,
            member_span,
            scope_context,
            type_interner,
            string_table,
        )? {
            if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
                return Err(calls_data::invalid_builtin_call(
                    InvalidBuiltinCallReason::MissingParentheses,
                    Some(member_name),
                    member_span,
                )
                .into());
            }
            token_stream.advance();

            validate_retained_step_access(
                receiver_access,
                chain_access,
                member.op.requires_mutable_receiver(),
                &ReceiverAccessDiagnostic::CollectionBuiltin {
                    method_name: member_name,
                },
                member_span,
            )?;

            let expectations = builtin_argument_expectations(builtin_argument_slots(
                scope_context,
                &member.rule.expected_argument_shapes,
            ));
            let arguments = parse_call_arguments_typed_with_numeric_retention(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                &expectations,
                CallArgumentSyntax::UnsupportedBuiltinMember {
                    member_name: Some(member_name),
                    takes_no_arguments: member.rule.expected_argument_shapes.is_empty(),
                },
                path_fork,
            )?;

            if matches!(member.op, CollectionBuiltinOp::Get)
                && token_stream.current_tag().is_assignment_operator()
            {
                // Ordinary argument validation (arity, access, type) runs before the static target
                // rejection, so a bad argument reports ahead of the target. The validated result is
                // discarded: the rejected target never completes the call and no RHS is parsed.
                if let ParsedCallArguments::Ready(parsed) = arguments {
                    resolve_builtin_member_arguments(
                        BuiltinCallSite {
                            member_name: collection_builtin_op_source_name(member.op),
                            member_span,
                        },
                        parsed,
                        &expectations,
                        scope_context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                }
                return Err(names_declarations::invalid_assignment_target(
                    InvalidAssignmentTargetReason::CollectionGetTargetNotWritable,
                    None,
                    None,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            // A fallible builtin keeps its success slot as the receiver of a further member step.
            // Completion recovers the carrier before that step reads it, so the step is never
            // resolved against a carrier type and the authored member is still reported at its own
            // position.
            steps.push(RetainedPostfixStep::Builtin(Box::new(
                RetainedBuiltinStep {
                    kind: RetainedBuiltinKind::Collection(member.op),
                    member_name,
                    member_span,
                    arguments: arguments.map_values(OwnedExpr::id),
                    expected_argument_shapes: member.rule.expected_argument_shapes,
                    success_shape: member.rule.success_shape,
                    error_return_type_id: member.rule.error_return_type_id,
                    authored_handling: DeferredCallHandling::Unhandled,
                },
            )));
            current_shape = member.rule.success_shape;
            receiver_access = RetainedReceiverAccess::temporary();
            continue;
        }

        // A map builtin member resolves against the receiver's construction shape with the same
        // shared operation rule the completed chain uses, including the property-style `length`.
        if let Some(member) = resolve_map_builtin_member(
            MapBuiltinReceiver::Shape(resolved_shape),
            member_name,
            member_span,
            scope_context,
            type_interner,
            string_table,
        )? {
            let expected_shapes = member.rule.expected_argument_shapes;
            let success_shape = member.rule.success_shape;
            let mut arguments = ParsedCallArguments::Ready(Vec::new());

            if member.op.is_property() {
                if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
                    return Err(calls_data::invalid_builtin_call(
                        InvalidBuiltinCallReason::MapLengthIsProperty,
                        Some(member_name),
                        member_span,
                    )
                    .into());
                }

                validate_retained_step_access(
                    receiver_access,
                    chain_access,
                    member.op.requires_mutable_receiver(),
                    &ReceiverAccessDiagnostic::MapBuiltin {
                        method_name: member_name,
                    },
                    member_span,
                )?;

                token_stream.advance();

                if token_stream.current_tag().is_assignment_operator() {
                    return Err(names_declarations::invalid_assignment_target(
                        InvalidAssignmentTargetReason::ReadOnlyMapProperty,
                        None,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    )
                    .into());
                }
            } else {
                if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
                    return Err(calls_data::invalid_builtin_call(
                        InvalidBuiltinCallReason::MissingParentheses,
                        Some(member_name),
                        member_span,
                    )
                    .into());
                }

                validate_retained_step_access(
                    receiver_access,
                    chain_access,
                    member.op.requires_mutable_receiver(),
                    &ReceiverAccessDiagnostic::MapBuiltin {
                        method_name: member_name,
                    },
                    member_span,
                )?;

                token_stream.advance();

                let expectations = builtin_argument_expectations(builtin_argument_slots(
                    scope_context,
                    &expected_shapes,
                ));
                arguments = parse_call_arguments_typed_with_numeric_retention(
                    token_stream,
                    scope_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    &expectations,
                    CallArgumentSyntax::UnsupportedBuiltinMember {
                        member_name: Some(member_name),
                        takes_no_arguments: expected_shapes.is_empty(),
                    },
                    path_fork,
                )?;

                if matches!(member.op, MapBuiltinOp::Get)
                    && token_stream.current_tag().is_assignment_operator()
                {
                    // Same ordering as the collection lane: ordinary argument validation runs before
                    // the static target rejection; the validated result is discarded.
                    if let ParsedCallArguments::Ready(parsed) = arguments {
                        resolve_builtin_member_arguments(
                            BuiltinCallSite {
                                member_name: member.op.source_name(),
                                member_span,
                            },
                            parsed,
                            &expectations,
                            scope_context,
                            type_interner,
                            expression_builder,
                            const_values,
                            string_table,
                            path_fork,
                        )?;
                    }
                    return Err(names_declarations::invalid_assignment_target(
                        InvalidAssignmentTargetReason::MapGetTargetNotWritable,
                        None,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    )
                    .into());
                }
            }

            // A fallible builtin keeps its success slot as the receiver of a further member step,
            // exactly as the collection path does. Completion recovers the carrier before that step
            // reads it.
            steps.push(RetainedPostfixStep::Builtin(Box::new(
                RetainedBuiltinStep {
                    kind: RetainedBuiltinKind::Map(member.op),
                    member_name,
                    member_span,
                    arguments: arguments.map_values(OwnedExpr::id),
                    expected_argument_shapes: expected_shapes,
                    success_shape,
                    error_return_type_id: member.rule.error_return_type_id,
                    authored_handling: DeferredCallHandling::Unhandled,
                },
            )));
            current_shape = success_shape;
            receiver_access = RetainedReceiverAccess::temporary();
            continue;
        }

        if let Some(type_id) = current_type_id
            && let Some(method_entry) = lookup_receiver_method(
                scope_context,
                type_id,
                member_name,
                type_interner.environment(),
            )
        {
            if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
                return Err(calls_data::invalid_receiver_call(
                    InvalidReceiverCallReason::MustUseParentheses,
                    None,
                    Some(member_name),
                    None,
                    None,
                    member_span,
                )
                .into());
            }
            token_stream.advance();

            // This receiver remains a place only when every preceding step was a non-inlined field
            // projection; calls and computed results use temporary access rules.
            let access_diagnostic = ReceiverAccessDiagnostic::ReceiverMethod {
                method_name: member_name,
            };
            validate_retained_step_access(
                receiver_access,
                chain_access,
                method_entry.receiver_mutable,
                &access_diagnostic,
                member_span,
            )?;

            let initial_expectations = expectations_from_receiver_method_signature(
                &method_entry.signature.parameters[1..],
                path_fork,
            );
            let argument_syntax = CallArgumentSyntax::Supported {
                callee_name: Some(member_name),
            };
            let generic_template =
                scope_context.lookup_generic_function_template(&method_entry.function_path);
            let (method_path, signature, generic_request, arguments) = if let Some(template) =
                generic_template
            {
                let raw_arguments = parse_call_arguments_typed_with_expectations(
                    token_stream,
                    scope_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    &initial_expectations,
                    argument_syntax,
                    path_fork,
                )?;
                let instantiation =
                    instantiate_generic_receiver_method(GenericReceiverMethodInstantiationInput {
                        template,
                        receiver: GenericReceiverMethodReceiver::KnownType(type_id),
                        authored_arguments: &raw_arguments,
                        call_span: member_span,
                        scope_context,
                        type_interner,
                        expression_builder,
                        string_table,
                        path_fork,
                    })?;
                (
                    instantiation.instance_path,
                    instantiation.signature,
                    Some(instantiation.request),
                    raw_arguments
                        .into_iter()
                        .map(|argument| {
                            DeferredCallArgument::Ready(argument.map_value(OwnedExpr::id))
                        })
                        .collect(),
                )
            } else {
                let arguments = match parse_call_arguments_typed_with_numeric_retention(
                    token_stream,
                    scope_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    &initial_expectations,
                    argument_syntax,
                    path_fork,
                )? {
                    ParsedCallArguments::Ready(arguments) => arguments
                        .into_iter()
                        .map(|argument| {
                            DeferredCallArgument::Ready(argument.map_value(OwnedExpr::id))
                        })
                        .collect(),
                    ParsedCallArguments::Retained(arguments) => arguments,
                };
                (
                    method_entry.function_path,
                    method_entry.signature.clone(),
                    None,
                    arguments,
                )
            };

            let private_failure_lane =
                scope_context.source_call_has_private_failure_lane(method_entry.function_path);
            let private_failure_candidate =
                (private_failure_lane && signature.error_return_type_id().is_none()).then(|| {
                    if scope_context.generic_template_validation {
                        method_entry.function_path
                    } else {
                        method_path.to_owned()
                    }
                });
            let step_result_shape = signature
                .single_success_return_type_id()
                .map(|type_id| intern_concrete_shape(scope_context, type_id))
                .transpose()?;
            // The concrete request takes its ledger slot here, after every fallible step above, so a
            // failed parse never leaves a reserved slot behind. Template validation publishes
            // nothing and keeps no slot.
            let reserved_generic_request = match generic_request {
                Some(request) if !scope_context.generic_template_validation => {
                    Some(ReservedGenericRequest {
                        ticket: scope_context.reserve_pending_generic_request(),
                        request,
                    })
                }
                _ => None,
            };

            steps.push(RetainedPostfixStep::Method(Box::new(
                DeferredReceiverCall {
                    method_path,
                    method_name: member_name,
                    callee: DeferredReceiverCallee::Concrete(signature),
                    unchanged_expectations: if generic_template.is_none() {
                        Some(initial_expectations)
                    } else {
                        None
                    },
                    receiver: DeferredCallReceiver::Chained,
                    receiver_mutable: method_entry.receiver_mutable,
                    authored_marker_span: chain_access.authored_marker_span,
                    arguments,
                    call_span: member_span,
                    generic_request: reserved_generic_request,
                    private_failure_candidate,
                    authored_handling: DeferredCallHandling::Unhandled,
                },
            )));
            // A multi-slot call has no single value, so the next member, if any, is rejected at
            // its own step instead of ending the chain with a pending error.
            current_shape = step_result_shape;
            receiver_access = RetainedReceiverAccess::temporary();
            continue;
        }

        // A construction-only shape has no canonical receiver type, so only the typed arms above
        // can match it. The rejected member is kept for its completed receiver to report.
        let Some(current_type_id) = current_type_id else {
            return rejected_chain_member(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                steps,
                body_symbol::PendingNumericMemberAccess {
                    member_name,
                    member_span,
                },
            );
        };

        // An unresolved generic receiver keeps its generic method deferral: bound dispatch is the
        // later generic-consumer work, and no numeric type may be searched for a matching method.
        if generic_parameter_receiver_type(current_type_id, type_interner.environment()) {
            return rejected_chain_member(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                steps,
                body_symbol::PendingNumericMemberAccess {
                    member_name,
                    member_span,
                },
            );
        }

        // No handler matched on the known receiver type: the ordinary member diagnostic, exactly as
        // the completed chain reports it, with the known receiver type and its member names.
        let next_token = token_stream.token_ref_at_offset(1).map(|token| token.tag());
        return Err(unknown_member_diagnostic(
            current_type_id,
            member_name,
            member_span,
            next_token,
            type_interner,
            path_fork,
        ));
    }

    Ok(RetainedPostfixSteps {
        steps,
        invalid_member: None,
    })
}

/// The ordinary member diagnostic one known receiver type reports for a member it does not carry.
///
/// WHAT: classifies the rejection as a choice payload, an external member, or an unknown member,
///       and lists the receiver type's own member names.
/// WHY: the parse-time rejection of a known receiver and the completion-time rejection of a
///      declared envelope's concrete owner must report the same diagnostic.
fn unknown_member_diagnostic(
    receiver_type_id: TypeId,
    member_name: StringId,
    member_span: Option<SourceSpan>,
    next_token: Option<TokenTag>,
    type_interner: &AstTypeInterner<'_>,
    path_fork: &PathInternerFork,
) -> ExpressionParseError {
    let reason = if type_interner
        .environment()
        .variants_for(receiver_type_id)
        .is_some()
        && next_token != Some(TokenTag::OPEN_PARENTHESIS)
    {
        if next_token.is_some_and(TokenTag::is_assignment_operator) {
            InvalidFieldAccessReason::ChoicePayloadMutation
        } else {
            InvalidFieldAccessReason::ChoicePayloadDeferred
        }
    } else if type_id_is_external(receiver_type_id, type_interner) {
        InvalidFieldAccessReason::UnknownExternalMember
    } else {
        InvalidFieldAccessReason::UnknownMember
    };

    let known_fields = collect_known_member_names(receiver_type_id, type_interner, path_fork);
    calls_data::invalid_field_access(
        reason,
        Some(member_name),
        Some(receiver_type_id),
        known_fields,
        member_span,
    )
    .into()
}

/// Build one retained field step from its resolved facts.
fn retained_field_step(field: ResolvedFieldMember, span: Option<SourceSpan>) -> RetainedFieldStep {
    RetainedFieldStep {
        field_name: field.field_name,
        type_id: field.type_id,
        value_mode: field.value_mode,
        const_record_state: field.const_record_state,
        const_inline_value: field.const_inline_value,
        span,
    }
}

/// The authored `!` propagation suffix at the current token, when one is authored.
///
/// WHAT: reports the exact operator span; `!=` is never propagation and stays for the ordinary
///       common-mistake diagnostic.
pub(crate) fn authored_propagation_suffix(
    token_stream: &AstCursor<'_>,
) -> Option<Option<SourceSpan>> {
    (token_stream.current_tag() == TokenTag::BANG
        && token_stream.peek_next_tag() != Some(TokenTag::ASSIGN))
    .then(|| Some(token_stream.current_span()))
}

/// Parse the authored member syntax of one already-rejected occurrence through the shared owners.
///
/// WHAT: consumes the authored dot, the member name and every following argument list through the
///       shared member-name and call-argument owners, then consumes any further authored steps the
///       same way; returns the rejected member's name and exact span for its delayed diagnostic.
/// WHY: the member has no signature at any numeric type, so its arguments cannot be validated — but
///      they are authored syntax whose own nested uses must still register, and delimiter or
///      postfix bookkeeping must stay in the shared owners instead of a second scanner.
///
/// PRECONDITION: the cursor is on the authored dot.
fn consume_invalid_member_access(
    token_stream: &mut AstCursor<'_>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(StringId, Option<SourceSpan>), ExpressionParseError> {
    let dot_span = Some(token_stream.current_span());
    token_stream.advance();

    if token_stream.position() >= token_stream.length()
        || token_stream.current_tag() == TokenTag::EOF
    {
        return Err(calls_data::invalid_field_access(
            InvalidFieldAccessReason::ExpectedNameAfterDot,
            None,
            None,
            Vec::new(),
            dot_span,
        )
        .into());
    }

    let member_span = Some(token_stream.current_span());
    let member_name = parse_member_name_typed(token_stream, string_table)?;
    consume_rejected_member_steps(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;
    Ok((member_name, member_span))
}

/// Consume one rejected member and every following step without recursing over authored input.
///
/// WHAT: parses each member and argument list through their shared owners, retaining no callee
///       contract for the rejected member.
/// WHY: nested calls keep their own parameter expectations, and chain length must not consume
///      compiler stack or introduce another delimiter/member grammar.
fn consume_rejected_member_steps(
    token_stream: &mut AstCursor<'_>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    loop {
        // `parse_member_name_typed` only reads the name token, so consume it before checking for
        // arguments or the next dot. This also resumes bare properties at their continuation.
        token_stream.advance();

        if token_stream.current_tag() == TokenTag::OPEN_PARENTHESIS {
            // The unknown member has no callee identity or contract. Parse its ordinary named and
            // positional values once so nested real calls still register.
            let _ = parse_call_arguments_without_expectations(
                token_stream,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?;
        }

        if token_stream.current_tag() != TokenTag::DOT {
            return Ok(());
        }
        token_stream.advance();
        if token_stream.position() >= token_stream.length()
            || token_stream.current_tag() == TokenTag::EOF
        {
            return Err(calls_data::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                Some(token_stream.current_span()),
            )
            .into());
        }
        parse_member_name_typed(token_stream, string_table)?;
    }
}

/// Consume a rejected member and its following steps, keeping the steps resolved before it.
///
/// WHAT: consumes the rejected member's authored syntax, then returns the chain-so-far with that
///       member as the occurrence its completed receiver reports.
/// WHY: a rejected member has no type until the receiver completes, so the ordinary diagnostic
///      waits for that type while the authored syntax after it still parses and registers its uses.
///
/// PRECONDITION: the cursor is on the rejected member's name, which has been read but not consumed.
#[allow(
    clippy::too_many_arguments,
    reason = "rejected member reporting keeps the token stream, scope, type interner, expression builder, const store, string table, path fork, steps, and occurrence as separate borrows"
)]
fn rejected_chain_member(
    token_stream: &mut AstCursor<'_>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    steps: Vec<RetainedPostfixStep>,
    occurrence: body_symbol::PendingNumericMemberAccess,
) -> Result<RetainedPostfixSteps, ExpressionParseError> {
    consume_rejected_member_steps(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;
    Ok(RetainedPostfixSteps {
        steps,
        invalid_member: Some(occurrence),
    })
}
