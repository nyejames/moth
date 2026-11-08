//! Tests for module-artifact retention and boundary bijection invariants.
//!
//! WHAT: verifies the exact optional normal/support surface shape, single-module preflight
//!       rejection and retained artefact-row consistency.
//! WHY: these tests own the module store's local lanes; combined module/generated transactions
//!       are covered by `compilation_tests` instead.

use super::{CompiledModuleArtifactId, ModuleArtifactStore, ModuleId, ProviderSlot};
use crate::build_system::create_project_modules::compiled_boundary::CompiledGraphBoundary;
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_identity::ModuleIdentityRecord;
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
};
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, Module, ModuleRootActivity,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootFiles, ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use std::path::PathBuf;
use std::sync::Arc;

fn assert_compiler_bug(payload: Box<dyn std::any::Any + Send>, expected_message: &str) {
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert_eq!(report.stage, CompilerStage::ProjectBuildAssembly);
    assert_eq!(report.message.as_ref(), expected_message);
}

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

/// Build one structural graph node with exactly the given facade and entry roots.
fn test_graph(
    package: &StablePackageIdentity,
    logical_module_path: &str,
    normal_root: Option<&str>,
    support_root: Option<&str>,
    entry_root: Option<&str>,
) -> ProjectModuleGraph {
    let record = ModuleIdentityRecord::new(
        PathBuf::from(format!("/{logical_module_path}")),
        ModuleRootFiles::new(
            normal_root.map(PathBuf::from),
            support_root.map(PathBuf::from),
            entry_root.map(PathBuf::from),
        ),
        PathBuf::from(logical_module_path),
        package,
    )
    .expect("test module identity should be valid");
    ProjectModuleGraph::from_test_records(vec![record])
}

fn interface_for(origin: StableModuleOriginIdentity) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: origin,
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

fn origin(
    package: &StablePackageIdentity,
    logical_module_path: &str,
    role: ModuleRootRole,
) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        logical_module_path.to_owned(),
        role,
    )
}

fn artifact_with_interfaces(
    normal: Option<StableModuleOriginIdentity>,
    support: Option<StableModuleOriginIdentity>,
) -> CompiledModuleArtifact {
    let mut artifact = artifact_with_context(
        ModuleMaterialisationContext::from_identities_for_test(Vec::new()),
    );
    artifact.normal_interface = normal.map(interface_for);
    artifact.support_interface = support.map(interface_for);
    artifact
}

fn boundary_with_success_artifact(
    graph: ProjectModuleGraph,
    artifact: CompiledModuleArtifact,
) -> CompiledGraphBoundary {
    let store = ModuleArtifactStore {
        slots: vec![ProviderSlot::Successful(CompiledModuleArtifactId(0))],
        artifacts: vec![artifact],
        contexts_by_declaration: rustc_hash::FxHashMap::default(),
        materialisation_rows: Vec::new(),
    };
    CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
        scheduling_cutoff: None,
    }
}

fn artifact_with_context(context: ModuleMaterialisationContext) -> CompiledModuleArtifact {
    CompiledModuleArtifact {
        module: Module {
            executable: ModuleExecutable {
                hir: HirModule::new(),
                resource_table: ModuleResourceTable::new(),
                type_environment: TypeEnvironment::new(),
                borrow_analysis: BorrowCheckReport::default(),
                numeric_proofs: NumericProofs::default(),
                path_table: Arc::new(PathInternerFork::empty().snapshot_table()),
            },
            link_facts: ModuleLinkFacts {
                external_package_registry: Arc::new(ExternalPackageRegistry::new()),
                external_import_candidates: Vec::new(),
                functions: HirModuleLinkFacts::default(),
            },
            metadata: ModuleCompilerMetadata {
                entry_point: PathBuf::new(),
                const_top_level_fragments: Vec::new(),
                root_activity: ModuleRootActivity::default(),
                doc_fragments: Vec::new(),
                materialisation_context: Some(Arc::new(context)),
            },
        },
        normal_interface: None,
        support_interface: None,
    }
}

