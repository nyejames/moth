//! Build-owned resource input and missing-target watch registry.
//!
//! WHAT: records the physical files discovered by Stage 0 before semantic resource origins,
//! output placement or byte reads exist.
//! WHY: graph activity and output liveness are separate. A referenced resource must be known and
//! watchable even when no reachable output ultimately emits it, while the compiler-facing table
//! remains free of filesystem and byte-source policy.
use crate::build_system::utils::stable_content_hash;
use crate::builder_surface::definition::damaged_package_set_report;
use crate::builder_surface::package_metadata::PackageSetMembership;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

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
/// `canonical_source_path` is both the physical source key and the existing-file watch key. A
/// deferred shipped alias keeps its registered spelling until settlement; once it owns the read
/// path, the single field is updated to the canonical spelling. Missing-target watch interests
/// remain separate because they have no source ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceInputRecord {
    source_id: ResourceSourceId,
    canonical_source_path: PathBuf,
    /// Registration provenance that decides a read-failure report for this physical source.
    package_set_membership: PackageSetMembership,
    content: ResourceContentState,
    bytes: Option<Vec<u8>>,
}

#[cfg(test)]
impl ResourceInputRecord {
    pub(crate) fn content(&self) -> ResourceContentState {
        self.content
    }

    pub(crate) fn canonical_source_path(&self) -> &Path {
        &self.canonical_source_path
    }
}

/// Build-lifetime physical resource registry for one compilation boundary.
///
/// Physical sources are keyed by their registered spelling, so repeated authored references share
/// one opaque `ResourceSourceId`. Stable semantic origins are attached explicitly after their
/// owning module publishes them. Emission advances only sources named by the live output plan and
/// caches their hash and bytes for the rest of the build. Installed alias spellings resolve to one
/// canonical read record at that first read boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResourceInputRegistry {
    records: Vec<ResourceInputRecord>,
    /// Registered spelling to its logical record. A settled shipped alias also inserts its
    /// resolved canonical spelling here, so later exact registrations reuse the same source ID.
    by_canonical_path: FxHashMap<PathBuf, ResourceSourceId>,
    /// Shipped installed-alias registrations that still need one canonical resolution.
    pending_ids: Vec<ResourceSourceId>,
    /// Logical alias ID to the one record that owns the physical bytes. Sparse and one hop only.
    redirects: FxHashMap<ResourceSourceId, ResourceSourceId>,
    source_by_origin: FxHashMap<StableResourceOriginId, ResourceSourceId>,
    missing_watch_interests: Vec<ResourceWatchInterest>,
}

