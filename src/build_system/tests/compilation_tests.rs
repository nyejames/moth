//! Tests for atomic publication of a module artefact, generated delta and resource associations.
//!
//! WHAT: protects the combined build-boundary transaction that commits all three retained lanes.
//! WHY: a valid module, generated delta or resource association must not become visible when
//!       another lane fails preflight. Single-store publication invariants belong to
//!       `module_artifact_store_tests` and `resource_inputs_tests`.

use super::super::create_project_modules_tests::successful_compilation_value;
use super::{BoundaryPremergeFailure, ModuleBoundaryPublication, premerge_failure_to_reports};
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, ProjectFrontendCompilation,
};
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::{
    ModuleArtifactStore, ProviderSlot,
};
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::build_config::{
    BuildConfigValueOrigin, BuildInputName, BuildInputType, ConfigResolutionRecord,
    FoldedConfigProjectFieldDependency, PrimitiveBuildInputType, PrimitiveBuildValue,
    build_config_fingerprint,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::SuccessfulWarningBoundary;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, PublicFoldedValue};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;
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
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use crate::projects::settings::ProjectMetadataField;
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

#[allow(
    clippy::too_many_arguments,
    reason = "test publication mirrors the production boundary with mutable module/generated/materialisation/resource owners, module identity and expected origin, artifact, generated delta and resource associations kept separate"
)]
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
) -> Result<(), InfrastructureFailure> {
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
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    generated_identity_for_origin(name, &origin, "@page.moth")
}

fn generated_identity_for_origin(
    name: &str,
    origin: &StableModuleOriginIdentity,
    module_path: &str,
) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            origin.clone(),
            module_path.to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::FixedScalar(FixedScalar::I32),
        )]),
        Box::new([]),
    )
}

fn generated_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
        escapes_builtin_failure: false,
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
            materialisation_context: None,
        },
    };
    module.executable.hir.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: Vec::new(),
        returns: Box::new([]),
        error_return: None,
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

    assert!(error.message.contains("unknown source ID"));
    assert_eq!(modules.artifact_count(), 0);
    assert_eq!(generated.sidecars().count(), 0);
    assert_eq!(resource_inputs.source_for_origin(&resource_origin), None);
}

#[test]
fn combined_publication_module_bug_raises_before_any_lane_mutation() {
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
    let expected_bug_message = format!(
        "Generated declaration identity {:?} is duplicated inside one materialisation context",
        duplicate_materialisation_identity()
    );
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        publish_module_and_generated(
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
    }))
    .expect_err(
        "an invalid module publication must raise a compiler bug before either lane commits",
    );
    assert_compiler_bug(payload, &expected_bug_message);
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
    let actual_origin = artifact.interface.module_origin.clone();
    let expected_bug_message = format!(
        "ModuleId 0 artefact interface origin {:?} disagrees with graph node origin {:?}",
        actual_origin, wrong_origin
    );

    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        publish_module_and_generated(
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
    }))
    .expect_err("an expected-origin mismatch must raise a compiler bug before publication");
    assert_compiler_bug(payload, &expected_bug_message);
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

    assert!(error.message.contains("disagrees with its record identity"));
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
    let expected_bug_message = format!(
        "Generated declaration identity {:?} is duplicated inside one materialisation context",
        duplicate_materialisation_identity()
    );
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        publish_module_and_generated(
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
    }))
    .expect_err("a module preflight bug must raise before committing any lane");
    assert_compiler_bug(payload, &expected_bug_message);
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
    let int_type =
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32));
    let bool_type = CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool);
    let one = PublicFoldedValue::FixedScalar(
        FixedScalarValue::signed(FixedScalar::I32, 1).expect("a test value fits I32"),
    );
    let two = PublicFoldedValue::FixedScalar(
        FixedScalarValue::signed(FixedScalar::I32, 2).expect("a test value fits I32"),
    );

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
        span: None,
    });
    let string_table = StringTable::new();

    let fields = super::config_boundary::effective_project_fields(&config, &string_table)
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
fn effective_project_fields_classify_fixed_and_metadata_kinds() {
    let mut config = Config::new(PathBuf::from("/project"));
    config.project_name = "docs".to_owned();
    config.entry_root = PathBuf::from("src");
    config.project_config_loaded = true;
    config.extra_project_fields.push(ProjectMetadataField {
        name: "complex".to_owned(),
        type_identity: CanonicalTypeIdentity::AnonymousConstRecord,
        value: PublicFoldedValue::Record(Vec::new()),
        span: None,
    });
    let string_table = StringTable::new();

    let fields = super::config_boundary::effective_project_fields(&config, &string_table)
        .expect("the effective project snapshot should build");

    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["name", "entry_root", "complex"]
    );
    assert!(matches!(
        &fields[0].kind,
        super::config_boundary::EffectiveProjectFieldKind::FixedPrimitive {
            value_type: BuildInputType::Primitive(PrimitiveBuildInputType::String),
            required: true,
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
        super::config_boundary::EffectiveProjectFieldKind::Metadata
    ));

    let fixed_names = super::config_boundary::fixed_project_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(fixed_names, vec!["name", "entry_root"]);
    assert!(super::config_boundary::input_contract_facts(&fields).is_empty());
}

