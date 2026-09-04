//! WHAT: regression coverage for build-owned entry/package resource liveness and package roots.
//! WHY: these tests exercise the real `ProjectCompilation` assembly path with closed semantic
//! interfaces and retained link facts, so broad module scans or missing nominal roots fail at the
//! exact handoff where they would otherwise widen or shrink the selected resource set.

use crate::build_system::build::{
    ProjectAssemblyError, ProjectCompilation, validate_frontend_facade_boundaries,
};
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, CompiledSourcePackage, CompletedSourcePackageRegistry,
    ProjectFrontendCompilation,
};
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
use crate::build_system::create_project_modules::module_identity::{
    ModuleId, ModuleIdentityRecord,
};
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, ProjectContextEscapeReason, RuleDiagnosticKind,
};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedValue,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts_with_string_table;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ResolvedConstFragment,
};
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, Module, ModuleRootActivity,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicFunctionCategory, PublicFunctionSemantics, PublicReceiverMethodCategory,
    PublicReceiverMethodSemantics, PublicReturnTypeSlot, PublicSemanticInterface,
    PublicStructSemantics,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, OriginConstantId, OriginDeclarationId, OriginFunctionId, OriginTypeCategory,
    OriginTypeId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};

fn assert_project_context_diagnostic(
    error: ProjectAssemblyError,
    expected_reason: ProjectContextEscapeReason,
    expected_location: Option<(&str, CharPosition, CharPosition)>,
) {
    let mut string_table = StringTable::new();
    let messages = error.into_messages(&mut string_table);
    assert_eq!(messages.error_count(), 1);

    let diagnostics = messages.diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::ProjectContextEscape)
    );
    assert_eq!(diagnostic.identity().code, "MOTH-RULE-0086");
    let expected_reason_key = match expected_reason {
        ProjectContextEscapeReason::ExportedDeclaration => {
            "project_context_escape.exported_declaration"
        }
        ProjectContextEscapeReason::ReachableExecutable => {
            "project_context_escape.reachable_executable"
        }
    };
    assert_eq!(diagnostic.identity().reason_key, Some(expected_reason_key));
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::ProjectContextEscape { reason } if *reason == expected_reason
    ));
    let scope = diagnostic
        .primary_location
        .scope
        .to_portable_string(&messages.string_table);
    if let Some((expected_scope, expected_start, expected_end)) = expected_location {
        assert_eq!(scope, expected_scope);
        assert_eq!(diagnostic.primary_location.start_pos, expected_start);
        assert_eq!(diagnostic.primary_location.end_pos, expected_end);
    } else {
        assert!(
            !scope.is_empty(),
            "ProjectContext facade diagnostic must retain a source scope"
        );
    }
}

use std::path::{Path, PathBuf};
use std::sync::Arc;

fn resource(
    module_origin: &StableModuleOriginIdentity,
    logical_path: &str,
) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        module_origin.clone(),
        PortableResourcePath::from_portable_spelling(logical_path.to_owned())
            .expect("synthetic resource path should be valid"),
    )
}

