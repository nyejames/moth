//! HIR pattern matching data.
//!
//! WHAT: lowered pattern arms for HIR match terminators.
//! WHY: AST validates patterns and exhaustiveness; HIR preserves the validated matching contract for
//! backend lowering.

use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::ChoiceId;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

#[derive(Debug, Clone)]
pub struct HirMatchArm {
    pub pattern: HirPattern,
    pub guard: Option<HirExpression>,
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
    Literal(HirExpression),
    OptionNone,
    OptionValue {
        value: HirExpression,
    },
    OptionRelational {
        op: HirRelationalPatternOp,
        value: HirExpression,
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
        value: HirExpression,
    },
    ChoiceVariant {
        choice_id: ChoiceId,
        variant_index: usize,
    },
}

impl HirMatchArm {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match &mut self.pattern {
            HirPattern::Literal(value)
            | HirPattern::OptionValue { value }
            | HirPattern::OptionRelational { value, .. }
            | HirPattern::Relational { value, .. } => value.remap_string_ids(remap),
            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. } => {}
        }

        if let Some(guard) = &mut self.guard {
            guard.remap_string_ids(remap);
        }
    }
}
