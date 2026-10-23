//! Body-owned numeric construction state for the scalar inference path.
//!
//! WHAT: one dense origin/equality table per independent function root. It retains
//!       eligible pending numeric bindings, their exact receiving witnesses, their
//!       retained literal recipes and the narrow continuation records that resume
//!       numeric-dependent statement construction once, at the body root.
//! WHY: numeric receivers, copies and writes may appear anywhere in the body, so a
//!      binding's single static type is selected from every permitted use before any
//!      default applies. Construction state never enters canonical type identity,
//!      completed AST payloads, HIR, or public interfaces; the body-root completion
//!      materialises every retained literal once and releases the tables.
//!
//! Ownership: this module is construction-only. It must not mirror statements, and it
//! must not fabricate a `Declaration`, `Expression` or `TypeId` for an unresolved value.
//!
//! Expression recipes retain raw four-byte IDs. Completion attaches those IDs to the phase
//! builder only when they re-enter construction and strips transient owner handles before
//! retaining a completed result.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, MultiBindTarget, MultiBindTargetKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_validation::ParameterExpectation;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::failure_facts::TypedErrorProducer;
use crate::compiler_frontend::ast::expressions::generic_nominal_retention::DeferredGenericNominalCall;
use crate::compiler_frontend::ast::generic_functions::numeric_origins::GenericErrorReturn;
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionTemplate, GenericRequestTicket,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeFrameId;
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeBinding, KnownShapeBindingRange, KnownShapeId,
    numeric_classes_join_refused, register_ordered_shape_witness, register_shape_witness,
    union_shape_classes,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, RelationalPatternOp,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::TypeMismatchContext;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::type_mismatch;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId, builtin_type_ids};
use crate::compiler_frontend::instrumentation::ast_counters::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::type_coercion::parse_context::{
    ExpectedCollectionContext, ExpectedMapContext, ExpectedType,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};
use moth_lexical::numeric::grammar::NumericLiteralKind;
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Identity of one independent body-owned numeric construction domain.
///
/// WHAT: a process-unique key for the origin tables one body root owns.
/// WHY: module-wide stores retain records while several bodies parse (a nested function body
///      completes before its declaring body). A record keyed by its owning domain lets a body
///      take exactly its own work; no binding, recipe or continuation may cross domains, and a
///      dense ID from one domain must never be interpreted against another domain's tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NumericConstructionDomainId(u64);

/// Distinguishes independently owned construction domains for the lifetime of the process.
static NEXT_NUMERIC_CONSTRUCTION_DOMAIN: AtomicU64 = AtomicU64::new(1);

impl NumericConstructionDomainId {
    fn fresh() -> Self {
        Self(NEXT_NUMERIC_CONSTRUCTION_DOMAIN.fetch_add(1, Ordering::Relaxed))
    }
}

/// Dense checked binding ID inside one body-owned construction domain.
///
/// Valid only with the `NumericConstructionState` that created it. It never substitutes
/// for a `TypeId`, `PathId` or `Declaration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NumericBindingId(u32);

impl NumericBindingId {
    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Address one existing binding row by its dense table index.
    ///
    /// WHY: the completion driver walks a table by index; the compact ID must be rebuilt only
    ///      from a checked index, never from arbitrary arithmetic.
    pub(crate) fn from_dense_index(index: usize) -> Result<Self, CompilerError> {
        checked_dense_id(index, "binding").map(Self)
    }
}

/// Dense origin/equality node ID inside one body-owned construction domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NumericClassId(u32);

impl NumericClassId {
    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Dense continuation ID for one retained numeric-dependent statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NumericContinuationId(u32);

impl NumericContinuationId {
    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Dense forward-producer ID for one retained forward result awaiting its class.
///
/// WHAT: names one registry row that holds a forward fragment until its natural result is computed.
/// WHY: a forward result is fixed by its own operation, so it completes once, before a connected
///      class chooses a default, and every consumer reuses that single result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ForwardProducerId(u32);

impl ForwardProducerId {
    pub(crate) fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Address one existing registry row by its dense table index.
    pub(crate) fn from_dense_index(index: usize) -> Result<Self, CompilerError> {
        checked_dense_id(index, "forward producer").map(Self)
    }
}

/// One retained forward fragment and its lifecycle in the registry.
///
/// WHAT: the fragment's class and shape, plus its state: pending, resolving (its completion is on the
///       stack), completed (its natural expression is ready), taken, or abandoned.
/// WHY: each authored fragment completes exactly once; a demand that re-enters a resolving producer
///      skips it, so a genuine cycle falls back to its class's ordinary defaults.
#[derive(Debug)]
struct ForwardProducer {
    class: NumericClassId,
    shape: KnownShapeId,
    authored_order: u64,
    state: ForwardProducerState,
}

#[derive(Debug)]
// Keep pending fragments inline in the dense registry rather than allocating per producer.
// Completed results retain only their committed identity until the one-shot take.
#[allow(clippy::large_enum_variant)]
enum ForwardProducerState {
    Pending(DeferredNumericExpression),
    Resolving,
    Completed(ExprId),
    Taken,
}

/// Authored literal family of a retained recipe.
///
/// WHAT: whether a raw literal recipe contains only whole-number tokens or at least one
///       decimal/exponent token. The family selects the I32/F64 default when no receiver
///       resolves the class and rejects an incompatible receiver during materialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericFamily {
    Whole,
    Decimal,
}

impl NumericFamily {
    /// Merge two authored families; any decimal token selects the floating family.
    pub(crate) fn merge(self, other: NumericFamily) -> NumericFamily {
        match (self, other) {
            (NumericFamily::Whole, NumericFamily::Whole) => NumericFamily::Whole,
            _ => NumericFamily::Decimal,
        }
    }

    /// The language default for an unconstrained binding in this family.
    pub(crate) fn default_type_id(self) -> TypeId {
        match self {
            NumericFamily::Whole => builtin_type_ids::fixed_scalar(FixedScalar::I32),
            NumericFamily::Decimal => builtin_type_ids::fixed_scalar(FixedScalar::F64),
        }
    }

    /// Derive the family of a retained recipe from its authored literal tokens.
    ///
    /// WHAT: one flat scan of the ordered fragment. Group markers carry no literal token.
    /// WHY: groups stay flat in the RPN stream, so a decimal token anywhere inside parentheses
    /// selects the floating family for the whole recipe.
    pub(crate) fn of_items(items: &[ExpressionRpnItem]) -> NumericFamily {
        let mut family = NumericFamily::Whole;
        for item in items {
            if let ExpressionRpnItem::PendingNumericLiteral { token, .. } = item
                && token.kind != NumericLiteralKind::WholeNumber
            {
                family = NumericFamily::Decimal;
            }
        }
        family
    }
}

/// One authored receiving site that requires an exact type for a class.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReceiverWitness {
    pub(crate) type_id: TypeId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) authored_order: u64,
}

/// One real producer fact for a retained value-production slot.
///
/// WHAT: either a concrete authored `TypeId` or the exact `then` value whose numeric
///       recipe still determines the slot.
/// WHY: multi-bind targets must be declared at their source position without inventing a
///      type or copying a producer expression; pending records keep only stable continuation
///      and slot IDs.
#[derive(Debug, Clone)]
pub(crate) enum MultiBindProducerFact {
    Known(TypeId),
    Pending(PendingMultiBindProducer),
}

/// How one retained `then` value establishes a multi-bind slot.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PendingMultiBindProducerKind {
    /// A direct reference shares the source binding's numeric equality class.
    Binding(NumericBindingId),
    /// A literal-only recipe is selected by the slot's receiver or its authored family default.
    Raw(NumericFamily),
    /// A directed result resolves from its operands and only then validates the slot receiver.
    Forward,
}

/// Routing to one retained `then` value that contributes to a multi-bind slot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingMultiBindProducer {
    pub(crate) continuation: NumericContinuationId,
    pub(crate) value_index: usize,
    pub(crate) kind: PendingMultiBindProducerKind,
    pub(crate) span: Option<SourceSpan>,
    /// The literal shape this producer's fragment owns, when it is one retained collection or map.
    ///
    /// WHY: only a literal carries its own known shape; scalar arithmetic and other fragments
    ///      answer `None` so no shape is invented for them.
    pub(crate) shape: Option<KnownShapeId>,
}

/// What a pending numeric binding's initializer is.
#[derive(Debug, Clone)]
pub(crate) enum NumericRecipe {
    /// Literal-only recipe: a receiver may type its raw leaves directly.
    Raw {
        items: Vec<ExpressionRpnItem>,
        family: NumericFamily,
    },
    /// Recipe over named operands: forward-only, resolved from its operands.
    ///
    /// WHAT: `shape` is the directed value's known shape when its fragment is one retained literal
    /// or one operand with a known shape; `None` for named arithmetic.
    /// WHY: the binding relation reads this recipe alone, so the shape is decided once at
    /// registration.
    Directed {
        items: Vec<ExpressionRpnItem>,
        shape: Option<KnownShapeId>,
    },
    /// An inferred multi-bind target whose type comes from retained `then` producer slots.
    ///
    /// WHAT: compact producer routes plus their direct numeric origins, never cloned RPN.
    /// WHY: the target is visible to later statements before its value block completes; body-root
    ///      completion selects its type from the real producers and then completes the block once.
    MultiBindSlot {
        producers: Vec<PendingMultiBindProducer>,
        /// The one known shape merged from every producer's known source, when any exists.
        ///
        /// WHY: the slot owns its merged shape once so receivers read it without re-querying each
        ///      source binding.
        shape: Option<KnownShapeId>,
    },
    /// Copy/equality relationship with another binding's origin class.
    ///
    /// WHAT: retains the source binding (creation order guarantees it is already declared) and
    ///      whether the author wrote `copy`, so completion can build the exact authored shape.
    /// WHY: a copy shares the source's numeric constraint set; remembering the authored form
    ///      keeps the completed declaration faithful without a second expression mirror.
    Alias {
        source: NumericBindingId,
        is_copy: bool,
    },
    /// One element of a pending collection source, typed from that source's element node.
    ///
    /// WHAT: names the element node of the source literal's known shape (a leaf, a concrete type or a
    ///       nested shape). A pending multi-bind target of a generic call's declared result names
    ///       that result's shape the same way.
    /// WHY: a loop item is a numeric position of its source literal. Sharing the node keeps its
    ///      witnesses on the same origin classes without naming a source binding, so inline literal
    ///      sources, projected nested shapes and generic results need no hidden binding.
    CollectionElement { element: KnownShapeId },
    /// A pending `catch` handler's error binding: type-only, with no initializer of its own.
    ///
    /// WHAT: names the generic error return shape of the protected call; the binding's type is that
    ///       shape materialised once its forward producers settle.
    /// WHY: `err` in a handler over a numeric-dependent generic error must read the same origin
    ///      classes as its producer, so the name carries the shape instead of a fabricated type;
    ///      the runtime payload stays the existing catch edge.
    CaughtError { shape: KnownShapeId },
    /// A pending `match` payload capture: its type is the payload field shape through the
    /// subject's bindings, materialised once its forward producers settle.
    ///
    /// WHAT: names the payload's envelope shape; covers a choice field of a retained generic
    ///       subject and the payload of a retained optional subject.
    /// WHY: a late read of the capture witnesses the origin's generic parameter, so the capture
    ///      must keep the origin classes of its constructor argument instead of a fixed type.
    PatternPayload { shape: KnownShapeId },
    /// A range loop counter: its type is the range iteration domain selected from this header's
    /// completed operands.
    ///
    /// WHAT: owns the retained header slots, completes them once, selects the exact domain
    ///      through the shared range policy and hands the finished spec to the loop construct.
    /// WHY: the counter's type is a forward result of its operands' selected types, and the
    ///      operands belong to a loop construct the counter's own body cannot wait for; keeping
    ///      the slots here resolves the counter before any binding that reads it, so header
    ///      completion stays independent of loop-body resumption.
    RangeCounter { range: Box<DeferredRangeSpec> },
}

/// One construction-only pending numeric binding.
#[derive(Debug, Clone)]
pub(crate) struct PendingBindingRow {
    pub(crate) name: StringId,
    pub(crate) path: PathId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) binding_span: Option<SourceSpan>,
    pub(crate) value_mode: ValueMode,
    pub(crate) mutable: bool,
    pub(crate) class: NumericClassId,
    /// The retained recipe, taken exactly once by body-root completion.
    ///
    /// WHY: the authored fragment belongs to completion; the row keeps it until the binding
    ///      resolves and hands it over by move (`None` afterwards) instead of cloning RPN per
    ///      probe or resolution.
    pub(crate) recipe: Option<NumericRecipe>,
    pub(crate) origin: BindingOriginQueries,
    /// Scope frame that authored the binding, retained so completion upgrades the canonical
    /// name slot in place.
    pub(crate) frame: ScopeFrameId,
}

/// Birth-time origin facts shared by every binding-origin query.
///
/// WHAT: aliases inherit the source's fully registered declaration facts in one constant-time read.
///       The source is already declared, including its annotation and partial reference view.
///       A binding's own annotation or view then overrides the inherited counterpart.
/// WHY: these facts survive recipe removal at completion. Reading an alias must neither walk
///      flat source bindings nor lose a type boundary when completion moves its recipe. Equality
///      classes remain separate: joining a raw and a forward origin does not change eligibility.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BindingOriginQueries {
    known_shape: Option<KnownShapeId>,
    annotation: Option<KnownShapeId>,
    reference_view: Option<KnownShapeId>,
    is_forward: bool,
}

/// How strongly a numeric origin pins its family against later exact peers.
///
/// WHAT: `Bound` is an origin fixed by a binding, shape or caught error. Its admitted receivers
///       come from its own merged family, so a bound whole keeps integer receivers and a bound
///       binary float keeps binary float receivers. `LoopItem` is a whole origin read as a
///       collection loop item, which admits only integer receivers. The stronger pin wins when
///       classes merge.
/// WHY: the family already lives on the row, so the pin records only that a binding fixed it.
///      Every pin shares the one receiver policy in `family_receives_recipe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum NumericPin {
    /// No binding pins this origin.
    #[default]
    Unpinned,
    /// A bound origin: its merged family is fixed at every nesting level, so peers may receive it
    /// but never reclassify it.
    Bound,
    /// A whole origin read as a collection loop item: every exact peer must be integer, and a
    /// later integer receiver still selects the width.
    LoopItem,
}

/// One origin/equality node. Non-root rows keep only the union-find link.
#[derive(Debug, Clone)]
struct ClassRow {
    parent: u32,
    rank: u8,
    /// Earliest exact receiver and earliest receiver with a different type, by authored key.
    first_receiver: Option<ReceiverWitness>,
    differing_receiver: Option<ReceiverWitness>,
    /// Authored literal family merged from the leaves of this origin; `None` until a literal leaf
    /// or bound declaration notes one.
    family: Option<NumericFamily>,
    /// How strongly this origin pins its family against later peers.
    pin: NumericPin,
    /// Earliest non-integer receiver or refused peer, retained even before a loop pin.
    loop_ineligible_receiver: Option<ReceiverWitness>,
    /// Created for a named forward fragment: its natural result is established by completion
    /// before any default applies. Union ORs it into the surviving root, so every merged member
    /// reports the forward fact whichever row won.
    forward: bool,
    /// Pending producer candidates in registration order; union moves the loser's index here.
    forward_producers: BinaryHeap<Reverse<u32>>,
}

impl ClassRow {
    /// If insertion changes the first type, the old first becomes the earliest differing
    /// witness: it preceded every old witness, including those omitted from the summary.
    fn insert_receiver(&mut self, witness: ReceiverWitness) {
        let Some(first) = self.first_receiver else {
            self.first_receiver = Some(witness);
            return;
        };
        increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        let key = |witness: ReceiverWitness| (witness.authored_order, witness.type_id.0);
        if witness.type_id == first.type_id {
            if key(witness) < key(first) {
                self.first_receiver = Some(witness);
            }
        } else if key(witness) < key(first) {
            self.first_receiver = Some(witness);
            self.differing_receiver = Some(first);
        } else {
            retain_earliest_receiver(&mut self.differing_receiver, witness);
        }
    }
}

fn retain_earliest_receiver(summary: &mut Option<ReceiverWitness>, witness: ReceiverWitness) {
    if let Some(existing) = *summary {
        increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        if (existing.authored_order, existing.type_id.0)
            <= (witness.authored_order, witness.type_id.0)
        {
            return;
        }
    }
    *summary = Some(witness);
}