fn publish(
    store: &mut ModuleArtifactStore,
    artifact: CompiledModuleArtifact,
    expected_normal: Option<&StableModuleOriginIdentity>,
    expected_support: Option<&StableModuleOriginIdentity>,
) {
    let publication = store
        .preflight_success(
            ModuleId::from_index(0),
            &artifact,
            expected_normal,
            expected_support,
        )
        .expect("the exact optional surface shape should publish");
    store.reserve_success_commit(&publication);
    store.commit_success(publication, artifact);
}

#[test]
fn boundary_validation_rejects_missing_successful_artefact_row() {
    let graph = test_graph(
        &StablePackageIdentity::project_local("test"),
        "missing",
        Some("@mod.moth"),
        None,
        None,
    );
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
        scheduling_cutoff: None,
    };

    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        boundary.validate_invariants()
    }))
    .expect_err("a successful slot must reference an existing artefact row");
    assert_compiler_bug(payload, "ModuleId 0 references missing artifact 0");
}

#[test]
fn module_success_preflight_rejects_duplicate_rows_without_mutation() {
    let identity = duplicate_materialisation_identity();
    let expected_message = format!(
        "Generated declaration identity {:?} is duplicated inside one materialisation context",
        identity
    );
    let artifact = artifact_with_context(ModuleMaterialisationContext::from_identities_for_test(
        vec![identity.clone(), identity],
    ));
    let store = ModuleArtifactStore::new(1);

    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.preflight_success(ModuleId::from_index(0), &artifact, None, None)
    }))
    .expect_err("duplicate materialisation rows must raise a compiler bug");
    assert_compiler_bug(payload, &expected_message);

    assert_eq!(store.artifact_count(), 0);
    assert_eq!(
        store.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(store.materialisation_locations().count(), 0);
}

#[test]
fn paired_publication_rejects_wrong_support_origin_without_mutation() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(
        &package,
        "paired",
        Some("@mod.moth"),
        Some("@package.moth"),
        None,
    );
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, expected_support) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let expected_support = expected_support.expect("the test graph declares a support root");
    let artifact =
        artifact_with_interfaces(Some(expected_normal.clone()), Some(expected_normal.clone()));

    let store = ModuleArtifactStore::new(1);
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.preflight_success(
            ModuleId::from_index(0),
            &artifact,
            Some(expected_normal),
            Some(expected_support),
        )
    }))
    .expect_err("a wrong support origin must reject publication");
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert_eq!(report.stage, CompilerStage::ProjectBuildAssembly);
    assert!(
        report.message.contains("support interface origin")
            && report
                .message
                .contains("disagrees with graph node support origin"),
        "unexpected paired publication failure: {}",
        report.message
    );

    assert_eq!(store.artifact_count(), 0);
    assert_eq!(store.materialisation_locations().count(), 0);
}

#[test]
fn paired_publication_rejects_missing_support_interface_without_mutation() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(
        &package,
        "paired",
        Some("@mod.moth"),
        Some("@package.moth"),
        None,
    );
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, expected_support) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let expected_support = expected_support.expect("the test graph declares a support root");
    let artifact = artifact_with_interfaces(Some(expected_normal.clone()), None);

    let store = ModuleArtifactStore::new(1);
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.preflight_success(
            ModuleId::from_index(0),
            &artifact,
            Some(expected_normal),
            Some(expected_support),
        )
    }))
    .expect_err("a paired graph must reject a missing support interface");
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert!(
        report.message.contains("missing the support interface"),
        "unexpected paired publication failure: {}",
        report.message
    );

    assert_eq!(store.artifact_count(), 0);
    assert_eq!(store.materialisation_locations().count(), 0);
}

