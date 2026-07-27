//! Project-boundary semantic source set for one normal entry module.
//!
//! WHAT: owns the deterministic set of boundary-local project `SourceId`s that are the
//! compiler-semantic members of one entry module's project compilation. The set is built from
//! the structural references already retained by the live reachable traversal and resolved
//! through the [`ModuleNamespaceSet`]; it never rescans or re-resolves through paths.
//! WHY: replaces ad hoc path-based reachable-file membership with authoritative `SourceId`
//! membership. Cross-module provider roots, source-package facades and binding-package inputs
//! remain explicit interface/external inputs for the donor compiler and never enter the set.
//! Assembly projects canonical IO/cache handles from the retained index only after membership is
//! complete; `SourceId` is the stored identity and deterministic sort key.

use super::module_identity::ModuleId;
use super::source_tree_index::{SourceClassification, SourceId, SourceOwnership, SourceTreeIndex};
use crate::compiler_frontend::compiler_errors::CompilerError;

/// One deterministic project-boundary semantic source set for a normal entry module.
///
/// WHAT: the authoritative set of same-owner compiler-semantic `SourceId`s for one entry
/// module. Members are sorted by `SourceId` (portable logical identity order) so the set is
/// deterministic and independent of traversal order, file-creation order and checkout root.
/// WHY: consumed by `PreparedSourceInput` assembly as the membership authority that selects and
/// orders project semantic inputs from the retained scan cache. Cross-module provider roots,
/// source-package facades, binding-package and provider-owned inputs never enter the set.
pub(super) struct SemanticSourceSet {
    entry_module_id: ModuleId,
    source_ids: Vec<SourceId>,
}

impl SemanticSourceSet {
    /// Create a set seeded with the entry module's root source.
    ///
    /// `entry_source_id` is the `SourceId` of the entry module's root file, resolved from the
    /// project `SourceTreeIndex`. The seed's ownership is verified against `entry_module_id`
    /// so an impossible index/namespace mismatch is surfaced as an internal `CompilerError`
    /// rather than silently accepted.
    pub(super) fn from_entry_source(
        entry_source_id: SourceId,
        entry_module_id: ModuleId,
        source_tree_index: &SourceTreeIndex,
    ) -> Result<Self, CompilerError> {
        let record = source_tree_index.source(entry_source_id);
        verify_same_owner(entry_source_id, record.ownership(), entry_module_id)?;

        verify_compiler_semantic(entry_source_id, record.classification())?;

        Ok(Self {
            entry_module_id,
            source_ids: vec![entry_source_id],
        })
    }

    /// The `ModuleId` of the entry module this set belongs to.
    pub(super) fn entry_module_id(&self) -> ModuleId {
        self.entry_module_id
    }

    /// Add one same-owner compiler-semantic source resolved through the project namespace.
    ///
    /// `source_id` must be owned by the entry module. The ownership check is deferred to
    /// [`finish`](Self::finish) so the builder can accumulate during the traversal without a
    /// per-insertion index lookup; the final sort/dedup pass verifies every member.
    pub(super) fn add_same_owner_source(&mut self, source_id: SourceId) {
        self.source_ids.push(source_id);
    }

    /// Finalize the set: sort by `SourceId`, deduplicate and verify every member is owned by the
    /// entry module.
    ///
    /// Returns an internal `CompilerError` if any member's ownership does not match the entry
    /// module, which would indicate an impossible project source membership/owner mismatch.
    pub(super) fn finish(
        mut self,
        source_tree_index: &SourceTreeIndex,
    ) -> Result<Self, CompilerError> {
        self.source_ids.sort_unstable();
        self.source_ids.dedup();

        for source_id in &self.source_ids {
            let record = source_tree_index.source(*source_id);
            verify_same_owner(*source_id, record.ownership(), self.entry_module_id)?;
            verify_compiler_semantic(*source_id, record.classification())?;
        }

        Ok(self)
    }

    /// The finalized members in deterministic `SourceId` order.
    pub(super) fn members(&self) -> &[SourceId] {
        &self.source_ids
    }

    /// The `SourceId`s of all semantic members in deterministic order, for focused tests.
    #[cfg(test)]
    pub(super) fn source_ids(&self) -> Vec<SourceId> {
        self.source_ids.clone()
    }
}

/// Verify that `source_id` is owned by `expected_module_id`.
fn verify_same_owner(
    source_id: SourceId,
    ownership: SourceOwnership,
    expected_module_id: ModuleId,
) -> Result<(), CompilerError> {
    match ownership {
        SourceOwnership::Owned(owner_module_id) if owner_module_id == expected_module_id => Ok(()),
        _ => Err(CompilerError::compiler_error(format!(
            "Project semantic source set received source ID {} with ownership {:?} that does not \
             match the entry module ID {}; a project semantic member must be owned by the entry \
             module",
            source_id.index(),
            ownership,
            expected_module_id.index(),
        ))),
    }
}

fn verify_compiler_semantic(
    source_id: SourceId,
    classification: &SourceClassification,
) -> Result<(), CompilerError> {
    if matches!(classification, SourceClassification::CompilerSemantic(_)) {
        return Ok(());
    }

    Err(CompilerError::compiler_error(format!(
        "Project semantic source set received provider-owned source ID {}; every semantic member must be compiler semantic",
        source_id.index(),
    )))
}