fn map_synthetic_function_declaration_locations(
    hir: &mut HirModule,
    entry_point: &str,
    string_table: &mut StringTable,
) {
    let scope = SourceLocation::from_path(Path::new(entry_point), string_table).scope;
    for (index, function) in hir.functions.iter().enumerate() {
        let line_number = index as i32 + 1;
        let location = SourceLocation::new(
            scope.clone(),
            CharPosition {
                line_number,
                char_column: 1,
            },
            CharPosition {
                line_number,
                char_column: 8,
            },
        );
        hir.side_table.map_function(&location, function);
    }
}
/// Build one retained module with exact HIR link facts and optional resource-bearing functions.
///
/// WHAT: models only the lanes consumed by entry/package assembly: function origins, cross-module
/// calls, executable resource uses, and compile-time fragments. WHY: source parsing would obscure
/// the ownership boundary under test and would not let the fixture provide a closed facade surface.
fn synthetic_module(
    entry_point: &str,
    functions: Vec<(
        OriginFunctionId,
        Vec<CallTarget>,
        Option<StableResourceOriginId>,
    )>,
    start_function: Option<FunctionId>,
    const_fragments: Vec<StableResourceOriginId>,
) -> Module {
    let mut resource_table = ModuleResourceTable::new();
    let function_rows = functions
        .into_iter()
        .map(|(origin, calls, resource_origin)| {
            let resource_id = resource_origin
                .map(|origin| resource_table.intern_origin(origin, SourceLocation::default()));
            (origin, calls, resource_id)
        })
        .collect::<Vec<_>>();

    let mut hir = HirModule::new();
    hir.regions = vec![HirRegion::lexical(RegionId(0), None)];
    for (index, (origin, calls, resource_id)) in function_rows.iter().enumerate() {
        let function_id = FunctionId(index as u32);
        let block_id = BlockId(index as u32);
        let mut statements = Vec::with_capacity(calls.len() + usize::from(resource_id.is_some()));
        for (statement_index, target) in calls.iter().cloned().enumerate() {
            statements.push(HirStatement {
                id: HirNodeId((index * 100 + statement_index + 1) as u32),
                kind: HirStatementKind::Call {
                    target,
                    args: Vec::new(),
                    result: None,
                },
                location: SourceLocation::default(),
            });
        }
        if let Some(resource_id) = resource_id {
            statements.push(HirStatement {
                id: HirNodeId((index * 100 + calls.len() + 1) as u32),
                kind: HirStatementKind::Expr(HirExpression {
                    id: HirValueId((index * 100 + calls.len() + 1) as u32),
                    kind: HirExpressionKind::StructuralString {
                        pieces: vec![
                            crate::compiler_frontend::ast::const_values::store::ConstStringPiece::Resource(
                                *resource_id,
                            ),
                        ],
                    },
                    ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE,
                    value_kind: ValueKind::RValue,
                    region: RegionId(0),
                }),
                location: SourceLocation::default(),
            });
        }

        hir.blocks.push(HirBlock {
            id: block_id,
            region: RegionId(0),
            locals: Vec::new(),
            statements,
            terminator: HirTerminator::Return(HirExpression {
                id: HirValueId((index * 100 + 99) as u32),
                kind: HirExpressionKind::TupleConstruct {
                    elements: Vec::new(),
                },
                ty: crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE,
                value_kind: ValueKind::Const,
                region: RegionId(0),
            }),
        });
        hir.functions.push(HirFunction {
            id: function_id,
            entry: block_id,
            params: Vec::new(),
            return_type: crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE,
        });
        hir.function_origins.insert(
            function_id,
            if Some(function_id) == start_function {
                HirFunctionOrigin::EntryStart
            } else {
                HirFunctionOrigin::Normal
            },
        );
        hir.function_ids_by_origin
            .insert(origin.clone(), function_id);
    }
    hir.start_function = start_function;
    hir.function_provenance = hir
        .functions
        .iter()
        .map(|function| (function.id, SyntheticInterfaceProvenance::empty()))
        .collect();

    let mut function_string_table = StringTable::new();
    map_synthetic_function_declaration_locations(&mut hir, entry_point, &mut function_string_table);
    let function_link_facts =
        collect_module_function_link_facts_with_string_table(&hir, &function_string_table)
            .expect("synthetic HIR should produce function link facts");
    Module {
        executable: ModuleExecutable {
            hir,
            resource_table,
            type_environment:
                crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::from(entry_point),
            warnings: Vec::new(),
            const_top_level_fragments: const_fragments
                .into_iter()
                .enumerate()
                .map(|(runtime_insertion_index, origin)| ResolvedConstFragment {
                    runtime_insertion_index,
                    location: SourceLocation::default(),
                    value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(
                        origin,
                    )]),
                })
                .collect(),
            root_activity: ModuleRootActivity {
                has_non_trivial_root_body: start_function.is_some(),
                const_fragment_count: usize::from(!function_rows.is_empty()),
                ..ModuleRootActivity::default()
            },
            doc_fragments: Vec::new(),
            materialisation_context: None,
        },
    }
}

