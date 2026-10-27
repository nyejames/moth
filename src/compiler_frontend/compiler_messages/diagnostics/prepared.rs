//! Captured diagnostics at the owning-stage boundary.
//!
//! WHAT: pairs one compact draft with its type-display snapshot and pending identity/path owners.
//! WHY: the boundary accumulator can merge prepared drafts without retaining the live type environment.

use super::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayCapacityError, type_display_snapshot::TypeDisplaySnapshot,
    type_display_snapshot_builder::TypeDisplaySnapshotBuilder,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticCapacityResource, DiagnosticSeverity};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

/// One family draft and its captured type snapshot while a boundary run is accumulated.
pub(crate) struct PreparedDiagnostic {
    pub(super) draft: DiagnosticDraft,
    pub(super) types: TypeDisplaySnapshot,
    /// The first source row that covers the diagnostic.
    pub(super) owner: Option<usize>,
    /// The first explicit path row that covers the diagnostic.
    pub(super) explicit_path: Option<usize>,
    /// The first identity-less source row that covers the diagnostic.
    pub(super) identityless_owner: Option<usize>,
    /// The path-table merge whose remap the draft and snapshot follow.
    pub(super) merge: Option<usize>,
}

impl PreparedDiagnostic {
    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        self.draft.severity()
    }

    pub(crate) fn append_type_paths(
        &self,
        paths: &mut Vec<crate::compiler_frontend::symbols::path_interner::PathId>,
    ) {
        self.types.required_paths(paths);
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.draft.remap_string_ids(remap);
        self.types.remap_string_ids(remap);
    }

    pub(crate) fn rebind_source_ids(
        &mut self,
        rebind: &mut impl FnMut(SourceId) -> Result<SourceId, InfrastructureFailure>,
    ) -> Result<(), InfrastructureFailure> {
        self.draft.rebind_source_ids(rebind)
    }

    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn label_count(&self) -> usize {
        self.draft.label_count()
    }

    pub(crate) fn requires_source_identity(&self) -> bool {
        self.draft.primary_span().is_some()
            || (0..self.draft.label_count()).any(|index| self.draft.label_span(index).is_some())
    }

    pub(crate) fn attach_frozen_identity_handle_if_missing(
        &mut self,
        handle: FrozenIdentityHandle,
    ) {
        self.draft.attach_frozen_identity_handle_if_missing(handle);
    }

    pub(crate) fn set_explicit_path(&mut self, explicit_path: Option<usize>) {
        self.explicit_path = explicit_path;
    }
}

/// Captures pending type words while the owning stage still has its type environment.
pub(crate) fn prepare_draft(
    mut draft: DiagnosticDraft,
    environment: Option<&TypeEnvironment>,
) -> PreparedDiagnostic {
    let mut builder = TypeDisplaySnapshotBuilder::new(environment);
    let types = match draft.capture_types(&mut |type_id| builder.display_top_level(type_id)) {
        Ok(()) => builder.finish(),
        Err(TypeDisplayCapacityError) => {
            draft = draft.faulted_by(DiagnosticCapacityResource::TypeDisplays);
            TypeDisplaySnapshot::default()
        }
    };

    PreparedDiagnostic {
        draft,
        types,
        owner: None,
        explicit_path: None,
        identityless_owner: None,
        merge: None,
    }
}