#[test]
fn effective_project_fields_keep_private_input_and_receiving_field_distinct() {
    let mut config = Config::new(PathBuf::from("/project"));
    config.project_name = "docs".to_owned();
    config.entry_root = PathBuf::from("src");
    config.project_config_loaded = true;

    let mut string_table = StringTable::new();
    let input_name = BuildInputName::new("release_version").expect("valid input name");
    let input_field_name = string_table.intern(input_name.as_str());
    let input_contract = BuildInputType::Primitive(PrimitiveBuildInputType::String);
    let input_value = Some(PrimitiveBuildValue::String("2.0".to_owned()));
    config
        .config_resolution_records
        .push(ConfigResolutionRecord {
            field_name: input_field_name,
            input_name: input_name.clone(),
            contract: input_contract,
            required: false,
            default: input_value.clone(),
            value: input_value.clone(),
            origin: BuildConfigValueOrigin::DeclarationDefault,
            fingerprint: build_config_fingerprint(
                input_name.as_str(),
                input_contract,
                input_value.as_ref(),
            ),
            qualifier_span: None,
            value_location: None,
        });
    let receiving_field_name = string_table.intern("version");
    config
        .project_field_config_dependencies
        .push(FoldedConfigProjectFieldDependency {
            field_name: receiving_field_name,
            input_names: vec![input_name.clone()],
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
            value: PublicFoldedValue::String(OwnedFoldedString::Text("2.0".to_owned())),
            span: None,
        });

    let fields = super::config_boundary::effective_project_fields(&config, &string_table)
        .expect("the effective project snapshot should build");
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["version", "name", "entry_root", "release_version"]
    );
    assert!(matches!(
        &fields[0].kind,
        super::config_boundary::EffectiveProjectFieldKind::InputDependent { input_names }
            if input_names == &vec![input_name.clone()]
    ));
    assert!(matches!(
        &fields[3].kind,
        super::config_boundary::EffectiveProjectFieldKind::InputContract { .. }
    ));

    let fixed_names = super::config_boundary::fixed_project_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(fixed_names, vec!["name", "entry_root"]);
    let direct_names = super::config_boundary::input_contract_facts(&fields)
        .into_iter()
        .map(|fact| fact.name().as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(direct_names, vec!["release_version"]);

    let globals = super::config_boundary::build_project_globals_interface(&config, &fields)
        .expect("the project-global provider should build")
        .expect("a loaded config should publish project globals");
    let public_names = globals
        .interface()
        .export_bindings
        .iter()
        .map(|binding| binding.public_name())
        .collect::<Vec<_>>();
    assert!(public_names.contains(&"version"));
    assert!(!public_names.contains(&"release_version"));
}

/// Build one minimal semantic result whose path fork carries one module-local path node, so
/// publication must allocate into the build table to merge it.
fn semantic_result_with_local_path()
-> crate::compiler_frontend::module_compilation::ModuleSemanticResult {
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    semantic_result_with_local_path_in_domain(origin, StringTable::new(), PathInternerFork::empty())
}

fn semantic_result_with_local_path_in_domain(
    origin: StableModuleOriginIdentity,
    mut string_table: StringTable,
    mut path_fork: PathInternerFork,
) -> crate::compiler_frontend::module_compilation::ModuleSemanticResult {
    let mut artifact = invalid_artifact();
    artifact.module.metadata.materialisation_context = None;
    artifact.interface.module_origin = origin.clone();
    path_fork
        .try_intern_portable_path(&format!("{origin:?}/published-path"), &mut string_table)
        .expect("the module-local path should intern into the fork");
    assert_ne!(path_fork.base_len(), 0, "fork must carry local nodes");

    crate::compiler_frontend::module_compilation::ModuleSemanticResult {
        module: artifact.module,
        generated_delta: GeneratedFunctionDelta::from_records(Vec::new()),
        resource_source_associations: Vec::new(),
        string_table,
        path_fork,
        public_interface: artifact.interface,
    }
}

#[test]
fn forced_path_exhaustion_during_module_publication_reports_capacity_diagnostic() {
    use crate::compiler_frontend::compiler_messages::{
        DiagnosticSeverity, PreparedFailure, SourceSpanCapacityResource,
        diagnostics::{
            boundary_accumulator::{BoundaryAccumulator, SuccessfulWarningBoundary},
            families::token_lexical::SourceSpanCapacityView,
            freeze::InstallMode,
        },
    };
    use crate::compiler_frontend::symbols::path_interner::{
        ForcedExhaustionGuard, PathInternerBuilder,
    };

    let compiled = semantic_result_with_local_path();
    let string_table = StringTable::new();
    let mut path_interner = PathInternerBuilder::new();
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );

    let _gate = ForcedExhaustionGuard::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let (string_table, mut warning_sink) = warning_owner.compiler_parts_mut();
    let error = super::publish_compiled_module(
        &mut modules,
        &mut generated,
        &mut materialisations,
        &mut resource_inputs,
        ModuleId::from_index(0),
        &origin,
        compiled,
        DiagnosticBag::new(),
        Vec::new(),
        // A production `path_fork` forks from the build table, so its inherited base already
        // contains the root node; the empty fixture fork reports the same base length.
        0,
        1,
        string_table,
        &mut path_interner,
        Some(&mut warning_sink),
        SuccessfulWarningBoundary::Project,
    )
    .expect_err("authored path exhaustion must fail module publication");

    let PreparedFailure::Diagnosed(batch) = error else {
        panic!("authored publication exhaustion must produce the diagnosed capacity lane");
    };
    assert_eq!(batch.diagnostic_count(), 1);
    assert!(batch.has_errors());
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("diagnosed capacity error should freeze");
    assert_eq!(reports.counts().errors, 1);
    let (_, diagnostic) = reports
        .iter()
        .next()
        .expect("exhaustion must retain one frozen diagnostic");
    assert_eq!(diagnostic.severity(), DiagnosticSeverity::Error);
    let facts = diagnostic
        .family_view::<SourceSpanCapacityView>()
        .expect("exhaustion must retain the typed capacity facts");
    assert_eq!(facts.start, 0);
    assert_eq!(facts.length, u32::MAX as usize);
    assert_eq!(facts.resource, SourceSpanCapacityResource::LogicalPath);
}

#[test]
fn source_finish_infrastructure_dominates_diagnostics_and_preserves_causes() {
    use crate::compiler_frontend::compiler_errors::{
        InfrastructureErrors, InfrastructureFailureKind,
    };
    use crate::compiler_frontend::compiler_messages::CompilerStage;
    use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
    use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};

    let root_string_table = StringTable::new();
    let primary =
        InfrastructureFailure::internal(CompilerStage::AstConstruction, "original failure");
    let errors = InfrastructureErrors::new(primary);
    let finish = InfrastructureFailure::filesystem(
        CompilerStage::SourceLoading,
        std::path::Path::new("finish-source.moth"),
        "first finish failure",
    );
    let failure =
        PreparedFailure::Infrastructure(Box::new(errors)).with_infrastructure_cause(finish);
    let failure = failure.with_infrastructure_cause(InfrastructureFailure::internal(
        CompilerStage::SourceLoading,
        "second finish failure",
    ));
    let PreparedFailure::Infrastructure(errors) = failure else {
        panic!("source finalization must preserve all infrastructure causes");
    };
    let causes = errors.iter().collect::<Vec<_>>();
    assert_eq!(causes.len(), 3);
    assert_eq!(&*causes[0].message, "original failure");
    assert_eq!(causes[0].kind, InfrastructureFailureKind::Internal);
    assert_eq!(causes[0].stage, CompilerStage::AstConstruction);
    assert_eq!(causes[0].host_path, None);
    assert_eq!(&*causes[1].message, "first finish failure");
    assert_eq!(causes[1].kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(causes[1].stage, CompilerStage::SourceLoading);
    assert_eq!(
        causes[1].host_path.as_deref(),
        Some(std::path::Path::new("finish-source.moth")),
    );
    assert_eq!(&*causes[2].message, "second finish failure");
    assert_eq!(causes[2].kind, InfrastructureFailureKind::Internal);
    assert!(causes[2].details.is_empty());
    drop(causes);

    let batch = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from(token_lexical::unexpected_trailing_comma(None)),
        &root_string_table,
        None,
        || None,
    )
    .expect("fixture diagnostic should capture in the root string domain");
    assert_eq!(batch.diagnostic_count(), 1);
    assert!(batch.has_errors());
    let mut accumulator = BoundaryAccumulator::with_root_string_table(root_string_table);
    accumulator.append_prepared(batch, None);
    accumulator.append_infrastructure(errors);
    let result = accumulator.finish(InstallMode::Frozen);
    let Err(errors) = result else {
        panic!("infrastructure must dominate simultaneous diagnostics");
    };
    let messages = errors
        .iter()
        .map(|cause| &*cause.message)
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        [
            "original failure",
            "first finish failure",
            "second finish failure"
        ],
        "infrastructure dominance must retain every ordered cause"
    );
}

