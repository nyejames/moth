//! Shared case-folded public-route conflict detection.
//!
//! WHAT: owns the folded-key index, authored-order comparison, folding helper and diagnostic-path
//!       interning used both by the eager facade route prepass and by the header-owned
//!       public-export collector.
//! WHY: routes can be prepared in separate Stage 0 lanes. One owner for the conflict rule keeps
//!      the resulting diagnostic identical whichever lane observes the case-only pair first.
//!      Project builds observe it in the prepass; lanes without a prepass still use the collector.

use rustc_hash::FxHashMap;

use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One route claim's authored position within its source.
///
/// WHAT: the source identity plus the authored `PathSyntaxId` row.
/// WHY: the collector merges two collection passes, so insertion order is not authored order.
///      `PathSyntaxId` is source-local and monotonic in authored order; claims from different
///      sources keep insertion order because their rows are not comparable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteClaimOrder {
    source: SourceId,
    path_syntax: PathSyntaxId,
}

impl RouteClaimOrder {
    pub(crate) const fn new(source: SourceId, path_syntax: PathSyntaxId) -> Self {
        Self {
            source,
            path_syntax,
        }
    }

    /// Whether this claim was authored before the other claim in the same source.
    pub(crate) fn precedes(self, other: Self) -> bool {
        self.source == other.source && self.path_syntax < other.path_syntax
    }
}

/// One route claim retained by the folded-key index.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaseRouteClaim<Id> {
    pub(crate) id: Id,
    pub(crate) span: SourceSpan,
    pub(crate) order: RouteClaimOrder,
}

impl<Id> CaseRouteClaim<Id> {
    pub(crate) const fn new(id: Id, span: SourceSpan, order: RouteClaimOrder) -> Self {
        Self { id, span, order }
    }
}

/// A case-folded route key index that reports the first conflicting pair.
///
/// WHAT: maps one ASCII-lowercased route spelling to its first authored claim; a second distinct
///       spelling that folds to the same key returns the earlier/later pair in authored order.
/// WHY: a map lookup keeps one fold and one probe per distinct route instead of a pairwise
///      component comparison that re-allocates for every earlier route.
pub(crate) struct CaseFoldedRouteIndex<Id> {
    by_folded_key: FxHashMap<String, CaseRouteClaim<Id>>,
}

impl<Id> Default for CaseFoldedRouteIndex<Id> {
    fn default() -> Self {
        Self {
            by_folded_key: FxHashMap::default(),
        }
    }
}

impl<Id: Copy> CaseFoldedRouteIndex<Id> {
    /// Record one already-folded route spelling.
    ///
    /// Returns `Err((earlier, later))` when a second claim folds onto an existing key. Exact
    /// spelling reuse belongs to the caller, so a repeated claim that reaches this index is
    /// always a conflict.
    pub(crate) fn insert(
        &mut self,
        folded_key: String,
        claim: CaseRouteClaim<Id>,
    ) -> Result<(), (CaseRouteClaim<Id>, CaseRouteClaim<Id>)> {
        if let Some(existing) = self.by_folded_key.get(&folded_key).copied() {
            let (earlier, later) = if claim.order.precedes(existing.order) {
                (claim, existing)
            } else {
                (existing, claim)
            };
            return Err((earlier, later));
        }
        self.by_folded_key.insert(folded_key, claim);
        Ok(())
    }
}

/// ASCII-fold one route spelling for case-insensitive conflict lookup.
pub(crate) fn fold_route_spelling(spelling: &str) -> String {
    spelling.to_ascii_lowercase()
}

/// Intern one rendered route spelling into a diagnostic-only path fork.
///
/// WHAT: walks the `/`-separated components and reuses already-interned children.
/// WHY: an authored route outlives the facade fork that issued its path identity, so each
///      conflict owner reports it from the canonical spelling. `None` reports exhaustion of the
///      compact path-node domain, which the prepass treats as an invariant violation and the
///      namespace lane reports as a capacity diagnostic.
pub(crate) fn try_intern_route_spelling(
    spelling: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Option<PathId> {
    let mut path = PathId::ROOT;
    for component in spelling.split('/') {
        let component = string_table.intern(component);
        path = path_fork.try_intern_child(path, component)?;
    }
    Some(path)
}