/// One retained expression whose construction needs resolved numeric origins.
///
/// WHAT: the ordered RPN fragment (still containing pending literal/binding/cast items),
///       the receiving expectation and the authored scope that owned the position.
/// WHY: numeric-dependent positions must resume the existing concrete constructor and
///      validation once, in their original body position, without re-parsing tokens.
#[derive(Debug, Clone)]
pub(crate) struct DeferredNumericExpression {
    pub(crate) items: Vec<ExpressionRpnItem>,
    pub(crate) expected: ExpectedType,
    pub(crate) value_mode: ValueMode,
    pub(crate) span: Option<SourceSpan>,
    /// Authored trailing handling already consumed at the deferral point.
    ///
    /// WHY: `!` propagation is decided from the token stream before numeric resolution, so the
    ///      resumed expression must apply the exact authored suffix instead of re-reading tokens.
    pub(crate) suffix: DeferredExpressionSuffix,
}

/// Build the retained-expression record for one raised numeric deferral.
pub(crate) fn deferred_numeric_expression(
    items: Vec<ExpressionRpnItem>,
    expected: ExpectedType,
    value_mode: ValueMode,
    span: Option<SourceSpan>,
    suffix: DeferredExpressionSuffix,
) -> DeferredNumericExpression {
    DeferredNumericExpression {
        items,
        expected,
        value_mode,
        span,
        suffix,
    }
}

/// What followed a deferred expression at its receiving position.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeferredExpressionSuffix {
    /// Nothing followed the expression.
    None,
    /// Authored postfix `!` propagation, with its exact span.
    Propagate { span: Option<SourceSpan> },
    /// Authored postfix optional propagation (`?`), with its exact span.
    OptionalPropagate { span: Option<SourceSpan> },
}

/// Routing and authored metadata of one call argument, retained without its value.
#[derive(Debug, Clone)]
pub(crate) struct DeferredCallArgumentShape {
    pub(crate) target_param: Option<StringId>,
    pub(crate) access_mode: CallAccessMode,
    pub(crate) marker_span: Option<SourceSpan>,
    pub(crate) value_span: Option<SourceSpan>,
    pub(crate) target_span: Option<SourceSpan>,
    pub(crate) parameter_slot: Option<ParameterSlot>,
    pub(crate) following_separator_span: Option<SourceSpan>,
    /// The numeric origin of a retained argument routed to a declared generic parameter. `None`
    /// for arguments that are complete or carry no generic-linked origin.
    pub(crate) origin_shape: Option<KnownShapeId>,
}

/// A typed-error producer as a fragment summary records it before its numeric origins settle.
///
/// WHAT: a completed producer carries its concrete error type; a pending producer is a retained
///       generic call whose error type a numeric origin decides at completion.
/// WHY: a pending producer is still an actual typed error for eligibility, so forward metadata
///      must not drop it or default its error type when the fragment summary is captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypedErrorFact {
    Completed(TypedErrorProducer),
    Pending {
        span: Option<SourceSpan>,
        error_return: KnownShapeId,
    },
}

/// One call argument: completed, or retained until numeric resolution.
#[derive(Debug, Clone)]
// `Ready` moves a completed argument into its retained argument list once. Boxing it would add
// one heap allocation per ready argument joining such a list.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DeferredCallArgument<E = ExprId> {
    Ready(CallArgument<E>),
    Retained {
        shape: DeferredCallArgumentShape,
        expression: DeferredNumericExpression,
    },
    /// A forward fragment held by the registry; its class completes it at the first demand.
    Forward {
        shape: DeferredCallArgumentShape,
        producer: ForwardProducerId,
        /// The fragment's first and conflicting typed-error producers, captured at registration
        /// because its items no longer travel with the argument.
        typed_errors: (Option<TypedErrorFact>, Option<TypedErrorFact>),
    },
}

impl<E> DeferredCallArgument<E> {
    /// Change ready edges without changing retained numeric recipes.
    pub(crate) fn map_value<T>(self, map: impl FnOnce(E) -> T) -> DeferredCallArgument<T> {
        match self {
            Self::Ready(argument) => DeferredCallArgument::Ready(argument.map_value(map)),
            Self::Retained { shape, expression } => {
                DeferredCallArgument::Retained { shape, expression }
            }
            Self::Forward {
                shape,
                producer,
                typed_errors,
            } => DeferredCallArgument::Forward {
                shape,
                producer,
                typed_errors,
            },
        }
    }
}

/// Authored handling consumed at a call's receiving site.
///
/// WHAT: whether anything followed the call when it was retained, and the exact span of an
///       authored postfix `!`.
/// WHY: `!` is decided from the token stream at retention time, so a retained call must carry the
///      authored decision; resuming an authored `!` as an unhandled value would silently drop the
///      propagation, and re-reading tokens at the body root is forbidden.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeferredCallHandling {
    /// Nothing followed the call at its receiving site.
    Unhandled,
    /// Authored postfix `!` propagation, with its operator span.
    Propagate { span: Option<SourceSpan> },
}

/// A non-generic source function call retained until its numeric arguments resolve.
///
/// WHAT: stable callee identity, the ordered success result types and optional error type, the
///       expectations used at argument intake, and the ordered arguments, each completed or
///       retained.
/// WHY: compatibility cannot run before argument types are selected; retaining the unchanged
///      expectations and only the result facts completion reads avoids rebuilding parameter views
///      or cloning the complete signature when the call resumes.
#[derive(Debug, Clone)]
pub(crate) struct DeferredSourceCall {
    pub(crate) function_path: PathId,
    pub(crate) success_result_type_ids: Vec<TypeId>,
    pub(crate) error_return_type_id: Option<TypeId>,
    pub(crate) expectations: Vec<ParameterExpectation>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) call_span: Option<SourceSpan>,
    /// The authored handling consumed at the call's receiving site.
    pub(crate) authored_handling: DeferredCallHandling,
}

/// A generic free-function call retained until its numeric-dependent arguments resolve.
///
/// WHAT: the parsed callee template, the ordered argument list (each completed or retained), the
///       declared success slots as known shapes linked to their numeric origins, and the reserved
///       instantiation-request ticket.
/// WHY: the concrete signature cannot be matched before the origins it links to are selected. The
///      parse keeps argument routing and exact spans so completion never parses arguments again,
///      and the request publishes at completion through its ticket in source order.
#[derive(Debug, Clone)]
pub(crate) struct DeferredGenericCall {
    pub(crate) template: GenericFunctionTemplate,
    /// Intake expectations stay valid through numeric completion; retaining them avoids projecting
    /// the same template parameter view again before inference.
    pub(crate) parameter_expectations: Vec<ParameterExpectation>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) call_span: Option<SourceSpan>,
    /// The authored handling consumed at the call's receiving site.
    pub(crate) authored_handling: DeferredCallHandling,
    /// One known shape per declared success slot, linked to the numeric origins it carries.
    pub(crate) result_shapes: Vec<KnownShapeId>,
    /// The owned immediate expected-result types; completion runs after the receiving site.
    pub(crate) expected_result_type_ids: Option<Box<[TypeId]>>,
    /// The callee's declared error slot as linked from its numeric origins at parse time.
    ///
    /// WHAT: `Retained` names a generic error whose type a numeric origin still decides; it stays
    ///       pending until completion. `None` means the callee declares no error slot.
    /// WHY: a pending error is never collapsed into `None`, so the parse-time link can be checked
    ///      exactly against the completed signature.
    pub(crate) error_return: Option<GenericErrorReturn>,
    /// The source-order slot this call publishes its instantiation request into. `None` marks the
    /// template-validation lane, which instantiates nothing.
    pub(crate) ticket: Option<GenericRequestTicket>,
}

/// A call-shaped expression retained in one RPN operand position.
#[derive(Debug, Clone)]
pub(crate) enum DeferredCall {
    /// A free (non-receiver) source function call.
    Source(Box<DeferredSourceCall>),
    /// A source method call whose receiver is already an ordinary concrete value.
    ///
    /// WHY: argument compatibility for a known concrete receiver cannot run before the
    ///      arguments' numeric origins are selected, while the callee, receiver expression and
    ///      slot routing are already concrete; the payload keeps those facts with the producer
    ///      that parsed them, and completion resumes the ordinary method constructor once.
    Receiver(Box<crate::compiler_frontend::ast::field_access::DeferredReceiverCall>),
    /// An external-package function call whose numeric-dependent arguments need resolution.
    ///
    /// WHY: the external ABI's parameter slots are already concrete, while an argument that
    ///      references a pending numeric origin has no value yet; the producer keeps the callee
    ///      identity and argument routing, and completion resumes the ordinary host constructor
    ///      exactly once.
    External(Box<crate::compiler_frontend::ast::expressions::function_calls::DeferredExternalCall>),
    /// A struct constructor whose numeric-dependent field arguments need resolution.
    ///
    /// WHY: the constructed nominal type is the call's single success value while an argument that
    ///      references a pending numeric origin has no value yet; the producer owns the constructor
    ///      and resumes it exactly once at the body root.
    Struct(Box<crate::compiler_frontend::ast::expressions::struct_instance::DeferredStructCall>),
    /// A choice-variant constructor whose numeric-dependent arguments need resolution.
    ///
    /// WHY: the choice type is the call's single success value while an argument that references a
    ///      pending numeric origin has no value yet; the producer re-derives the variant fields at
    ///      completion and resumes the ordinary constructor exactly once.
    Choice(Box<crate::compiler_frontend::ast::expressions::choice_constructor::DeferredChoiceCall>),
    /// A generic free-function call retained until its numeric-dependent arguments resolve.
    ///
    /// WHY: the concrete signature is selected only after the linked origins resolve; the producer
    ///      keeps the template, argument routing and reserved ticket, and completion runs the
    ///      shared generic resolver exactly once.
    Generic(Box<DeferredGenericCall>),
    /// A generic struct or choice constructor whose numeric-dependent arguments need resolution.
    ///
    /// WHY: the instance is selected only after its linked origins resolve, so the producer keeps
    ///      the declared self shape, the seeded bindings and the authored fields or variant;
    ///      completion infers and finishes the instance exactly once. It carries no call count,
    ///      error slot or bang semantics, so it is not a `Struct` or `Choice` call.
    GenericNominal(Box<DeferredGenericNominalCall>),
}

impl DeferredCall {
    /// The ordered success-slot types of this retained call's parsed signature.
    ///
    /// WHAT: the concrete result slots the authored callee declares, read without completing the
    ///       call. A generic call has none until its numeric origins resolve, so it reports `None`
    ///       instead of a declared slot list that still names generic parameters.
    /// WHY: multi-bind targets and catch sites need exact slot types while the value is pending;
    ///      the external lane supplies its own host-signature projection.
    pub(crate) fn success_return_type_ids(&self) -> Option<Vec<TypeId>> {
        Some(match self {
            DeferredCall::Source(deferred) => deferred.success_result_type_ids.clone(),
            DeferredCall::Receiver(deferred) => {
                deferred.concrete_signature()?.success_return_type_ids()
            }
            DeferredCall::External(deferred) => deferred.result_type_ids.clone(),
            DeferredCall::Struct(deferred) => vec![deferred.type_id],
            DeferredCall::Choice(deferred) => vec![deferred.choice_type_id],
            DeferredCall::Generic(_) | DeferredCall::GenericNominal(_) => return None,
        })
    }

    /// The declared success-slot count, read without completing the call or allocating a list.
    ///
    /// WHAT: counts every success slot of a resolved signature, and of a generic template whose
    ///       generic parameters stay unbound until completion.
    /// WHY: the authored `!` decision needs only whether the call declares a value, and a generic
    ///      call declares one before its numeric origins resolve.
    pub(crate) fn declared_success_slot_count(&self) -> usize {
        match self {
            DeferredCall::Source(deferred) => deferred.success_result_type_ids.len(),
            DeferredCall::Receiver(deferred) => {
                deferred.declared_signature().success_return_slot_count()
            }
            DeferredCall::External(deferred) => deferred.result_type_ids.len(),
            DeferredCall::Struct(_) | DeferredCall::Choice(_) | DeferredCall::GenericNominal(_) => {
                1
            }
            DeferredCall::Generic(deferred) => {
                deferred.template.signature.success_return_slot_count()
            }
        }
    }

    /// The single declared success-slot type, read only for the optional-operand decision.
    ///
    /// WHAT: a generic call reports its template's declared slot, whose generic parameters stay
    ///       unbound until completion.
    /// WHY: the `!` wording asks only whether that slot is an `Option`, which the declared type
    ///      answers exactly; the generic type never reaches a value or a diagnostic.
    pub(crate) fn declared_single_success_return_type_id(&self) -> Option<TypeId> {
        match self {
            DeferredCall::Generic(deferred) => {
                deferred.template.signature.single_success_return_type_id()
            }
            DeferredCall::Receiver(deferred) => deferred
                .declared_signature()
                .single_success_return_type_id(),
            other => other.single_success_return_type_id(),
        }
    }

    /// The exact result type of a retained single-success-slot call, without allocating a slot list.
    ///
    /// WHAT: delegates source and receiver signatures to their shared scalar projection and reads
    ///       already-stored result types for the other call variants.
    /// WHY: producer discovery needs only the scalar result shape and must not turn absent or
    ///      multi-slot results into a fabricated type.
    pub(crate) fn single_success_return_type_id(&self) -> Option<TypeId> {
        match self {
            DeferredCall::Source(deferred) => match deferred.success_result_type_ids.as_slice() {
                [type_id] => Some(*type_id),
                _ => None,
            },
            DeferredCall::Receiver(deferred) => deferred.single_success_return_type_id(),
            DeferredCall::External(deferred) => match deferred.result_type_ids.as_slice() {
                [type_id] => Some(*type_id),
                _ => None,
            },
            DeferredCall::Struct(deferred) => Some(deferred.type_id),
            DeferredCall::Choice(deferred) => Some(deferred.choice_type_id),
            DeferredCall::Generic(_) | DeferredCall::GenericNominal(_) => None,
        }
    }

    /// The single result shape of a retained generic call with one declared success slot.
    ///
    /// WHAT: the shape the call's declared result links to, read without completing the call.
    /// WHY: a generic result has no exact type until completion, so a consumer that needs an
    ///      operand before then takes the shape and never a fabricated type.
    pub(crate) fn single_success_result_shape(&self) -> Option<KnownShapeId> {
        match self {
            DeferredCall::Generic(deferred) => match deferred.result_shapes.as_slice() {
                [shape] => Some(*shape),
                _ => None,
            },
            DeferredCall::Receiver(deferred) => deferred.single_success_result_shape(),
            DeferredCall::GenericNominal(deferred) => Some(deferred.result_shape),
            _ => None,
        }
    }

    /// The ordered result shapes of a retained generic call, one per declared success slot.
    ///
    /// WHAT: the shapes the declared results link to, read without completing the call. `None` for
    ///       every other call kind, whose results carry exact types.
    /// WHY: a multi-bind over a generic call binds each target to its declared slot's shape, so the
    ///      targets join the call's numeric origins before completion.
    pub(crate) fn generic_result_shapes(&self) -> Option<&[KnownShapeId]> {
        match self {
            DeferredCall::Generic(deferred) => Some(deferred.result_shapes.as_slice()),
            DeferredCall::Receiver(deferred) => deferred.generic_result_shapes(),
            _ => None,
        }
    }

    /// The fallible error return of the call's signature, when it declares one.
    ///
    /// WHAT: `Concrete` for a resolved error type; `Retained` for a generic call whose error type
    ///       depends on a numeric origin that completes later. `None` means no declared error slot.
    /// WHY: a pending error must stay distinct from an absent error, so consumers that need the
    ///      exact type match `Retained` explicitly and never read it as `None`.
    pub(crate) fn error_return(&self) -> Option<GenericErrorReturn> {
        match self {
            DeferredCall::Source(deferred) => deferred
                .error_return_type_id
                .map(GenericErrorReturn::Concrete),
            DeferredCall::Receiver(deferred) => deferred.error_return(),
            DeferredCall::External(deferred) => deferred
                .error_return_type_id
                .map(GenericErrorReturn::Concrete),
            // A constructor's own construction is infallible; a retained argument's typed error
            // still reaches the fragment's handler contract through its own failure facts.
            DeferredCall::Struct(_) | DeferredCall::Choice(_) | DeferredCall::GenericNominal(_) => {
                None
            }
            DeferredCall::Generic(deferred) => deferred.error_return,
        }
    }