/// Build one successful check-only semantic result and its transient warning bag, each interned
/// against the module's isolated string-table domain.
fn check_only_success_result_with_warnings() -> (
    crate::compiler_frontend::module_compilation::ModuleSemanticResult,
    DiagnosticBag,
    Vec<DiagnosticBag>,
    String,
) {
    use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::type_mismatch;
    use crate::compiler_frontend::compiler_messages::{DiagnosticSeverity, TypeMismatchContext};
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut artifact = artifact_without_context();
    artifact.interface.module_origin = origin;

    // Production check-only sidecars compile inside the module's string/path domains, so
    // warnings from both owners share one domain. Pre-seeding the aggregate prefix first
    // gives the module domain a real inherited prefix, exercising a genuine component remap.
    let mut module_string_table = StringTable::new();
    module_string_table.intern("shared-prefix-component");
    let mut module_path_fork = PathInternerFork::empty();
    module_path_fork
        .try_intern_portable_path("shared/prefix", &mut module_string_table)
        .expect("the aggregate prefix path should intern");
    let warning_path = module_path_fork
        .try_intern_portable_path("sidecar/lib-target.moth", &mut module_string_table)
        .expect("the direct warning path should intern");
    let mut module_type_environment = TypeEnvironment::new();
    let module_expected = module_type_environment.register_synthetic_generic_parameter(
        module_string_table.intern("TCheckOnlyModuleExpected"),
    );
    let module_found = module_type_environment
        .register_synthetic_generic_parameter(module_string_table.intern("TCheckOnlyModuleFound"));
    let mut module_warnings = DiagnosticBag::new();
    module_warnings.push(
        imports::missing_import_target(warning_path, None)
            .with_severity(DiagnosticSeverity::Warning),
    );
    module_warnings.push(
        type_mismatch(
            module_expected,
            module_found,
            TypeMismatchContext::Assignment,
            None,
        )
        .with_severity(DiagnosticSeverity::Warning),
    );
    let sidecar_path = module_path_fork
        .try_intern_portable_path("sidecar/generated-target.moth", &mut module_string_table)
        .expect("the sidecar warning path should intern");
    // The sidecar takes the fixture's empty executable and link lanes; the semantic result keeps
    // the rebuilt base module beside its direct warning and the sidecar's sibling warning bag.
    let Module {
        executable: mut base_executable,
        link_facts: base_link_facts,
        metadata: base_metadata,
    } = artifact.module;
    base_executable.type_environment = module_type_environment;
    let mut sidecar_type_environment = TypeEnvironment::new();
    let sidecar_expected = sidecar_type_environment.register_synthetic_generic_parameter(
        module_string_table.intern("TCheckOnlySidecarExpected"),
    );
    let sidecar_found = sidecar_type_environment
        .register_synthetic_generic_parameter(module_string_table.intern("TCheckOnlySidecarFound"));
    let sidecar_module = Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            resource_table: ModuleResourceTable::new(),
            type_environment: sidecar_type_environment,
            borrow_analysis: BorrowCheckReport::default(),
            numeric_proofs: NumericProofs::default(),
            path_table: Arc::new(PathInternerFork::empty().snapshot_table()),
        },
        link_facts: base_link_facts,
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::from("@sidecar.moth"),
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            materialisation_context: None,
        },
    };
    let mut sidecar_warnings = DiagnosticBag::new();
    sidecar_warnings.push(
        imports::missing_import_target(sidecar_path, None)
            .with_severity(DiagnosticSeverity::Warning),
    );
    sidecar_warnings.push(
        type_mismatch(
            sidecar_expected,
            sidecar_found,
            TypeMismatchContext::Assignment,
            None,
        )
        .with_severity(DiagnosticSeverity::Warning),
    );

    let identity = generated_identity("check_only_sidecar");
    let summary = generated_summary();
    let sidecar = GeneratedFunctionSidecar::new(identity.clone(), sidecar_module);
    (
        crate::compiler_frontend::module_compilation::ModuleSemanticResult {
            module: Module {
                executable: base_executable,
                link_facts: ModuleLinkFacts {
                    external_package_registry: Arc::new(ExternalPackageRegistry::new()),
                    external_import_candidates: Vec::new(),
                    functions: HirModuleLinkFacts::default(),
                },
                metadata: base_metadata,
            },
            generated_delta: GeneratedFunctionDelta::from_records(vec![
                CompletedGeneratedFunction {
                    identity: identity.clone(),
                    summary,
                    sidecar,
                },
            ]),
            resource_source_associations: Vec::new(),
            string_table: module_string_table,
            path_fork: module_path_fork,
            public_interface: artifact.interface,
        },
        module_warnings,
        vec![sidecar_warnings],
        "sidecar/lib-target.moth".to_owned(),
    )
}