#[test]
fn normal_only_publication_rejects_unexpected_support_interface_without_mutation() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(&package, "single", Some("@mod.moth"), None, None);
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, expected_support) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    assert!(
        expected_support.is_none(),
        "the test graph must be normal-only"
    );
    let unexpected_support = origin(&package, "single", ModuleRootRole::Support);
    let artifact =
        artifact_with_interfaces(Some(expected_normal.clone()), Some(unexpected_support));

    let store = ModuleArtifactStore::new(1);
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.preflight_success(
            ModuleId::from_index(0),
            &artifact,
            Some(expected_normal),
            None,
        )
    }))
    .expect_err("a single-facade graph must reject an unexpected support interface");
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert!(
        report.message.contains("unexpected support interface"),
        "unexpected single-root publication failure: {}",
        report.message
    );

    assert_eq!(store.artifact_count(), 0);
    assert_eq!(store.materialisation_locations().count(), 0);
}

#[test]
fn publication_rejects_a_missing_normal_surface() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(&package, "normal", Some("@mod.moth"), None, None);
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, _) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let artifact = artifact_with_interfaces(None, None);

    let store = ModuleArtifactStore::new(1);
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        store.preflight_success(
            ModuleId::from_index(0),
            &artifact,
            Some(expected_normal),
            None,
        )
    }))
    .expect_err("a normal facade root must reject a missing normal interface");
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert!(
        report.message.contains("missing the normal interface"),
        "unexpected missing-normal failure: {}",
        report.message
    );
}

#[test]
fn entry_only_publication_publishes_no_interface() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(&package, "entry", None, None, Some("@entry.moth"));
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, expected_support) = node.publication_surfaces();
    assert!(expected_normal.is_none() && expected_support.is_none());
    let artifact = artifact_with_interfaces(None, None);

    let mut store = ModuleArtifactStore::new(1);
    publish(&mut store, artifact, None, None);

    assert!(
        store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Normal)
            .unwrap()
            .is_none(),
        "an entry-only module must not fabricate a normal interface"
    );
    assert!(
        store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Support)
            .unwrap()
            .is_none(),
        "an entry-only module must not fabricate a support interface"
    );
}

#[test]
fn support_only_publication_publishes_only_the_support_surface() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    // A co-located entry root never adds a facade: publication surfaces are keyed only by the
    // normal and support roles.
    for entry_root in [None, Some("@entry.moth")] {
        let graph = test_graph(&package, "support", None, Some("@package.moth"), entry_root);
        let node = graph.node(ModuleId::from_index(0));
        let (expected_normal, expected_support) = node.publication_surfaces();
        assert!(
            expected_normal.is_none(),
            "a support-only module with entry root {entry_root:?} has no normal surface"
        );
        let expected_support = expected_support.expect("the test graph declares a support root");
        let artifact = artifact_with_interfaces(None, Some(expected_support.clone()));

        let mut store = ModuleArtifactStore::new(1);
        publish(&mut store, artifact, None, Some(expected_support));

        let support = store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Support)
            .unwrap()
            .expect("the support surface should resolve");
        assert_eq!(support.module_origin.role(), ModuleRootRole::Support);
        assert!(
            store
                .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Normal)
                .unwrap()
                .is_none(),
            "a support-only module with entry root {entry_root:?} has no normal surface to fall back to"
        );
    }
}

#[test]
fn paired_publication_with_exact_surface_origins_resolves_both_roles() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    // A co-located entry root never adds a facade: the same exact paired surface shape must hold
    // for both the normal+support and triple-root inventories.
    for entry_root in [None, Some("@entry.moth")] {
        let graph = test_graph(
            &package,
            "paired",
            Some("@mod.moth"),
            Some("@package.moth"),
            entry_root,
        );
        let node = graph.node(ModuleId::from_index(0));
        let (expected_normal, expected_support) = node.publication_surfaces();
        let expected_normal = expected_normal.expect("the test graph declares a normal root");
        let expected_support = expected_support.expect("the test graph declares a support root");
        let artifact = artifact_with_interfaces(
            Some(expected_normal.clone()),
            Some(expected_support.clone()),
        );

        let mut store = ModuleArtifactStore::new(1);
        publish(
            &mut store,
            artifact,
            Some(expected_normal),
            Some(expected_support),
        );

        let normal = store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Normal)
            .unwrap()
            .expect("the normal surface should resolve");
        let support = store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Support)
            .unwrap()
            .expect("the support surface should resolve");
        assert_eq!(normal.module_origin.role(), ModuleRootRole::Normal);
        assert_eq!(support.module_origin.role(), ModuleRootRole::Support);
        assert_ne!(normal.module_origin, support.module_origin);
    }
}