    /// The authored span of this retained call, used by statement-level diagnostics.
    pub(crate) fn call_span(&self) -> Option<SourceSpan> {
        match self {
            DeferredCall::Source(deferred) => deferred.call_span,
            DeferredCall::Receiver(deferred) => deferred.call_span,
            DeferredCall::External(deferred) => deferred.call_span,
            DeferredCall::Struct(deferred) => deferred.call_span,
            DeferredCall::Choice(deferred) => deferred.call_span,
            DeferredCall::Generic(deferred) => deferred.call_span,
            DeferredCall::GenericNominal(deferred) => deferred.call_span,
        }
    }
}

/// One expression retained because its nested statement bodies still hold numeric continuations.
///
/// WHAT: names the expression-level constructors that own statement bodies and therefore cannot
///       be built at their receiving site until the body root resumes those bodies.
/// WHY: a value-producing block and a catch handler are expressions over statement bodies. They
///      must rebuild through their owning module's existing constructor once the bodies resolve —
///      never as a call-shaped placeholder and never as an empty typed value. The constructor
///      inputs live with the owning module (their data), and this vocabulary only names them.
#[derive(Debug, Clone)]
pub(crate) enum DeferredExpression {
    /// A retained value-producing block (`if`/`match` at a closed receiving site).
    ValueBlock(
        Box<crate::compiler_frontend::ast::statements::value_production::DeferredValueBlock>,
    ),
    /// A retained handled fallible expression whose catch handler body holds continuations.
    CatchHandler(
        Box<crate::compiler_frontend::ast::statements::fallible_handling::DeferredCatchHandler>,
    ),
    /// A retained postfix operand continuation owned by the field-access module.
    OperandContinuation(
        Box<crate::compiler_frontend::ast::field_access::RetainedOperandContinuation>,
    ),
    /// A retained collection literal whose element values may still hold numeric fragments.
    Collection(Box<DeferredCollectionLiteral>),
    /// A retained map literal whose key and value values may still hold numeric fragments.
    Map(Box<DeferredMapLiteral>),
}

/// One element, key or value of a retained collection or map literal.
///
/// WHAT: a completed value keeps its type; a retained numeric fragment still needs its leaf type.
/// WHY: a numeric element cannot pick its leaf type until every constraint across the literal and
///      its peers is known, so it stays a fragment until body-root completion resolves it.
#[derive(Debug, Clone)]
// `Ready` is a completed literal element moved into its entry list once. Boxing it would add
// one heap allocation per completed element.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DeferredLiteralValue {
    /// A completed value whose type is already fixed.
    Ready(ExprId),
    /// A retained numeric fragment whose shape has not joined its side yet.
    Fragment(DeferredNumericExpression),
    /// A retained numeric fragment joined to its side; `shape` is the fragment's own entry shape.
    /// `forward` records whether that shape held a forward leaf before it joined, because joining
    /// ORs a forward peer's flag into the shared root and must not make a raw literal look forward.
    Joined {
        fragment: DeferredNumericExpression,
        shape: KnownShapeId,
        forward: bool,
    },
    /// A forward numeric fragment joined to its side; the forward-producer registry holds it.
    Forward(ForwardProducerId),
}

/// A retained collection literal in source order.
///
/// WHAT: owns its element shape, its element values and the explicit context when one was authored.
/// WHY: completion materialises the element type once every leaf constraint across the literal is known.
#[derive(Debug, Clone)]
pub(crate) struct DeferredCollectionLiteral {
    pub(crate) shape: KnownShapeId,
    pub(crate) element_shape: KnownShapeId,
    pub(crate) elements: Vec<DeferredLiteralValue>,
    pub(crate) collection_context: Option<ExpectedCollectionContext>,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) value_mode: ValueMode,
}

/// A retained map literal in source order.
#[derive(Debug, Clone)]
pub(crate) struct DeferredMapLiteral {
    pub(crate) shape: KnownShapeId,
    pub(crate) key_shape: KnownShapeId,
    pub(crate) value_shape: KnownShapeId,
    pub(crate) entries: Vec<DeferredMapEntry>,
    pub(crate) map_context: Option<ExpectedMapContext>,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) value_mode: ValueMode,
}

/// One retained map entry: a key and its value, either possibly a fragment.
#[derive(Debug, Clone)]
pub(crate) struct DeferredMapEntry {
    pub(crate) key: DeferredLiteralValue,
    pub(crate) value: DeferredLiteralValue,
}

/// Body-root completion services for one retained expression constructor.
///
/// WHAT: the exact services a retained value block or catch handler needs to resume inside the
///       body-root completion session: nested-body splicing, retained-operand completion and the
///       session's interning state.
/// WHY: rebuilding must reuse the owning module's constructor and the session's single
///      construction state. No second parser, no duplicated inference, and no borrowed session
///      copy exists; the retained data moves through these calls exactly once.
pub(crate) trait DeferredExpressionCompletion<'env> {
    /// The session's interning services as one disjoint borrow.
    ///
    /// Hold these while calling a constructor; they must be dropped before any `resolve_*` call.
    fn session_services(&mut self) -> DeferredExpressionServices<'_, 'env>;

    /// The exact receiving type that a numeric shape's class merges to, when one is selected.
    ///
    /// WHAT: reads the class's receiving witnesses through the same merge the body-root completion
    ///       applies to every class, so a protected fragment takes its receiver from its class.
    /// WHY: a catch handler's protected fragment must receive one width from every reachable
    ///      fallback and every later use before its literal recipe resolves. Only the completion
    ///      session holds the numeric state and the type interner together, so the class is read
    ///      there.
    fn shape_receiver(
        &mut self,
        shape: KnownShapeId,
    ) -> Result<Option<TypeId>, ExpressionParseError>;

    /// Splice one retained nested body in authored order, resuming any continuation on demand.
    fn resolve_deferred_body(
        &mut self,
        body: DeferredBody,
    ) -> Result<Vec<AstNode>, ExpressionParseError>;

    /// Splice one retained nested body under the given produced-value expectations.
    ///
    /// WHAT: runs `resolve_deferred_body` with `expectations` installed for the duration of the
    ///       call and the previous state restored afterwards.
    /// WHY: the enclosing catch handler derives the success slots at completion, so the retained
    ///      fallback values must see them before they materialise — never be coerced after. A
    ///      nested block whose own receiver differs installs its own contract: `Some(slots)` for a
    ///      known or mixed block, `None` for an inferred one, which must mask an enclosing
    ///      override instead of inheriting it.
    ///
    /// REQUIRED: every implementer must make the expectations visible to value-production
    /// completion, so a retained fallback materialises at the enclosing site's slot.
    fn resolve_deferred_body_with_expectations(
        &mut self,
        body: DeferredBody,
        expectations: Option<&[TypeId]>,
    ) -> Result<Vec<AstNode>, ExpressionParseError>;

    /// Resolve one retained continuation by ID under the enclosing site's expectations and take its
    /// completed statement nodes.
    ///
    /// WHAT: makes `expectations` visible to value-production completion, resolves the continuation,
    ///       takes the nodes it published, then restores the previous expectations.
    /// WHY: a retained handler body resumes at a receiving site that only became known at
    ///      completion, so its values must materialise at those slots — never at a parse-time
    ///      default.
    fn resolve_continuation_nodes(
        &mut self,
        id: NumericContinuationId,
        expectations: &[TypeId],
    ) -> Result<Vec<AstNode>, ExpressionParseError>;

    /// Complete one retained statement expression (condition, scrutinee, guard, produced value).
    fn resolve_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError>;

    /// Complete one retained statement expression that an enclosing catch protects.
    ///
    /// WHAT: completes the fragment's construction exactly like `resolve_statement_expression`,
    ///       but never closes its typed-error boundary validation.
    /// WHY: a catch-protected value's unresolved typed-error producer is recovered by the catch
    ///      the caller attaches afterwards; validating the protected fragment on its own would
    ///      report the authored catch as unhandled before it exists. The recovered expression is
    ///      validated at the enclosing position instead, with no global suppression.
    fn resolve_protected_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError>;

    /// Complete one retained ordered fragment against its authored expectation.
    fn resolve_fragment(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: &ValueMode,
    ) -> Result<OwnedExpr, ExpressionParseError>;

    /// Complete one retained call argument against its authored receiving shape.
    ///
    /// WHAT: a built argument passes through; a retained argument's fragment completes and is
    ///       re-wrapped with its recorded routing (parameter name, access mode, slot and spans).
    /// WHY: a call whose arguments hold numeric origins resumes its ordinary constructor at the
    ///      body root, so call-owning producers share this one argument-placement path instead of
    ///      rebuilding routing or re-defaulting an origin.
    fn complete_call_argument(
        &mut self,
        argument: DeferredCallArgument,
    ) -> Result<CallArgument<OwnedExpr>, ExpressionParseError>;

    /// Whether an enclosing catch protects the typed-error producers of the subtree being completed.
    ///
    /// WHAT: reports the session's protection state, which is set while a catch-protected statement
    ///       expression completes and restored afterwards.
    /// WHY: a closed typed-error boundary validates only when no enclosing catch recovers the
    ///      producer; consumers read this query instead of tracking the state themselves.
    ///
    /// REQUIRED: no default. Every implementer answers from its own protection state.
    fn typed_errors_are_protected(&self) -> bool;

    /// Complete every pending forward producer whose class is connected to `shape`.
    ///
    /// WHAT: pulls each connected producer's natural result exactly once, so its witness reaches
    ///       the shape before any class of `shape` materialises. Producers already resolving are skipped.
    /// WHY: a forward result is fixed by its operation; a default chosen first would retype it.
    fn settle_forward_producers(&mut self, shape: KnownShapeId)
    -> Result<(), ExpressionParseError>;

    /// Take one forward producer's natural result, completing it first while still pending.
    ///
    /// WHAT: returns the completed expression exactly once. A pending producer completes against
    ///       `expected` when a consumer supplies it, otherwise against its class's exact receiver.
    /// WHY: a consumer must receive the single evaluation a connected class may already have pulled.
    fn take_forward_producer(
        &mut self,
        producer: ForwardProducerId,
        expected: Option<ExpectedType>,
    ) -> Result<OwnedExpr, ExpressionParseError>;
}

/// One disjoint borrow of the completion session's interning services.
pub(crate) struct DeferredExpressionServices<'session, 'env> {
    pub(crate) context: &'session ScopeContext,
    pub(crate) type_interner: &'session mut AstTypeInterner<'env>,
    pub(crate) expression_builder: &'session mut AstExpressionBuilder,
    pub(crate) const_values: &'session mut ConstValueStore,
    pub(crate) string_table: &'session mut StringTable,
    pub(crate) path_fork: &'session mut PathInternerFork,
}

/// One nested statement body retained while its numeric continuations are unresolved.
///
/// WHY: nested blocks share the enclosing function's inference boundary, so their statements
///      must survive in authored order until the body root completes.
#[derive(Debug, Clone)]
pub(crate) struct DeferredBody {
    pub(crate) draft: BodyDraft,
    pub(crate) scope: PathId,
}

/// A nested construct retained until its retained bodies complete.
#[derive(Debug)]
pub(crate) enum DeferredConstruct {
    If(Box<DeferredIf>),
    Match(Box<DeferredMatch>),
    Loop(Box<DeferredLoop>),
}

/// One retained `if`/`else` statement.
#[derive(Debug)]
pub(crate) struct DeferredIf {
    pub(crate) condition: StatementExpression,
    pub(crate) then_body: DeferredBody,
    pub(crate) else_body: Option<DeferredBody>,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
    pub(crate) then_scope: PathId,
    pub(crate) else_scope: Option<PathId>,
    pub(crate) generic_request_start: usize,
    pub(crate) then_request_end: usize,
    pub(crate) else_request_end: usize,
}

/// One match pattern whose numeric value could not materialise before the subject was known.
///
/// WHAT: the authored relational operator (when the pattern is relational) plus the retained
///       value fragment.
/// WHY: pattern values are typed by the match subject; when the subject is a pending numeric
///      origin the pattern keeps its authored fragment here and body completion materialises it
///      at the selected subject type instead of fabricating a typed pattern early.
#[derive(Debug, Clone)]
pub(crate) struct RetainedPattern {
    pub(crate) relational: Option<RelationalPatternOp>,
    pub(crate) value: DeferredNumericExpression,
}

/// The authored pattern of one retained match arm.
///
/// WHAT: exactly one of a fully built pattern, a retained numeric pattern value, or a pending choice
///       pattern whose capture types resolve from the subject's concrete instance.
/// WHY: a numeric literal/relational pattern over a pending subject has no typed `Expression` to
///      build at parse time, and a typed sentinel is forbidden; making the forms exclusive
///      removes the unreachable combinations instead of encoding them as options.
#[derive(Debug, Clone)]
// `Ready` is a fully built arm pattern moved into its arm once. Boxing it would add one heap
// allocation per arm.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DeferredArmPattern {
    Ready(MatchPattern),
    Retained(RetainedPattern),
    PendingChoice(PendingChoicePattern),
    PendingOption(PendingOptionPattern),
}

/// A pending choice arm pattern: variant identity is known at parse time.
///
/// WHAT: names the declared choice variant by tag and records its captures; each capture's field
///       type is read from the subject's resolved owner when the arm is finished.
/// WHY: a generic subject's variants come from its declaration, but a payload field's type is only
///      meaningful through the instance's bindings, which are final only at body completion.
/// MUST NOT: carry a field type here; `finish_pending_choice_pattern` reads it from the owner.
#[derive(Debug, Clone)]
pub(crate) struct PendingChoicePattern {
    pub(crate) nominal_path: PathId,
    pub(crate) tag: usize,
    pub(crate) captures: Vec<PendingChoiceCapture>,
    pub(crate) span: Option<SourceSpan>,
}

/// One payload capture of a pending choice arm, before its field type is resolved.
///
/// WHAT: a record field index and the arm-local binding name that receives it.
#[derive(Debug, Clone)]
pub(crate) struct PendingChoiceCapture {
    pub(crate) field_index: usize,
    pub(crate) binding_path: PathId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) binding_span: Option<SourceSpan>,
}

/// A pending optional arm pattern: the payload shape is known at parse time, and the optional
/// subject's concrete inner type is read when the arm is finished.
///
/// WHAT: a present arm either captures the payload or compares it with a value; `none` needs no
///       payload and stays a materialised `DeferredArmPattern::Ready` pattern.
/// WHY: a retained optional's inner shape is fixed by its declaration, but the arm's concrete
///      type waits for the body root, as a retained choice's payload fields do.
#[derive(Debug, Clone)]
// `PresentValue` and `RetainedValue` own an arm value moved into its arm once. Boxing either
// would add one heap allocation per arm.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PendingOptionPattern {
    PresentCapture(PendingOptionCapture),
    /// A built value pattern whose literal needs only the inner type to check equality.
    PresentValue(MatchPattern),
    /// A retained numeric value or relational pattern resolved at the body root.
    RetainedValue(RetainedPattern),
}

/// One present capture of a pending optional arm, before its payload type is resolved.
///
/// WHAT: the arm-local binding name and its path; the binding's type is the optional's inner
///       shape, materialised at body completion.
#[derive(Debug, Clone)]
pub(crate) struct PendingOptionCapture {
    pub(crate) name: StringId,
    pub(crate) binding_path: PathId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) binding_span: Option<SourceSpan>,
}

impl PendingOptionPattern {
    /// The source span of the pending arm pattern, for diagnostics about unreachable arms.
    pub(crate) fn span(&self) -> Option<SourceSpan> {
        match self {
            Self::PresentCapture(capture) => capture.span,
            Self::PresentValue(pattern) => pattern.span(),
            Self::RetainedValue(retained) => retained.value.span,
        }
    }
}

/// One retained match arm.
#[derive(Debug, Clone)]
pub(crate) struct DeferredMatchArm {
    pub(crate) pattern: DeferredArmPattern,
    pub(crate) guard: Option<StatementExpression>,
    pub(crate) body: DeferredBody,
}

