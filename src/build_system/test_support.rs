//! Test-only helpers that assemble real graph boundaries around bare modules.
//!
//! WHAT: wraps one flat `Vec<Module>` into a single-project [`ProjectCompilation`] so unit tests
//!       can exercise entry assembly, linking and backend behaviour without running the full
//!       frontend pipeline.
//! WHY: production `build.rs` must not carry a flat-module test-construction shape; test
//!       support owns the synthetic origin and test facade interface instead.
//! MUST NOT: be used outside `#[cfg(test)]` code or claim to be a production construction path.

use crate::build_system::build::{ProjectAssemblyError, ProjectCompilation};
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, CompletedSourcePackageRegistry,
};
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_frontend::module_compilation::{CompiledModuleArtifact, Module};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use std::path::PathBuf;

/// Assemble one success-only project compilation from bare test modules.
pub(crate) fn project_compilation_from_test_modules(
    modules: Vec<Module>,
) -> Result<ProjectCompilation, ProjectAssemblyError> {
    project_compilation_from_test_modules_with_resources(modules, ResourceInputRegistry::new())
}

/// Assemble test modules with an explicit Stage 0 resource registry.
pub(crate) fn project_compilation_from_test_modules_with_resources(
    modules: Vec<Module>,
    resource_inputs: ResourceInputRegistry,
) -> Result<ProjectCompilation, ProjectAssemblyError> {
    let module_count = modules.len();
    // Bare test modules with entry metadata are real entries; modules without it model API-only
    // facades so publication keeps one interface shape per graph role.
    let roles: Vec<ModuleRootRole> = modules
        .iter()
        .map(|module| {
            if module.metadata.entry.is_some() {
                ModuleRootRole::Entry
            } else {
                ModuleRootRole::Normal
            }
        })
        .collect();
    let graph = ProjectModuleGraph::from_standalone_roots(
        roles
            .iter()
            .enumerate()
            .map(|(index, role)| {
                let origin = StableModuleOriginIdentity::from_portable_path(
                    StablePackageIdentity::project_local("test"),
                    format!("module_{index}"),
                    *role,
                );
                (origin, PathBuf::from(format!("module_{index}.moth")))
            })
            .collect(),
    );
    let mut module_store = ModuleArtifactStore::new(module_count);
    for (index, module) in modules.into_iter().enumerate() {
        let module_id = ModuleId::from_index(index);
        let normal_interface =
            (roles[index] == ModuleRootRole::Normal).then(|| test_public_interface(index));
        module_store.publish_success(
            module_id,
            CompiledModuleArtifact {
                module,
                normal_interface,
                support_interface: None,
            },
        )?;
    }
    let project = CompiledGraphBoundary {
        structure: graph,
        modules: module_store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
        scheduling_cutoff: None,
    };
    ProjectCompilation::from_successful_boundaries(
        project,
        CompletedSourcePackageRegistry::new(),
        resource_inputs,
    )
}

/// Build an immutable `PublicSemanticInterface` for one API-only test artefact.
///
/// The origin path is unique per module so entry assembly and interface lookup behave like real
/// artefacts; production publication always supplies the completed interface.
fn test_public_interface(module_index: usize) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            format!("module_{module_index}"),
            ModuleRootRole::Normal,
        ),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}
