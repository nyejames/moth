//! Core types for the value-production subsystem.
//!
//! WHAT: defines the shapes that represent produced values, active production targets,
//! and all-path branch exit summaries.
//! WHY: these types cross parser boundaries (dispatcher, catch handler and value
//! receivers) and need one canonical definition.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::expression::FallibleHandling;
use crate::compiler_frontend::ast::expressions::expression_store::{ExprId, OwnedExpr, PlaceId};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::numeric_construction::StatementExpression;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use super::receiver::RetainedValueBlock;

/// Values produced by a `then` statement inside a value-producing block.
///
/// WHAT: one or more expressions that are returned from the nearest active value-producing
/// block to its receiving site.
/// WHY: a statement-shaped marker is needed so `then` can see locals declared earlier in
/// the same body, and so HIR lowering can distinguish value production from ordinary
/// expression statements.
#[derive(Clone, Debug)]
pub struct ProducedValues {
    pub expressions: Vec<ExprId>,
    pub span: Option<SourceSpan>,
}

/// Target that `then` statements inside a value-producing block should produce values for.
///
/// WHAT: carries the expected result types and source location of the receiving site that
/// activated the value production.
/// WHY: the parser needs this to validate arity and apply contextual coercion at the point
/// where `then` values are parsed, before HIR lowering allocates result locals.
#[derive(Clone, Debug)]
pub struct ActiveValueProductionTarget {
    pub result_type_ids: Vec<TypeId>,
    /// The receiver kind keeps diagnostics receiver-aware without scattering boolean flags.
    pub receiver_kind: ValueReceiverKind,
    /// When `result_type_ids` is empty but the receiver still expects a specific
    /// number of produced values (e.g. multi-bind with some inferred slots), this
    /// tells `parse_retained_produced_values` how many expressions to read after `then`.
    pub expected_arity: Option<usize>,
    /// Per-slot expected types for mixed known/inferred multi-bind.
    ///
    /// WHAT: empty when every slot is already in `result_type_ids`. Otherwise one
    /// entry per target, with `Some` for slots that must be parsed in a known
    /// receiving context such as `none`.
    /// WHY: inferred multi-bind still has to type known optional slots at parse
    /// time, even though unknown siblings are inferred later.
    pub known_slot_types: Vec<Option<TypeId>>,
}

impl ActiveValueProductionTarget {
    pub fn known(result_type_ids: Vec<TypeId>, receiver_kind: ValueReceiverKind) -> Self {
        Self {
            result_type_ids,
            receiver_kind,
            expected_arity: None,
            known_slot_types: Vec::new(),
        }
    }

    pub fn mixed(slot_types: &[Option<TypeId>], receiver_kind: ValueReceiverKind) -> Self {
        if let Some(known) = slot_types.iter().copied().collect::<Option<Vec<_>>>() {
            return Self::known(known, receiver_kind);
        }

        Self {
            result_type_ids: Vec::new(),
            receiver_kind,
            expected_arity: Some(slot_types.len()),
            known_slot_types: slot_types.to_vec(),
        }
    }

    /// The provisional target for a catch handler whose receiving site is not resolved yet.
    ///
    /// WHAT: `slot_count` inferred produced values under the catch receiver, so a raw numeric
    ///       fallback is retained and typed by the site that resolves later. An unresolved fragment
    ///       passes one slot; a pending generic call passes its declared arity.
    /// WHY: a retained fragment's success contract is unknown when its handler body is parsed, so
    ///      the body must parse through the inferred path instead of a known or empty slot list —
    ///      completion revalidates arity and slot types against the resolved site. A zero count is
    ///      a zero-success known contract, never an inferred list.
    pub fn provisional_catch_handler(slot_count: usize) -> Self {
        if slot_count == 0 {
            return Self::known(Vec::new(), ValueReceiverKind::CatchHandler);
        }
        Self {
            result_type_ids: Vec::new(),
            receiver_kind: ValueReceiverKind::CatchHandler,
            expected_arity: Some(slot_count),
            known_slot_types: vec![None; slot_count],
        }
    }

    pub fn needs_slot_inference(&self) -> bool {
        self.result_type_ids.is_empty() && self.expected_arity.is_some()
    }