#[test]
fn boundary_validation_rejects_paired_artifact_without_support_interface() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(
        &package,
        "paired",
        Some("@mod.moth"),
        Some("@package.moth"),
        None,
    );
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, _) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let artifact = artifact_with_interfaces(Some(expected_normal.clone()), None);
    let boundary = boundary_with_success_artifact(graph, artifact);

    let error = boundary
        .validate_invariants()
        .expect_err("boundary validation must reject the malformed successful artifact");
    assert!(
        error.message.contains("missing the support interface"),
        "unexpected boundary failure: {}",
        error.message
    );
}

#[test]
fn boundary_validation_rejects_paired_artifact_with_wrong_support_role() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(
        &package,
        "paired",
        Some("@mod.moth"),
        Some("@package.moth"),
        None,
    );
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, _) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let wrong_support = origin(&package, "paired", ModuleRootRole::Normal);
    let artifact = artifact_with_interfaces(Some(expected_normal.clone()), Some(wrong_support));
    let boundary = boundary_with_success_artifact(graph, artifact);

    let error = boundary
        .validate_invariants()
        .expect_err("boundary validation must reject the malformed successful artifact");
    assert!(
        error.message.contains("support interface origin")
            && error
                .message
                .contains("disagrees with graph node support origin"),
        "unexpected boundary failure: {}",
        error.message
    );
}

#[test]
fn boundary_validation_rejects_unexpected_support_interface_on_single_facade() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(&package, "single", Some("@mod.moth"), None, None);
    let node = graph.node(ModuleId::from_index(0));
    let (expected_normal, _) = node.publication_surfaces();
    let expected_normal = expected_normal.expect("the test graph declares a normal root");
    let unexpected_support = origin(&package, "single", ModuleRootRole::Support);
    let artifact =
        artifact_with_interfaces(Some(expected_normal.clone()), Some(unexpected_support));
    let boundary = boundary_with_success_artifact(graph, artifact);

    let error = boundary
        .validate_invariants()
        .expect_err("boundary validation must reject the malformed successful artifact");
    assert!(
        error.message.contains("unexpected support interface"),
        "unexpected boundary failure: {}",
        error.message
    );
}

#[test]
fn boundary_validation_rejects_support_surface_on_entry_only_module() {
    let package = StablePackageIdentity::project_local("module-artifact-tests");
    let graph = test_graph(&package, "entry", None, None, Some("@entry.moth"));
    let unexpected_support = origin(&package, "entry", ModuleRootRole::Support);
    let artifact = artifact_with_interfaces(None, Some(unexpected_support));
    let boundary = boundary_with_success_artifact(graph, artifact);

    let error = boundary
        .validate_invariants()
        .expect_err("entry-only publication must reject a fabricated support surface");
    assert!(
        error.message.contains("unexpected support interface"),
        "unexpected boundary failure: {}",
        error.message
    );
}

#[test]
fn paired_diagnosis_marks_the_shared_slot_and_both_surfaces_unavailable() {
    let mut store = ModuleArtifactStore::new(1);
    store
        .mark_diagnosed(ModuleId::from_index(0))
        .expect("diagnosis should own the existing module slot");

    assert_eq!(
        store.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Diagnosed
    );
    assert_eq!(store.artifact_count(), 0);
    assert!(
        store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Normal)
            .unwrap()
            .is_none(),
        "a diagnosed paired module must not expose a normal interface"
    );
    assert!(
        store
            .interface_for_surface(ModuleId::from_index(0), ModuleRootRole::Support)
            .unwrap()
            .is_none(),
        "a diagnosed paired module must not expose a support interface"
    );
}
