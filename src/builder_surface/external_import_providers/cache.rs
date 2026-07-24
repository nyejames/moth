//! Build-owned cache for external import provider results.
//!
//! WHAT: stores resolved external imports keyed by canonical source path and provider kind
//!       so repeated imports of the same file within one build reuse the first result.
//! WHY: avoids re-parsing the same external file multiple times when several Moth
//!      source files import it.

use super::provider::{ExternalImportProviderKind, ResolvedExternalImport};
use std::collections::HashMap;
use std::path::PathBuf;

/// Cache key that uniquely identifies one external source file + provider combination.
///
/// WHAT: the same canonical path resolved by different provider kinds (e.g. a file that
///       happens to match both `.js` and `.ts` registrations) must produce distinct cache
///       entries so providers do not collide.
/// WHY: preserves correct isolation when multiple providers are registered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalImportCacheKey {
    pub canonical_source_path: PathBuf,
    pub provider_kind: ExternalImportProviderKind,
}

/// Build-owned cache for external import resolution.
///
/// WHAT: maps cache keys to previously resolved imports within a single build.
/// WHY: provider parsing can be expensive; caching keeps compile times predictable.
#[derive(Clone, Debug, Default)]
pub struct ExternalImportProviderCache {
    entries: HashMap<ExternalImportCacheKey, ResolvedExternalImport>,
}

impl ExternalImportProviderCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Returns a cached resolved import if one exists for the given key.
    pub fn get(&self, key: &ExternalImportCacheKey) -> Option<&ResolvedExternalImport> {
        self.entries.get(key)
    }

    /// Inserts a resolved import into the cache.
    pub fn insert(&mut self, key: ExternalImportCacheKey, value: ResolvedExternalImport) {
        self.entries.insert(key, value);
    }

    /// Returns true if the cache contains an entry for the given key.
    pub fn contains_key(&self, key: &ExternalImportCacheKey) -> bool {
        self.entries.contains_key(key)
    }
}