fn refresh_synthetic_function_link_facts(module: &mut Module) {
    let entry_point = module.metadata.entry_point.to_string_lossy().into_owned();
    let mut function_string_table = StringTable::new();
    map_synthetic_function_declaration_locations(
        &mut module.executable.hir,
        &entry_point,
        &mut function_string_table,
    );
    module.link_facts.functions = collect_module_function_link_facts_with_string_table(
        &module.executable.hir,
        &function_string_table,
    )
    .expect("synthetic HIR should refresh function link facts");
}

fn empty_interface(module_origin: StableModuleOriginIdentity) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin,
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

fn artifact(module: Module, interface: PublicSemanticInterface) -> CompiledModuleArtifact {
    CompiledModuleArtifact { module, interface }
}

fn boundary(
    graph: ProjectModuleGraph,
    artifacts: Vec<CompiledModuleArtifact>,
) -> CompiledGraphBoundary {
    let mut store = ModuleArtifactStore::new(artifacts.len());
    for (index, artifact) in artifacts.into_iter().enumerate() {
        store
            .publish_success(ModuleId::from_index(index), artifact)
            .expect("synthetic artifact should publish into its graph slot");
    }
    CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    }
}

fn module_record(
    root_directory: &str,
    root_file: &str,
    role: ModuleRootRole,
    logical_module_path: &str,
    package: &StablePackageIdentity,
) -> ModuleIdentityRecord {
    ModuleIdentityRecord::new(
        PathBuf::from(root_directory),
        PathBuf::from(root_file),
        role,
        PathBuf::from(logical_module_path),
        package,
    )
    .expect("synthetic module identity should be valid")
}

fn const_declaration(
    module_origin: &StableModuleOriginIdentity,
    name: &str,
    resource_origin: StableResourceOriginId,
) -> PublicDeclarationRecord {
    PublicDeclarationRecord {
        origin: OriginDeclarationId::Constant(OriginConstantId::new(
            module_origin.clone(),
            name.to_owned(),
        )),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
            folded_value: PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Resource(resource_origin),
            ])),
        }),
    }
}

fn exported_binding(
    facade_origin: &StableModuleOriginIdentity,
    public_name: &str,
    origin: OriginDeclarationId,
) -> ExportBinding {
    ExportBinding::new(facade_origin.clone(), public_name.to_owned(), origin)
}

#[test]
fn package_facade_rejects_project_context_declaration_provenance() {
    let package = StablePackageIdentity::project_local("assembly-provenance-declaration");
    let facade_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "".to_owned(),
        ModuleRootRole::ProjectPackageFacade,
    );
    let declaration_origin = OriginConstantId::new(facade_origin.clone(), "config".to_owned());
    let declaration_resource = resource(&facade_origin, "assets/config.svg");
    let mut declaration = const_declaration(&facade_origin, "config", declaration_resource);
    declaration.synthetic_interface_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "project",
            "config",
        ));
    let mut facade_interface = empty_interface(facade_origin.clone());
    facade_interface.export_bindings = vec![exported_binding(
        &facade_origin,
        "config",
        OriginDeclarationId::Constant(declaration_origin),
    )];
    facade_interface.declarations = vec![declaration];

    let graph = ProjectModuleGraph::from_test_records(vec![module_record(
        "/synthetic/provenance-declaration",
        "/synthetic/provenance-declaration/+package.moth",
        ModuleRootRole::ProjectPackageFacade,
        "",
        &package,
    )]);
    let project_boundary = boundary(
        graph,
        vec![artifact(
            synthetic_module("+package.moth", Vec::new(), None, Vec::new()),
            facade_interface,
        )],
    );

    let frontend = ProjectFrontendCompilation::new(
        project_boundary,
        CompletedSourcePackageRegistry::new(),
        ResourceInputRegistry::new(),
    )
    .expect("synthetic frontend should satisfy retained-boundary invariants");
    let check_error = validate_frontend_facade_boundaries(&frontend)
        .expect_err("check facade validation should reject ProjectContext provenance");
    assert_project_context_diagnostic(
        check_error,
        ProjectContextEscapeReason::ExportedDeclaration,
        None,
    );

    let ProjectFrontendCompilation {
        project,
        source_packages,
        resource_inputs,
        ..
    } = frontend;
    let build_error = match ProjectCompilation::from_successful_boundaries(
        project,
        source_packages,
        resource_inputs,
    ) {
        Ok(_) => {
            panic!("project facade assembly should reject ProjectContext declaration provenance")
        }
        Err(error) => error,
    };
    assert_project_context_diagnostic(
        build_error,
        ProjectContextEscapeReason::ExportedDeclaration,
        None,
    );
}

