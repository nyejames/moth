//! Lightweight diagnostic-discovery stop metadata.
//!
//! WHAT: a copyable one-byte set naming why a module's optional diagnostic discovery ended.
//! WHY: report and outcome owners need the stop reason beside their results, but no expression,
//!      token or common diagnostic record should grow for a state most builds never set. Normal
//!      success stores the empty set and allocates nothing.

/// Why an optional diagnostic-discovery pass stopped before checking all remaining work.
///
/// A flag is set only after a real user error and actual skipped work. Reaching a limit exactly on
/// the last available unit claims nothing about unvisited work and sets no flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiagnosticDiscoveryStops(u8);

impl DiagnosticDiscoveryStops {
    /// No discovery work was skipped.
    pub(crate) const NONE: Self = Self(0);

    /// The module retained its configured number of primary errors.
    pub(crate) const MODULE_ERROR_ALLOWANCE: Self = Self(1 << 0);

    /// The additional-body token allowance for AST recovery was exhausted.
    pub(crate) const AST_RECOVERY_WORK_GUARD: Self = Self(1 << 1);

    /// The command stopped admitting new compilation work at its project threshold.
    ///
    /// This flag describes whole-command scheduling, so it is set on the command's accumulator
    /// only after a real returned primary error and a refused admission. It never belongs to one
    /// module's batch.
    pub(crate) const PROJECT_SCHEDULING_CUTOFF: Self = Self(1 << 2);

    /// True when no discovery work was skipped.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// One plain sentence naming every stop reason, or `None` for complete discovery.
    ///
    /// WHAT: the caller-owned presentation line for shortened checking.
    /// WHY: terminal and dev-server report owners print the same truthful wording without
    ///      counting undiscovered errors or learning the flag layout.
    pub fn stop_line(self) -> Option<String> {
        let reasons = match (
            self.contains(Self::MODULE_ERROR_ALLOWANCE),
            self.contains(Self::AST_RECOVERY_WORK_GUARD),
            self.contains(Self::PROJECT_SCHEDULING_CUTOFF),
        ) {
            (true, true, true) => {
                "module error limit, AST recovery work guard and project scheduling cutoff"
            }
            (true, true, false) => "module error limit and AST recovery work guard",
            (true, false, true) => "module error limit and project scheduling cutoff",
            (false, true, true) => "AST recovery work guard and project scheduling cutoff",
            (true, false, false) => "module error limit",
            (false, true, false) => "AST recovery work guard",
            (false, false, true) => "project scheduling cutoff",
            (false, false, false) => return None,
        };
        Some(format!(
            "Diagnostic discovery stopped early ({reasons}): later code was not checked."
        ))
    }

    /// True when every flag in `other` is already set.
    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

const _: () = assert!(
    std::mem::size_of::<DiagnosticDiscoveryStops>() == 1,
    "discovery stop metadata stays one byte"
);