#[test]
fn check_only_success_batches_render_local_paths_through_production_construction() {
    use crate::build_system::create_project_modules::compilation::canonical::check_only_success_batch;
    use crate::build_system::create_project_modules::compiled_boundary::{
        CompiledGraphBoundary, CompletedSourcePackageRegistry, ProjectFrontendCompilation,
        TransientPreparedBatch,
    };
    use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
    use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
    use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
    use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::{
        choices_control_flow, names_declarations,
    };
    use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
    use crate::compiler_frontend::compiler_messages::render::terse;

    let (compiled, warnings, sidecar_warnings, _spelling) =
        check_only_success_result_with_warnings();
    // Route through the real canonical check-only success construction: it collects the
    // direct and generated-sidecar warnings and attaches the issuing path fork's snapshot.
    // If production ever stops attaching the snapshot, the prepared diagnostics lose their
    // path context and the spelling assertions below fail.
    let batch = check_only_success_batch(compiled, warnings, sidecar_warnings)
        .expect("check-only path attachment must pair with its own string table")
        .expect("the fixture should produce warning diagnostics");

    // Root-domain package diagnostics and the project batch enter the command tail in production
    // order: source packages are staged before project check-only transients.
    let mut root_string_table = StringTable::new();
    root_string_table.intern("sidecar");
    let package_name = root_string_table.intern("package_transient_warning");
    let package_warning = names_declarations::unknown_value_name(package_name, None)
        .with_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Warning);
    let package_warning_code = package_warning.code();
    let package_batch = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from(package_warning),
        &root_string_table,
        None,
        || None,
    )
    .expect("package transient warning should capture in the command string domain");
    let mut source_packages = CompletedSourcePackageRegistry::new();
    let package_id = source_packages
        .publish(
            super::super::create_project_modules_tests::compiled_package("check-only-transient"),
            &[],
        )
        .expect("test source package should publish");
    source_packages
        .set_source_database(
            package_id,
            Arc::new(crate::compiler_frontend::source::SourceDatabase::empty()),
        )
        .expect("test package source database should attach");

    // One empty project boundary mirrors the canonical check-only lane's project domain.
    let project = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(Vec::new()),
        modules: ModuleArtifactStore::new(0),
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let frontend = ProjectFrontendCompilation::new(
        project,
        source_packages,
        ResourceInputRegistry::new(),
        vec![
            TransientPreparedBatch::package(package_id, package_batch),
            TransientPreparedBatch::project(batch),
        ],
    )
    .expect("transient check-only warning frontend should validate");

    // A colliding aggregate entry forces the append's component remap to be non-identity:
    // "sidecar" already sits at aggregate index 0, so every module-local StringId shifts and
    // only a correct per-batch remap can still resolve the path components.
    let canonical_warning = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from(choices_control_flow::unreachable_match_arm(None)),
        &root_string_table,
        None,
        || None,
    )
    .expect("canonical warning should capture in the command string domain");
    let mut warning_owner =
        crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator::with_root_string_table(root_string_table);
    {
        let (_, mut warning_sink) = warning_owner.compiler_parts_mut();
        warning_sink.append(
            SuccessfulWarningBoundary::Project,
            crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::SuccessfulWarningPosition::BaseModule(0),
            canonical_warning,
        );
    }
    let mut frontend_value = match frontend.into_compilation_result(&mut warning_owner, None, None)
    {
        Ok(CompilationOutcome::Success {
            value,
            warnings: None,
        }) => value,
        _ => panic!("inner frontend should stage check-only warnings without freezing them"),
    };
    let project_source_database = frontend_value.project_source_database.take();
    drop(frontend_value);
    let result = crate::build_system::build::finish_successful_project_warnings(
        &mut warning_owner,
        project_source_database,
        (),
    );
    let reports = match &result {
        Ok(CompilationOutcome::Success {
            warnings: Some(reports),
            ..
        }) => reports,
        _ => panic!("check-only warnings should remain on successful compilation"),
    };
    let rendered = terse::format_terse_compilation_result(&result);
    assert_eq!(
        rendered.len(),
        6,
        "canonical, package, and project check-only warnings should render: {rendered:?}"
    );
    assert_eq!(reports.counts().errors, 0);
    assert_eq!(reports.counts().warnings, 6);
    let warning_codes = reports
        .iter_severity(crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Warning)
        .map(|(_, view)| view.descriptor().code)
        .collect::<Vec<_>>();
    assert_eq!(
        warning_codes[0], "MOTH-RULE-0022",
        "canonical module warnings must precede check-only transient warnings: {warning_codes:?}"
    );
    assert_eq!(
        warning_codes[1], package_warning_code,
        "the package transient must follow canonical warnings and precede project transients: {warning_codes:?}"
    );
    let direct = rendered
        .iter()
        .position(|line| line.contains("sidecar/lib-target.moth"))
        .expect("the direct warning should remain present");
    let package_transient = rendered
        .iter()
        .position(|line| line.contains("package_transient_warning"))
        .expect("the package transient warning should remain present");
    assert!(
        package_transient < direct,
        "package transients must precede project transients: {rendered:?}"
    );
    let generated = rendered
        .iter()
        .position(|line| line.contains("sidecar/generated-target.moth"))
        .expect("the generated warning should remain present");
    assert!(direct < generated, "warning order changed: {rendered:?}");
    for spelling in ["sidecar/lib-target.moth", "sidecar/generated-target.moth"] {
        assert!(
            rendered.iter().any(|line| line.contains(spelling)),
            "check-only warning must render its original path spelling {spelling:?}: {rendered:?}"
        );
    }
    assert!(
        rendered.iter().any(|line| {
            line.contains("TCheckOnlyModuleExpected") && line.contains("TCheckOnlyModuleFound")
        }),
        "module warnings must render through the module's environment: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| {
            line.contains("TCheckOnlySidecarExpected") && line.contains("TCheckOnlySidecarFound")
        }),
        "sidecar warnings must render through the sidecar's environment: {rendered:?}"
    );
    assert!(
        rendered.iter().all(|line| !line.contains("TypeId(")),
        "check-only warnings with owning environments must not render raw type IDs: {rendered:?}"
    );
}