#[test]
fn source_package_facade_rejects_project_context_reachable_private_helper() {
    let package = StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "external");
    let root_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "external/@package.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let child_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "external/lib/@lib.moth".to_owned(),
        ModuleRootRole::Support,
    );
    let exported_origin = OriginFunctionId::new_free(child_origin.clone(), "public".to_owned());
    let private_identity = ModulePrivateExecutableIdentity::new(
        child_origin.clone(),
        "@lib.moth".to_owned(),
        ModulePrivateExecutableCategory::FreeFunction,
        "helper".to_owned(),
        None,
    );
    let helper_origin = OriginFunctionId::new_free(child_origin.clone(), "helper".to_owned());

    let mut child_module = synthetic_module(
        "external/lib/@lib.moth",
        vec![
            (
                exported_origin.clone(),
                vec![CallTarget::ModulePrivate(private_identity.clone())],
                None,
            ),
            (helper_origin, Vec::new(), None),
        ],
        None,
        Vec::new(),
    );
    child_module
        .executable
        .hir
        .function_ids_by_private_origin
        .insert(private_identity, FunctionId(1));
    child_module.executable.hir.function_provenance.insert(
        FunctionId(1),
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "project",
            "helper",
        )),
    );
    refresh_synthetic_function_link_facts(&mut child_module);

    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(exported_origin.clone()),
        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: Vec::new(),
            returns: Vec::new(),
            error_return: None,
        }),
    };
    let root_interface = PublicSemanticInterface {
        module_origin: root_origin.clone(),
        export_bindings: vec![exported_binding(
            &root_origin,
            "public",
            OriginDeclarationId::Function(exported_origin),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![declaration],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let package_graph = ProjectModuleGraph::from_test_records(vec![
        module_record(
            "/synthetic/external",
            "/synthetic/external/@package.moth",
            ModuleRootRole::Normal,
            "external/@package.moth",
            &package,
        ),
        module_record(
            "/synthetic/external/lib",
            "/synthetic/external/lib/@lib.moth",
            ModuleRootRole::Support,
            "external/lib/@lib.moth",
            &package,
        ),
    ]);
    let package_boundary = boundary(
        package_graph,
        vec![
            artifact(
                synthetic_module("external/@package.moth", Vec::new(), None, Vec::new()),
                root_interface,
            ),
            artifact(child_module, empty_interface(child_origin)),
        ],
    );
    let mut source_packages = CompletedSourcePackageRegistry::new();
    source_packages
        .publish(
            CompiledSourcePackage {
                package_identity: package,
                root_module_id: ModuleId::from_index(0),
                boundary: package_boundary,
            },
            &[],
        )
        .expect("source package should publish before external-facade validation");

    let result = ProjectCompilation::from_successful_boundaries(
        boundary(
            ProjectModuleGraph::from_normal_roots(Vec::new()),
            Vec::new(),
        ),
        source_packages,
        ResourceInputRegistry::new(),
    );
    let error = match result {
        Ok(_) => panic!("source-package facade should reject ProjectContext helper provenance"),
        Err(error) => error,
    };
    assert_project_context_diagnostic(
        error,
        ProjectContextEscapeReason::ReachableExecutable,
        Some((
            "external/lib/@lib.moth",
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 8,
            },
        )),
    );
}

#[test]
fn source_package_facade_rejects_project_context_public_declaration() {
    let package = StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "direct");
    let root_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "direct/@package.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let constant_origin = OriginConstantId::new(root_origin.clone(), "config".to_owned());
    let mut declaration = const_declaration(
        &root_origin,
        "config",
        resource(&root_origin, "assets/config.svg"),
    );
    declaration.synthetic_interface_provenance =
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "project",
            "config",
        ));
    let root_interface = PublicSemanticInterface {
        module_origin: root_origin.clone(),
        export_bindings: vec![exported_binding(
            &root_origin,
            "config",
            OriginDeclarationId::Constant(constant_origin),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![declaration],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let package_graph = ProjectModuleGraph::from_test_records(vec![module_record(
        "/synthetic/direct",
        "/synthetic/direct/@package.moth",
        ModuleRootRole::Normal,
        "direct/@package.moth",
        &package,
    )]);
    let package_boundary = boundary(
        package_graph,
        vec![artifact(
            synthetic_module("direct/@package.moth", Vec::new(), None, Vec::new()),
            root_interface,
        )],
    );
    let mut source_packages = CompletedSourcePackageRegistry::new();
    source_packages
        .publish(
            CompiledSourcePackage {
                package_identity: package,
                root_module_id: ModuleId::from_index(0),
                boundary: package_boundary,
            },
            &[],
        )
        .expect("source package should publish before external-facade validation");

    let result = ProjectCompilation::from_successful_boundaries(
        boundary(
            ProjectModuleGraph::from_normal_roots(Vec::new()),
            Vec::new(),
        ),
        source_packages,
        ResourceInputRegistry::new(),
    );
    let error = match result {
        Ok(_) => {
            panic!("source-package facade should reject ProjectContext declaration provenance")
        }
        Err(error) => error,
    };
    assert_project_context_diagnostic(error, ProjectContextEscapeReason::ExportedDeclaration, None);
}

#[test]
fn entry_union_keeps_entry_fragment_and_excludes_linked_module_fragment() {
    let project_package = StablePackageIdentity::project_local("assembly-entry-project");
    let provider_package =
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "provider");
    let page_origin = StableModuleOriginIdentity::from_portable_path(
        project_package.clone(),
        "src/@page.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let provider_module_origin = StableModuleOriginIdentity::from_portable_path(
        provider_package.clone(),
        "provider/@provider.moth".to_owned(),
        ModuleRootRole::Normal,
    );
    let helper_origin =
        OriginFunctionId::new_free(provider_module_origin.clone(), "helper".to_owned());
    let entry_resource = resource(&page_origin, "assets/entry.svg");
    let linked_resource = resource(&provider_module_origin, "assets/linked.svg");

    let project_graph = ProjectModuleGraph::from_normal_roots(vec![(
        page_origin.clone(),
        PathBuf::from("src"),
        PathBuf::from("src/@page.moth"),
    )]);
    let page_module = synthetic_module(
        "@page.moth",
        vec![(
            OriginFunctionId::new_free(page_origin.clone(), "page_start".to_owned()),
            vec![CallTarget::CrossModule(helper_origin.clone())],
            None,
        )],
        Some(FunctionId(0)),
        vec![entry_resource.clone()],
    );
    let project_boundary = boundary(
        project_graph,
        vec![artifact(page_module, empty_interface(page_origin))],
    );

    let package_graph = ProjectModuleGraph::from_normal_roots(vec![(
        provider_module_origin.clone(),
        PathBuf::from("provider"),
        PathBuf::from("provider/@provider.moth"),
    )]);
    let provider_module = synthetic_module(
        "provider/@provider.moth",
        vec![(helper_origin, Vec::new(), None)],
        None,
        vec![linked_resource],
    );
    let provider_boundary = boundary(
        package_graph,
        vec![artifact(
            provider_module,
            empty_interface(provider_module_origin.clone()),
        )],
    );
    let mut source_packages = CompletedSourcePackageRegistry::new();
    source_packages
        .publish(
            CompiledSourcePackage {
                package_identity: provider_package,
                root_module_id: ModuleId::from_index(0),
                boundary: provider_boundary,
            },
            &[],
        )
        .expect("synthetic provider package should publish");

    let compilation = ProjectCompilation::from_successful_boundaries(
        project_boundary,
        source_packages,
        ResourceInputRegistry::new(),
    )
    .expect("synthetic entry boundaries should assemble");
    let entry = compilation
        .entries()
        .into_iter()
        .next()
        .expect("the active page should create one project entry");
    let entry_paths = entry
        .resource_union
        .iter()
        .map(|origin| origin.logical_path().as_str().to_owned())
        .collect::<Vec<_>>();

    assert!(
        entry_paths.iter().any(|path| path == "assets/entry.svg"),
        "entry-owned fragment resource should remain live: {entry_paths:?}"
    );
    assert!(
        !entry_paths.iter().any(|path| path == "assets/linked.svg"),
        "linked module const-fragment resource must not leak into entry union: {entry_paths:?}"
    );
}

#[test]
fn package_facade_rejects_project_context_reachable_private_helper() {
    let package = StablePackageIdentity::project_local("assembly-provenance-helper");
    let facade_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "".to_owned(),
        ModuleRootRole::ProjectPackageFacade,
    );
    let child_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "provider/@provider.moth".to_owned(),
        ModuleRootRole::Support,
    );
    let exported_origin = OriginFunctionId::new_free(child_origin.clone(), "exported".to_owned());
    let private_identity = ModulePrivateExecutableIdentity::new(
        child_origin.clone(),
        "@provider.moth".to_owned(),
        ModulePrivateExecutableCategory::FreeFunction,
        "helper".to_owned(),
        None,
    );
    let helper_origin = OriginFunctionId::new_free(child_origin.clone(), "helper".to_owned());

    let mut child_module = synthetic_module(
        "provider/+provider.moth",
        vec![
            (
                exported_origin.clone(),
                vec![CallTarget::ModulePrivate(private_identity.clone())],
                None,
            ),
            (helper_origin, Vec::new(), None),
        ],
        None,
        Vec::new(),
    );
    child_module
        .executable
        .hir
        .function_ids_by_private_origin
        .insert(private_identity.clone(), FunctionId(1));
    child_module.executable.hir.function_provenance.insert(
        FunctionId(1),
        SyntheticInterfaceProvenance::single(SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "project",
            "helper",
        )),
    );
    refresh_synthetic_function_link_facts(&mut child_module);
    let declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(exported_origin.clone()),
        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: Vec::new(),
            returns: Vec::new(),
            error_return: None,
        }),
    };
    let mut facade_interface = empty_interface(facade_origin.clone());
    facade_interface.export_bindings = vec![exported_binding(
        &facade_origin,
        "exported",
        OriginDeclarationId::Function(exported_origin),
    )];
    facade_interface.declarations = vec![declaration];

    let graph = ProjectModuleGraph::from_test_records(vec![
        module_record(
            "/synthetic/provenance-helper",
            "/synthetic/provenance-helper/+package.moth",
            ModuleRootRole::ProjectPackageFacade,
            "",
            &package,
        ),
        module_record(
            "/synthetic/provenance-helper/provider",
            "/synthetic/provenance-helper/provider/+provider.moth",
            ModuleRootRole::Support,
            "provider/@provider.moth",
            &package,
        ),
    ]);
    let project_boundary = boundary(
        graph,
        vec![
            artifact(
                synthetic_module("+package.moth", Vec::new(), None, Vec::new()),
                facade_interface,
            ),
            artifact(child_module, empty_interface(child_origin)),
        ],
    );

    let result = ProjectCompilation::from_successful_boundaries(
        project_boundary,
        CompletedSourcePackageRegistry::new(),
        ResourceInputRegistry::new(),
    );
    let error = match result {
        Ok(_) => panic!("project facade should reject ProjectContext helper provenance"),
        Err(error) => error,
    };
    assert_project_context_diagnostic(
        error,
        ProjectContextEscapeReason::ReachableExecutable,
        Some((
            "provider/+provider.moth",
            CharPosition {
                line_number: 2,
                char_column: 1,
            },
            CharPosition {
                line_number: 2,
                char_column: 8,
            },
        )),
    );
}

