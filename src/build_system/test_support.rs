//! Test-only helpers that assemble real graph boundaries around bare modules.
//!
//! WHAT: wraps one flat `Vec<Module>` into a single-project [`ProjectCompilation`] exactly as
//!       the moved `from_test_modules` constructor did, so unit tests can exercise entry
//!       assembly, linking and backend behaviour without running the full frontend pipeline.
//! WHY: production `build.rs` must not carry a flat-module test-construction shape; test
//!       support owns the synthetic origin and empty public interface instead.
//! MUST NOT: be used outside `#[cfg(test)]` code or claim to be a production construction path.

use crate::build_system::build::{CompiledModuleArtifact, Module, ProjectCompilation};
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, CompletedSourcePackageRegistry,
};
use crate::build_system::create_project_modules::generated_worklist::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use std::path::PathBuf;

/// Assemble one success-only project compilation from bare test modules.
pub(crate) fn project_compilation_from_test_modules(
    modules: Vec<Module>,
) -> Result<ProjectCompilation, CompilerError> {
    let module_count = modules.len();
    let graph = ProjectModuleGraph::from_normal_roots(
        (0..module_count)
            .map(|index| {
                let origin = StableModuleOriginIdentity::from_portable_path(
                    StablePackageIdentity::project_local("test"),
                    format!("module_{index}"),
                    ModuleRootRole::Normal,
                );
                let root_path = PathBuf::from(format!("@module_{index}.moth"));
                (origin, root_path.clone(), root_path)
            })
            .collect(),
    );
    let mut module_store = ModuleArtifactStore::new(module_count);
    for (index, module) in modules.into_iter().enumerate() {
        let module_id = ModuleId::from_index(index);
        module_store.publish_success(
            module_id,
            CompiledModuleArtifact {
                module,
                interface: test_public_interface(index),
            },
        )?;
    }
    let project = CompiledGraphBoundary {
        structure: graph,
        modules: module_store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    ProjectCompilation::from_successful_boundaries(project, CompletedSourcePackageRegistry::new())
}

/// Build an immutable `PublicSemanticInterface` for one test-constructed artefact.
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