impl ResourceInputRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register one existing canonical resource target, deduplicating its physical source.
    ///
    /// Ordinary registrations never carry compiler-shipped package-set provenance. A shipped
    /// runtime asset whose path Stage 0 already proved canonical uses
    /// [`Self::register_source_with_membership`]; a direct installed alias uses
    /// [`Self::register_deferred_shipped_source`] so a later read failure can identify a damaged
    /// installation.
    pub(crate) fn register_source(&mut self, canonical_source_path: PathBuf) -> ResourceSourceId {
        self.register_source_with_membership(canonical_source_path, PackageSetMembership::Ordinary)
    }

    /// Register one existing canonical resource target with its registration provenance.
    ///
    /// One registered spelling yields one source ID. Duplicate registrations merge membership with
    /// shipped provenance winning: a shipped asset may also be reached by an ordinary registration,
    /// and a failed read still means the installed package set is incomplete regardless of which
    /// registration ran first. The supplied spelling is already canonical, so this lane performs
    /// no filesystem work and allocates no second path.
    pub(crate) fn register_source_with_membership(
        &mut self,
        canonical_source_path: PathBuf,
        package_set_membership: PackageSetMembership,
    ) -> ResourceSourceId {
        if let Some(source_id) = self.by_canonical_path.get(&canonical_source_path).copied() {
            // The caller just proved the exact spelling is canonical, so a matching deferred row
            // no longer needs resolution. Other aliases still settle against this owner.
            self.pending_ids.retain(|pending| *pending != source_id);
            let owner = self.physical_owner(source_id);
            self.merge_membership_into(owner, package_set_membership);
            return source_id;
        }

        let source_id = ResourceSourceId::from_index(self.records.len());
        self.records.push(ResourceInputRecord {
            source_id,
            canonical_source_path: canonical_source_path.clone(),
            package_set_membership,
            content: ResourceContentState::Unhashed,
            bytes: None,
        });
        self.by_canonical_path
            .insert(canonical_source_path, source_id);
        source_id
    }

    /// Register one compiler-shipped installed asset whose byte path is not yet proven canonical.
    ///
    /// Only direct built-in runtime assets use this lane. Package modules and provider-resolved JS
    /// already carry Stage 0 canonical paths and stay on [`Self::register_source_with_membership`].
    /// Registration still performs no filesystem work; the first content-read boundary resolves
    /// every pending installed alias before reading any bytes.
    pub(crate) fn register_deferred_shipped_source(
        &mut self,
        registered_source_path: PathBuf,
    ) -> ResourceSourceId {
        if let Some(source_id) = self.by_canonical_path.get(&registered_source_path).copied() {
            let owner = self.physical_owner(source_id);
            self.merge_membership_into(owner, PackageSetMembership::CompilerShipped);
            return source_id;
        }

        let source_id = ResourceSourceId::from_index(self.records.len());
        self.records.push(ResourceInputRecord {
            source_id,
            canonical_source_path: registered_source_path.clone(),
            package_set_membership: PackageSetMembership::CompilerShipped,
            content: ResourceContentState::Unhashed,
            bytes: None,
        });
        self.by_canonical_path
            .insert(registered_source_path, source_id);
        self.pending_ids.push(source_id);
        source_id
    }

    /// The one canonical record ID that owns a logical registration's bytes.
    fn physical_owner(&self, source_id: ResourceSourceId) -> ResourceSourceId {
        let owner = self.redirects.get(&source_id).copied().unwrap_or(source_id);
        debug_assert!(
            !self.redirects.contains_key(&owner),
            "resource redirects must point directly to a physical owner"
        );
        owner
    }

    fn merge_membership_into(
        &mut self,
        source_id: ResourceSourceId,
        package_set_membership: PackageSetMembership,
    ) {
        let record = &mut self.records[source_id.index()];
        record.package_set_membership = record.package_set_membership.merge(package_set_membership);
    }

    /// Resolve every pending shipped installed alias before the registry performs a content read.
    ///
    /// Settling every pending source, rather than only the requested one, lets an authored-first
    /// read inherit positive shipped provenance before bytes are cached. A resolved spelling either
    /// becomes the physical owner (one path update plus one exact canonical index entry) or
    /// redirects directly to an existing owner. A resolution that cannot prove a canonical key
    /// keeps the supplied spelling, so the deferred read failure retains today's host path and
    /// damaged-set report.
    fn settle_deferred_sources(&mut self) {
        if self.pending_ids.is_empty() {
            return;
        }

        let pending_batch = std::mem::take(&mut self.pending_ids);
        debug_assert!(
            pending_batch
                .windows(2)
                .all(|pair| pair[0].index() < pair[1].index()),
            "pending resource IDs stay in registration order"
        );
        for &source_id in &pending_batch {
            let source_index = source_id.index();
            let canonicalized = fs::canonicalize(&self.records[source_index].canonical_source_path);
            match canonicalized {
                Ok(canonical_path) => match self.by_canonical_path.get(&canonical_path).copied() {
                    Some(existing_id) => {
                        self.redirect_to_existing_owner(source_id, existing_id);
                    }
                    None => {
                        // No physical owner exists for this canonical spelling: this row becomes
                        // the one read owner. Update its single path and add the exact canonical
                        // spelling.
                        self.records[source_index].canonical_source_path = canonical_path.clone();
                        self.by_canonical_path.insert(canonical_path, source_id);
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // The target vanished before the first metadata resolution. Metadata alone may
                    // still prove the intended canonical spelling; use it only to join a key
                    // proven canonical by Stage 0 or live settlement. A key owned by any pending
                    // row in this batch is still provisional (including an earlier failed key), so
                    // matching it could create a second redirect hop once that row resolves.
                    let Some(candidate_path) = resolve_missing_source_spelling(
                        &self.records[source_index].canonical_source_path,
                    ) else {
                        continue;
                    };
                    let Some(existing_id) = self.by_canonical_path.get(&candidate_path).copied()
                    else {
                        continue;
                    };
                    if pending_batch
                        .binary_search_by_key(&existing_id.index(), |id| id.index())
                        .is_ok()
                    {
                        continue;
                    }
                    self.redirect_to_existing_owner(source_id, existing_id);
                }
                Err(_) => {
                    // The supplied spelling remains the byte source and watch key. The first read
                    // reports the same failure as before, now with shipped provenance attached.
                }
            }
        }
    }

    /// Redirect one logical registration to the physical owner, merging positive provenance.
    fn redirect_to_existing_owner(
        &mut self,
        source_id: ResourceSourceId,
        existing_id: ResourceSourceId,
    ) {
        let owner = self.physical_owner(existing_id);
        if owner == source_id {
            return;
        }
        let package_set_membership = self.records[source_id.index()].package_set_membership;
        self.redirects.insert(source_id, owner);
        self.merge_membership_into(owner, package_set_membership);
    }

    /// Preflight a complete compiler-produced origin/source association batch.
    ///
    /// Every source ID, existing attachment and pair within the batch is checked before the
    /// returned receipt can be reserved or committed. No registry state is mutated on failure.
    pub(crate) fn preflight_resource_source_associations(
        &self,
        associations: &[ResourceSourceAssociation],
    ) -> Result<ResourceSourcePublication, InfrastructureFailure> {
        let mut pending_by_origin: FxHashMap<&StableResourceOriginId, ResourceSourceId> =
            FxHashMap::default();
        let mut new_associations = Vec::new();

        for association in associations {
            let source_id = association.source;
            let Some(record) = self.records.get(source_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource origin attachment references unknown source ID {}",
                    source_id.index()
                )));
            };
            if record.source_id != source_id {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource origin attachment source ID {} disagrees with its record ID {}",
                    source_id.index(),
                    record.source_id.index()
                )));
            }

            if let Some(existing_source_id) = self.source_by_origin.get(&association.origin) {
                if *existing_source_id != source_id {
                    return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
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
                    return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
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
    /// Test-only: production emission reads bytes through [`Self::read_source`]. Both share one
    /// owner-resolution and hashing helper, so this does not duplicate production lookups.
    #[cfg(test)]
    pub(crate) fn hash_source(
        &mut self,
        source_id: ResourceSourceId,
    ) -> Result<u64, InfrastructureFailure> {
        self.hash_owner_source(source_id)
            .map(|(_owner, content_hash)| content_hash)
    }

    /// Resolve one logical ID to its physical owner and hash that owner's bytes.
    ///
    /// This is the first content-read boundary: every pending shipped alias is settled before any
    /// bytes are read, then the logical ID redirects to the canonical read record.
    fn hash_owner_source(
        &mut self,
        source_id: ResourceSourceId,
    ) -> Result<(ResourceSourceId, u64), InfrastructureFailure> {
        self.settle_deferred_sources();
        let owner = self.physical_owner(source_id);
        let content_hash = self.hash_physical_source(owner)?;
        Ok((owner, content_hash))
    }

    /// Hash the record that owns one physical byte source.
    fn hash_physical_source(
        &mut self,
        source_id: ResourceSourceId,
    ) -> Result<u64, InfrastructureFailure> {
        let source_index = source_id.index();
        let (content_state, source_path, package_set_membership) = self
            .records
            .get(source_index)
            .map(|record| {
                (
                    record.content,
                    &record.canonical_source_path,
                    record.package_set_membership,
                )
            })
            .ok_or_else(|| {
                InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource source ID {} is outside the source registry",
                    source_id.index()
                ))
            })?;

        match content_state {
            ResourceContentState::Unhashed => {
                // The filesystem read completes before the record below is borrowed mutably to
                // store the cached result: the path is borrowed, never copied.
                let bytes = fs::read(source_path).map_err(|error| {
                    let mut reason = format!(
                        "Failed to read resource source '{}': {error}",
                        portable_path_text(source_path)
                    );
                    // Only a registration the compiler ships as part of its package set makes a
                    // failed read evidence of a damaged installation. The physical path and
                    // registration order never decide that lane.
                    if let Some(report) = damaged_package_set_report(package_set_membership) {
                        reason.push_str(". ");
                        reason.push_str(&report);
                    }
                    InfrastructureFailure::filesystem(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, source_path, reason)
                })?;
                // An invalidation fact only. Resource identity and output placement stay keyed
                // by stable semantic origins and planned paths.
                let content_hash = stable_content_hash(&bytes);
                let record = self.records.get_mut(source_index).ok_or_else(|| {
                    InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
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
    /// same cached bytes, allowing several planned origins to emit from one physical source even
    /// when they registered different alias spellings.
    pub(crate) fn read_source(
        &mut self,
        source_id: ResourceSourceId,
    ) -> Result<&[u8], InfrastructureFailure> {
        let (owner, content_hash) = self.hash_owner_source(source_id)?;
        let record = self.records.get_mut(owner.index()).ok_or_else(|| {
            InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "resource source ID {} disappeared while reading",
                    owner.index()
                ),
            )
        })?;

        if matches!(record.content, ResourceContentState::Hashed { .. }) {
            record.content = ResourceContentState::Read { content_hash };
        }

        record.bytes.as_deref().ok_or_else(|| {
            InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                format!(
                    "resource source ID {} has no cached bytes after hashing",
                    owner.index()
                ),
            )
        })
    }

    /// Prove the registry's dense IDs and canonical-path index still agree at a boundary handoff.
    pub(crate) fn validate(&self) -> Result<(), InfrastructureFailure> {
        for (index, record) in self.records().iter().enumerate() {
            if record.source_id.index() != index {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource source ID {} is stored at record index {index}",
                    record.source_id.index()
                )));
            }
            if self.by_canonical_path.get(&record.canonical_source_path) != Some(&record.source_id)
            {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource source {:?} is absent from its canonical-path index",
                    record.canonical_source_path
                )));
            }
        }

        for (alias_id, owner_id) in &self.redirects {
            if *alias_id == *owner_id {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource source ID {} redirects to itself",
                    alias_id.index()
                )));
            }
            if self.redirects.contains_key(owner_id) {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource source ID {} redirects to another redirected source ID {}",
                    alias_id.index(),
                    owner_id.index()
                )));
            }
            let Some(alias) = self.records.get(alias_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource redirect source ID {} is outside the source registry",
                    alias_id.index()
                )));
            };
            let Some(owner) = self.records.get(owner_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource redirect target ID {} is outside the source registry",
                    owner_id.index()
                )));
            };
            if alias.source_id != *alias_id || owner.source_id != *owner_id {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource redirect IDs {} -> {} disagree with their record IDs",
                    alias_id.index(),
                    owner_id.index()
                )));
            }
            if alias.content != ResourceContentState::Unhashed || alias.bytes.is_some() {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "redirected resource source ID {} owns content state instead of its owner",
                    alias_id.index()
                )));
            }
            if owner
                .package_set_membership
                .merge(alias.package_set_membership)
                != owner.package_set_membership
            {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource redirect target ID {} did not inherit source ID {} membership",
                    owner_id.index(),
                    alias_id.index()
                )));
            }
        }

        for pending_id in &self.pending_ids {
            let Some(record) = self.records.get(pending_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "pending resource source ID {} is outside the source registry",
                    pending_id.index()
                )));
            };
            if record.source_id != *pending_id
                || !record.package_set_membership.is_compiler_shipped()
                || self.redirects.contains_key(pending_id)
            {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "pending resource source ID {} is not an unresolved shipped registration",
                    pending_id.index()
                )));
            }
        }

        for source_id in self.by_canonical_path.values() {
            let Some(record) = self.records.get(source_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource path points outside the source registry at ID {}",
                    source_id.index()
                )));
            };
            if record.source_id != *source_id {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource path points to source ID {}, but that record owns ID {}",
                    source_id.index(),
                    record.source_id.index()
                )));
            }
        }

        for (origin, source_id) in &self.source_by_origin {
            let Some(record) = self.records.get(source_id.index()) else {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "resource origin {origin:?} points outside the source registry at ID {}",
                    source_id.index()
                )));
            };
            if record.source_id != *source_id {
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
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
                return Err(InfrastructureFailure::internal(crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly, format!(
                    "missing resource watch {:?} overlaps an existing source",
                    interest.canonical_path
                )));
            }
        }

        Ok(())
    }
}

/// Best-effort canonical spelling for a path whose target is missing.
///
/// A dangling leaf symlink still records its intended target; a regular missing leaf may sit under
/// a symlinked directory. Both cases use only metadata and a single link read. The result is used
/// solely to find an already registered canonical key, never to rewrite the missing registration or
/// classify membership. Multi-hop symlink chains and targets whose parents are also missing are
/// intentionally not reconstructed.
fn resolve_missing_source_spelling(registered_path: &Path) -> Option<PathBuf> {
    let parent = registered_path.parent()?;
    let canonical_parent = fs::canonicalize(parent).ok()?;

    if let Ok(target) = fs::read_link(registered_path) {
        let target_path = if target.is_absolute() {
            target
        } else {
            canonical_parent.join(target)
        };
        let target_parent = target_path.parent()?;
        let canonical_target_parent = fs::canonicalize(target_parent).ok()?;
        return Some(canonical_target_parent.join(target_path.file_name()?));
    }

    let file_name = registered_path.file_name()?;
    Some(canonical_parent.join(file_name))
}

#[cfg(test)]
#[path = "../tests/resource_inputs_tests.rs"]
mod tests;