    /// The receiving contract per produced value.
    ///
    /// WHAT: the known slots when the receiver type is already established, otherwise the
    ///       per-slot expectations (`None` for a slot that only resolves later).
    /// WHY: a value-producing position derives its per-value expectations from one place, so a
    ///      known slot keeps the ordinary receiver expectation and an inferred slot stays
    ///      unresolved instead of being mistaken for an error-only contract.
    pub fn slot_expected_types(&self) -> Vec<Option<TypeId>> {
        if self.result_type_ids.is_empty() {
            if self.receiver_kind == ValueReceiverKind::Declaration && self.expected_arity.is_none()
            {
                // An unannotated declaration's initializer infers its single type from the
                // authored value, so its one slot is genuinely unknown — never an error-only
                // contract and never zero values.
                return vec![None];
            }

            self.known_slot_types.clone()
        } else {
            self.result_type_ids.iter().copied().map(Some).collect()
        }
    }

    /// How many produced values this target expects.
    pub fn expected_value_count(&self) -> usize {
        self.expected_arity
            .unwrap_or_else(|| self.slot_expected_types().len())
    }
}

/// Classification of the site that receives produced values.
///
/// WHAT: identifies why a value-production target was activated.
/// WHY: type-mismatch diagnostics distinguish declarations and returns from
/// assignment, multi-bind and catch receivers without boolean flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueReceiverKind {
    Declaration,
    Assignment,
    Return,
    MultiBind,
    CatchHandler,
}

/// One produced value while its body-root continuation is live.
///
/// WHAT: a retained statement expression, a value already completed for a receiving slot, or
///       the short-lived state while body-root completion owns that expression.
/// WHY: a multi-bind target may need the result type of a forward producer before its enclosing
///      value block can finish; moving the expression through `Taken` keeps one evaluation and
///      returns the completed value to the authored `then` node.
#[derive(Clone, Debug)]
pub enum ThenValueSlot {
    Pending(StatementExpression),
    Taken,
    Completed(ExprId),
}

/// Continuation for a `then` statement whose fallback values still hold numeric origins.
///
/// WHAT: the authored values in slot order, with the authored receiver kind, statement span and
///       scope, so the body root completes each value at its resolved receiver slot and rebuilds
///       the authored `then` node at its position.
/// WHY: the receiving site may not be resolved when the fallback is authored (a catch protecting
///      a fragment whose contract is unresolved), so each value must resume where the resolved
///      slot decides its type instead of materialising at the language default first.
#[derive(Clone, Debug)]
pub struct ThenValueContinuation {
    /// The authored fallback values in slot order, or a value already completed for its receiver.
    pub values: Vec<ThenValueSlot>,
    /// `Some` for a slot whose type the receiving site already knows and `None` for a slot that
    /// only resolves at the body root.
    ///
    /// WHY: completion must materialise each fallback at its real receiver — the stored contract
    /// for a known site, or the resolved slots when the site was provisional — so the two cases
    /// are never conflated with "no expectation".
    pub slot_expected_types: Vec<Option<TypeId>>,
    /// The authored receiver kind, for the slot-mismatch diagnostic once slots resolve.
    pub receiver_kind: ValueReceiverKind,
    pub span: Option<SourceSpan>,
    pub scope: PathId,
}

/// A value-producing control-flow block used as an expression at closed receiving sites.
///
/// WHAT: represents `if`, `match` and `catch` shapes that produce values instead
/// of executing statements for side effects.
/// WHY: receiving sites need to distinguish value blocks from ordinary expressions so
/// they can validate arity, type, and completeness before HIR lowering.
#[derive(Clone, Debug)]
pub enum ValueBlock {
    If(ValueIfBlock),
    LexicalScope(ValueLexicalScope),
    Match(ValueMatchBlock),
    Catch(ValueCatchBlock),
}

impl ValueBlock {
    pub(crate) fn visit_place_ids(&self, visitor: &mut dyn FnMut(PlaceId)) {
        match self {
            Self::If(block) => {
                for node in block.then_body.iter().chain(&block.else_body) {
                    node.visit_place_ids(visitor);
                }
            }
            Self::LexicalScope(block) => {
                for node in &block.body {
                    node.visit_place_ids(visitor);
                }
            }
            Self::Match(block) => {
                for arm in &block.arms {
                    for node in &arm.body {
                        node.visit_place_ids(visitor);
                    }
                }
                if let Some(body) = &block.default {
                    for node in body {
                        node.visit_place_ids(visitor);
                    }
                }
            }
            Self::Catch(block) => {
                if let FallibleHandling::Handler { body, .. } = &block.handler {
                    for node in body {
                        node.visit_place_ids(visitor);
                    }
                }
            }
        }
    }

