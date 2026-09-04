//! Tests for atomic publication of a module artefact, generated delta and resource associations.
//!
//! WHAT: protects the combined build-boundary transaction that commits all three retained lanes.
//! WHY: a valid module, generated delta or resource association must not become visible when
//!       another lane fails preflight. Single-store publication invariants belong to
//!       `module_artifact_store_tests` and `resource_inputs_tests`.

use super::ModuleBoundaryPublication;
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::{
    ModuleArtifactStore, ProviderSlot,
};
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::build_config::{
    BuildConfigValueOrigin, BuildInputType, ConfigResolutionRecord, PrimitiveBuildInputType,
    PrimitiveBuildValue, build_config_fingerprint,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};

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
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::module_resources::{
    ModuleResourceTable, ResourceSourceAssociation,
};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
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
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use crate::projects::settings::ProjectMetadataField;
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

fn resource_association(
    module_origin: &StableModuleOriginIdentity,
    source: ResourceSourceId,
) -> ResourceSourceAssociation {
    ResourceSourceAssociation {
        origin: StableResourceOriginId::module_owned(
            module_origin.clone(),
            PortableResourcePath::from_portable_spelling("assets/compiler-owned.svg".to_owned())
                .expect("test resource path should be valid"),
        ),
        source,
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_module_and_generated(
    modules: &mut ModuleArtifactStore,
    generated: &mut BoundaryGeneratedFunctionStore,
    materialisations: &mut ProviderMaterialisationRegistry,
    resource_inputs: &mut ResourceInputRegistry,
    module_id: ModuleId,
    expected_origin: &StableModuleOriginIdentity,
    artifact: CompiledModuleArtifact,
    generated_delta: GeneratedFunctionDelta,
    resource_source_associations: Vec<ResourceSourceAssociation>,
) -> Result<(), CompilerError> {
    super::publish_module_and_generated(ModuleBoundaryPublication {
        modules,
        generated,
        materialisations,
        resource_inputs,
        module_id,
        expected_origin,
        artifact,
        generated_delta,
        resource_source_associations,
    })
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
                resource_table: ModuleResourceTable::new(),
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
            resource_table: ModuleResourceTable::new(),
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
fn combined_publication_commits_compiler_resource_association_delta() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let artifact = artifact_without_context();
    let expected_origin = artifact.interface.module_origin.clone();
    let source = resource_inputs.register_source(PathBuf::from("/project/assets/logo.svg"));
    let association = resource_association(&expected_origin, source);
    let resource_origin = association.origin.clone();

    publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        GeneratedFunctionDelta::from_records(Vec::new()),
        vec![association],
    )
    .expect("a valid compiler association delta should publish with its module");

    assert_eq!(
        resource_inputs.source_for_origin(&resource_origin),
        Some(source),
        "publication must attach the compiler-produced origin/source pair"
    );
    assert_eq!(modules.artifact_count(), 1);
    assert_eq!(generated.sidecars().count(), 0);
}

#[test]
fn combined_publication_rejects_unknown_resource_source_without_mutation() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let artifact = artifact_without_context();
    let expected_origin = artifact.interface.module_origin.clone();
    let association = resource_association(&expected_origin, ResourceSourceId::from_index(99));
    let resource_origin = association.origin.clone();

    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        GeneratedFunctionDelta::from_records(Vec::new()),
        vec![association],
    )
    .expect_err("an unknown source ID must fail resource preflight");

    assert!(error.msg.contains("unknown source ID"));
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(generated.sidecars().count(), 0);
    assert_eq!(resource_inputs.source_for_origin(&resource_origin), None);
}

#[test]
fn combined_publication_preflights_both_fallible_lanes_before_mutation() {
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let generated_delta = GeneratedFunctionDelta::from_records(Vec::new());
    let artifact = invalid_artifact();
    let expected_origin = artifact.interface.module_origin.clone();

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let source = resource_inputs.register_source(PathBuf::from("/project/assets/logo.svg"));
    let association = resource_association(&expected_origin, source);
    let resource_origin = association.origin.clone();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta,
        vec![association],
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
    assert_eq!(resource_inputs.source_for_origin(&resource_origin), None);
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
    let mut resource_inputs = ResourceInputRegistry::new();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &wrong_origin,
        artifact,
        generated_delta,
        Vec::new(),
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
    let mut resource_inputs = ResourceInputRegistry::new();
    let source = resource_inputs.register_source(PathBuf::from("/project/assets/logo.svg"));
    let association = resource_association(&expected_origin, source);
    let before = resource_inputs.clone();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta_with_identity_mismatch(),
        vec![association],
    )
    .expect_err("a generated preflight failure must reject the combined publication");

    assert!(error.msg.contains("disagrees with its record identity"));
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(
        modules.slot(ModuleId::from_index(0)).unwrap(),
        ProviderSlot::Unavailable
    );
    assert_eq!(generated.sidecars().count(), 0);
    assert_eq!(
        resource_inputs, before,
        "generated preflight failure must not attach a pending resource association"
    );
}