/// One retained `if <subject> is:` match statement.
#[derive(Debug, Clone)]
pub(crate) struct DeferredMatch {
    pub(crate) scrutinee: StatementExpression,
    pub(crate) arms: Vec<DeferredMatchArm>,
    pub(crate) default: Option<DeferredBody>,
    /// The authored arm index where the `else` arm appeared, retained so completion can reproduce
    /// the unreachable-arm policy exactly once. Invariant: `is_some() == default.is_some()`.
    pub(crate) else_arm_index: Option<usize>,
    pub(crate) exhaustiveness: MatchExhaustiveness,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// One loop header's range operands before their numeric origins resolve.
///
/// WHAT: the authored start/end/step positions, each already completed or retained, plus the
///       authored end kind and header span.
/// WHY: a range operand may reference a pending numeric origin whose single static type is only
///      selected at the body root; the header must keep the authored positions (including an
///      omitted start and an omitted step) so completion materialises them once at the selected
///      domain and the shared range policy still selects and checks that exact domain.
#[derive(Debug, Clone)]
pub(crate) struct DeferredRangeSpec {
    pub(crate) start: Option<StatementExpression>,
    pub(crate) end: StatementExpression,
    pub(crate) end_kind: crate::compiler_frontend::ast::ast_nodes::RangeEndKind,
    pub(crate) step: Option<StatementExpression>,
    pub(crate) span: Option<SourceSpan>,
}

/// One deferred range loop header's declared bindings.
///
/// WHAT: the retained counter plus the positional index binding.
/// WHY: a deferred range header's counter can never be `Ready` — the header defers exactly
///      because an operand references a pending origin, and the counter's type is a forward
///      result of the operands' selected domains. The row keeps the authored name/path/span/mode
///      in one place, so the body sees the counter without a fabricated `TypeId`.
#[derive(Debug, Clone)]
pub(crate) struct RetainedRangeCounter {
    /// Construction row that owns the authored counter name/path/span/mode; its `RangeCounter`
    /// recipe owns the header's operand slots.
    pub(crate) counter: NumericBindingId,
    /// The positional index binding: always the concrete `I32` declaration.
    pub(crate) index: Option<Declaration>,
}

/// One deferred range loop header.
///
/// WHAT: the operand owner plus (when the header authored bindings) the retained counter.
/// WHY: exclusive metadata, mirroring `DeferredArmPattern`: a bound header's operands belong to
///      its counter row, which resolves before any binding that reads the counter; an unbound
///      header has no counter and completes its own operands when the loop finishes.
#[derive(Debug, Clone)]
pub(crate) enum DeferredRangeHeader {
    /// Ready = the header is final; only the loop body forced the loop to stay retained.
    ///
    /// WHAT: the concrete parse-time bindings and range spec exactly as the ordinary header
    ///       constructor built and checked them.
    /// WHY: a concrete range header (`loop 0..10 |i|:` over a body that holds numeric
    ///      continuations) must not be re-completed at the body root: the shared range policy
    ///      already ran once at parse time, and the counter/index declarations are real
    ///      authored AST values, never placeholders.
    Ready {
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
    },
    /// The loop binds a counter; the counter row owns the operands and computes the domain.
    Bound(Box<RetainedRangeCounter>),
    /// No bindings authored; the loop construct completes its own operand slots.
    Unbound(Box<DeferredRangeSpec>),
}

/// Retained loop header of one loop statement.
#[derive(Debug)]
pub(crate) enum DeferredLoopHeader {
    Conditional {
        condition: StatementExpression,
    },
    Range(Box<DeferredRangeHeader>),
    Collection {
        bindings: Box<crate::compiler_frontend::ast::ast_nodes::LoopBindings>,
        iterable: StatementExpression,
        /// The item binding declared before the body, when its element type is still pending.
        ///
        /// WHY: a pending item completes with the header, so the body sees one resolved declaration.
        pending_item: Option<NumericBindingId>,
    },
}

/// One retained loop statement.
#[derive(Debug)]
pub(crate) struct DeferredLoop {
    pub(crate) header: DeferredLoopHeader,
    pub(crate) body: DeferredBody,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// A statement target that is a pending numeric binding.
#[derive(Debug, Clone)]
pub(crate) struct PendingPlaceTarget {
    pub(crate) binding: NumericBindingId,
    pub(crate) span: Option<SourceSpan>,
}

/// Continuation for `name = <pending>` / `name ~= <pending>` declarations.
///
/// Covers both an unannotated binding whose single static type is still being collected and an
/// annotated declaration whose initializer references pending origins: the annotation registers
/// as an exact receiver, so one completion path materialises either shape at its selected type.
#[derive(Debug)]
pub(crate) struct BindingContinuation {
    pub(crate) binding: NumericBindingId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// One retained assignment target: a pending binding, projected place or already-resolved place.
///
/// WHAT: each target carries only the facts its resumption needs.
/// WHY: a ready target exists only because its right-hand side deferred, so the mutation must
///      still resume with the authored place, declaration value mode and declaration span —
///      never a fabricated declaration, and never a second lookup rule beside the pending arm.
#[derive(Debug, Clone)]
pub(crate) enum DeferredAssignmentTarget {
    /// The target is a pending numeric binding whose place is only known at completion.
    Pending(PendingPlaceTarget),
    /// The target is an already-resolved place; only the statement's value deferred.
    Ready {
        target: crate::compiler_frontend::ast::expressions::expression_store::PlaceId,
        /// The authored declaration's value mode, used by the promoted compound arithmetic.
        declaration_value_mode: ValueMode,
        /// The authored declaration's binding span, for the immutable-target diagnostic.
        declaration_span: Option<SourceSpan>,
    },
    /// A parsed field/index projection whose root's numeric leaves are still pending.
    ///
    /// WHY: a projection preserves its authored operand and resolves to the ordinary place once
    /// the root's one static type is complete; no source is parsed again.
    Retained {
        target: DeferredNumericExpression,
        declaration_value_mode: ValueMode,
        declaration_span: Option<SourceSpan>,
    },
}

/// Continuation for a simple assignment whose target or value needs resolution.
#[derive(Debug, Clone)]
pub(crate) struct AssignmentContinuation {
    pub(crate) target: DeferredAssignmentTarget,
    /// The authored right-hand side: already built, or retained until the target's type is known.
    ///
    /// WHY: a raw-literal right-hand side must materialise at the target's selected type, so a
    ///      retained value records its authored fragment rather than an early default.
    pub(crate) value: StatementExpression,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// Continuation for a compound update whose target or value needs resolution.
#[derive(Debug, Clone)]
pub(crate) struct CompoundAssignmentContinuation {
    pub(crate) target: DeferredAssignmentTarget,
    pub(crate) operator: Operator,
    /// The authored right-hand side: already built, or retained until the target's type is known.
    pub(crate) value: StatementExpression,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// Continuation for a statement-position expression.
#[derive(Debug, Clone)]
// Retained fragments stay inline; completed positions carry only their committed identity.
#[allow(clippy::large_enum_variant)]
pub(crate) enum StatementExpression {
    /// The expression is already completed in the phase owner.
    Built(ExprId),
    /// The expression could not be built; its ordered fragment is retained.
    Retained(DeferredNumericExpression),
}

/// Continuation for an expression statement.
#[derive(Debug, Clone)]
pub(crate) struct ExpressionStatementContinuation {
    pub(crate) expression: StatementExpression,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// Continuation for `return` / `return!` with pending slots.
#[derive(Debug, Clone)]
pub(crate) struct ReturnContinuation {
    pub(crate) slots: Vec<StatementExpression>,
    pub(crate) is_error_return: bool,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// The value of one retained multi-bind statement.
///
/// WHAT: exactly one of the two source-call shapes the multi-bind gate accepts, never both.
/// WHY: a catch-protected multi-slot call emits a single `CatchHandler` item instead of the bare
///      call item, so the continuation must distinguish the shapes without inferring anything
///      from a general expression; the ordered target slots always come from the already parsed
///      source signature captured at retention.
#[derive(Debug)]
pub(crate) enum DeferredMultiBindValue {
    /// A retained source call; its signature supplies the ordered result slots.
    Call(DeferredCall),
    /// A retained catch-protected source call, rebuilt by the ordinary handler completion.
    ///
    /// `success_slot_count` is the authored callee's success-slot count captured from the parsed
    /// signature at retention, so completion checks the target arity without inspecting the
    /// rebuilt expression tree.
    Catch {
        handler:
            Box<crate::compiler_frontend::ast::statements::fallible_handling::DeferredCatchHandler>,
        success_slot_count: usize,
    },
    /// A retained value block (or other retained value-producing expression).
    ///
    /// WHAT: the whole block's authored receiving contract plus the deferred record it completes
    ///       with, completed exactly once at the body root.
    /// WHY: an ordinary concrete value block whose produced values still hold numeric origins must
    ///      be owned by this statement rather than escaping as a hard parse error, and its targets
    ///      resolve from the block's own authored slots, never from a fabricated type.
    RetainedExpression {
        /// One entry per produced value: the authored target slots the block was parsed with.
        slot_expected_types: Vec<Option<TypeId>>,
        deferred: Box<DeferredNumericExpression>,
    },
}

/// One multi-bind target whose concrete type may resolve at body-root completion.
#[derive(Debug)]
pub(crate) enum DeferredMultiBindTarget {
    Ready(MultiBindTarget),
    Pending {
        binding: NumericBindingId,
        kind: MultiBindTargetKind,
        span: Option<SourceSpan>,
        /// Whether this new target's own recipe resolves from retained producer routes.
        producer_resolves_type: bool,
    },
}

/// Continuation for a multi-bind statement whose value still needs resolution.
///
/// WHAT: ordered targets, per-slot raw/forward producer routes and the retained value whose
///       producer expressions resolve at the body root.
/// WHY: inferred numeric targets are visible through the ordinary pending frame owner at parse
///      time; completion routes only eligible producers, preserves the block's inference contract,
///      and emits the `MultiBind` exactly once.
#[derive(Debug)]
pub(crate) struct MultiBindContinuation {
    pub(crate) targets: Vec<DeferredMultiBindTarget>,
    /// Raw/forward producers that may receive a numeric hint after their target resolves.
    pub(crate) producer_routes: Vec<(usize, PendingMultiBindProducer)>,
    /// The retained value (bare call, catch-protected call or value block).
    pub(crate) value: DeferredMultiBindValue,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) scope: PathId,
}

/// One retained numeric-dependent statement.
#[derive(Debug)]
pub(crate) enum NumericContinuation {
    Binding(BindingContinuation),
    Assignment(AssignmentContinuation),
    CompoundAssignment(CompoundAssignmentContinuation),
    ExpressionStatement(ExpressionStatementContinuation),
    Return(ReturnContinuation),
    /// A `then` statement whose fallback values still hold numeric origins.
    ///
    /// WHAT: the authored values in slot order with the authored receiver kind, span and scope.
    /// WHY: the receiving site may not be resolved when the fallback is authored, so each value
    ///      resumes where the resolved slot decides its type instead of materialising at the
    ///      language default first; the payload and its finisher are owned by value-production.
    ThenValue(crate::compiler_frontend::ast::statements::value_production::ThenValueContinuation),
    /// A multi-bind statement whose retained call arguments still hold numeric origins.
    MultiBind(MultiBindContinuation),
    /// An `assert` statement whose condition/message arguments still hold numeric origins.
    ///
    /// WHY: the payload belongs to the assertion statement contract (its expectations and
    ///      finisher live in `statements::asserts`), so the shared core carries the record
    ///      without owning its shape. It is boxed so the inline record does not size every
    ///      numeric continuation row.
    Assertion(Box<crate::compiler_frontend::ast::statements::asserts::AssertionContinuation>),
    /// A nested construct whose retained bodies still hold numeric continuations.
    Construct(Box<DeferredConstruct>),
}

/// One body position: a completed node or a retained numeric continuation.
#[derive(Debug, Clone)]
// `Node` is one completed statement pushed into the body's ordered list once. Boxing it would
// add one heap allocation per body statement.
#[allow(clippy::large_enum_variant)]
pub(crate) enum BodyEntry {
    Node(AstNode),
    Pending(NumericContinuationId),
}

/// Ordered statement list of one body scope before numeric completion.
///
/// WHAT: keeps the authored order and lexical position of every statement while some
///       statements are still numeric-dependent continuations.
/// WHY: completing at the owning function root must insert each resumed statement at its
///      original position, preserving evaluation order and single evaluation.
#[derive(Debug, Clone, Default)]
pub(crate) struct BodyDraft {
    pub(crate) entries: Vec<BodyEntry>,
}

impl BodyDraft {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn push_node(&mut self, node: AstNode) {
        self.entries.push(BodyEntry::Node(node));
    }

    pub(crate) fn extend_nodes(&mut self, nodes: Vec<AstNode>) {
        self.entries.extend(nodes.into_iter().map(BodyEntry::Node));
    }

    pub(crate) fn push_pending(&mut self, id: NumericContinuationId) {
        self.entries.push(BodyEntry::Pending(id));
    }

    /// True when at least one entry still needs numeric completion.
    pub(crate) fn has_pending(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, BodyEntry::Pending(_)))
    }

    /// Convert one nested body that retained no numeric-dependent position.
    ///
    /// WHAT: returns the authored nodes when every entry completed where it was parsed, and a
    ///       precise infrastructure failure naming the host position when any entry is still a
    ///       pending continuation.
    /// WHY: a host without a deferred-construct lane (an expression-level value block or handler
    ///      body) must never silently drop a retained statement, fabricate a node, or default an
    ///      origin early; the explicit failure names the unsupported position instead.
    pub(crate) fn into_nodes_for_host(
        self,
        position: &str,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        if self.has_pending() {
            return Err(CompilerError::compiler_error(format!(
                "a numeric-dependent statement was retained inside {position}, which has no \
                 body-root numeric resumption lane"
            ))
            .into());
        }
        Ok(self
            .entries
            .into_iter()
            .map(|entry| match entry {
                BodyEntry::Node(node) => node,
                BodyEntry::Pending(_) => {
                    unreachable!("a body draft proven free of pending continuations still held one")
                }
            })
            .collect())
    }
}

/// A read view of the retained continuation table, detached while one read-only walk runs.
///
/// WHAT: the table's dense entries, reachable only through
///       `NumericConstructionState::with_detached_continuations`, which keeps the state's own table
///       empty for the span of the walk.
/// WHY: the walk reads retained `then` bodies while it merges receiver classes in the same state.
///      Continuations are read by reference, so no body is copied or collected.
pub(crate) struct DetachedContinuations {
    entries: Vec<Option<NumericContinuation>>,
}

impl DetachedContinuations {
    /// Read one detached continuation without taking ownership of its authored payload.
    pub(crate) fn get(&self, id: NumericContinuationId) -> Option<&NumericContinuation> {
        self.entries.get(id.as_usize()).and_then(Option::as_ref)
    }
}

/// One independent body-owned numeric construction domain.
#[derive(Debug)]
pub(crate) struct NumericConstructionState {
    /// Process-unique key of the body root that owns these tables.
    domain: NumericConstructionDomainId,
    classes: Vec<ClassRow>,
    /// Body-owned known-shape nodes, addressed by dense `KnownShapeId`.
    shapes: Vec<KnownShape>,
    /// Reused traversal-local marks prevent declaration pins expanding shared shape edges.
    shape_pin_marks: Vec<u32>,
    shape_pin_epoch: u32,
    /// Authored provenance reused when concrete peers project into other shapes.
    shape_authored_order: Vec<u64>,
    bindings: Vec<PendingBindingRow>,
    /// WHAT: parse-time receivers and binding declarations share one authored-order counter.
    /// WHY: completion reuses a declaration's saved key, so late storage seeds and union-root
    ///      choices cannot move an earlier authored witness behind a later receiver.
    next_authored_order: u64,
    binding_authored_order: Vec<u64>,
    /// Retained continuations, taken by dense ID exactly once at body-root completion.
    continuations: Vec<Option<NumericContinuation>>,
    /// Forward results held until their first consumer or a connected class completes them.
    forward_producers: Vec<ForwardProducer>,
    /// Edges of declared-substitution nodes, addressed by the ranges those nodes hold.
    declared_bindings: Vec<KnownShapeBinding>,
    /// True once the owning body has completed and released its construction tables.
    released: bool,
}

impl Default for NumericConstructionState {
    fn default() -> Self {
        Self::new()
    }
}

impl NumericConstructionState {
    pub(crate) fn new() -> Self {
        Self {
            domain: NumericConstructionDomainId::fresh(),
            classes: Vec::new(),
            shapes: Vec::new(),
            shape_pin_marks: Vec::new(),
            shape_pin_epoch: 0,
            shape_authored_order: Vec::new(),
            bindings: Vec::new(),
            next_authored_order: 0,
            binding_authored_order: Vec::new(),
            continuations: Vec::new(),
            forward_producers: Vec::new(),
            declared_bindings: Vec::new(),
            released: false,
        }
    }

