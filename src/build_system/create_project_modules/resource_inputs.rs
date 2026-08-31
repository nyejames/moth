//! Build-owned resource input and missing-target watch registry.
//!
//! WHAT: records the physical files discovered by Stage 0 before semantic resource origins,
//! output placement or byte reads exist.
//! WHY: graph activity and output liveness are separate. A referenced resource must be known and
//! watchable even when no reachable output ultimately emits it, while the compiler-facing table
//! remains free of filesystem and byte-source policy.
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Preflight receipt for a complete compiler-produced origin/source association batch.
///
/// Only associations not already present in the build registry are retained. The receipt is
/// populated without mutating the registry, then reserved and committed by the combined module
/// publication operation.
#[derive(Debug)]
pub(crate) struct ResourceSourcePublication {
    new_associations: Vec<ResourceSourceAssociation>,
}

/// The build-owned content state of one physical resource source.
///
/// Stage 0 registers and watches sources without reading them. Output emission advances this state
/// only for planned live sources. Hashing caches the source bytes so a later read can reuse the same
/// filesystem result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceContentState {
    Unhashed,
    Hashed { content_hash: u64 },
    Read { content_hash: u64 },
}

/// One build-only watch interest for a missing file target.
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

/// One existing physical resource source registered by Stage 0.
///
/// `canonical_source_path` is both the physical source key and the existing-file watch key.
/// Missing-target watch interests remain separate because they have no source ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceInputRecord {
    source_id: ResourceSourceId,
    canonical_source_path: PathBuf,
    content: ResourceContentState,
    bytes: Option<Vec<u8>>,
}

#[cfg(test)]
impl ResourceInputRecord {
    pub(crate) fn content(&self) -> ResourceContentState {
        self.content
    }
}

/// Build-lifetime physical resource registry for one compilation boundary.
///
/// Physical sources are keyed by canonical path, so repeated authored references share one opaque
/// `ResourceSourceId`. Stable semantic origins are attached explicitly after their owning module
/// publishes them. Emission advances only sources named by the live output plan and caches their
/// hash and bytes for the rest of the build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResourceInputRegistry {
    records: Vec<ResourceInputRecord>,
    by_canonical_path: FxHashMap<PathBuf, ResourceSourceId>,
    source_by_origin: FxHashMap<StableResourceOriginId, ResourceSourceId>,
    missing_watch_interests: Vec<ResourceWatchInterest>,
}