#[test]
fn canonical_tail_success_retains_warning_source_context() {
    use crate::build_system::create_project_modules::compiled_boundary::{
        CompiledGraphBoundary, CompletedSourcePackageRegistry, ProjectFrontendCompilation,
        TransientPreparedBatch,
    };
    use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
    use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
    use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
    use crate::compiler_frontend::compiler_messages::{DiagnosticBag, DiagnosticSeverity};
    use crate::compiler_frontend::source::SourceDatabase;
    use std::path::Path;

    let path = Path::new("/project/trailing.moth");
    let mut strings = StringTable::new();
    let name = strings.intern("trailing-warning");
    let mut source = SourceDatabase::build(std::iter::once(path), path, None, &mut strings)
        .expect("project source should build");
    let source_id = source
        .get_by_canonical_path(path)
        .expect("registered source")
        .id;
    source
        .retain_text(source_id, "original trailing source".to_owned())
        .expect("source snapshot should be retained");
    let warning = names_declarations::unknown_value_name(name, None)
        .with_severity(DiagnosticSeverity::Warning);
    let expected_warning_code = warning.code();
    let batch = PreparedDiagnosticBatch::capture_from_root_string_table(
        DiagnosticBag::from_diagnostics(vec![warning]),
        &strings,
        None,
        || None,
    )
    .expect("the warning should capture against the project root table");
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(strings);
    let project = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(Vec::new()),
        modules: ModuleArtifactStore::new(0),
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let frontend = ProjectFrontendCompilation::new(
        project,
        CompletedSourcePackageRegistry::new(),
        ResourceInputRegistry::new(),
        vec![TransientPreparedBatch::project(batch)],
    )
    .expect("empty frontend should validate");
    let mut frontend_value =
        match frontend.into_compilation_result(&mut warning_owner, Some(Arc::new(source)), None) {
            Ok(CompilationOutcome::Success {
                value,
                warnings: None,
            }) => value,
            _ => panic!("inner frontend should stage warnings without freezing them"),
        };
    let project_source_database = frontend_value.project_source_database.take();
    drop(frontend_value);
    let result = crate::build_system::build::finish_successful_project_warnings(
        &mut warning_owner,
        project_source_database,
        (),
    );
    let reports = match &result {
        Ok(CompilationOutcome::Success {
            warnings: Some(reports),
            ..
        }) => reports,
        _ => panic!("canonical frontend warning should remain on success"),
    };
    assert_eq!(reports.counts().warnings, 1);
    assert_eq!(reports.counts().errors, 0);
    let (warning_report, warning_view) = reports
        .iter()
        .next()
        .expect("the canonical warning should remain");
    assert_eq!(warning_view.descriptor().code, expected_warning_code);
    assert_eq!(warning_view.severity(), DiagnosticSeverity::Warning);
    let warning_identity = warning_report.context();
    assert!(
        warning_identity.get(source_id).is_some(),
        "the original physical source must retain its identity slot"
    );
    let line_index = warning_identity
        .line_index(source_id)
        .expect("the original physical source line index");
    assert_eq!(line_index.line_text(0), Some("original trailing source"));
    let path_id = warning_identity
        .source_logical_path(source_id)
        .expect("logical source path");
    assert_eq!(
        warning_identity.render_path(path_id, &mut Vec::new()),
        "trailing.moth"
    );
}

#[cfg(feature = "data_layout_memory_probe")]
#[test]
fn canonical_tail_retains_donor_only_package_identity() {
    use crate::build_system::BuildProfile;
    use crate::build_system::create_project_modules::compiled_boundary::FrozenRenderRetentionMetrics;
    use crate::build_system::create_project_modules::{
        FrontendCompilationMode, compile_project_frontend_with_inputs,
    };
    use crate::builder_surface::{BuilderSurface, PackageOrigin};
    use crate::compiler_frontend::build_config::BuildConfigInputSet;
    use crate::compiler_frontend::compiler_errors::CompilationOutcome;
    use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
    use std::fs;

    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let project_directory = tempfile::tempdir().expect("the requester project should exist");
    let donor_directory = tempfile::tempdir().expect("the donor package should exist");
    let source_root = project_directory.path().join("src");
    fs::create_dir_all(&source_root).expect("the requester source root should exist");
    let requester_path = source_root.join("@page.moth");
    let donor_path = donor_directory.path().join("@mod.moth");
    const REQUESTER: &str = "@pkg outer\nvalue I32 = outer(1)\n";
    const DONOR: &str = "inner type U |text String, result U| -> U:\n    return result\n;\n\
        export:\n    outer type T |value T| -> I32:\n        return inner(value, 1)\n    ;\n;\n";
    fs::write(&requester_path, REQUESTER).expect("the requester should be written");
    fs::write(&donor_path, DONOR).expect("the donor should be written");
    fs::write(
        project_directory.path().join("config.moth"),
        "project #= (name = \"snapshots\", entry_root = \"src\")\nhtml #= ()\n",
    )
    .expect("the project config should be written");

    let mut config = Config::new(project_directory.path().to_path_buf());
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut surface = BuilderSurface::with_mandatory_core();
    surface.source_packages.register_filesystem_root(
        "pkg",
        donor_directory.path().to_path_buf(),
        PackageOrigin::Builder,
    );
    let mut project_source_files = None;
    let mut project_source_builder = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &StyleDirectiveRegistry::built_ins(),
        &mut surface,
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
    .expect("the requester diagnosis should retain the frontend transaction");

    let reports = match frontend {
        CompilationOutcome::Diagnosed(reports) => reports,
        CompilationOutcome::Success { .. } => {
            panic!("the requester should be diagnosed by the concrete donor call")
        }
    };
    let counts = reports.counts();
    assert_eq!(counts.errors, 1);
    assert_eq!(counts.warnings, 0);

    let (
        expected_source_snapshot_bytes,
        expected_extended_span_rows,
        expected_source_identity_slots,
        expected_path_rows,
        expected_path_storage_bytes,
        retained_identity_count,
        retained_path_table_count,
    ) = {
        let mut seen_identities = std::collections::HashSet::new();
        let mut seen_path_tables = std::collections::HashSet::new();
        let mut source_snapshot_bytes = 0;
        let mut extended_span_rows = 0;
        let mut source_identity_slots = 0;
        let mut path_rows = 0;
        let mut path_storage_bytes = 0;
        for identity in reports.contexts() {
            let identity_address = identity as *const _ as usize;
            if seen_identities.insert(identity_address) {
                let metrics = identity.retention_metrics();
                source_snapshot_bytes += metrics.source_snapshot_bytes;
                extended_span_rows += metrics.extended_span_rows;
                source_identity_slots += metrics.source_identity_slots;
            }
            let paths = identity.paths();
            let path_address = paths as *const _ as usize;
            if seen_path_tables.insert(path_address) {
                path_rows += paths.len();
                path_storage_bytes += paths.storage_bytes();
            }
        }
        (
            source_snapshot_bytes,
            extended_span_rows,
            source_identity_slots,
            path_rows,
            path_storage_bytes,
            seen_identities.len(),
            seen_path_tables.len(),
        )
    };
    let retention = FrozenRenderRetentionMetrics::from_reports(&reports);

    assert_eq!(reports.iter().count(), 1);
    assert_eq!(retention.diagnostic_records, 1);
    assert_eq!(
        retention.source_snapshot_bytes,
        expected_source_snapshot_bytes
    );
    assert_eq!(retention.extended_span_rows, expected_extended_span_rows);
    assert_eq!(
        retention.source_identity_slots,
        expected_source_identity_slots
    );
    assert_eq!(
        retention.retained_identity_contexts,
        retained_identity_count
    );
    assert_eq!(retention.path_table_count, retained_path_table_count);
    assert_eq!(retention.retained_identity_contexts, 2);
    assert_eq!(retention.path_table_count, 2);
    assert_eq!(
        retention.path_table_node_rows, expected_path_rows,
        "all path nodes retained by frozen diagnostic identities must be counted"
    );
    assert_eq!(
        retention.path_table_storage_bytes, expected_path_storage_bytes,
        "all path storage retained by frozen diagnostic identities must be counted"
    );
}

