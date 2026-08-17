//! AST-phase-local fold cache for TIR templates.
//!
//! WHAT: stores the result of folding a specific TIR view (exact identity plus
//!       safe fold-context dimensions) so that repeated folds of the same
//!       effective view can reuse the previous result.
//!
//! WHY: parent templates can reference the same child template multiple times,
//!      and bottom-up folding can otherwise re-fold identical subtrees. A cache
//!      keyed by the stable dimensions of the fold input removes that redundant
//!      work without changing user-visible output.
//!
//! ## Cache lifetime
//!
//! The cache is AST-phase-local. It lives on `TirFoldContext`, which is
//! created and dropped during one compile-time template fold operation. It does
//! not survive into HIR, backend, or public API data and is never global or
//! static.
//!
//! ## Key safety
//!
//! The key includes only dimensions that are stable and identity-bearing today:
//! - exact `TirViewIdentity` (module-local root, pipeline phase and view context);
//! - const-loop iteration limit;
//! - whether the active fold-binding stack is empty.
//!
//! It does not include the binding stack contents. The cache is only valid when
//! bindings are empty, which is recorded as a boolean guard.

use std::collections::HashMap;

use crate::compiler_frontend::ast::templates::template_folding::TemplateFoldResult;
use crate::compiler_frontend::ast::templates::tir::view::TirViewIdentity;

// -------------------------
//  Cache key
// -------------------------

/// Deterministic key for one TIR fold cache entry.
///
/// WHAT: identifies the exact effective view and fold context that produced a
///       cached fold result. Two folds with equal keys must produce equal output.
///
/// WHY: the cache must not return stale results when the input view or context
///      changes. Embedding the exact view identity makes the key precise;
///      recording loop-limit and empty-bindings guards keeps the remaining
///      context dimensions explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TirFoldCacheKey {
    /// Exact module-local view identity.
    pub(crate) identity: TirViewIdentity,

    /// Const-loop iteration limit active during folding.
    pub(crate) loop_iteration_limit: usize,

    /// True only when the active fold-binding stack is empty.
    ///
    /// WHAT: fold bindings are pushed and restored during branch option captures
    ///       and loop iterations. The cache cannot safely share results across
    ///       different binding states, so it only caches views seen with no
    ///       active bindings.
    pub(crate) bindings_empty: bool,
}

// -------------------------
//  Cache
// -------------------------

/// AST-phase-local cache for TIR fold emissions.
///
/// WHAT: maps a deterministic `TirFoldCacheKey` to the previously computed
///       `TemplateFoldResult`.
///
/// WHY: avoids re-folding the same effective TIR view within one fold context.
///      The key is precise enough that cached results remain valid for the exact
///      view they were computed from.
#[derive(Clone, Debug, Default)]
pub(crate) struct TirFoldCache {
    entries: HashMap<TirFoldCacheKey, TemplateFoldResult>,
}

impl TirFoldCache {
    /// Creates an empty cache.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the cached emission for `key`, if any.
    pub(crate) fn get(&self, key: &TirFoldCacheKey) -> Option<&TemplateFoldResult> {
        self.entries.get(key)
    }

    /// Stores `emission` under `key`, returning any previous emission.
    pub(crate) fn insert(
        &mut self,
        key: TirFoldCacheKey,
        result: TemplateFoldResult,
    ) -> Option<TemplateFoldResult> {
        self.entries.insert(key, result)
    }
}