    pub(crate) fn visit_place_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut PlaceId)) {
        match self {
            Self::If(block) => {
                for node in block.then_body.iter_mut().chain(&mut block.else_body) {
                    node.visit_place_ids_mut(visitor);
                }
            }
            Self::LexicalScope(block) => {
                for node in &mut block.body {
                    node.visit_place_ids_mut(visitor);
                }
            }
            Self::Match(block) => {
                for arm in &mut block.arms {
                    for node in &mut arm.body {
                        node.visit_place_ids_mut(visitor);
                    }
                }
                if let Some(body) = &mut block.default {
                    for node in body {
                        node.visit_place_ids_mut(visitor);
                    }
                }
            }
            Self::Catch(block) => {
                if let FallibleHandling::Handler { body, .. } = &mut block.handler {
                    for node in body {
                        node.visit_place_ids_mut(visitor);
                    }
                }
            }
        }
    }

    pub(crate) fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId)) {
        match self {
            Self::If(block) => {
                visitor(&mut block.condition);
                for node in block.then_body.iter_mut().chain(&mut block.else_body) {
                    node.visit_expression_ids_mut(visitor);
                }
            }
            Self::LexicalScope(block) => {
                for node in &mut block.body {
                    node.visit_expression_ids_mut(visitor);
                }
            }
            Self::Match(block) => {
                visitor(&mut block.scrutinee);
                for arm in &mut block.arms {
                    arm.pattern.visit_expression_ids_mut(visitor);
                    if let Some(guard) = &mut arm.guard {
                        visitor(guard);
                    }
                    for node in &mut arm.body {
                        node.visit_expression_ids_mut(visitor);
                    }
                }
                if let Some(body) = &mut block.default {
                    for node in body {
                        node.visit_expression_ids_mut(visitor);
                    }
                }
            }
            Self::Catch(block) => {
                visitor(&mut block.handled_value);
                if let FallibleHandling::Handler { body, .. } = &mut block.handler {
                    for node in body {
                        node.visit_expression_ids_mut(visitor);
                    }
                }
            }
        }
    }

    /// Enumerate retained leaves without expanding the expression graph.
    ///
    /// The store validates each leaf in its own domain before accepting the control-flow
    /// payload. Both branches remain graph-valid even when static selection later removes one.
    pub(crate) fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
        match self {
            Self::If(block) => {
                visitor(block.condition);
                for node in block.then_body.iter().chain(&block.else_body) {
                    node.visit_expression_ids(visitor);
                }
            }
            Self::LexicalScope(block) => {
                for node in &block.body {
                    node.visit_expression_ids(visitor);
                }
            }
            Self::Match(block) => {
                visitor(block.scrutinee);
                for arm in &block.arms {
                    arm.pattern.visit_expression_ids(visitor);
                    if let Some(guard) = arm.guard {
                        visitor(guard);
                    }
                    for node in &arm.body {
                        node.visit_expression_ids(visitor);
                    }
                }
                if let Some(body) = &block.default {
                    for node in body {
                        node.visit_expression_ids(visitor);
                    }
                }
            }
            Self::Catch(block) => {
                visitor(block.handled_value);
                if let FallibleHandling::Handler { body, .. } = &block.handler {
                    for node in body {
                        node.visit_expression_ids(visitor);
                    }
                }
            }
        }
    }
}

/// Shared receiver parse result.
///
/// WHAT: known and single-inferred receivers wrap a finished expression.
/// Mixed multi-bind returns the structural block so slot inference can finish
/// before construction wraps once. A block whose nested bodies still hold numeric
/// continuations returns the retained record instead, so its receiving site can
/// splice it into that position's own fragment.
/// WHY: `expression_build` must only see final `result_type_ids`, and no position
/// may observe an unresolved origin or a fabricated block.
#[derive(Debug)]
pub enum ParsedReceiverValue {
    /// A transient completed result; durable AST nodes store only its expression ID.
    Complete(OwnedExpr),
    NeedsSlotInference {
        block: ValueBlock,
        span: Option<SourceSpan>,
    },
    /// The block's nested bodies retained numeric continuations for the enclosing body root.
    Retained(Box<RetainedValueBlock>),
}

