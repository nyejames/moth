//! Const-execution budget vocabulary.
//!
//! WHAT: names which resource a const evaluation allowance limits and the unit that allowance is
//!       counted in.
//! WHY: the executor refuses the attempt and the diagnostic renderer names the refusal, so one
//!      shared vocabulary keeps the category and unit identical in both places.

/// Which resource a const evaluation attempt exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstBudgetCategory {
    Work,
    Memory,
}

impl ConstBudgetCategory {
    /// The prose name for this category in advice text.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Memory => "retained memory",
        }
    }
}

/// Unit in which one budget limit is expressed.
///
/// WHY: configured work and memory limits share the same refusal type as structural hard limits
///      (call depth, retained values, expression depth). Advice must name the limit's real unit
///      instead of assuming every work refusal is measured in steps or every memory refusal in
///      MiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstBudgetUnit {
    Steps,
    Mib,
    RetainedValues,
    Pieces,
    CallFrames,
    ExpressionLevels,
    HandlerLevels,
    ProjectionLevels,
}

impl ConstBudgetUnit {
    /// The category that owns this unit's allowance.
    ///
    /// WHAT: the one unit-to-category mapping, used to pick the refusal reason and its advice.
    /// WHY: every refusal carries both facts, so deriving the category here keeps them from
    ///      disagreeing at a second, hand-written mapping.
    pub(crate) fn category(self) -> ConstBudgetCategory {
        match self {
            Self::Steps
            | Self::CallFrames
            | Self::ExpressionLevels
            | Self::HandlerLevels
            | Self::ProjectionLevels => ConstBudgetCategory::Work,
            Self::Mib | Self::RetainedValues | Self::Pieces => ConstBudgetCategory::Memory,
        }
    }
}