#[test]
fn published_warnings_capture_types_and_remap_paths_after_environment_drop() {
    use crate::build_system::create_project_modules::compiled_boundary::CompiledGraphBoundary;
    use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
    use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
    use crate::compiler_frontend::compiler_messages::TypeMismatchContext;
    use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::{
        BoundaryAccumulator, SuccessfulWarningBoundary,
    };
    use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::type_mismatch;
    use crate::compiler_frontend::compiler_messages::render::terse;
    use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder};

    // The build table interns "zeta.moth" first. The module fork then inherits that prefix and
    // interns "lib.moth" locally, while an earlier module's merge adds "alpha.moth" to the build
    // table before this module publishes. Publishing therefore needs a non-identity path remap
    // that must preserve the warning's original spelling.
    let mut string_table = StringTable::new();
    let mut path_interner = PathInternerBuilder::new();
    string_table.intern("zeta.moth");
    path_interner
        .try_intern_portable_path("zeta.moth", &mut string_table)
        .expect("the global warning path should intern");

    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("combined-publication-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );
    // Mirror one interleaved publication wave: this module's fork is taken first and interns
    // "lib.moth" into its local suffix, then an earlier sibling's merge adds "alpha.moth" to the
    // build tables at the index this module's local node expected. Publishing therefore needs a
    // genuinely non-identity path remap that must preserve the warning's original spelling.
    let mut compiled = semantic_result_with_local_path();
    let mut warnings = DiagnosticBag::new();
    compiled.module.metadata.materialisation_context = None;
    let string_base_len = string_table.len();
    let module_string_fork = string_table.fork_source().fork_for_module();
    let mut module_path_fork = path_interner.fork_source().fork_for_module();
    let mut module_string_table = module_string_fork.into_parts().0;
    let local_path = module_path_fork
        .try_intern_portable_path("lib.moth", &mut module_string_table)
        .expect("the module-local warning path should intern");
    let mut type_environment = TypeEnvironment::new();
    let type_name = module_string_table.intern("TPublishedWarning");
    let other_type_name = module_string_table.intern("TOtherPublishedWarning");
    let named_type = type_environment.register_synthetic_generic_parameter(type_name);
    let other_named_type = type_environment.register_synthetic_generic_parameter(other_type_name);
    compiled.module.executable.type_environment = type_environment;
    compiled.string_table = module_string_table;
    compiled.path_fork = module_path_fork;
    warnings.push(
        imports::missing_import_target(local_path, None).with_severity(DiagnosticSeverity::Warning),
    );
    warnings.push(
        type_mismatch(
            named_type,
            other_named_type,
            TypeMismatchContext::Assignment,
            None,
        )
        .with_severity(DiagnosticSeverity::Warning),
    );
    assert_ne!(
        local_path,
        PathId::ROOT,
        "the fixture must carry a local-only path id"
    );

    // The sibling fork inherits the same build prefix (both path and string tables), exactly
    // like a concurrent production worker. A fresh string table would mis-issue the sibling's
    // "alpha.moth" as StringId(0), aliasing the inherited "zeta.moth" in the base-lookup
    // domain and making the merge a silent no-op.
    let mut earlier_path_fork = path_interner.fork_source().fork_for_module();
    let mut earlier_string_table = string_table.fork_source().fork_for_module().into_parts().0;
    earlier_path_fork
        .try_intern_portable_path("alpha.moth", &mut earlier_string_table)
        .expect("the earlier module path should intern");
    let earlier_string_remap = string_table.merge_from(&earlier_string_table);
    path_interner
        .merge_delta_from(&earlier_path_fork, &earlier_string_remap)
        .expect("the earlier module path merge should succeed");
    let module_path_fork_base_len = compiled.path_fork.base_len();
    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let (frozen_string_table, frozen_path_table) = {
        let (string_table, mut warning_sink) = warning_owner.compiler_parts_mut();
        super::publish_compiled_module(
            &mut modules,
            &mut generated,
            &mut materialisations,
            &mut resource_inputs,
            ModuleId::from_index(0),
            &origin,
            compiled,
            warnings,
            Vec::new(),
            string_base_len,
            module_path_fork_base_len,
            string_table,
            &mut path_interner,
            Some(&mut warning_sink),
            SuccessfulWarningBoundary::Project,
        )
        .expect("non-identity publication should succeed");
        (
            Arc::new(string_table.clone().freeze()),
            Arc::new(path_interner.clone().freeze()),
        )
    };

    // Mirroring the canonical tail: after every publication the boundary installs one final
    // table built from the merged builder, so the executable reads paths from the shared Arc
    // instead of a per-module snapshot.
    let mut boundary = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(vec![(
            origin.clone(),
            PathBuf::new(),
            PathBuf::from("@page.moth"),
        )]),
        modules,
        generated,
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    drop(materialisations);
    boundary.install_boundary_identity(
        Arc::clone(&frozen_path_table),
        Arc::clone(&frozen_string_table),
    );
    let boundary = boundary.finish().expect("the test boundary should finish");
    warning_owner.commit_successful_warnings(SuccessfulWarningBoundary::Project, None);
    let warning_outcome = warning_owner
        .finish_successful_warnings(())
        .expect("successful warning reports should freeze");
    let warnings = match warning_outcome {
        CompilationOutcome::Success {
            warnings: Some(warnings),
            ..
        } => warnings,
        other => panic!("the successful module should retain its warning reports: {other:?}"),
    };
    assert_eq!(warnings.counts().warnings, 2);
    assert_eq!(warnings.counts().errors, 0);
    let (_, warning) = warnings
        .iter()
        .find(|(_, view)| {
            view.family_view::<crate::compiler_frontend::compiler_messages::diagnostics::families::imports::MissingImportTargetView>()
                .is_some()
        })
        .expect("the path warning should freeze into a report");
    assert_eq!(
        warning.severity(),
        crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Warning
    );
    let facts = warning
        .family_view::<crate::compiler_frontend::compiler_messages::diagnostics::families::imports::MissingImportTargetView>()
        .expect("frozen warning should retain its typed missing-target path");
    let remapped_path = facts.path;

    assert_ne!(
        remapped_path, local_path,
        "publication must remap the module-local warning path identity"
    );
    let rendered_path = frozen_path_table.render_portable_frozen(
        remapped_path,
        &frozen_string_table,
        &mut Vec::new(),
    );
    assert_eq!(rendered_path, "lib.moth");
    drop(boundary);

    // The module and its owning environment are dropped before the captured type warning renders.
    let rendered_messages = warnings
        .iter()
        .filter_map(|(report, view)| {
            view.family_view::<crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView>()?;
            Some(terse::format_terse_diagnostic_view(
                view,
                crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext::for_report(report),
            ))
        })
        .collect::<Vec<_>>();
    assert_eq!(rendered_messages.len(), 1);
    assert!(
        rendered_messages[0].contains("TPublishedWarning")
            && rendered_messages[0].contains("TOtherPublishedWarning"),
        "the successful module warning should retain real type names after its environment is dropped: {rendered_messages:?}",
    );
    assert!(
        !rendered_messages[0].contains("TypeId("),
        "a live module environment must not degrade typed warnings to raw IDs: {rendered_messages:?}",
    );
}
#[test]
fn generated_sidecar_warnings_flow_through_success_results() {
    use crate::build_system::build::ProjectCompilation;
    use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::UnknownValueNameView;
    use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};

    let mut render_warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let frontend = generated_sidecar_warning_frontend(&mut render_warning_owner);
    let mut frontend_value =
        match frontend.into_compilation_result(&mut render_warning_owner, None, None) {
            Ok(CompilationOutcome::Success {
                value,
                warnings: None,
            }) => value,
            _ => panic!("inner generated boundary should stage its warnings"),
        };
    let project_source_database = frontend_value.project_source_database.take();
    drop(frontend_value);
    let result = crate::build_system::build::finish_successful_project_warnings(
        &mut render_warning_owner,
        project_source_database,
        (),
    );
    let reports = match &result {
        Ok(CompilationOutcome::Success {
            warnings: Some(reports),
            ..
        }) => reports,
        _ => panic!("generated warnings should remain on success"),
    };
    let rendered_names = reports
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(report, view)| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("generated warning should retain its typed name fact");
            let context = DiagnosticRenderContext::for_report(report);
            context.string_table.resolve(facts.name).to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rendered_names,
        vec![
            "generated_project.moth".to_owned(),
            "generated_package.moth".to_owned(),
        ],
        "project and package warning order should remain stable",
    );
    assert_eq!(reports.counts().warnings, 2);
    assert_eq!(reports.counts().errors, 0);
    let rendered_codes = reports
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(_, view)| view.descriptor().code.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        rendered_codes.len(),
        2,
        "project and source-package generated sidecar warnings should render"
    );
    assert!(
        rendered_codes.windows(2).all(|pair| pair[0] == pair[1]),
        "rendered generated warnings should share one code: {rendered_codes:?}"
    );
    assert_eq!(
        terse::format_terse_compilation_result(&result).len(),
        2,
        "the successful result should render each generated warning"
    );

    let mut compilation_warning_owner =
        BoundaryAccumulator::with_root_string_table(StringTable::new());
    let frontend = generated_sidecar_warning_frontend(&mut compilation_warning_owner);
    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("generated sidecar boundaries should assemble");
    assert_eq!(
        compilation.module_count(),
        4,
        "success-only compilation should retain both base modules and generated sidecars"
    );
    drop(compilation);
    let compilation_result: CompilationResult<()> =
        crate::build_system::build::finish_successful_project_warnings(
            &mut compilation_warning_owner,
            None,
            (),
        );
    let warning_messages = match &compilation_result {
        Ok(CompilationOutcome::Success {
            warnings: Some(reports),
            ..
        }) => reports,
        _ => panic!("success-only compilation should publish warnings"),
    };
    let compilation_names = warning_messages
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(report, view)| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("compiled warning should retain its typed name fact");
            let context = DiagnosticRenderContext::for_report(report);
            context.string_table.resolve(facts.name).to_owned()
        })
        .collect::<Vec<_>>();
    let compilation_codes = warning_messages
        .iter_severity(DiagnosticSeverity::Warning)
        .map(|(_, view)| view.descriptor().code.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        compilation_codes.len(),
        2,
        "success-only compilation should retain project and source-package sidecar warnings"
    );
    assert_eq!(
        compilation_names, rendered_names,
        "success-only compilation warning order should match frontend rendering"
    );
    assert_eq!(
        compilation_codes, rendered_codes,
        "frontend rendering and success-only compilation must retain identical warning codes"
    );
    assert_eq!(warning_messages.counts().warnings, 2);
    assert_eq!(warning_messages.counts().errors, 0);
}