/// Single `if` value-producing block.
///
/// WHAT: `if condition then a else b` or the colon/block equivalent.
/// WHY: carries both branches as statement bodies so `then` can see locals declared
/// earlier in the same branch.
#[derive(Clone, Debug)]
pub struct ValueIfBlock {
    pub condition: ExprId,
    pub then_body: Vec<AstNode>,
    pub else_body: Vec<AstNode>,
    pub then_scope: PathId,
    pub else_scope: PathId,
    pub span: Option<SourceSpan>,
    pub generic_request_ranges: IfGenericRequestRanges,
    /// Expected result types for each produced value slot.
    ///
    /// WHAT: one type per value produced by `then` in each branch.
    /// WHY: HIR lowering needs the individual slot types to allocate result locals,
    ///      and the AST expression success shape is derived from these slots.
    pub result_type_ids: Vec<TypeId>,
}

/// One statically selected value-producing body.
///
/// WHAT: preserves the authored branch body after Stage 4 removes its known Bool condition.
/// WHY: HIR still needs the ordinary value-block target for `then` values, but must not receive
/// a runtime branch or the inactive body.
#[derive(Clone, Debug)]
pub struct ValueLexicalScope {
    pub body: Vec<AstNode>,
    pub scope: PathId,
    pub result_type_ids: Vec<TypeId>,
}

/// Full value-producing match block.
///
/// WHAT: `if value is:` or a one-arm option/choice predicate used at a closed
/// receiving site, with each reachable arm producing values via `then` or
/// terminating.
/// WHY: this reuses statement match parsing and HIR match CFG lowering while keeping
/// value-block result slots explicit for hidden result-local allocation.
#[derive(Clone, Debug)]
pub struct ValueMatchBlock {
    pub scrutinee: ExprId,
    pub arms: Vec<MatchArm>,
    pub default: Option<Vec<AstNode>>,
    pub exhaustiveness: MatchExhaustiveness,
    pub span: Option<SourceSpan>,
    pub result_type_ids: Vec<TypeId>,
}

/// Value-producing catch block.
///
/// WHAT: wraps a handled fallible expression whose catch handler body uses
/// `ThenValue` statements to produce the recovered success values.
/// WHY: catch recovery now shares the same value-block lowering target as `if`
/// and match blocks, instead of carrying catch-specific terminal fallback values.
#[derive(Clone, Debug)]
pub struct ValueCatchBlock {
    pub handled_value: ExprId,
    pub handler: FallibleHandling,
    pub result_type_ids: Vec<TypeId>,
}

/// Independent all-path exit facts for a statement sequence.
///
/// WHAT: records whether any reachable path can fall through, produce values, or
/// terminate. These facts are independent: a body may produce on one path and
/// terminate on another without falling through.
/// WHY: a tri-state enum cannot represent mixed produce/terminate completeness,
/// which the value-producing contract accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchExitSummary {
    pub can_fall_through: bool,
    pub produces_value: bool,
    pub terminates: bool,
}

impl BranchExitSummary {
    /// Empty or ordinary statement sequence: control continues.
    pub const FALLS_THROUGH: Self = Self {
        can_fall_through: true,
        produces_value: false,
        terminates: false,
    };

    /// `then` produces values and does not continue.
    pub const PRODUCES: Self = Self {
        can_fall_through: false,
        produces_value: true,
        terminates: false,
    };

    /// `return`, `return!` or a literal-false assertion stops the path.
    pub const TERMINATES: Self = Self {
        can_fall_through: false,
        produces_value: false,
        terminates: true,
    };

    /// Unions alternative branches such as `if`/`else` or match arms.
    pub fn union(self, other: Self) -> Self {
        Self {
            can_fall_through: self.can_fall_through || other.can_fall_through,
            produces_value: self.produces_value || other.produces_value,
            terminates: self.terminates || other.terminates,
        }
    }

    /// Sequences the next statement onto paths that still fall through.
    pub fn then_sequence(self, next: Self) -> Self {
        if !self.can_fall_through {
            return self;
        }

        Self {
            can_fall_through: next.can_fall_through,
            produces_value: self.produces_value || next.produces_value,
            terminates: self.terminates || next.terminates,
        }
    }
}