#[test]
fn package_union_follows_facade_export_and_excludes_hidden_child_export() {
    let package = StablePackageIdentity::project_local("assembly-package-project");
    let facade_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "".to_owned(),
        ModuleRootRole::ProjectPackageFacade,
    );
    let child_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "provider/@provider.moth".to_owned(),
        ModuleRootRole::Support,
    );
    let exported_origin = OriginConstantId::new(child_origin.clone(), "A".to_owned());
    let hidden_origin = OriginConstantId::new(child_origin.clone(), "B".to_owned());
    let exported_resource = resource(&child_origin, "assets/a.svg");
    let hidden_resource = resource(&child_origin, "assets/b.svg");

    let facade_interface = PublicSemanticInterface {
        module_origin: facade_origin.clone(),
        export_bindings: vec![exported_binding(
            &facade_origin,
            "A",
            OriginDeclarationId::Constant(exported_origin.clone()),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![const_declaration(
            &child_origin,
            "A",
            exported_resource.clone(),
        )],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };
    let child_interface = PublicSemanticInterface {
        module_origin: child_origin.clone(),
        export_bindings: vec![
            exported_binding(
                &child_origin,
                "A",
                OriginDeclarationId::Constant(exported_origin),
            ),
            exported_binding(
                &child_origin,
                "B",
                OriginDeclarationId::Constant(hidden_origin),
            ),
        ],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![
            const_declaration(&child_origin, "A", exported_resource),
            const_declaration(&child_origin, "B", hidden_resource),
        ],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let graph = ProjectModuleGraph::from_test_records(vec![
        module_record(
            "/synthetic/project",
            "/synthetic/project/+package.moth",
            ModuleRootRole::ProjectPackageFacade,
            "",
            &package,
        ),
        module_record(
            "/synthetic/project/provider",
            "/synthetic/project/provider/+provider.moth",
            ModuleRootRole::Support,
            "provider/@provider.moth",
            &package,
        ),
    ]);
    let project_boundary = boundary(
        graph,
        vec![
            artifact(
                synthetic_module("+package.moth", Vec::new(), None, Vec::new()),
                facade_interface,
            ),
            artifact(
                synthetic_module("provider/+provider.moth", Vec::new(), None, Vec::new()),
                child_interface,
            ),
        ],
    );
    let compilation = ProjectCompilation::from_successful_boundaries(
        project_boundary,
        CompletedSourcePackageRegistry::new(),
        ResourceInputRegistry::new(),
    )
    .expect("synthetic package boundary should assemble");
    let paths = compilation
        .package_assembly()
        .expect("facade should produce package assembly")
        .resource_union()
        .iter()
        .map(|origin| origin.logical_path().as_str().to_owned())
        .collect::<Vec<_>>();

    assert!(
        paths.iter().any(|path| path == "assets/a.svg"),
        "facade-reexported child resource should be selected: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path == "assets/b.svg"),
        "child export hidden by the facade must remain absent: {paths:?}"
    );
}

#[test]
fn package_union_roots_receiver_method_from_facade_closed_nominal() {
    let package = StablePackageIdentity::project_local("assembly-nominal-project");
    let facade_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "".to_owned(),
        ModuleRootRole::ProjectPackageFacade,
    );
    let child_origin = StableModuleOriginIdentity::from_portable_path(
        package.clone(),
        "provider/@provider.moth".to_owned(),
        ModuleRootRole::Support,
    );
    let card_origin = OriginTypeId::new(
        child_origin.clone(),
        "Card".to_owned(),
        OriginTypeCategory::Struct,
    );
    let make_origin = OriginFunctionId::new_free(child_origin.clone(), "make".to_owned());
    let render_origin = OriginFunctionId::new_receiver(
        child_origin.clone(),
        "render".to_owned(),
        card_origin.clone(),
    );
    let card_resource = resource(&child_origin, "assets/card.svg");

    let card_declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(card_origin.clone()),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: Vec::new(),
            fields: Vec::new(),
            receiver_methods: vec![PublicReceiverMethodSemantics {
                method_origin: render_origin.clone(),
                category: PublicReceiverMethodCategory::ConcreteLocal,
                parameters: Vec::new(),
                returns: vec![PublicReturnTypeSlot {
                    type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
                }],
                error_return: None,
            }],
        }),
    };
    let make_declaration = PublicDeclarationRecord {
        origin: OriginDeclarationId::Function(make_origin.clone()),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Function(PublicFunctionSemantics {
            category: PublicFunctionCategory::ConcreteLocal,
            parameters: Vec::new(),
            returns: vec![PublicReturnTypeSlot {
                type_identity: CanonicalTypeIdentity::SourceNominal(card_origin),
            }],
            error_return: None,
        }),
    };
    let mut declarations = vec![card_declaration, make_declaration];
    declarations.sort_by(|left, right| left.origin.cmp(&right.origin));
    let facade_interface = PublicSemanticInterface {
        module_origin: facade_origin.clone(),
        export_bindings: vec![exported_binding(
            &facade_origin,
            "make",
            OriginDeclarationId::Function(make_origin.clone()),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations,
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let graph = ProjectModuleGraph::from_test_records(vec![
        module_record(
            "/synthetic/nominal",
            "/synthetic/nominal/+package.moth",
            ModuleRootRole::ProjectPackageFacade,
            "",
            &package,
        ),
        module_record(
            "/synthetic/nominal/provider",
            "/synthetic/nominal/provider/+provider.moth",
            ModuleRootRole::Support,
            "provider/@provider.moth",
            &package,
        ),
    ]);
    let child_module = synthetic_module(
        "provider/+provider.moth",
        vec![
            (make_origin, Vec::new(), None),
            (render_origin, Vec::new(), Some(card_resource.clone())),
        ],
        None,
        Vec::new(),
    );
    let project_boundary = boundary(
        graph,
        vec![
            artifact(
                synthetic_module("+package.moth", Vec::new(), None, Vec::new()),
                facade_interface,
            ),
            artifact(child_module, empty_interface(child_origin)),
        ],
    );
    let compilation = ProjectCompilation::from_successful_boundaries(
        project_boundary,
        CompletedSourcePackageRegistry::new(),
        ResourceInputRegistry::new(),
    )
    .expect("synthetic nominal package should assemble");
    let paths = compilation
        .package_assembly()
        .expect("facade should produce package assembly")
        .resource_union()
        .iter()
        .map(|origin| origin.logical_path().as_str().to_owned())
        .collect::<Vec<_>>();

    assert!(
        paths.iter().any(|path| path == "assets/card.svg"),
        "receiver method reachable from the facade's closed nominal should be rooted: {paths:?}"
    );
}