fn generated_sidecar_warning_frontend(
    warning_owner: &mut BoundaryAccumulator,
) -> ProjectFrontendCompilation {
    use crate::builder_surface::{BuilderSurface, PackageOrigin};
    use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;

    let mut project_path_interner = PathInternerBuilder::new();
    let project_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("generated-warning-project"),
        "@page.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let project_boundary = publish_generated_sidecar_boundary(
        warning_owner,
        &mut project_path_interner,
        project_origin,
        "@page.moth",
        SuccessfulWarningBoundary::Project,
        None,
        "generated_project.moth",
    );

    let package_identity = StablePackageIdentity::source_package(PackageOrigin::Builder, "warnpkg");
    let package_origin = StableModuleOriginIdentity::from_portable_path(
        package_identity.clone(),
        "@mod.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let mut package_path_interner = PathInternerBuilder::new();
    let _staged_package_boundary = publish_generated_sidecar_boundary(
        warning_owner,
        &mut package_path_interner,
        package_origin,
        "@mod.moth",
        SuccessfulWarningBoundary::SourcePackage(0),
        Some(package_identity),
        "generated_package.moth",
    );

    let _temp = tempfile::tempdir().expect("generated-warning project should be created");
    let project_root = _temp.path().to_path_buf();
    let source_root = project_root.join("src");
    let package_root = project_root.join("packages/warnpkg");
    std::fs::create_dir_all(&source_root).expect("project source root should be created");
    std::fs::create_dir_all(&package_root).expect("source-package root should be created");
    std::fs::write(
        project_root.join("config.moth"),
        "project #= (name = \"docs\", entry_root = \"src\")\nhtml #= ()\n",
    )
    .expect("project config should be written");
    std::fs::write(source_root.join("@page.moth"), "value = 1\n")
        .expect("project page should be written");
    std::fs::write(
        package_root.join("@mod.moth"),
        r#"shell #= [:<span>[$slot]</span>]
unused_insert #= [$insert("unused"): unused]

wrap type T |value T| -> String:
    return [shell: generated]
;

result = wrap(42)
io.line(result)
"#,
    )
    .expect("source-package module should be written");

    let mut config = Config::new(project_root.clone());
    config.project_name = "docs".to_owned();
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    builder_surface.source_packages.register_filesystem_root(
        "warnpkg",
        package_root,
        PackageOrigin::Builder,
    );
    let compiled_frontend = successful_compilation_value(
        crate::build_system::create_project_modules::compile_project_frontend(
            &mut config,
            crate::build_system::BuildProfile::Dev,
            None,
            &style_directives,
            &mut builder_surface,
            warning_owner,
        ),
    );
    let ProjectFrontendCompilation {
        project: compiled_project,
        source_packages,
        resource_inputs,
        transient_batches,
        project_source_database,
    } = compiled_frontend;
    drop(compiled_project);

    let mut project_boundary = project_boundary;
    project_boundary.install_boundary_identity(
        Arc::new(project_path_interner.freeze()),
        Arc::new(warning_owner.string_table().clone().freeze()),
    );
    let project_boundary = project_boundary
        .finish()
        .expect("published project boundary should finish");
    let mut frontend = ProjectFrontendCompilation::new(
        project_boundary,
        source_packages,
        resource_inputs,
        transient_batches,
    )
    .expect("published generated-warning frontend should validate");
    frontend.project_source_database = project_source_database;
    frontend
}

