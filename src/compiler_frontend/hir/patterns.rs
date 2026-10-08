//! HIR pattern matching data.
//!
//! WHAT: lowered pattern arms for HIR match terminators.
//! WHY: AST validates patterns and exhaustiveness; HIR preserves the validated matching contract for
//! backend lowering.

use crate::compiler_frontend::hir::ids::{ChoiceId, HirValueId};

#[derive(Debug, Clone)]
pub struct HirMatchArm {
    pub pattern: HirPattern,
    pub guard: Option<HirValueId>,
    pub body: crate::compiler_frontend::hir::ids::BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirRelationalPatternOp {
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Debug, Clone)]
pub enum HirPattern {
    Literal(HirValueId),
    OptionNone,
    OptionValue {
        value: HirValueId,
    },
    OptionRelational {
        op: HirRelationalPatternOp,
        value: HirValueId,
    },
    /// Matches any present option value (tag is `some`).
    ///
    /// WHAT: corresponds to `|name|` on an optional scrutinee.
    /// The capture local registration and payload assignment are handled
    /// separately by the match-capture lowering path.
    OptionPresent,
    Wildcard,
    Relational {
        op: HirRelationalPatternOp,
        value: HirValueId,
    },
    ChoiceVariant {
        choice_id: ChoiceId,
        variant_index: usize,
    },
}
