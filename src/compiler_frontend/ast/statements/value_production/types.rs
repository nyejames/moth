//! Core types for the value-production subsystem.
//!
//! WHAT: defines the shapes that represent produced values, active production targets,
//! and all-path branch exit summaries.
//! WHY: these types cross parser boundaries (dispatcher, catch handler and value
//! receivers) and need one canonical definition.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::ast_nodes::MatchExhaustiveness;
use crate::compiler_frontend::ast::expressions::expression::{Expression, FallibleHandling};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Values produced by a `then` statement inside a value-producing block.
///
/// WHAT: one or more expressions that are returned from the nearest active value-producing
/// block to its receiving site.
/// WHY: a statement-shaped marker is needed so `then` can see locals declared earlier in
/// the same body, and so HIR lowering can distinguish value production from ordinary
/// expression statements.
#[derive(Clone, Debug)]
pub struct ProducedValues {
    pub expressions: Vec<Expression>,
    pub location: SourceLocation,
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
    /// tells `parse_produced_values_typed` how many expressions to read after `then`.
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

    pub fn needs_slot_inference(&self) -> bool {
        self.result_type_ids.is_empty() && self.expected_arity.is_some()
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

/// Shared receiver parse result.
///
/// WHAT: known and single-inferred receivers wrap a finished expression.
/// Mixed multi-bind returns the structural block so slot inference can finish
/// before construction wraps once.
/// WHY: `expression_build` must only see final `result_type_ids`.
#[derive(Debug)]
pub enum ParsedReceiverValue {
    Complete(Expression),
    NeedsSlotInference(ValueBlock),
}

/// Single `if` value-producing block.
///
/// WHAT: `if condition then a else b` or the colon/block equivalent.
/// WHY: carries both branches as statement bodies so `then` can see locals declared
/// earlier in the same branch.
#[derive(Clone, Debug)]
pub struct ValueIfBlock {
    pub condition: Expression,
    pub then_body: Vec<AstNode>,
    pub else_body: Vec<AstNode>,
    pub then_scope: InternedPath,
    pub else_scope: InternedPath,
    pub location: SourceLocation,
    pub generic_request_ranges: IfGenericRequestRanges,
    /// Expected result types for each produced value slot.
    ///
    /// WHAT: one type per value produced by `then` in each branch.
    /// WHY: HIR lowering needs the individual slot types to allocate result locals,
    ///      and the AST expression type is derived from these (single type or tuple).
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
    pub scope: InternedPath,
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
    pub scrutinee: Expression,
    pub arms: Vec<MatchArm>,
    pub default: Option<Vec<AstNode>>,
    pub exhaustiveness: MatchExhaustiveness,
    pub location: SourceLocation,
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
    pub handled_value: Box<Expression>,
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