fn publish_generated_sidecar_boundary(
    warning_owner: &mut BoundaryAccumulator,
    path_interner: &mut PathInternerBuilder,
    origin: StableModuleOriginIdentity,
    module_path: &str,
    warning_boundary: SuccessfulWarningBoundary,
    package_identity: Option<StablePackageIdentity>,
    warning_name: &str,
) -> CompiledGraphBoundary {
    use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
    use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;

    let mut modules = ModuleArtifactStore::new(1);
    let mut generated = BoundaryGeneratedFunctionStore::default();
    let mut materialisations = ProviderMaterialisationRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();
    {
        let (string_table, mut warning_sink) = warning_owner.compiler_parts_mut();
        if let Some(package_identity) = package_identity {
            warning_sink.set_source_package_identity(warning_boundary, package_identity);
        }
        let string_table_base_len = string_table.len();
        let module_string_table = string_table.fork_source().fork_for_module().into_parts().0;
        let module_path_fork = path_interner.fork_source().fork_for_module();
        let path_base_len = module_path_fork.base_len();
        let mut compiled = semantic_result_with_local_path_in_domain(
            origin.clone(),
            module_string_table,
            module_path_fork,
        );
        compiled.module.metadata.entry_point = PathBuf::from(module_path);
        compiled.public_interface.module_origin = origin.clone();
        let mut generated_records = Vec::new();
        let mut generated_warning_bags = Vec::new();
        let sidecar_name = format!("{warning_name}_sidecar");
        let identity = generated_identity_for_origin(&sidecar_name, &origin, module_path);
        let summary = generated_summary();
        generated_records.push(CompletedGeneratedFunction {
            identity: identity.clone(),
            summary: summary.clone(),
            sidecar: generated_sidecar(identity, summary),
        });
        let mut sidecar_warnings = DiagnosticBag::new();
        sidecar_warnings.push(
            names_declarations::unknown_value_name(
                compiled.string_table.intern(warning_name),
                None,
            )
            .with_severity(DiagnosticSeverity::Warning),
        );
        generated_warning_bags.push(sidecar_warnings);
        compiled.generated_delta = GeneratedFunctionDelta::from_records(generated_records);

        super::publish_compiled_module(
            &mut modules,
            &mut generated,
            &mut materialisations,
            &mut resource_inputs,
            ModuleId::from_index(0),
            &origin,
            compiled,
            DiagnosticBag::new(),
            generated_warning_bags,
            string_table_base_len,
            path_base_len,
            string_table,
            path_interner,
            Some(&mut warning_sink),
            warning_boundary,
        )
        .expect("production module publication should capture generated warnings");
    }

    CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(vec![(
            origin,
            PathBuf::from(module_path),
            PathBuf::from(module_path),
        )]),
        modules,
        generated,
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    }
}

#[test]
fn premerge_failure_keeps_every_diagnosed_failure_beside_its_siblings() {
    // WHAT: a premerge boundary failure reports every diagnosed batch it carries.
    // WHY: retention failures from different owners are independent facts; dropping a sibling
    //      would hide a second user error behind the first.
    use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
    use crate::compiler_frontend::compiler_messages::{
        PreparedFailure, SourceSpanCapacityResource,
    };

    let string_table = StringTable::new();
    let failures = vec![
        PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(token_lexical::logical_path_capacity()),
            string_table.clone(),
            None,
            None,
        )),
        PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(token_lexical::source_span_capacity(
                0,
                u32::MAX,
                SourceSpanCapacityResource::Token,
                None,
            )),
            string_table.clone(),
            None,
            None,
        )),
    ];
    let reports = premerge_failure_to_reports(
        BoundaryPremergeFailure::project(failures),
        &mut BoundaryAccumulator::new(),
        None,
    )
    .expect("diagnosed premerge failures should freeze");
    assert_eq!(
        reports.counts().errors,
        2,
        "a premerge failure must keep every diagnosed batch beside its siblings"
    );
}