    /// Intern one known-shape node and return its dense ID.
    ///
    /// WHAT: appends to the body-owned shape table and fails rather than wrap when the table
    ///       exhausts the 32-bit ID space.
    /// WHY: IDs are the only shared handle to a shape, so the table alone owns its structure and no
    ///      descriptor graph is cloned per binding or use.
    pub(crate) fn intern_shape(&mut self, shape: KnownShape) -> ConstructionResult<KnownShapeId> {
        let authored_order = self.take_authored_order();
        self.intern_ordered_shape(shape, authored_order)
    }

    pub(crate) fn intern_ordered_shape(
        &mut self,
        shape: KnownShape,
        authored_order: u64,
    ) -> ConstructionResult<KnownShapeId> {
        self.ensure_live("a known shape")?;
        let id = KnownShapeId::from_dense_index(self.shapes.len())?;
        self.shapes.push(shape);
        self.shape_authored_order.push(authored_order);
        Ok(id)
    }

    pub(crate) fn shape_authored_order(&self, shape: KnownShapeId) -> u64 {
        self.shape_authored_order[shape.as_usize()]
    }

    /// Intern one declared-substitution node and return its dense ID.
    ///
    /// WHAT: appends `edges` in canonical parameter-identity order to the body-owned binding
    ///       table, then records their range. Every ID is checked before either table grows.
    /// WHY: a node holds only a `Copy` range, so a declared envelope never owns a graph of its
    ///      edges and reads stay allocation-free.
    pub(crate) fn intern_declared_substitution(
        &mut self,
        declared_type_id: TypeId,
        edges: &[KnownShapeBinding],
    ) -> ConstructionResult<KnownShapeId> {
        self.ensure_live("a declared substitution")?;
        let id = KnownShapeId::from_dense_index(self.shapes.len())?;
        let start = checked_dense_id(self.declared_bindings.len(), "declared binding")?;
        let end_len = self
            .declared_bindings
            .len()
            .checked_add(edges.len())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "numeric construction declared binding table length overflowed",
                )
            })?;
        let end = checked_dense_id(end_len, "declared binding")?;
        self.declared_bindings.extend_from_slice(edges);
        // Producers supply one edge per parameter. Canonical identity order lets readers
        // binary-search individual edges and merge envelopes without allocating a lookup map.
        self.declared_bindings[start as usize..end as usize]
            .sort_unstable_by_key(|edge| edge.parameter.0);
        self.shapes.push(KnownShape::DeclaredSubstitution {
            declared_type_id,
            bindings: KnownShapeBindingRange::new(start, end),
        });
        let authored_order = edges
            .iter()
            .map(|edge| self.shape_authored_order(edge.shape))
            .min()
            .unwrap_or(0);
        self.shape_authored_order.push(authored_order);
        Ok(id)
    }

    /// The declared-substitution edge at `index` in the body-owned binding table.
    ///
    /// WHY: a range is only built by `intern_declared_substitution`, so an out-of-range index is
    ///      an internal invariant break, not a user error.
    pub(crate) fn declared_binding(&self, index: usize) -> KnownShapeBinding {
        self.declared_bindings[index]
    }

    /// The shape bound to `parameter` inside a declared-substitution range, if the range names it.
    ///
    /// WHAT: binary-searches the canonical range by parameter identity, never declaration position.
    /// WHY: signature projection reads individual parameters without rescanning the envelope.
    pub(crate) fn declared_binding_shape(
        &self,
        range: KnownShapeBindingRange,
        parameter: GenericParameterId,
    ) -> Option<KnownShapeId> {
        let edges = &self.declared_bindings[range.indices()];
        let index = edges
            .binary_search_by(|edge| {
                increment_ast_counter(AstCounter::NumericDeclaredBindingComparisons);
                edge.parameter.0.cmp(&parameter.0)
            })
            .ok()?;
        Some(edges[index].shape)
    }

    /// The shape node for an ID, copied out of the table.
    ///
    /// WHY: nodes are `Copy`, so a caller reads one without holding a borrow of the table.
    pub(crate) fn shape(&self, shape: KnownShapeId) -> KnownShape {
        self.shapes[shape.as_usize()]
    }

    /// Start a pin walk without allocating a visited set for each declaration.
    ///
    /// WHY: shape identities stay dense for the body's lifetime. Epochs invalidate earlier walks
    ///      while retaining their scratch capacity, so later stronger pins still reach every leaf.
    pub(crate) fn begin_shape_pin_walk(&mut self) -> u32 {
        self.shape_pin_marks.resize(self.shapes.len(), 0);
        if self.shape_pin_epoch == u32::MAX {
            self.shape_pin_marks.fill(0);
            self.shape_pin_epoch = 0;
        }
        self.shape_pin_epoch += 1;
        self.shape_pin_epoch
    }

    pub(crate) fn visit_shape_pin_node(&mut self, shape: KnownShapeId, epoch: u32) -> bool {
        let mark = &mut self.shape_pin_marks[shape.as_usize()];
        if *mark == epoch {
            return false;
        }
        *mark = epoch;
        true
    }

    /// The numeric class a collection element's loop item shares with its source literal.
    ///
    /// WHAT: a numeric element returns its own class, so the item and the literal share one leaf.
    ///       Any other element gets a fresh class that the item's recipe, not its class default,
    ///       resolves.
    /// WHY: a numeric loop item must constrain the source literal's leaves directly, not a copy.
    pub(crate) fn collection_element_class(
        &mut self,
        element: KnownShapeId,
    ) -> ConstructionResult<NumericClassId> {
        match self.shape(element) {
            KnownShape::Numeric(class) => Ok(class),
            KnownShape::Concrete { .. }
            | KnownShape::Collection { .. }
            | KnownShape::Map { .. }
            | KnownShape::DeclaredSubstitution { .. }
            | KnownShape::Optional { .. } => Ok(self.create_class()?),
        }
    }

    /// The element node of a collection shape a loop item draws its descriptor from.
    ///
    /// WHAT: reads the `element` of a `Collection` node. Every other shape answers a compiler error.
    /// WHY: a pending loop source names its collection node, while the item's class and recipe are
    ///      defined over the element node.
    pub(crate) fn collection_element_shape(
        &self,
        collection: KnownShapeId,
    ) -> ConstructionResult<KnownShapeId> {
        match self.shape(collection) {
            KnownShape::Collection { element, .. } => Ok(element),
            KnownShape::Concrete { .. }
            | KnownShape::Numeric(_)
            | KnownShape::Map { .. }
            | KnownShape::DeclaredSubstitution { .. }
            | KnownShape::Optional { .. } => Err(CompilerError::compiler_error(
                "a collection loop item was declared over a known shape that is not a collection",
            )
            .into()),
        }
    }

    /// The identity of this construction domain.
    ///
    /// WHAT: stable for the whole lifetime of the body root that owns these tables.
    /// WHY: module-wide pending records must select their owning body's work without guessing a
    ///      frame or reusing a dense ID from another domain.
    pub(crate) fn domain_id(&self) -> NumericConstructionDomainId {
        self.domain
    }

    /// Reject new construction work after the owning body released its domain.
    ///
    /// WHAT: the single lifetime guard every registering entry point consults.
    /// WHY: a body root must complete and release exactly once; a registration arriving after
    ///      release belongs to no completion pass and would otherwise never resolve.
    fn ensure_live(&self, registration: &str) -> Result<(), CompilerError> {
        if self.released {
            return Err(CompilerError::compiler_error(format!(
                "{registration} was registered after its body root released the numeric \
                 construction domain"
            )));
        }
        Ok(())
    }

    /// The authored value mode of a live pending binding row.
    ///
    /// WHAT: bounds-checked row lookup; a foreign or released binding ID returns `None`.
    /// WHY: template heads must read the declared mode without defaulting the binding, and an
    ///      unresolvable row must surface as a broken compiler invariant, never as a value.
    pub(crate) fn binding_value_mode(&self, id: NumericBindingId) -> Option<ValueMode> {
        self.bindings.get(id.as_usize()).map(|row| row.value_mode)
    }

    // --------------------------
    //  Origins and equalities
    // --------------------------

    pub(crate) fn create_class(&mut self) -> Result<NumericClassId, CompilerError> {
        self.ensure_live("a numeric origin")?;
        let id = NumericClassId(checked_dense_id(self.classes.len(), "origin")?);
        self.classes.push(ClassRow {
            parent: id.0,
            rank: 0,
            first_receiver: None,
            differing_receiver: None,
            family: None,
            pin: NumericPin::Unpinned,
            loop_ineligible_receiver: None,
            forward: false,
            forward_producers: BinaryHeap::new(),
        });
        Ok(id)
    }

    pub(crate) fn classes_share_root(
        &mut self,
        left: NumericClassId,
        right: NumericClassId,
    ) -> bool {
        self.find_root(left) == self.find_root(right)
    }

    fn find_root(&mut self, class: NumericClassId) -> NumericClassId {
        let mut current = class.0 as usize;
        loop {
            let parent = self.classes[current].parent as usize;
            if parent == current {
                return NumericClassId(current as u32);
            }
            let grandparent = self.classes[parent].parent as usize;
            self.classes[current].parent = grandparent as u32;
            current = grandparent;
        }
    }

    /// Merge two classes into one equality class, keeping their receiving witnesses.
    pub(crate) fn union_classes(
        &mut self,
        left: NumericClassId,
        right: NumericClassId,
    ) -> NumericClassId {
        let left_root = self.find_root(left);
        let right_root = self.find_root(right);
        if left_root == right_root {
            return left_root;
        }
        let (winner, loser) = if self.classes[left_root.as_usize()].rank
            >= self.classes[right_root.as_usize()].rank
        {
            (left_root, right_root)
        } else {
            (right_root, left_root)
        };
        let first = self.classes[loser.as_usize()].first_receiver.take();
        let differing = self.classes[loser.as_usize()].differing_receiver.take();
        // These two witnesses preserve every merged conflict, even if the merged first changes.
        for witness in [first, differing].into_iter().flatten() {
            self.classes[winner.as_usize()].insert_receiver(witness);
        }
        let loser_family = self.classes[loser.as_usize()].family;
        let winner_family = self.classes[winner.as_usize()].family;
        self.classes[winner.as_usize()].family = match (winner_family, loser_family) {
            (Some(winner_family), Some(loser_family)) => Some(winner_family.merge(loser_family)),
            (family, None) | (None, family) => family,
        };
        let loser_pin = self.classes[loser.as_usize()].pin;
        self.classes[winner.as_usize()].pin = self.classes[winner.as_usize()].pin.max(loser_pin);
        if let Some(witness) = self.classes[loser.as_usize()]
            .loop_ineligible_receiver
            .take()
        {
            retain_earliest_receiver(
                &mut self.classes[winner.as_usize()].loop_ineligible_receiver,
                witness,
            );
        }
        let loser_forward = self.classes[loser.as_usize()].forward;
        self.classes[winner.as_usize()].forward |= loser_forward;
        let mut loser_producers =
            std::mem::take(&mut self.classes[loser.as_usize()].forward_producers);
        self.classes[winner.as_usize()]
            .forward_producers
            .append(&mut loser_producers);
        self.classes[loser.as_usize()].parent = winner.0;
        if self.classes[winner.as_usize()].rank == self.classes[loser.as_usize()].rank {
            self.classes[winner.as_usize()].rank += 1;
        }
        winner
    }

    /// Register one exact receiving witness on a class.
    pub(crate) fn add_receiver_witness(
        &mut self,
        class: NumericClassId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        environment: &TypeEnvironment,
    ) {
        let authored_order = self.take_authored_order();
        self.add_ordered_receiver_witness(class, type_id, span, authored_order, environment);
    }

    pub(crate) fn take_authored_order(&mut self) -> u64 {
        let order = self.next_authored_order;
        self.next_authored_order += 1;
        order
    }

    pub(crate) fn binding_authored_order(&self, binding: NumericBindingId) -> u64 {
        self.binding_authored_order[binding.as_usize()]
    }

    pub(crate) fn add_ordered_receiver_witness(
        &mut self,
        class: NumericClassId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        authored_order: u64,
        environment: &TypeEnvironment,
    ) {
        let root = self.find_root(class);
        let witness = ReceiverWitness {
            type_id,
            span,
            authored_order,
        };
        let row = &mut self.classes[root.as_usize()];
        row.insert_receiver(witness);
        // Record the shared policy before a pin exists, so a later loop pin needs no replay.
        if !family_receives_recipe(Some(NumericFamily::Whole), type_id, environment) {
            retain_earliest_receiver(&mut row.loop_ineligible_receiver, witness);
        }
    }

    /// The earliest authored exact receiving witness, read through a shared borrow.
    ///
    /// WHAT: follows parent links to the class root without path compression and returns the
    ///       earliest authored witness type registered there.
    /// WHY: handler discovery reads numeric state while it is borrowed immutably, whereas
    ///      `first_receiver_witness` compresses paths through `&mut self`.
    pub(crate) fn first_receiver_type(&self, class: NumericClassId) -> Option<TypeId> {
        let mut root = class.as_usize();
        while self.classes[root].parent as usize != root {
            root = self.classes[root].parent as usize;
        }
        self.classes[root].first_receiver.map(|witness| {
            increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
            witness.type_id
        })
    }

    /// The earliest authored exact receiver, including its span and parse-time provenance.
    ///
    /// WHY: projected concrete leaves and completion errors share the retained authored witness.
    pub(crate) fn first_receiver_witness(
        &mut self,
        class: NumericClassId,
    ) -> Option<ReceiverWitness> {
        let root = self.find_root(class);
        self.classes[root.as_usize()].first_receiver.inspect(|_| {
            increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        })
    }

    /// Record the authored family of one literal leaf on its origin class.
    pub(crate) fn note_class_family(&mut self, class: NumericClassId, family: NumericFamily) {
        let root = self.find_root(class);
        let row = &mut self.classes[root.as_usize()];
        row.family = Some(match row.family {
            Some(existing) => existing.merge(family),
            None => family,
        });
    }

    /// The merged authored family of one origin class (`Whole` when no leaf noted one).
    pub(crate) fn class_family(&mut self, class: NumericClassId) -> NumericFamily {
        let root = self.find_root(class);
        self.classes[root.as_usize()]
            .family
            .unwrap_or(NumericFamily::Whole)
    }

    /// Pin a numeric origin so peers cannot reclassify its family.
    ///
    /// WHAT: a bound origin keeps its noted whole or binary-float family. A whole loop item takes
    ///       the stronger integer-only pin; a binary-float loop item keeps the ordinary bound pin.
    ///       A stronger pin replaces a weaker one and is never weakened by a later bound.
    /// WHY: a loop introduces a named item without changing its source family. Both families
    ///      remain fixed, while only whole items need the integer-only receiver replay.
    pub(crate) fn pin_numeric_origin(&mut self, class: NumericClassId, pin: NumericPin) {
        let root = self.find_root(class);
        let row = &mut self.classes[root.as_usize()];
        let pin = match (pin, row.family) {
            (NumericPin::Bound, Some(_)) | (NumericPin::LoopItem, Some(NumericFamily::Decimal)) => {
                NumericPin::Bound
            }
            (NumericPin::LoopItem, Some(NumericFamily::Whole)) => NumericPin::LoopItem,
            _ => return,
        };
        row.pin = row.pin.max(pin);
    }

    /// The family a pinned origin admits from its receivers, or `None` when the origin is unpinned.
    ///
    /// WHAT: a loop item admits only integer receivers and reports `Whole`. A bound origin reports
    ///       its own merged family.
    /// WHY: every pin reads the one receiver policy in `family_receives_recipe`, so no second table
    ///      decides which receivers a pin admits.
    pub(crate) fn class_pinned_family(&mut self, class: NumericClassId) -> Option<NumericFamily> {
        let root = self.find_root(class);
        let row = &self.classes[root.as_usize()];
        match row.pin {
            NumericPin::Unpinned => None,
            NumericPin::Bound => row.family,
            NumericPin::LoopItem => Some(NumericFamily::Whole),
        }
    }

