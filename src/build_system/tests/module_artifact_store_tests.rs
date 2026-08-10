//! Tests for module-artifact retention and boundary bijection invariants.
//!
//! WHAT: verifies single-module preflight rejection and retained artefact-row consistency.
//! WHY: these tests own the module store's local lanes; combined module/generated transactions
//!       are covered by `compilation_tests` instead.

use super::{CompiledModuleArtifactId, ModuleArtifactStore, ModuleId, ProviderSlot};
use crate::build_system::build::{
    CompiledModuleArtifact, Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
    ModuleRootActivity,
};
use crate::build_system::create_project_modules::compiled_boundary::CompiledGraphBoundary;
use crate::build_system::create_project_modules::generated_worklist::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use std::path::PathBuf;
use std::sync::Arc;

fn duplicate_materialisation_identity() -> GeneratedDeclarationIdentity {
    GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("module-artifact-tests"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        ),
        "@page.moth".to_owned(),
        ModulePrivateExecutableCategory::GenericFunction,
        "duplicate".to_owned(),
        None,
    ))
}

fn artifact_with_context(context: ModuleMaterialisationContext) -> CompiledModuleArtifact {
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("module-artifact-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    CompiledModuleArtifact {
        module: Module {
            executable: ModuleExecutable {
                hir: HirModule::new(),
                type_environment: TypeEnvironment::new(),
                borrow_analysis: BorrowCheckReport::default(),
            },
            link_facts: ModuleLinkFacts {
                external_package_registry: Arc::new(ExternalPackageRegistry::new()),
                external_import_candidates: Vec::new(),
                functions: HirModuleLinkFacts::default(),
            },
            metadata: ModuleCompilerMetadata {
                entry_point: PathBuf::new(),
                warnings: Vec::new(),
                const_top_level_fragments: Vec::new(),
                root_activity: ModuleRootActivity::default(),
                doc_fragments: Vec::new(),
                rendered_path_usages: Vec::new(),
                materialisation_context: Some(context),
            },
        },
        interface: PublicSemanticInterface {
            module_origin: origin,
            export_bindings: Vec::new(),
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        },
    }
}

#[test]
fn boundary_validation_rejects_missing_successful_artefact_row() {
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            "missing".to_owned(),
            ModuleRootRole::Normal,
        ),
        PathBuf::from("@missing.moth"),
        PathBuf::from("@missing.moth"),
    )]);
    let store = ModuleArtifactStore {
        slots: vec![ProviderSlot::Successful(CompiledModuleArtifactId(0))],
        artifacts: Vec::new(),
        contexts_by_declaration: rustc_hash::FxHashMap::default(),
        materialisation_rows: Vec::new(),
    };
    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };

    let error = boundary
        .validate_invariants()
        .expect_err("a successful slot must reference an existing artefact row");
    assert!(
        error.msg.contains("references missing artifact 0"),
        "unexpected missing-artefact error: {error:?}"
    );
}

#[test]
fn module_success_preflight_rejects_duplicate_rows_without_mutation() {
    let identity = duplicate_materialisation_identity();
    let artifact = artifact_with_context(ModuleMaterialisationContext::from_identities_for_test(
        vec![identity.clone(), identity],
    ));
    let store = ModuleArtifactStore::new(1);

    let error = store
        .preflight_success(
            ModuleId::from_index(0),
            &artifact,
            &artifact.interface.module_origin,
        )
        .expect_err("duplicate materialisation rows must fail during preflight");

    assert!(
        error
            .msg
            .contains("duplicated inside one materialisation context")
    );
    assert_eq!(store.artifact_count(), 0);
    assert_eq!(
        store.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(store.materialisation_locations().count(), 0);
}
