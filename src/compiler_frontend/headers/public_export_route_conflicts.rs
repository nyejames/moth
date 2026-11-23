//! Shared case-folded public-route conflict detection.
//!
//! WHAT: owns the folded-key index, authored-order comparison, folding helper and diagnostic-path
//!       interning used while a facade file is prepared and by the Stage 0 namespace summary.
//! WHY: one facade's routes are authored only in its root file, so header preparation is the
//!      single place that can reject a case-only conflict before any consumer resolves a route.
//!      The namespace summary reuses only the folding and interning rules for its own keys.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One route claim's authored position within its source.
///
/// WHAT: the source identity plus the authored `PathSyntaxId` row.
/// WHY: declaration and re-export routes reach validation through different collection passes, so
///      insertion order is not authored order. `PathSyntaxId` is source-local and monotonic in
///      authored order; claims from different sources keep insertion order because their rows are
///      not comparable.
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
struct CaseRouteClaim {
    route: PathId,
    span: SourceSpan,
    order: RouteClaimOrder,
}

impl CaseRouteClaim {
    const fn new(route: PathId, span: SourceSpan, order: RouteClaimOrder) -> Self {
        Self { route, span, order }
    }
}

/// A case-folded route key index that reports the first conflicting pair.
///
/// WHAT: maps one ASCII-lowercased route spelling to its first authored claim; a second distinct
///       spelling that folds to the same key returns the earlier/later pair in authored order.
/// WHY: a map lookup keeps one fold and one probe per distinct route instead of a pairwise
///      component comparison that re-allocates for every earlier route.
#[derive(Default)]
struct CaseFoldedRouteIndex {
    by_folded_key: FxHashMap<String, CaseRouteClaim>,
}

impl CaseFoldedRouteIndex {
    /// Record one already-folded route spelling.
    ///
    /// Returns `Err((earlier, later))` when a second claim folds onto an existing key. Exact
    /// spelling reuse belongs to the caller, so a repeated claim that reaches this index is
    /// always a conflict.
    fn insert(
        &mut self,
        folded_key: String,
        claim: CaseRouteClaim,
    ) -> Result<(), (CaseRouteClaim, CaseRouteClaim)> {
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

/// One authored route operand from a facade file's retained declaration or clause facts.
#[derive(Clone, Copy, Debug)]
pub(super) struct PublicExportRouteClaim {
    pub(super) route: PathId,
    pub(super) span: SourceSpan,
    pub(super) path_syntax: PathSyntaxId,
}

/// Validate one facade file's merged declaration and re-export route claims once.
///
/// WHAT: keeps the earliest authored claim per exact spelling, folds each distinct route once and
///       returns the existing two-site MOTH-RULE-0092 diagnostic for the first case-only conflict.
/// WHY: every route of one facade is authored in that facade's root file, where the original
///      `PathId`s and `PathSyntaxId` authored order still exist. Validating here means neither
///      Stage 0 nor the public-export collector needs a second folded-route index.
pub(super) fn case_conflicting_public_export_route_diagnostic(
    claims: impl IntoIterator<Item = PublicExportRouteClaim>,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Option<DiagnosticDraft> {
    let mut claims = claims.into_iter().collect::<Vec<_>>();
    // All claims belong to one facade file. The source identity stays in the key so a future
    // multi-source caller cannot compare source-local PathSyntaxId rows across files.
    claims.sort_by_key(|claim| (claim.span.source().index(), claim.path_syntax));

    let mut exact_spellings = FxHashSet::default();
    let mut folded = CaseFoldedRouteIndex::default();
    // `render_portable` clears the scratch before filling it, so one buffer serves every claim.
    let mut scratch = Vec::new();
    for claim in claims {
        if !exact_spellings.insert(claim.route) {
            continue;
        }
        let spelling = path_fork.render_portable(claim.route, string_table, &mut scratch);
        match folded.insert(
            fold_route_spelling(&spelling),
            CaseRouteClaim::new(
                claim.route,
                claim.span,
                RouteClaimOrder::new(claim.span.source(), claim.path_syntax),
            ),
        ) {
            Ok(()) => {}
            Err((earlier, later)) => {
                return Some(generics_traits_api::case_conflicting_public_export_route(
                    later.route,
                    earlier.route,
                    Some(earlier.span),
                    Some(later.span),
                ));
            }
        }
    }
    None
}

/// Intern one rendered route spelling into a diagnostic-only path fork.
///
/// WHAT: walks the `/`-separated components and reuses already-interned children.
/// WHY: an authored route outlives the facade fork that issued its path identity, so each
///      conflict owner reports it from the canonical spelling. `None` reports exhaustion of the
///      compact path-node domain, which the namespace lane reports as a capacity diagnostic.
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