impl ResourceInputRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register one existing canonical resource target, deduplicating its physical source.
    pub(crate) fn register_source(&mut self, canonical_source_path: PathBuf) -> ResourceSourceId {
        if let Some(source_id) = self.by_canonical_path.get(&canonical_source_path).copied() {
            return source_id;
        }

        let source_id = ResourceSourceId::from_index(self.records.len());
        self.records.push(ResourceInputRecord {
            source_id,
            canonical_source_path: canonical_source_path.clone(),
            content: ResourceContentState::Unhashed,
            bytes: None,
        });
        self.by_canonical_path
            .insert(canonical_source_path, source_id);
        source_id
    }

    /// Preflight a complete compiler-produced origin/source association batch.
    ///
    /// Every source ID, existing attachment and pair within the batch is checked before the
    /// returned receipt can be reserved or committed. No registry state is mutated on failure.
    pub(crate) fn preflight_resource_source_associations(
        &self,
        associations: &[ResourceSourceAssociation],
    ) -> Result<ResourceSourcePublication, CompilerError> {
        let mut pending_by_origin: FxHashMap<&StableResourceOriginId, ResourceSourceId> =
            FxHashMap::default();
        let mut new_associations = Vec::new();

        for association in associations {
            let source_id = association.source;
            let Some(record) = self.records.get(source_id.index()) else {
                return Err(CompilerError::compiler_error(format!(
                    "resource origin attachment references unknown source ID {}",
                    source_id.index()
                )));
            };
            if record.source_id != source_id {
                return Err(CompilerError::compiler_error(format!(
                    "resource origin attachment source ID {} disagrees with its record ID {}",
                    source_id.index(),
                    record.source_id.index()
                )));
            }

            if let Some(existing_source_id) = self.source_by_origin.get(&association.origin) {
                if *existing_source_id != source_id {
                    return Err(CompilerError::compiler_error(format!(
                        "resource origin {:?} is attached to source ID {}, not {}",
                        association.origin,
                        existing_source_id.index(),
                        source_id.index()
                    )));
                }
                continue;
            }

            if let Some(existing_source_id) = pending_by_origin.get(&association.origin) {
                if *existing_source_id != source_id {
                    return Err(CompilerError::compiler_error(format!(
                        "resource origin {:?} is attached to source ID {}, not {}",
                        association.origin,
                        existing_source_id.index(),
                        source_id.index()
                    )));
                }
                continue;
            }

            pending_by_origin.insert(&association.origin, source_id);
            new_associations.push(association.clone());
        }

        Ok(ResourceSourcePublication { new_associations })
    }

    /// Reserve all map capacity needed by a successful association publication.
    pub(crate) fn reserve_resource_source_associations(
        &mut self,
        publication: &ResourceSourcePublication,
    ) {
        self.source_by_origin
            .reserve(publication.new_associations.len());
    }

    /// Commit a preflighted association batch without another fallible boundary.
    pub(crate) fn commit_resource_source_associations(
        &mut self,
        publication: ResourceSourcePublication,
    ) {
        for association in publication.new_associations {
            let previous = self
                .source_by_origin
                .insert(association.origin, association.source);
            debug_assert!(previous.is_none());
        }
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

    pub(crate) fn source_for_origin(
        &self,
        origin: &StableResourceOriginId,
    ) -> Option<ResourceSourceId> {
        self.source_by_origin.get(origin).copied()
    }

    /// Hash one live source, caching the bytes used to compute the hash for later emission.
    ///
    /// Stage 0 leaves every source unhashed. Central output orchestration calls this only after
    /// its complete destination preflight, so an unused watchable source remains untouched.
    pub(crate) fn hash_source(
        &mut self,
        source_id: ResourceSourceId,
        string_table: &mut StringTable,
    ) -> Result<u64, CompilerError> {
        let source_index = source_id.index();
        let (content_state, canonical_source_path) = self
            .records
            .get(source_index)
            .map(|record| (record.content, &record.canonical_source_path))
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "resource source ID {} is outside the source registry",
                    source_id.index()
                ))
            })?;

        match content_state {
            ResourceContentState::Unhashed => {
                // The filesystem read completes before the record below is borrowed mutably to
                // store the cached result: the canonical path is borrowed, never copied.
                let bytes = fs::read(canonical_source_path).map_err(|error| {
                    CompilerError::file_error(
                        canonical_source_path,
                        format!(
                            "Failed to read resource source '{}': {error}",
                            canonical_source_path.display()
                        ),
                        string_table,
                    )
                })?;
                let content_hash = resource_content_hash(&bytes);
                let record = self.records.get_mut(source_index).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "resource source ID {} disappeared while hashing",
                        source_id.index()
                    ))
                })?;
                record.bytes = Some(bytes);
                record.content = ResourceContentState::Hashed { content_hash };
                Ok(content_hash)
            }
            ResourceContentState::Hashed { content_hash }
            | ResourceContentState::Read { content_hash } => Ok(content_hash),
        }
    }

    /// Read one live source's cached bytes, advancing it to the read state.
    ///
    /// Hashing performs the sole filesystem read for a source. Repeated reads therefore borrow the
    /// same cached bytes, allowing several planned origins to emit from one physical source.
    pub(crate) fn read_source(
        &mut self,
        source_id: ResourceSourceId,
        string_table: &mut StringTable,
    ) -> Result<&[u8], CompilerError> {
        let content_hash = self.hash_source(source_id, string_table)?;
        let record = self.records.get_mut(source_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "resource source ID {} disappeared while reading",
                source_id.index()
            ))
        })?;

        if matches!(record.content, ResourceContentState::Hashed { .. }) {
            record.content = ResourceContentState::Read { content_hash };
        }

        record.bytes.as_deref().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "resource source ID {} has no cached bytes after hashing",
                source_id.index()
            ))
        })
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
        }

        for (origin, source_id) in &self.source_by_origin {
            let Some(record) = self.records.get(source_id.index()) else {
                return Err(CompilerError::compiler_error(format!(
                    "resource origin {origin:?} points outside the source registry at ID {}",
                    source_id.index()
                )));
            };
            if record.source_id != *source_id {
                return Err(CompilerError::compiler_error(format!(
                    "resource origin {origin:?} points to source ID {}, but that record owns ID {}",
                    source_id.index(),
                    record.source_id.index()
                )));
            }
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

/// Compute a stable per-build content hash without adding another dependency to the build system.
///
/// The hash is an invalidation fact only. Resource identity and output placement remain keyed by
/// their stable semantic origins and planned paths.
fn resource_content_hash(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;

    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
#[path = "../tests/resource_inputs_tests.rs"]
mod tests;
