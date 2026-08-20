//! Tests for atomic publication of a module artefact and its generated delta.
//!
//! WHAT: protects the combined build-boundary transaction that commits both retained lanes.
//! WHY: a valid module or generated delta must not become visible when its companion lane fails
//!       preflight. Single-store publication invariants belong to `module_artifact_store_tests`.

use super::publish_module_and_generated;
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::{
    ModuleArtifactStore, ProviderSlot,
};
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
};
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, CompletedGeneratedFunction, GeneratedFunctionDelta,
    GeneratedFunctionSidecar, Module, ModuleRootActivity, ProviderMaterialisationRegistry,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use std::path::PathBuf;
use std::sync::Arc;

fn duplicate_materialisation_identity() -> GeneratedDeclarationIdentity {
    GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("combined-publication-tests"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        ),
        "@page.moth".to_owned(),
        ModulePrivateExecutableCategory::GenericFunction,
        "duplicate".to_owned(),
        None,
    ))
}

fn invalid_artifact() -> CompiledModuleArtifact {
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    let identity = duplicate_materialisation_identity();
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
                materialisation_context: Some(Arc::new(
                    ModuleMaterialisationContext::from_identities_for_test(vec![
                        identity.clone(),
                        identity,
                    ]),
                )),
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

fn artifact_without_context() -> CompiledModuleArtifact {
    let mut artifact = invalid_artifact();
    artifact.module.metadata.materialisation_context = None;
    artifact
}

fn generated_identity(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("combined-publication-tests"),
                "main".to_owned(),
                ModuleRootRole::Normal,
            ),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    )
}

fn generated_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn generated_sidecar(
    identity: GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> GeneratedFunctionSidecar {
    let mut module = Module {
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
            materialisation_context: None,
        },
    };
    module.executable.hir.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: Vec::new(),
        return_type: crate::compiler_frontend::datatypes::ids::TypeId(0),
    });
    module
        .executable
        .hir
        .function_ids_by_generated
        .insert(identity.clone(), FunctionId(0));
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .insert(FunctionId(0), summary);
    GeneratedFunctionSidecar::new(identity, module)
}

fn generated_delta_with_valid_record() -> GeneratedFunctionDelta {
    let identity = generated_identity("valid");
    let summary = generated_summary();
    GeneratedFunctionDelta::from_records(vec![CompletedGeneratedFunction {
        identity: identity.clone(),
        summary: summary.clone(),
        sidecar: generated_sidecar(identity, summary),
    }])
}

fn generated_delta_with_identity_mismatch() -> GeneratedFunctionDelta {
    let summary = generated_summary();
    GeneratedFunctionDelta::from_records(vec![CompletedGeneratedFunction {
        identity: generated_identity("record"),
        summary,
        sidecar: generated_sidecar(generated_identity("sidecar"), generated_summary()),
    }])
}

#[test]
fn combined_publication_preflights_both_lanes_before_mutation() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let generated_delta = GeneratedFunctionDelta::from_records(Vec::new());
    let artifact = invalid_artifact();
    let expected_origin = artifact.interface.module_origin.clone();

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta,
    )
    .expect_err("an invalid module publication should fail before either lane commits");

    assert!(
        error
            .msg
            .contains("duplicated inside one materialisation context")
    );
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(
        modules.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(modules.materialisation_locations().count(), 0);
    assert_eq!(generated.sidecars().count(), 0);
}

#[test]
fn combined_publication_rejects_expected_origin_mismatch_without_mutation() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let generated_delta = GeneratedFunctionDelta::from_records(Vec::new());
    let artifact = artifact_without_context();
    let wrong_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "other".to_owned(),
        ModuleRootRole::Normal,
    );

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        ModuleId::from_index(0),
        &wrong_origin,
        artifact,
        generated_delta,
    )
    .expect_err("an expected-origin mismatch must fail before publication");

    assert!(error.msg.contains("disagrees with graph node origin"));
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(
        modules.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(generated.sidecars().count(), 0);
}

#[test]
fn combined_publication_rejects_generated_delta_without_committing_valid_module() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let artifact = artifact_without_context();
    let expected_origin = artifact.interface.module_origin.clone();

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta_with_identity_mismatch(),
    )
    .expect_err("a generated preflight failure must reject the combined publication");

    assert!(error.msg.contains("disagrees with its record identity"));
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(
        modules.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(generated.sidecars().count(), 0);
}

#[test]
fn combined_publication_rejects_module_after_valid_generated_preflight_without_mutating_generated()
{
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let artifact = invalid_artifact();
    let expected_origin = artifact.interface.module_origin.clone();

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta_with_valid_record(),
    )
    .expect_err("a module preflight failure must reject the combined publication");

    assert!(
        error
            .msg
            .contains("duplicated inside one materialisation context")
    );
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(
        modules.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(generated.sidecars().count(), 0);
}