    /// Record an exact peer that a pin refused at registration, without typing the class.
    ///
    /// WHY: a bound pin ignores refused incompatible peers, but a later loop item pin must
    /// still reject them, whichever traversal order registered the peer.
    pub(crate) fn refuse_ordered_pinned_peer(
        &mut self,
        class: NumericClassId,
        type_id: TypeId,
        span: Option<SourceSpan>,
        authored_order: u64,
    ) {
        let root = self.find_root(class);
        // Every refused peer remains a loop offender, regardless of its scalar family.
        retain_earliest_receiver(
            &mut self.classes[root.as_usize()].loop_ineligible_receiver,
            ReceiverWitness {
                type_id,
                span,
                authored_order,
            },
        );
    }

    /// Whether the merged authored family of a class is decimal.
    pub(crate) fn class_family_is_decimal(&mut self, class: NumericClassId) -> bool {
        let root = self.find_root(class);
        self.classes[root.as_usize()].family == Some(NumericFamily::Decimal)
    }

    /// Mark the class created for one named forward fragment.
    pub(crate) fn mark_forward(&mut self, class: NumericClassId) {
        self.classes[class.as_usize()].forward = true;
    }

    /// Whether a merged class carries a named forward fragment's natural result. Walks to the root
    /// without path compression, so every member of a union reports the same fact.
    pub(crate) fn class_is_forward(&self, class: NumericClassId) -> bool {
        let mut root = class.as_usize();
        while self.classes[root].parent as usize != root {
            root = self.classes[root].parent as usize;
        }
        self.classes[root].forward
    }

    // --------------------------
    //  Pending bindings
    // --------------------------

    pub(crate) fn register_binding(
        &mut self,
        mut row: PendingBindingRow,
    ) -> Result<NumericBindingId, CompilerError> {
        self.ensure_live("a pending numeric binding")?;
        let id = NumericBindingId(checked_dense_id(self.bindings.len(), "pending binding")?);
        row.origin = self.recipe_origin_queries(row.recipe.as_ref());
        let authored_order = self.take_authored_order();
        self.binding_authored_order.push(authored_order);
        self.bindings.push(row);
        Ok(id)
    }

    pub(crate) fn binding(&self, id: NumericBindingId) -> &PendingBindingRow {
        &self.bindings[id.as_usize()]
    }

    /// Read one binding's stable origin facts without following its alias recipe.
    fn binding_origin_queries(&self, binding: NumericBindingId) -> BindingOriginQueries {
        increment_ast_counter(AstCounter::NumericAliasOriginSteps);
        self.binding(binding).origin
    }

    /// Capture the recipe's origin once, before this binding becomes available to later aliases.
    ///
    /// WHY: producer lists and shape IDs are fixed at declaration birth. Class unions can change
    ///      their numeric leaves later, but cannot replace these shape or reference boundaries.
    fn recipe_origin_queries(&self, recipe: Option<&NumericRecipe>) -> BindingOriginQueries {
        let mut origin = BindingOriginQueries::default();
        match recipe {
            Some(NumericRecipe::Alias { source, .. }) => {
                origin = self.binding_origin_queries(*source);
            }
            Some(NumericRecipe::Directed { shape, .. }) => {
                origin.known_shape = *shape;
                origin.is_forward = shape.is_none();
            }
            Some(NumericRecipe::MultiBindSlot { producers, shape }) => {
                origin.known_shape = *shape;
                origin.is_forward = producers.iter().any(|producer| match producer.kind {
                    PendingMultiBindProducerKind::Binding(source) => {
                        self.binding_origin_queries(source).is_forward
                    }
                    PendingMultiBindProducerKind::Forward => true,
                    PendingMultiBindProducerKind::Raw(_) => false,
                });
            }
            Some(NumericRecipe::CollectionElement { element }) => {
                origin.known_shape = Some(*element);
            }
            Some(
                NumericRecipe::CaughtError { shape } | NumericRecipe::PatternPayload { shape },
            ) => {
                origin.known_shape = Some(*shape);
            }
            Some(NumericRecipe::Raw { .. } | NumericRecipe::RangeCounter { .. }) | None => {}
        }
        origin
    }

    /// Take one binding's retained recipe for its single completion pass.
    ///
    /// WHAT: moves the authored fragment out of the row, leaving `None`. The birth-time origin
    ///       facts remain available to completion and later reference reads.
    /// WHY: the recipe must reach completion exactly once and be materialised by move, never
    ///      cloned for a probe or a second resolution attempt.
    pub(crate) fn take_binding_recipe(
        &mut self,
        id: NumericBindingId,
    ) -> Result<NumericRecipe, CompilerError> {
        let row = &mut self.bindings[id.as_usize()];
        let recipe = row.recipe.take().ok_or_else(|| {
            CompilerError::compiler_error(
                "a pending numeric binding's recipe was taken twice by body-root completion",
            )
        })?;
        Ok(recipe)
    }

    /// All pending bindings in creation order.
    pub(crate) fn bindings(&self) -> &[PendingBindingRow] {
        &self.bindings
    }

    // --------------------------
    //  Continuations
    // --------------------------

    pub(crate) fn register_continuation(
        &mut self,
        continuation: NumericContinuation,
    ) -> Result<NumericContinuationId, CompilerError> {
        self.ensure_live("a numeric-dependent statement")?;
        let id = NumericContinuationId(checked_dense_id(self.continuations.len(), "continuation")?);
        self.continuations.push(Some(continuation));
        Ok(id)
    }

    /// The number of registered continuations, in dense ID order.
    pub(crate) fn continuation_count(&self) -> usize {
        self.continuations.len()
    }

    /// Read one live continuation without taking ownership of its authored payload.
    pub(crate) fn continuation(&self, id: NumericContinuationId) -> Option<&NumericContinuation> {
        self.continuations
            .get(id.as_usize())
            .and_then(Option::as_ref)
    }

    /// Run one read-only walk over the retained continuations while they are detached.
    ///
    /// WHAT: moves the continuation table out, hands the walk a read view of it together with this
    ///       state for receiver merges, then restores the table before returning.
    /// WHY: a fallback walk reads retained bodies and merges receiver classes in the same state.
    ///      Moving the table out lets both borrows coexist without collecting or copying bodies.
    pub(crate) fn with_detached_continuations<Output>(
        &mut self,
        walk: impl FnOnce(&DetachedContinuations, &mut Self) -> Output,
    ) -> Output {
        let detached = DetachedContinuations {
            entries: std::mem::take(&mut self.continuations),
        };
        let output = walk(&detached, self);
        self.continuations = detached.entries;
        output
    }

    /// Move one retained `then` value to body-root completion.
    ///
    /// WHAT: marks exactly one producer slot as taken while its expression is completed.
    /// WHY: an inferred multi-bind slot may need a forward producer's real result type before its
    ///      enclosing value block resumes; the completed expression is put back in this slot and
    ///      reused by the authored `then` node.
    pub(crate) fn take_then_value(
        &mut self,
        id: NumericContinuationId,
        value_index: usize,
    ) -> Result<StatementExpression, CompilerError> {
        let Some(Some(NumericContinuation::ThenValue(continuation))) =
            self.continuations.get_mut(id.as_usize())
        else {
            return Err(CompilerError::compiler_error(
                "multi-bind producer routing addressed a continuation that is not a live `then`",
            ));
        };
        let Some(slot) = continuation.values.get_mut(value_index) else {
            return Err(CompilerError::compiler_error(
                "multi-bind producer routing addressed a `then` value outside its slot list",
            ));
        };
        match std::mem::replace(
            slot,
            crate::compiler_frontend::ast::statements::value_production::ThenValueSlot::Taken,
        ) {
            crate::compiler_frontend::ast::statements::value_production::ThenValueSlot::Pending(
                value,
            ) => Ok(value),
            other => {
                *slot = other;
                Err(CompilerError::compiler_error(
                    "multi-bind producer routing tried to complete a `then` value more than once",
                ))
            }
        }
    }

    /// Return one completed forward producer to its authored `then` slot.
    pub(crate) fn complete_then_value(
        &mut self,
        id: NumericContinuationId,
        value_index: usize,
        expression: ExprId,
    ) -> Result<(), CompilerError> {
        let Some(Some(NumericContinuation::ThenValue(continuation))) =
            self.continuations.get_mut(id.as_usize())
        else {
            return Err(CompilerError::compiler_error(
                "multi-bind producer completion addressed a continuation that is not a live `then`",
            ));
        };
        let Some(slot) = continuation.values.get_mut(value_index) else {
            return Err(CompilerError::compiler_error(
                "multi-bind producer completion addressed a `then` value outside its slot list",
            ));
        };
        match std::mem::replace(
            slot,
            crate::compiler_frontend::ast::statements::value_production::ThenValueSlot::Completed(
                expression,
            ),
        ) {
            crate::compiler_frontend::ast::statements::value_production::ThenValueSlot::Taken => {
                Ok(())
            }
            other => {
                *slot = other;
                Err(CompilerError::compiler_error(
                    "multi-bind producer completion had no matching taken `then` value",
                ))
            }
        }
    }

    /// The authored produced values of a retained `then` continuation, when the record is live.
    ///
    /// WHAT: read-only access to the per-slot parsed values of one `then` statement.
    /// WHY: a multi-bind target resolves from the real producer per slot — a materialised value's
    ///      `TypeId` or a retained value's own numeric recipe/expectation — without re-parsing the
    ///      block or completing it first.
    pub(crate) fn continuation_then_values(
        &self,
        id: NumericContinuationId,
    ) -> Option<&crate::compiler_frontend::ast::statements::value_production::ThenValueContinuation>
    {
        match self
            .continuations
            .get(id.as_usize())
            .and_then(Option::as_ref)
        {
            Some(NumericContinuation::ThenValue(continuation)) => Some(continuation),
            _ => None,
        }
    }
    /// Add a resolved multi-bind receiver to one still-inferred raw `then` slot.
    ///
    /// WHAT: fills only an absent per-slot expectation on the authored continuation.
    /// WHY: raw literals need the final target type before body completion, while the enclosing
    ///      value block must retain its original inferred-slot contract to diagnose conflicts.
    pub(crate) fn set_then_value_expected_type(
        &mut self,
        id: NumericContinuationId,
        value_index: usize,
        type_id: TypeId,
    ) -> Result<(), CompilerError> {
        let Some(Some(NumericContinuation::ThenValue(continuation))) =
            self.continuations.get_mut(id.as_usize())
        else {
            return Err(CompilerError::compiler_error(
                "multi-bind raw producer routing addressed a continuation that is not a live `then`",
            ));
        };
        let Some(expected_type) = continuation.slot_expected_types.get_mut(value_index) else {
            return Err(CompilerError::compiler_error(
                "multi-bind raw producer routing addressed a `then` slot outside its receiver contract",
            ));
        };
        if expected_type.is_none() {
            *expected_type = Some(type_id);
        }
        Ok(())
    }

    /// Take one retained continuation for resumption.
    ///
    /// WHAT: moves the continuation out of its dense slot, leaving `None`.
    /// WHY: resumption consumes the retained fragment exactly once; a memoized second read is an
    ///      invariant failure, never a replay.
    pub(crate) fn take_continuation(
        &mut self,
        id: NumericContinuationId,
    ) -> Option<NumericContinuation> {
        self.continuations
            .get_mut(id.as_usize())
            .and_then(Option::take)
    }

    // --------------------------
    //  Completion support
    // --------------------------

    /// Release every construction table after the body completed.
    ///
    /// WHAT: drops pending bindings, equality state and continuation records once the
    ///       completed AST no longer needs them.
    /// WHY: no numeric origin, recipe or suspended check may outlive the body boundary.
    pub(crate) fn release(&mut self) {
        self.classes = Vec::new();
        self.shapes = Vec::new();
        self.shape_pin_marks = Vec::new();
        self.shape_pin_epoch = 0;
        self.shape_authored_order = Vec::new();
        self.bindings = Vec::new();
        self.binding_authored_order = Vec::new();
        self.continuations = Vec::new();
        self.forward_producers = Vec::new();
        self.declared_bindings = Vec::new();
        self.released = true;
    }

    // --------------------------
    //  Forward producer registry
    // --------------------------

    /// Register one forward fragment as a pending producer of its numeric class.
    ///
    /// WHAT: moves the fragment into the body-owned registry and returns its dense ID.
    /// WHY: the fragment completes once, when its first consumer or a connected class demands it,
    ///      rather than at its enclosing statement.
    pub(crate) fn register_forward_producer(
        &mut self,
        class: NumericClassId,
        shape: KnownShapeId,
        fragment: DeferredNumericExpression,
    ) -> Result<ForwardProducerId, CompilerError> {
        self.ensure_live("a forward producer")?;
        let id = ForwardProducerId::from_dense_index(self.forward_producers.len())?;
        let authored_order = self.take_authored_order();
        self.forward_producers.push(ForwardProducer {
            class,
            shape,
            authored_order,
            state: ForwardProducerState::Pending(fragment),
        });
        let root = self.find_root(class);
        self.classes[root.as_usize()]
            .forward_producers
            .push(Reverse(id.0));
        Ok(id)
    }

    pub(crate) fn forward_producer_authored_order(&self, producer: ForwardProducerId) -> u64 {
        self.forward_producers[producer.as_usize()].authored_order
    }

    /// Give the pending producers `owned` registered the immediate result's literal destination.
    ///
    /// WHAT: replaces the expected destination of each pending producer in `owned` whose class shares
    ///       the root of `class`. A producer a nested statement already settled keeps its natural result.
    /// WHY: only an immediate result solves its parameter into a literal receiver, so a raw leaf beside
    ///      a named operand takes that receiver. `owned` is the range registered while the receiving
    ///      position's own text parsed; an earlier binding's producer stays outside it even when its
    ///      class shares the root.
    pub(crate) fn set_pending_forward_destinations(
        &mut self,
        class: NumericClassId,
        destination: ExpectedType,
        owned: std::ops::Range<usize>,
    ) {
        let root = self.find_root(class);
        for index in owned {
            let Some(producer_class) = self.forward_producers.get(index).map(|row| row.class)
            else {
                continue;
            };
            if self.find_root(producer_class) != root {
                continue;
            }
            let state = &mut self.forward_producers[index].state;
            if let ForwardProducerState::Pending(fragment) = state {
                fragment.expected = destination;
            }
        }
    }

    /// The numeric class of `shape` when that class carries a forward witness, else `None`.
    ///
    /// WHAT: reads only the shape's own numeric class; compound and concrete shapes report `None`.
    /// WHY: only a forward class defers its producer to registration, so every other shape keeps
    ///      the existing joined path and never reaches the registry.
    pub(crate) fn forward_numeric_class(&self, shape: KnownShapeId) -> Option<NumericClassId> {
        match self.shape(shape) {
            KnownShape::Numeric(class) if self.class_is_forward(class) => Some(class),
            _ => None,
        }
    }

    /// The first pending producer connected to a numeric leaf of `shape`.
    ///
    /// WHAT: each equality root indexes its producers by registration ID. The earliest pending
    ///       head across the shape's leaves is the same producer a registry-wide scan would select.
    /// WHY: completed, resolving and abandoned rows leave the index once, rather than being
    ///      rescanned from zero after every pull. Union transfers the index with the class facts.
    pub(crate) fn next_pending_forward_producer(
        &mut self,
        shape: KnownShapeId,
    ) -> Result<Option<ForwardProducerId>, CompilerError> {
        match self.shape(shape) {
            KnownShape::Concrete { .. } => return Ok(None),
            KnownShape::Numeric(class) => {
                return Ok(self.pending_forward_producer_for_class(class));
            }
            _ => {}
        }
        self.next_pending_forward_producer_inner(shape, &mut FxHashMap::default())
    }

    fn pending_forward_producer_for_class(
        &mut self,
        class: NumericClassId,
    ) -> Option<ForwardProducerId> {
        let root = self.find_root(class).as_usize();
        loop {
            let Some(&Reverse(index)) = self.classes[root].forward_producers.peek() else {
                break None;
            };
            if matches!(
                self.forward_producers[index as usize].state,
                ForwardProducerState::Pending(_)
            ) {
                break Some(ForwardProducerId(index));
            }
            self.classes[root].forward_producers.pop();
        }
    }

