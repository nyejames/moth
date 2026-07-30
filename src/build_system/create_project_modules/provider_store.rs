//! Completed canonical module artefact storage and provider publication state.
//!
//! WHAT: stores successful immutable artefacts contiguously and maps every project `ModuleId` to
//! an unavailable, successful, diagnosed or blocked slot.
//! WHY: dependency waves need one build-owned authority that can publish only complete semantic
//! interfaces. Consumers borrow successful artefacts by dense ID instead of cloning interfaces or
//! opening provider syntax.
//! MUST NOT: store `ModuleSemanticDraft`, resolve source names or perform semantic compilation.

use super::module_identity::ModuleId;
use crate::build_system::build::CompiledModuleArtifact;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompiledModuleArtifactId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderSlot {
    Unavailable,
    Successful(CompiledModuleArtifactId),
    Diagnosed,
    Blocked,
}

pub(crate) struct ModuleProviderStore {
    slots: Vec<ProviderSlot>,
    artifacts: Vec<CompiledModuleArtifact>,
}

impl ModuleProviderStore {
    pub(crate) fn new(module_count: usize) -> Self {
        Self {
            slots: vec![ProviderSlot::Unavailable; module_count],
            artifacts: Vec::with_capacity(module_count),
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

    pub(crate) fn interface(
        &self,
        module_id: ModuleId,
    ) -> Result<Option<&PublicSemanticInterface>, CompilerError> {
        let ProviderSlot::Successful(artifact_id) = self.slot(module_id)? else {
            return Ok(None);
        };

        let artifact = self.artifacts.get(artifact_id.0).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Provider slot for ModuleId {} references missing artifact {}",
                module_id.index(),
                artifact_id.0
            ))
        })?;
        Ok(Some(&artifact.interface))
    }

    pub(crate) fn into_artifacts(self) -> Vec<CompiledModuleArtifact> {
        self.artifacts
    }

    pub(crate) fn materialisation_contexts(
        &self,
    ) -> impl Iterator<
        Item = &crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext,
    > {
        self.artifacts
            .iter()
            .filter_map(|artifact| artifact.module.metadata.materialisation_context.as_ref())
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
