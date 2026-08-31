//! Build-owned resource input and watch-interest registry.
//!
//! WHAT: records the physical files discovered by Stage 0 before semantic resource origins,
//! output placement or byte reads exist.
//! WHY: graph activity and output liveness are separate. A referenced resource must be known and
//! watchable even when no reachable output ultimately emits it, while the compiler-facing table
//! remains free of filesystem and byte-source policy.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::file_references::ResourceSourceId;

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// One build-only watch interest for a resource target.
///
/// The path is retained as an IO fact. No contents, hash, semantic origin, output path or URL is
/// stored here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceWatchInterest {
    canonical_path: PathBuf,
}

impl ResourceWatchInterest {
    fn new(canonical_path: PathBuf) -> Self {
        Self { canonical_path }
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

/// One unhashed physical resource source registered by Stage 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceInputRecord {
    source_id: ResourceSourceId,
    canonical_source_path: PathBuf,
    watch_interests: Vec<ResourceWatchInterest>,
}

impl ResourceInputRecord {
    pub(crate) fn source_id(&self) -> ResourceSourceId {
        self.source_id
    }

    pub(crate) fn canonical_source_path(&self) -> &Path {
        &self.canonical_source_path
    }

    pub(crate) fn watch_interests(&self) -> &[ResourceWatchInterest] {
        &self.watch_interests
    }
}

/// Build-lifetime physical resource registry for one compilation boundary.
///
/// Physical sources are keyed by canonical path, so repeated authored references share one
/// opaque `ResourceSourceId`. The registry deliberately has no semantic resource identity or
/// byte-reading API; those facts belong to later compilation and emission owners.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResourceInputRegistry {
    records: Vec<ResourceInputRecord>,
    by_canonical_path: FxHashMap<PathBuf, ResourceSourceId>,
    missing_watch_interests: Vec<ResourceWatchInterest>,
}

impl ResourceInputRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register one existing canonical resource target, deduplicating its physical source.
    pub(crate) fn register_source(&mut self, canonical_source_path: PathBuf) -> ResourceSourceId {
        if let Some(source_id) = self.by_canonical_path.get(&canonical_source_path).copied() {
            let record = &mut self.records[source_id.index()];
            if !record
                .watch_interests
                .iter()
                .any(|interest| interest.canonical_path() == canonical_source_path)
            {
                record
                    .watch_interests
                    .push(ResourceWatchInterest::new(canonical_source_path));
            }
            return source_id;
        }

        let source_id = ResourceSourceId::from_index(self.records.len());
        let watch_interest = ResourceWatchInterest::new(canonical_source_path.clone());
        self.records.push(ResourceInputRecord {
            source_id,
            canonical_source_path: canonical_source_path.clone(),
            watch_interests: vec![watch_interest],
        });
        self.by_canonical_path
            .insert(canonical_source_path, source_id);
        source_id
    }

    /// Retain a watch interest for a target that did not canonicalize.
    ///
    /// A missing target cannot receive a physical source ID. Keeping its authored candidate
    /// path still lets a later file creation trigger a rebuild without fabricating a resource.
    pub(crate) fn record_missing_target_watch(&mut self, candidate_path: PathBuf) {
        if !self
            .missing_watch_interests
            .iter()
            .any(|interest| interest.canonical_path() == candidate_path)
        {
            self.missing_watch_interests
                .push(ResourceWatchInterest::new(candidate_path));
        }
    }

    pub(crate) fn records(&self) -> &[ResourceInputRecord] {
        &self.records
    }

    pub(crate) fn missing_watch_interests(&self) -> &[ResourceWatchInterest] {
        &self.missing_watch_interests
    }

    /// Prove the registry's dense IDs and canonical-path index still agree at a boundary handoff.
    pub(crate) fn validate(&self) -> Result<(), CompilerError> {
        for (index, record) in self.records().iter().enumerate() {
            if record.source_id.index() != index {
                return Err(CompilerError::compiler_error(format!(
                    "resource source ID {} is stored at record index {index}",
                    record.source_id.index()
                )));
            }
            if self.by_canonical_path.get(&record.canonical_source_path) != Some(&record.source_id)
            {
                return Err(CompilerError::compiler_error(format!(
                    "resource source {:?} is absent from its canonical-path index",
                    record.canonical_source_path
                )));
            }
            if record.watch_interests.is_empty() {
                return Err(CompilerError::compiler_error(format!(
                    "resource source {:?} has no watch interest",
                    record.canonical_source_path
                )));
            }
            if !record
                .watch_interests
                .iter()
                .any(|interest| interest.canonical_path() == record.canonical_source_path)
            {
                return Err(CompilerError::compiler_error(format!(
                    "resource source {:?} has no canonical watch interest",
                    record.canonical_source_path
                )));
            }
            let _ = (
                record.source_id(),
                record.canonical_source_path(),
                record.watch_interests(),
            );
        }

        for interest in self.missing_watch_interests() {
            if self
                .records()
                .iter()
                .any(|record| record.canonical_source_path == interest.canonical_path)
            {
                return Err(CompilerError::compiler_error(format!(
                    "missing resource watch {:?} overlaps an existing source",
                    interest.canonical_path
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/resource_inputs_tests.rs"]
mod tests;