    /// Memoise only this demand: claiming a producer invalidates the next invocation's answers.
    fn next_pending_forward_producer_inner(
        &mut self,
        shape: KnownShapeId,
        memo: &mut FxHashMap<KnownShapeId, Option<ForwardProducerId>>,
    ) -> Result<Option<ForwardProducerId>, CompilerError> {
        if let Some(next) = memo.get(&shape) {
            return Ok(*next);
        }
        let next = match self.shape(shape) {
            KnownShape::Concrete { .. } => None,
            KnownShape::Numeric(class) => self.pending_forward_producer_for_class(class),
            KnownShape::Collection { element, .. } => {
                self.next_pending_forward_producer_inner(element, memo)?
            }
            KnownShape::Map { key, value } => {
                let key = self.next_pending_forward_producer_inner(key, memo)?;
                let value = self.next_pending_forward_producer_inner(value, memo)?;
                match (key, value) {
                    (Some(key), Some(value)) => Some(if key.0 <= value.0 { key } else { value }),
                    (key, value) => key.or(value),
                }
            }
            KnownShape::Optional { inner } => {
                self.next_pending_forward_producer_inner(inner, memo)?
            }
            KnownShape::DeclaredSubstitution { bindings, .. } => {
                let mut first: Option<ForwardProducerId> = None;
                for index in bindings.indices() {
                    let edge = self.declared_binding(index);
                    if let Some(next) =
                        self.next_pending_forward_producer_inner(edge.shape, memo)?
                        && first.is_none_or(|first| next.0 < first.0)
                    {
                        first = Some(next);
                    }
                }
                first
            }
        };
        memo.insert(shape, next);
        Ok(next)
    }

    /// Claim one pending forward fragment for completion, marking its producer as resolving.
    ///
    /// Returns `None` when the producer is not pending: it is already resolving, completed or taken.
    pub(crate) fn claim_pending_forward_producer(
        &mut self,
        producer: ForwardProducerId,
    ) -> Result<Option<(KnownShapeId, DeferredNumericExpression)>, CompilerError> {
        let row = self.forward_producer_mut(producer)?;
        if !matches!(row.state, ForwardProducerState::Pending(_)) {
            return Ok(None);
        }
        let ForwardProducerState::Pending(fragment) =
            std::mem::replace(&mut row.state, ForwardProducerState::Resolving)
        else {
            return Err(CompilerError::compiler_error(
                "a pending forward producer changed state while it was claimed",
            ));
        };
        Ok(Some((row.shape, fragment)))
    }

    /// Record the natural result of a producer claimed by `claim_pending_forward_producer`.
    pub(crate) fn complete_forward_producer(
        &mut self,
        producer: ForwardProducerId,
        expression: ExprId,
    ) -> Result<(), CompilerError> {
        let row = self.forward_producer_mut(producer)?;
        if !matches!(row.state, ForwardProducerState::Resolving) {
            return Err(CompilerError::compiler_error(
                "a forward producer completed without being claimed for resolution",
            ));
        }
        row.state = ForwardProducerState::Completed(expression);
        Ok(())
    }

    /// Take a completed producer's natural result exactly once.
    ///
    /// Returns `None` while the producer is pending or resolving; a taken producer fails at its caller.
    pub(crate) fn take_completed_forward_producer(
        &mut self,
        producer: ForwardProducerId,
    ) -> Result<Option<ExprId>, CompilerError> {
        let row = self.forward_producer_mut(producer)?;
        if !matches!(row.state, ForwardProducerState::Completed(_)) {
            return Ok(None);
        }
        let ForwardProducerState::Completed(expression) =
            std::mem::replace(&mut row.state, ForwardProducerState::Taken)
        else {
            return Err(CompilerError::compiler_error(
                "a completed forward producer changed state while it was taken",
            ));
        };
        Ok(Some(expression))
    }

    /// Mark the registry's current length so a speculative parse can roll back its producers.
    ///
    /// WHAT: returns the dense row count; every row at or after it was registered by the probe.
    /// WHY: a dropped probe leaves its pending producers connected to their classes, so the
    ///      rollback needs a boundary that covers only the rows the probe registered.
    pub(crate) fn forward_producer_checkpoint(&self) -> usize {
        self.forward_producers.len()
    }

