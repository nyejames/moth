//! Completed canonical module artefact storage and per-module outcome state.
//!
//! WHAT: stores successful immutable artefacts contiguously and retains the dense `ModuleId ->
//! CompiledModuleArtifactId` slot mapping plus diagnosed/blocked outcomes. It is the retained
//! artefact lane of one [`CompiledGraphBoundary`](super::compiled_boundary::CompiledGraphBoundary).
//! WHY: dependency waves need one build-owned authority that can publish only complete semantic
//! interfaces, and later entry selection and warning collection must resolve by dense `ModuleId`
//! without rebuilding an index or relying on vector layout.
//! MUST NOT: store `ModuleSemanticDraft`, resolve source names or perform semantic compilation.

use super::module_identity::ModuleId;
use crate::build_system::build::CompiledModuleArtifact;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use rustc_hash::FxHashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompiledModuleArtifactId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderSlot {
    Unavailable,
    Successful(CompiledModuleArtifactId),
    Diagnosed,
    Blocked,
}

pub(crate) struct ModuleArtifactStore {
    slots: Vec<ProviderSlot>,
    artifacts: Vec<CompiledModuleArtifact>,
    /// Boundary-owned index of every published materialisation context by its generic
    /// declaration identity.
    ///
    /// WHAT: lets generated materialisation resolve the declaring context by identity without
    ///       scanning all successful artefacts, and validates duplicate contexts at
    ///       publication before any module requests them.
    contexts_by_declaration: FxHashMap<GeneratedDeclarationIdentity, CompiledModuleArtifactId>,
}

impl ModuleArtifactStore {
    pub(crate) fn new(module_count: usize) -> Self {
        Self {
            slots: vec![ProviderSlot::Unavailable; module_count],
            artifacts: Vec::with_capacity(module_count),
            contexts_by_declaration: FxHashMap::default(),
        }
    }

    pub(crate) fn publish_success(
        &mut self,
        module_id: ModuleId,
        artifact: CompiledModuleArtifact,
    ) -> Result<(), CompilerError> {
        if self.slot(module_id)? != ProviderSlot::Unavailable {
            return Err(CompilerError::compiler_error(format!(
                "Provider slot for ModuleId {} was published more than once",
                module_id.index()
            )));
        }

        let artifact_id = CompiledModuleArtifactId(self.artifacts.len());
        if let Some(context) = artifact.module.metadata.materialisation_context.as_ref() {
            for identity in context.declaration_identities() {
                if self.contexts_by_declaration.contains_key(identity) {
                    return Err(CompilerError::compiler_error(format!(
                        "Generated declaration identity {:?} was published by more than one materialisation context (module slot {} and {})",
                        identity, self.contexts_by_declaration[identity].0, artifact_id.0
                    )));
                }
            }
            for identity in context.declaration_identities() {
                self.contexts_by_declaration
                    .insert(identity.clone(), artifact_id);
            }
        }
        self.artifacts.push(artifact);
        *self.slot_mut(module_id)? = ProviderSlot::Successful(artifact_id);
        Ok(())
    }

    pub(crate) fn mark_diagnosed(&mut self, module_id: ModuleId) -> Result<(), CompilerError> {
        self.transition_unavailable(module_id, ProviderSlot::Diagnosed)
    }

    pub(crate) fn mark_blocked(&mut self, module_id: ModuleId) -> Result<(), CompilerError> {
        self.transition_unavailable(module_id, ProviderSlot::Blocked)
    }

    pub(crate) fn slot(&self, module_id: ModuleId) -> Result<ProviderSlot, CompilerError> {
        self.slots.get(module_id.index()).copied().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Provider store received out-of-range ModuleId {} for {} slots",
                module_id.index(),
                self.slots.len()
            ))
        })
    }

    /// Require every dense slot to have reached a final outcome.
    ///
    /// A remaining `Unavailable` slot means a graph/job mismatch or scheduler regression; the
    /// boundary must never publish an incomplete module result.
    pub(crate) fn ensure_all_slots_completed(&self) -> Result<(), CompilerError> {
        if let Some((index, _)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| **slot == ProviderSlot::Unavailable)
        {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {index} never reached a completed outcome"
            )));
        }
        Ok(())
    }

    /// Require every dense slot to hold a successful artefact.
    ///
    /// This is the success-only gate used when assembling a linkable `ProjectCompilation`.
    pub(crate) fn ensure_all_successful(&self) -> Result<(), CompilerError> {
        if let Some((index, slot)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| !matches!(slot, ProviderSlot::Successful(_)))
        {
            return Err(CompilerError::compiler_error(format!(
                "ModuleId {index} is not successful ({slot:?}) but ProjectCompilation requires every module to succeed"
            )));
        }
        Ok(())
    }

    pub(crate) fn interface(
        &self,
        module_id: ModuleId,
    ) -> Result<Option<&PublicSemanticInterface>, CompilerError> {
        Ok(self
            .artifact(module_id)?
            .map(|artifact| &artifact.interface))
    }

    /// Resolve the successful artefact for one dense module identity.
    ///
    /// Returns `Ok(None)` for a diagnosed, blocked or unavailable slot. The mapping is retained
    /// from publication time, so lookup never depends on artefact vector order.
    pub(crate) fn artifact(
        &self,
        module_id: ModuleId,
    ) -> Result<Option<&CompiledModuleArtifact>, CompilerError> {
        let ProviderSlot::Successful(artifact_id) = self.slot(module_id)? else {
            return Ok(None);
        };

        let artifact = self.artifacts.get(artifact_id.0).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "ModuleId {} references missing artifact {}",
                module_id.index(),
                artifact_id.0
            ))
        })?;
        Ok(Some(artifact))
    }

    /// Iterate successful artefacts in deterministic `ModuleId` order.
    ///
    /// Publication follows dependency waves, which may differ from `ModuleId` order, so this
    /// iteration walks the retained slot mapping rather than the artefact vector.
    pub(crate) fn successful_artefacts_in_module_id_order(
        &self,
    ) -> impl Iterator<Item = &CompiledModuleArtifact> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            ProviderSlot::Successful(artifact_id) => self.artifacts.get(artifact_id.0),
            ProviderSlot::Unavailable | ProviderSlot::Diagnosed | ProviderSlot::Blocked => None,
        })
    }

    /// Resolve one published materialisation context by its generic declaration identity.
    pub(crate) fn materialisation_context_for(
        &self,
        identity: &GeneratedDeclarationIdentity,
    ) -> Result<
        Option<&crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext>,
        CompilerError,
    > {
        let Some(artifact_id) = self.contexts_by_declaration.get(identity) else {
            return Ok(None);
        };
        let artifact = self.artifacts.get(artifact_id.0).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Materialisation context index references missing artifact {}",
                artifact_id.0
            ))
        })?;
        Ok(artifact.module.metadata.materialisation_context.as_ref())
    }

    fn transition_unavailable(
        &mut self,
        module_id: ModuleId,
        next: ProviderSlot,
    ) -> Result<(), CompilerError> {
        let slot = self.slot_mut(module_id)?;
        if *slot != ProviderSlot::Unavailable {
            return Err(CompilerError::compiler_error(format!(
                "Provider slot for ModuleId {} was completed more than once",
                module_id.index()
            )));
        }
        *slot = next;
        Ok(())
    }

    fn slot_mut(&mut self, module_id: ModuleId) -> Result<&mut ProviderSlot, CompilerError> {
        let slot_count = self.slots.len();
        self.slots.get_mut(module_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Provider store received out-of-range ModuleId {} for {} slots",
                module_id.index(),
                slot_count
            ))
        })
    }
}