#[test]
fn combined_publication_rejects_module_after_valid_generated_preflight_without_mutating_generated()
{
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let artifact = invalid_artifact();
    let expected_origin = artifact.interface.module_origin.clone();

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let source = resource_inputs.register_source(PathBuf::from("/project/assets/logo.svg"));
    let association = resource_association(&expected_origin, source);
    let before = resource_inputs.clone();
    let error = publish_module_and_generated(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &expected_origin,
        artifact,
        generated_delta_with_valid_record(),
        vec![association],
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
    assert_eq!(
        resource_inputs, before,
        "module preflight failure must not attach a pending resource association"
    );
}

#[test]
fn project_metadata_fingerprint_tracks_name_type_and_value_structure() {
    let int_type = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int);
    let bool_type = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool);
    let one = PublicFoldedValue::Int(1);
    let two = PublicFoldedValue::Int(2);

    assert_eq!(
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &one),
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &one)
    );
    assert_ne!(
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &one),
        super::config_boundary::project_metadata_fingerprint("other", &int_type, &one)
    );
    assert_ne!(
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &one),
        super::config_boundary::project_metadata_fingerprint("count", &bool_type, &one)
    );
    assert_ne!(
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &one),
        super::config_boundary::project_metadata_fingerprint("count", &int_type, &two)
    );
}

#[test]
fn effective_project_fields_exclude_internal_unschematized_defaults() {
    let mut config = Config::new(PathBuf::from("/project"));
    config.project_name = "docs".to_owned();
    config.entry_root = PathBuf::from("src");
    config.project_config_loaded = true;
    config.extra_project_fields.push(ProjectMetadataField {
        name: "DisplayName".to_owned(),
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        value: PublicFoldedValue::String(OwnedFoldedString::Text("Docs".to_owned())),
        location: SourceLocation::default(),
    });
    let mut string_table = StringTable::new();

    let fields = super::config_boundary::effective_project_fields(&config, &mut string_table)
        .expect("the effective project snapshot should build");
    let names = fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["name", "entry_root", "DisplayName"]);
    let fixed_names = super::config_boundary::fixed_project_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(fixed_names, vec!["name", "entry_root"]);
}

#[test]
fn effective_project_fields_classify_fixed_direct_and_metadata_kinds() {
    let mut config = Config::new(PathBuf::from("/project"));
    config.project_name = "docs".to_owned();
    config.entry_root = PathBuf::from("src");
    config.project_config_loaded = true;

    let mut string_table = StringTable::new();
    let direct_contract = BuildInputType::Optional(PrimitiveBuildInputType::String);
    let direct_value = Some(PrimitiveBuildValue::String("configured".to_owned()));
    let direct_name = "configured";
    let direct_field_name = string_table.intern(direct_name);
    config
        .config_resolution_records
        .push(ConfigResolutionRecord {
            field_name: direct_field_name,
            contract: direct_contract,
            required: false,
            default: None,
            value: direct_value.clone(),
            origin: BuildConfigValueOrigin::ExplicitInput,
            fingerprint: build_config_fingerprint(
                direct_name,
                direct_contract,
                direct_value.as_ref(),
            ),
            qualifier_location: SourceLocation::default(),
            value_location: None,
        });
    config.extra_project_fields.push(ProjectMetadataField {
        name: "complex".to_owned(),
        type_identity: CanonicalTypeIdentity::AnonymousConstRecord,
        value: PublicFoldedValue::Record(Vec::new()),
        location: SourceLocation::default(),
    });

    let fields = super::config_boundary::effective_project_fields(&config, &mut string_table)
        .expect("the effective project snapshot should build");

    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["configured", "name", "entry_root", "complex"]
    );
    assert!(matches!(
        &fields[0].kind,
        super::config_boundary::EffectiveProjectFieldKind::DirectConfig {
            contract: BuildInputType::Optional(PrimitiveBuildInputType::String),
            required: false,
            ..
        }
    ));
    assert!(matches!(
        &fields[1].kind,
        super::config_boundary::EffectiveProjectFieldKind::FixedPrimitive {
            value_type: BuildInputType::Primitive(PrimitiveBuildInputType::String),
            required: true,
            ..
        }
    ));
    assert!(matches!(
        &fields[2].kind,
        super::config_boundary::EffectiveProjectFieldKind::FixedPrimitive {
            value_type: BuildInputType::Primitive(PrimitiveBuildInputType::String),
            required: true,
            ..
        }
    ));
    assert!(matches!(
        &fields[3].kind,
        super::config_boundary::EffectiveProjectFieldKind::Metadata
    ));

    let fixed_names = super::config_boundary::fixed_project_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(fixed_names, vec!["name", "entry_root"]);

    let direct_names = super::config_boundary::direct_project_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(direct_names, vec!["configured"]);
}