    fn forward_producer_mut(
        &mut self,
        producer: ForwardProducerId,
    ) -> Result<&mut ForwardProducer, CompilerError> {
        self.forward_producers
            .get_mut(producer.as_usize())
            .ok_or_else(|| {
                CompilerError::compiler_error("a forward producer ID named no registered row")
            })
    }
}

/// Convert one table length into the compact 32-bit dense ID domain.
///
/// WHAT: rejects a table that cannot be addressed by a compact ID with a structured compiler
///       error instead of truncating or wrapping.
/// WHY: dense IDs are the only identity in the body-owned construction tables; a wrapped ID
///      would alias an existing row, so an exhausted table is an internal resource failure.
fn checked_dense_id(len: usize, table: &'static str) -> Result<u32, CompilerError> {
    u32::try_from(len).map_err(|_| {
        CompilerError::compiler_error(format!(
            "numeric construction {table} table exhausted its compact 32-bit ID capacity"
        ))
    })
}

/// True when an ordered fragment still contains construction-only numeric items.
///
/// WHAT: group markers are structural and never pending, so a group that holds only completed
///       operands is not retained on its own account.
pub(crate) fn items_contain_pending(items: &[ExpressionRpnItem]) -> bool {
    items.iter().any(|item| {
        matches!(
            item,
            ExpressionRpnItem::PendingNumericLiteral { .. }
                | ExpressionRpnItem::PendingNumericBinding { .. }
                | ExpressionRpnItem::PendingNumericCast { .. }
                | ExpressionRpnItem::PendingNumericCall { .. }
                | ExpressionRpnItem::PendingNumericExpression { .. }
        )
    })
}

/// True when an ordered fragment contains items the evaluator cannot resolve itself.
///
/// WHAT: pending numeric literals resolve during ordinary evaluation; a binding reference, retained
///       cast or retained call needs body-root completion before the expression can be evaluated.
/// WHY: only the unresolvable items force a position to defer. Group markers are structural, so one
///      flat scan reaches every item inside a group at its own operand position.
pub(crate) fn items_contain_unresolvable_numeric(items: &[ExpressionRpnItem]) -> bool {
    items.iter().any(|item| match item {
        ExpressionRpnItem::PendingNumericBinding { .. }
        | ExpressionRpnItem::PendingNumericCast { .. }
        | ExpressionRpnItem::PendingNumericCall { .. }
        | ExpressionRpnItem::PendingNumericExpression { .. } => true,
        ExpressionRpnItem::PendingNumericLiteral { .. }
        | ExpressionRpnItem::GroupOpen { .. }
        | ExpressionRpnItem::GroupClose { .. }
        | ExpressionRpnItem::Operand(_)
        | ExpressionRpnItem::Operator { .. }
        | ExpressionRpnItem::ResolvedOperator(_) => false,
    })
}

/// Build the diagnostic for a receiver whose completed numeric value disagrees.
///
/// WHAT: primary label at the receiving site, secondary label at the retained recipe.
/// WHY: the authored receiver and the forward-only operand facts are both useful, and the
///      report must be deterministic regardless of traversal order.
pub(crate) fn numeric_receiver_conflict(
    expected: TypeId,
    found: TypeId,
    receiver_span: Option<SourceSpan>,
    recipe_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let diagnostic = type_mismatch(expected, found, TypeMismatchContext::General, receiver_span);
    match recipe_span {
        Some(recipe_span) => diagnostic.with_label(Some(recipe_span), None),
        None => diagnostic,
    }
}

/// Build the deterministic conflict diagnostic for two exact receivers.
pub(crate) fn conflicting_receiver_diagnostic(
    first: ReceiverWitness,
    second: ReceiverWitness,
) -> DiagnosticDraft {
    let diagnostic = type_mismatch(
        first.type_id,
        second.type_id,
        TypeMismatchContext::General,
        second.span,
    );
    match first.span {
        Some(first_span) => diagnostic.with_label(Some(first_span), None),
        None => diagnostic,
    }
}

/// Validate that a resolved class type can receive the class's authored family.
///
/// WHAT: rejects a receiver that cannot receive a raw recipe's authored family before any
///       literal is materialised at the wrong destination. A whole-number bound origin accepts
///       only integer destinations; a decimal/exponent bound origin accepts only binary floats.
/// WHY: the receivers that reach a raw recipe are constraints on an *established* binding origin
///      (a later direct use, or a mutation's concrete value). The immediate authored receiver of
///      an annotated raw initializer is not one of these: the declaration boundary materialises
///      its whole literal directly in F32/Byte/Dec through the evaluator's destination rules, so
///      whole-to-float/Dec/Byte stays legal exactly where the author wrote it. Family selection is
///      a receiving decision on the established origin, never an implicit conversion: a bound
///      whole origin stays integer and a bound decimal/exponent origin stays binary-float.
pub(crate) fn family_receives_recipe(
    family: Option<NumericFamily>,
    type_id: TypeId,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> bool {
    let Some(family) = family else {
        return true;
    };
    let Some(scalar) = environment.fixed_scalar(type_id) else {
        // A number-scale destination (Dec/Dec2) is neither a bound integer origin's destination
        // nor a bound binary-float origin's destination, so no family reaches it here.
        return false;
    };
    match family {
        // A bound whole-number origin retains the integer family; the later receiver may select
        // any signed or unsigned integer width, but never `Byte`.
        NumericFamily::Whole => matches!(
            scalar.class(),
            FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
        ),
        // A decimal-family recipe materialises at a binary float, never at a bounded integer or a
        // number-scale destination.
        NumericFamily::Decimal => matches!(scalar.class(), FixedScalarClass::BinaryFloat),
    }
}

/// Result of one construction-table operation.
///
/// WHAT: reuses the expression parser's existing failure lane: an authored numeric conflict is a
///       `DiagnosticDraft` and a broken construction invariant is a typed `CompilerError`.
///      resumes, so no second error envelope exists.
pub(crate) type ConstructionResult<T> = Result<T, ExpressionParseError>;

/// Merge the exact receiving witnesses of a class into one selected type.
///
/// WHAT: a loop-item pinned class admits only exact integer receivers, recorded or refused.
///       Two differing exact receivers are a deterministic two-site conflict, regardless of the
///       order in which the uses were parsed.
/// WHY: the first use in source order is not the winner; every permitted use participates
///      before any default or forward computation consumes the binding. The loop-item check runs
///      here rather than at registration, so a receiver before or after the loop declaration
///      gets the same result.
pub(crate) fn merged_receiver_type(
    state: &mut NumericConstructionState,
    class: NumericClassId,
) -> ConstructionResult<Option<TypeId>> {
    let root = state.find_root(class).as_usize();
    let row = &state.classes[root];
    if row.pin == NumericPin::LoopItem
        && let Some(offender) = row.loop_ineligible_receiver
    {
        increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        return Err(numeric_receiver_conflict(
            NumericFamily::Whole.default_type_id(),
            offender.type_id,
            offender.span,
            None,
        )
        .into());
    }
    let Some(first) = row.first_receiver else {
        return Ok(None);
    };
    increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
    if let Some(witness) = row.differing_receiver {
        increment_ast_counter(AstCounter::NumericReceiverWitnessesInspected);
        return Err(conflicting_receiver_diagnostic(first, witness).into());
    }
    Ok(Some(first.type_id))
}

/// The class of one pending binding.
pub(crate) fn binding_class(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> NumericClassId {
    state.binding(binding).class
}

/// One authored direct use of a pending numeric origin.
///
/// WHAT: the binding ID plus the authored span, mode and copy spelling of a fragment whose whole
///       syntax is that one reference (parentheses allowed).
/// WHY: a receiving boundary constrains an origin directly only when the authored use *is* that
///      origin. Inside a computation the boundary types literal leaves and the result, and must
///      not push a constraint back onto a named operand.
#[derive(Debug, Clone)]
pub(crate) struct DirectPendingReference {
    pub(crate) binding: NumericBindingId,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) is_copy: bool,
}

/// Read the single direct pending binding reference of a fragment, through any enclosing groups.
pub(crate) fn single_direct_pending_reference(
    items: &[ExpressionRpnItem],
) -> Option<DirectPendingReference> {
    match single_operand(items)? {
        ExpressionRpnItem::PendingNumericBinding {
            binding,
            span,
            value_mode: _,
            is_copy,
        } => Some(DirectPendingReference {
            binding: *binding,
            span: *span,
            is_copy: *is_copy,
        }),
        _ => None,
    }
}

/// Register the exact receiving witness of a direct pending origin reference.
///
/// WHAT: a binding with a known collection or map shape constrains its eligible numeric leaves
///       through that shape; a scalar binding adds the receiver to its own origin class.
/// WHY: a direct use is a receiving constraint; a mention inside a computation is not.
///
/// An annotated binding is already exact, so its direct use adds no witness.
/// A reference to a forward origin, directly or through an alias, adds no witness, because a later
/// consumer resolves only literal-only initializers. Eligibility follows the referenced recipe, not
/// the merged class, so a raw origin joined to a forward class keeps its exact receiver.
pub(crate) fn register_direct_reference_witness(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    reference: &DirectPendingReference,
    type_id: TypeId,
) {
    let origin = state.binding_origin_queries(reference.binding);
    if origin.annotation.is_some() || origin.is_forward {
        return;
    }
    register_binding_witness(
        state,
        environment,
        reference.binding,
        type_id,
        reference.span,
    );
}

/// Register one exact receiving witness for a pending binding.
///
/// WHAT: a binding with a known shape projects the receiver through that shape, so each numeric
///       leaf receives its own exact witness. Any other binding receives the witness on its class,
///       and only when the receiver is an exact scalar numeric type.
/// WHY: the receiver is the binding's single static type source, so both paths must agree. A scalar
///      class never receives a non-numeric type, so a collection or option receiver cannot widen it.
pub(crate) fn register_binding_witness(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    binding: NumericBindingId,
    type_id: TypeId,
    span: Option<SourceSpan>,
) {
    match binding_known_shape(state, binding) {
        Some(shape) => register_shape_witness(state, environment, shape, type_id, span),
        None => {
            if let Some(scalar) = numeric_witness_type(type_id, environment) {
                let class = binding_class(state, binding);
                state.add_receiver_witness(class, scalar, span, environment);
            }
        }
    }
}

/// Join a pending binding with the generic result shape of a call it is linked to.
///
/// WHAT: a binding with a known shape joins that shape with the slot. A numeric-only binding joins
///       its class with the slot's numeric class unless the shape join rule refuses the pair, in
///       which case it stays separate. A concrete slot type registers as an exact witness on a
///       shapeless binding and never joins a class, so a projected source cannot pull the binding
///       back through its origin. Non-numeric concrete types are skipped by the witness helper, so
///       ordinary slot validation reports them.
/// WHY: the slot's numeric origins must reach an existing pending binding before the call
///      completes, without the slot supplying a type it does not yet have. Multi-bind targets and
///      catch `then` values that name a binding share this one link.
pub(crate) fn union_pending_binding_with_generic_slot(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    binding: NumericBindingId,
    slot_shape: KnownShapeId,
) {
    if let Some(target_shape) = binding_reference_shape(state, binding) {
        union_shape_classes(state, environment, target_shape, slot_shape);
        return;
    }
    match state.shape(slot_shape) {
        KnownShape::Numeric(slot_class) => {
            let target_class = binding_class(state, binding);
            if !numeric_classes_join_refused(state, target_class, slot_class) {
                state.union_classes(target_class, slot_class);
            }
        }
        KnownShape::Concrete { type_id, span } => {
            let order = state.shape_authored_order(slot_shape);
            if let Some(shape) = binding_known_shape(state, binding) {
                register_ordered_shape_witness(state, environment, shape, type_id, span, order);
            } else if let Some(scalar) = numeric_witness_type(type_id, environment) {
                let class = binding_class(state, binding);
                state.add_ordered_receiver_witness(class, scalar, span, order, environment);
            }
        }
        _ => {}
    }
}

/// Join a catch-protected binding's class with the class of its protected literal.
///
/// WHAT: a binding declared from a retained catch fragment joins the class of that fragment's
///       protected literal, unless the shape join rule refuses the pair. Any other declaration
///       shape is left alone.
/// WHY: the protected literal and the binding it initialises are one value, so every later receiver
///      of the binding must reach the literal before its default applies.
pub(crate) fn join_protected_catch_class(
    state: &mut NumericConstructionState,
    class: NumericClassId,
    items: &[ExpressionRpnItem],
) {
    let Some(ExpressionRpnItem::PendingNumericExpression { expression, .. }) =
        single_operand(items)
    else {
        return;
    };
    let DeferredExpression::CatchHandler(payload) = expression.as_ref() else {
        return;
    };
    let Some(protected_shape) = payload.protected_shape else {
        return;
    };
    let KnownShape::Numeric(protected_class) = state.shape(protected_shape) else {
        return;
    };
    if !numeric_classes_join_refused(state, class, protected_class) {
        state.union_classes(class, protected_class);
    }
}

/// Register one declaration annotation on a pending binding.
///
/// WHAT: the annotation contributes its numeric witness to the binding's class. An optional
///       annotation contributes its payload, because the wrapper is storage and never a witness.
///       Every concrete annotation is then recorded as the exact reference shape reads see.
///       Registration finishes this declaration's birth before a later alias can inherit its facts.
/// WHY: an annotation names a declared type, not a receiver to join against the recipe's open
///      origin, so reads must neither pin nor widen that origin. A shape-bearing binding peels its
///      option layer through its own shape projection, so it receives the full annotation. A
///      generic parameter type stays open because its concrete form is known only at instantiation.
pub(crate) fn register_binding_annotation(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    binding: NumericBindingId,
    type_id: TypeId,
    span: Option<SourceSpan>,
) -> ConstructionResult<()> {
    let witness_type = annotation_witness_type(state, environment, binding, type_id);
    register_binding_witness(state, environment, binding, witness_type, span);
    if environment.contains_generic_parameter(type_id) {
        return Ok(());
    }
    let annotation = state.intern_shape(KnownShape::Concrete { type_id, span })?;
    state.bindings[binding.as_usize()].origin.annotation = Some(annotation);
    Ok(())
}

/// Record the partly solved reference view of a pending declaration.
///
/// WHAT: stores the projected shape that reads see in place of the binding's open recipe shape.
///       The declaration installs this view at birth, before later aliases can name it.
/// WHY: the view is a read boundary only, never an authored annotation, so witnesses keep reaching
///      the recipe shape and the unsolved leaves still complete against their own origin.
pub(crate) fn register_pending_reference_view(
    state: &mut NumericConstructionState,
    binding: NumericBindingId,
    view: KnownShapeId,
) {
    state.bindings[binding.as_usize()].origin.reference_view = Some(view);
}

/// The known shape node a pending binding's value has, when one is recorded.
///
/// WHAT: a directed literal recipe records its shape at birth; an alias inherits its source's
///       value shape; a loop item records its element node; a multi-bind slot records the shape
///       merged from its producers. Scalar and other recipes have no shape.
/// WHY: completion, cyclic demand and recipe witnesses need the original value shape, which a
///      resolved row keeps after its recipe moves out. Reference reads use
///      `binding_reference_shape` instead so exact annotations remain a type boundary.
pub(crate) fn binding_known_shape(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> Option<KnownShapeId> {
    state.binding_origin_queries(binding).known_shape
}

/// The exact annotation a pending binding's reference reads carry, when one is recorded.
///
/// WHAT: a concrete declaration annotation is recorded at birth; an alias inherits that boundary
///       unless its own declaration records another annotation.
/// WHY: an annotated value is exact, so reads through its name must see the annotated type, not the
///      open origin shape its recipe still completes against.
fn binding_annotation(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> Option<KnownShapeId> {
    state.binding_origin_queries(binding).annotation
}

/// The exact annotated type a pending binding's completed value must have, when one is recorded.
///
/// WHAT: the binding's recorded concrete annotation, including an inherited alias boundary.
/// WHY: completion coerces the initializer to the type that reads already present, and a cyclic
///      demand seeds the same storage type, so both agree on one annotated type.
pub(crate) fn binding_annotation_type(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> Option<TypeId> {
    // Annotations are only ever recorded as concrete shapes, so any other shape has no type.
    let KnownShape::Concrete { type_id, .. } = state.shape(binding_annotation(state, binding)?)
    else {
        return None;
    };
    Some(type_id)
}

/// The numeric destination a directed initializer's raw leaves take from its own annotation.
///
/// WHAT: the numeric witness of the binding's annotation, peeled by `annotation_witness_type`.
/// WHY: an initializer's own annotation is its only immediate receiver. Later class witnesses
///      validate the completed value and never retype its raw leaves.
pub(crate) fn directed_annotation_destination(
    state: &NumericConstructionState,
    environment: &TypeEnvironment,
    binding: NumericBindingId,
) -> Option<TypeId> {
    let annotated = binding_annotation_type(state, binding)?;
    let witness_type = annotation_witness_type(state, environment, binding, annotated);
    numeric_witness_type(witness_type, environment)
}

/// The type a declaration annotation contributes as a witness to its pending binding.
///
/// WHAT: a shape-bearing binding keeps the full annotation; a shapeless binding peels an optional
///       wrapper.
/// WHY: an optional wrapper is storage, so a scalar binding is typed by its payload. A shape
///      projects its own option layer, so it receives the full annotation.
fn annotation_witness_type(
    state: &NumericConstructionState,
    environment: &TypeEnvironment,
    binding: NumericBindingId,
    type_id: TypeId,
) -> TypeId {
    match binding_known_shape(state, binding) {
        Some(_) => type_id,
        None => environment.option_inner_type(type_id).unwrap_or(type_id),
    }
}

/// The shape a reference read of a pending binding sees: its annotation, else its partial view,
/// else its recipe shape.
///
/// WHAT: loop headers, receivers, and operand results read through this query.
/// WHY: an annotated binding presents its exact declared type, a partly solved one presents its
///      solved leaves, and completion and mutation witnesses keep the recipe shape from
///      `binding_known_shape`.
pub(crate) fn binding_reference_shape(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> Option<KnownShapeId> {
    let origin = state.binding_origin_queries(binding);
    origin
        .annotation
        .or(origin.reference_view)
        .or(origin.known_shape)
}

/// The collection node of a pending binding, for a loop source or item declaration.
///
/// WHY: only a collection can be iterated; maps and scalars answer `None` so the caller keeps the
///      ordinary non-iterable diagnostic.
pub(crate) fn pending_collection_shape(
    state: &NumericConstructionState,
    binding: NumericBindingId,
) -> Option<KnownShapeId> {
    binding_reference_shape(state, binding).filter(|shape| is_collection_node(state, *shape))
}

fn is_collection_node(state: &NumericConstructionState, shape: KnownShapeId) -> bool {
    matches!(state.shape(shape), KnownShape::Collection { .. })
}

/// The known shape node an inline retained fragment's value has, when it is exactly one literal or
/// one retained generic call with one declared result.
///
/// WHAT: a retained collection or map literal, looking through any enclosing groups, answers its
///       node ID; a retained catch handler answers the protected shape its fragment recorded at
///       retention; a retained generic call answers the shape its declared result links to. Any
///       other fragment answers `None`.
/// WHY: looking through enclosing groups is the same projection the retained operand path uses;
///      no lookup or reparse is needed, and a generic result keeps its numeric link across a join.
pub(crate) fn retained_known_shape(items: &[ExpressionRpnItem]) -> Option<KnownShapeId> {
    match single_operand(items)? {
        ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
            match expression.as_ref() {
                DeferredExpression::Collection(literal) => Some(literal.shape),
                DeferredExpression::Map(literal) => Some(literal.shape),
                DeferredExpression::CatchHandler(payload) => payload.protected_shape,
                DeferredExpression::ValueBlock(_) | DeferredExpression::OperandContinuation(_) => {
                    None
                }
            }
        }
        ExpressionRpnItem::PendingNumericCall { call, .. } => call.single_success_result_shape(),
        _ => None,
    }
}

/// The exact numeric witness a completed value contributes to a pending destination.
///
/// WHAT: returns the type only when it is one of the exact bounded numeric identities that can
///       receive or feed another numeric value. Non-numeric values keep their ordinary assignment
///       compatibility check instead of becoming a witness.
pub(crate) fn numeric_witness_type(
    type_id: TypeId,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> Option<TypeId> {
    if environment.fixed_scalar(type_id).is_some() || environment.number_scale(type_id).is_some() {
        Some(type_id)
    } else {
        None
    }
}

/// Classify one retained initializer fragment into its construction recipe.
///
/// WHAT: a fragment that is exactly one pending reference is an alias (copy/equality); a fragment
///       built only from literal tokens, groups and operators is a raw literal recipe; anything
///       else is a directed recipe over named operands and other completed values. A directed
///       recipe keeps the `directed_shape` its caller derived from the same fragment.
/// WHY: raw literal recipes may materialise at an exact receiver; directed recipes resolve
///      forward from their operands and only *validate* an exact receiver afterwards.
pub(crate) fn classify_recipe(
    items: Vec<ExpressionRpnItem>,
    directed_shape: Option<KnownShapeId>,
) -> NumericRecipe {
    if let Some(reference) = single_direct_pending_reference(&items) {
        return NumericRecipe::Alias {
            source: reference.binding,
            is_copy: reference.is_copy,
        };
    }
    if items.iter().all(item_is_literal_recipe_shape) {
        let family = NumericFamily::of_items(&items);
        return NumericRecipe::Raw { items, family };
    }
    NumericRecipe::Directed {
        items,
        shape: directed_shape,
    }
}

/// True when an item can materialise at a selected destination without a completed operand.
///
/// WHAT: group markers are neutral, so a group of literal leaves is a raw recipe,
///       like its bare leaves.
pub(crate) fn item_is_literal_recipe_shape(item: &ExpressionRpnItem) -> bool {
    match item {
        ExpressionRpnItem::PendingNumericLiteral { .. }
        | ExpressionRpnItem::Operator { .. }
        | ExpressionRpnItem::GroupOpen { .. }
        | ExpressionRpnItem::GroupClose { .. } => true,
        ExpressionRpnItem::Operand(_)
        | ExpressionRpnItem::ResolvedOperator(_)
        | ExpressionRpnItem::PendingNumericBinding { .. }
        | ExpressionRpnItem::PendingNumericCast { .. }
        | ExpressionRpnItem::PendingNumericCall { .. }
        | ExpressionRpnItem::PendingNumericExpression { .. } => false,
    }
}

/// Classify one retained `then` value without cloning or evaluating its RPN.
///
/// WHAT: links an exact pending binding, records an eligible raw-literal recipe, or keeps a
///       directed expression forward-only.
/// WHY: raw leaves may be selected by the multi-bind target, but a named operand establishes its
///      arithmetic result before the target can validate it; a receiver must not rewrite that
///      authored computation.
pub(crate) fn pending_multi_bind_producer_kind(
    items: &[ExpressionRpnItem],
) -> PendingMultiBindProducerKind {
    if let Some(reference) = single_direct_pending_reference(items) {
        return PendingMultiBindProducerKind::Binding(reference.binding);
    }
    let family = NumericFamily::of_items(items);
    if items.iter().all(item_is_literal_recipe_shape)
        && !raw_fragment_result_is_independently_fixed(items, family)
    {
        return PendingMultiBindProducerKind::Raw(family);
    }
    PendingMultiBindProducerKind::Forward
}

/// Whether one retained raw fragment's result domain is independent of its literal destination.
///
/// WHAT: true when the fragment's own operators fix its result independently of the receiving
///       type: a comparison resolves to the non-numeric `Bool` boundary, and an all-whole
///       fragment containing real division resolves through the integer default leaf domain to
///       an `F64` result (carried forward through any further whole arithmetic).
/// WHY: for such a fragment the receiving type constrains the completed result, never the literal
///      leaves: the leaves keep their authored default domain and the completed result is
///      validated against the receiver afterwards. A decimal or binary-float fragment stays
///      receiver-driven — `1.0 / 2.0` under an accepted decimal-family receiver materialises its
///      leaves and result at that receiver's domain — so only the whole-integer division and the
///      comparison boundary change the rule. Detection scans the flat retained items, where group
///      markers carry no operator, and classifies through the existing operator policy, never by
///      counting operators.
pub(crate) fn raw_fragment_result_is_independently_fixed(
    items: &[ExpressionRpnItem],
    family: NumericFamily,
) -> bool {
    let mut has_comparison = false;
    let mut has_division = false;
    for item in items {
        if let ExpressionRpnItem::Operator { operator, .. } = item {
            has_comparison |= matches!(
                operator,
                Operator::Equality
                    | Operator::NotEqual
                    | Operator::GreaterThan
                    | Operator::GreaterThanOrEqual
                    | Operator::LessThan
                    | Operator::LessThanOrEqual
                    | Operator::And
                    | Operator::Or
            );
            has_division |= matches!(operator, Operator::Divide);
        }
    }
    if has_comparison {
        return true;
    }
    matches!(family, NumericFamily::Whole) && has_division
}

/// Merge one alias binding into the class of the binding it copies.
///
/// WHAT: a copy shares the source's numeric constraint set, so every receiver registered for
///       either name participates in one selection.
/// WHY: copies connect the same numeric unknown rather than creating a second origin.
pub(crate) fn union_alias_classes(
    state: &mut NumericConstructionState,
    alias: NumericBindingId,
    source: NumericBindingId,
) {
    let alias_class = state.binding(alias).class;
    let source_class = state.binding(source).class;
    state.union_classes(alias_class, source_class);
}

/// Select a raw recipe's literal destination from its exact receivers or its family default.
///
/// WHAT: returns the numeric receiver that types the recipe's raw leaves, or `None` when the
///       family default or the evaluator's own destination rules must choose. A receiver that
///       cannot receive the established origin's family is a deterministic receiving conflict;
///       a non-numeric receiver does not type the leaves and is validated against the completed
///       result by `accept_forward_binding_result`.
/// WHY: receiving context precedes the language default, and the default is a language policy,
///      never a value-based width guess. The recipe itself is already owned by completion, so
///      only its family and first authored span cross this boundary.
pub(crate) fn resolve_raw_binding(
    state: &mut NumericConstructionState,
    binding: NumericBindingId,
    family: NumericFamily,
    recipe_span: Option<SourceSpan>,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    fixed_result: bool,
) -> ConstructionResult<Option<TypeId>> {
    let class = binding_class(state, binding);
    let Some(type_id) = merged_receiver_type(state, class)? else {
        return Ok(None);
    };
    // The fragment's own operator fixes the result domain (comparison -> Bool, real division ->
    // F64): the receiver constrains that completed result, never the literal leaves, so the
    // leaf-family acceptance check does not run and the leaves keep their authored family default.
    // The receiver is validated against the computed result by `accept_forward_binding_result`.
    if fixed_result {
        return Ok(None);
    }
    // A non-numeric receiver (for example an explicit `Bool` annotation on a comparison) must
    // never become the literal destination of the numeric leaves and must never be reported as a
    // family rejection: the completed result decides, one step later.
    if numeric_witness_type(type_id, environment).is_none() {
        return Ok(None);
    }
    if !family_receives_recipe(Some(family), type_id, environment) {
        let receiver_span = state
            .first_receiver_witness(class)
            .and_then(|witness| witness.span);
        return Err(numeric_receiver_conflict(
            type_id,
            family.default_type_id(),
            receiver_span,
            recipe_span,
        )
        .into());
    }
    Ok(Some(type_id))
}

/// Validate a forward-computed binding result against its registered receivers.
///
/// WHAT: a directed recipe's result is computed from its resolved operands; an exact receiver
///       that disagrees is a two-site conflict. The computed type itself lives in the completion
///       session, so this check owns no table write.
/// WHY: named operands stay forward-only, so a result-side receiver can validate the completed
///      operation but can never retype its operands.
pub(crate) fn accept_forward_binding_result(
    state: &mut NumericConstructionState,
    binding: NumericBindingId,
    computed: TypeId,
    recipe_span: Option<SourceSpan>,
) -> ConstructionResult<()> {
    let class = binding_class(state, binding);
    if let Some(witness_type) = merged_receiver_type(state, class)?
        && witness_type != computed
    {
        let receiver_span = state
            .first_receiver_witness(class)
            .and_then(|witness| witness.span);
        return Err(
            numeric_receiver_conflict(witness_type, computed, receiver_span, recipe_span).into(),
        );
    }
    Ok(())
}
