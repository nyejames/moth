//! Focused invariant tests for the compiled `Module` lane container.

use crate::build_system::build::ProjectCompilation;
use crate::build_system::create_project_modules::compiled_boundary::{
    BlockedModule, BlockedProvider, CompiledGraphBoundary, CompiledSourcePackage,
    CompletedSourcePackageRegistry, DiagnosedModule, ProjectFrontendCompilation,
};
use crate::build_system::create_project_modules::generated_store::BoundaryGeneratedFunctionStore;
use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, ReactiveInvalidationFact, ReactiveInvalidationKind,
};
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, NameNamespace,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalFunctionId, ExternalPackageId, ExternalPackageRegistry,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::reactivity::ReactiveSourceId;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
};
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, CompletedGeneratedFunction, GeneratedFunctionSidecar, Module,
    ModuleExternalImport, ModuleRootActivity,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

use std::path::PathBuf;
use std::sync::Arc;

/// Build the smallest valid HIR module with one entry start function, binding its name to a
/// caller-supplied interned path in the caller-owned string table.
fn minimal_hir_module(start_name_path: InternedPath) -> HirModule {
    let mut module = HirModule::new();
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.blocks = vec![HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(HirExpression {
            id: HirValueId(0),
            kind: HirExpressionKind::TupleConstruct { elements: vec![] },
            ty: NONE,
            value_kind: ValueKind::Const,
            region: RegionId(0),
        }),
    }];
    module.functions = vec![HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: NONE,
    }];
    module.start_function = Some(FunctionId(0));
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    module
        .side_table
        .bind_function_name(FunctionId(0), start_name_path);
    module
}

#[test]
fn remap_string_ids_routes_hir_and_link_fact_locations_through_their_lanes() {
    // WHAT: a module remaps both executable HIR names and diagnostic locations retained by
    //       per-function link facts, while resolved runtime asset paths remain unchanged.
    // WHY: module-local link facts feed later target diagnostics after build-table merging, so
    //      their source scopes must not retain worker-local string IDs.

    let mut local_string_table = StringTable::new();
    let start_name_path = InternedPath::from_single_str("start_entry", &mut local_string_table);

    let source_scope = InternedPath::from_single_str("source.moth", &mut local_string_table);
    let reactive_scope =
        InternedPath::from_single_str("reactive_source.moth", &mut local_string_table);
    let option_value_name = local_string_table.intern("value");
    let mut hir_module = minimal_hir_module(start_name_path);
    hir_module.blocks[0].statements.push(HirStatement {
        id: HirNodeId(1),
        kind: HirStatementKind::Expr(HirExpression {
            id: HirValueId(1),
            kind: HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: 1,
                fields: vec![HirVariantField {
                    name: Some(option_value_name),
                    value: HirExpression {
                        id: HirValueId(2),
                        kind: HirExpressionKind::Int(7),
                        ty: NONE,
                        value_kind: ValueKind::Const,
                        region: RegionId(0),
                    },
                }],
            },
            ty: NONE,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
        }),
        location: SourceLocation::new(
            source_scope.clone(),
            CharPosition {
                line_number: 3,
                char_column: 2,
            },
            CharPosition {
                line_number: 3,
                char_column: 8,
            },
        ),
    });
    hir_module.blocks[0].statements.push(HirStatement {
        id: HirNodeId(2),
        kind: HirStatementKind::Expr(HirExpression {
            id: HirValueId(3),
            kind: HirExpressionKind::MapLiteral(vec![]),
            ty: NONE,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
        }),
        location: SourceLocation::new(
            source_scope,
            CharPosition {
                line_number: 4,
                char_column: 2,
            },
            CharPosition {
                line_number: 4,
                char_column: 8,
            },
        ),
    });

    // Seed the merged table so the local "start_entry" id shifts during merge, proving the remap
    // is actually applied rather than being an identity no-op.
    let mut merged_string_table = StringTable::new();
    merged_string_table.intern("prefix");
    let remap = merged_string_table.merge_from(&local_string_table);
    assert!(
        !remap.is_identity(),
        "test remap must shift the local string id"
    );

    let asset_path = PathBuf::from("assets/drawing.js");
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .expect("test HIR should produce function link facts");
    let link_facts = ModuleLinkFacts {
        external_package_registry: Arc::new(ExternalPackageRegistry::new()),
        external_import_candidates: vec![ModuleExternalImport {
            package_id: ExternalPackageId(11),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: asset_path.clone(),
                asset_kind: String::from("js"),
            }),
            required_runtime_imports: vec![],
        }],
        functions: function_link_facts,
    };

    let entry_point = PathBuf::from("src/@page.moth");
    let mut borrow_analysis = BorrowCheckReport::default();
    borrow_analysis.analysis.reactive_invalidations.insert(
        HirNodeId(1),
        vec![ReactiveInvalidationFact {
            statement_id: HirNodeId(1),
            source: ReactiveSourceId(0),
            kind: ReactiveInvalidationKind::Assignment,
            location: SourceLocation::new(
                reactive_scope,
                CharPosition {
                    line_number: 8,
                    char_column: 4,
                },
                CharPosition {
                    line_number: 8,
                    char_column: 10,
                },
            ),
        }],
    );

    let mut module = Module {
        executable: ModuleExecutable {
            hir: hir_module,
            type_environment: TypeEnvironment::new(),
            borrow_analysis,
        },
        link_facts,
        metadata: ModuleCompilerMetadata {
            entry_point: entry_point.clone(),
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: ModuleRootActivity::default(),
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            materialisation_context: None,
        },
    };

    module.remap_string_ids(&remap);

    // The executable lane remapped the bound HIR name into the merged table exactly once.
    let resolved_name = module
        .executable
        .hir
        .side_table
        .function_name_path(FunctionId(0))
        .expect("start function name should be bound")
        .name_str(&merged_string_table);
    assert_eq!(resolved_name, Some("start_entry"));

    let statement = &module.executable.hir.blocks[0].statements[0];
    assert_eq!(
        statement.location.scope.name_str(&merged_string_table),
        Some("source.moth"),
        "executable statement locations should resolve through the merged string table"
    );
    let HirStatementKind::Expr(HirExpression {
        kind: HirExpressionKind::VariantConstruct { fields, .. },
        ..
    }) = &statement.kind
    else {
        panic!("test statement should retain its option construction");
    };
    assert_eq!(
        fields[0].name.map(|name| merged_string_table.resolve(name)),
        Some("value"),
        "variant payload names should resolve through the merged string table"
    );

    let reactive_location = &module
        .executable
        .borrow_analysis
        .analysis
        .reactive_invalidations[&HirNodeId(1)][0]
        .location;
    assert_eq!(
        reactive_location.scope.name_str(&merged_string_table),
        Some("reactive_source.moth"),
        "borrow-fact locations should resolve through the merged string table"
    );

    let reachability = collect_reachability_from_function_link_facts(
        &module.link_facts.functions,
        &[FunctionId(0)],
    )
    .expect("remapped function facts should remain linkable");
    let map_location = &reachability.reachable_map_uses[0].location;
    assert_eq!(
        map_location.scope.name_str(&merged_string_table),
        Some("source.moth"),
        "link-fact location should resolve through the merged string table"
    );

    // Runtime asset identity remains filesystem-owned rather than string-table-owned.
    let import = &module.link_facts.external_import_candidates[0];
    assert_eq!(import.package_id, ExternalPackageId(11));
    assert_eq!(
        import.runtime_asset.as_ref().unwrap().canonical_source_path,
        asset_path
    );

    // The metadata entry path is a PathBuf, not interned, so it is preserved.
    assert_eq!(module.metadata.entry_point, entry_point);
}

#[test]
fn entry_assembly_rejects_reachable_external_function_without_package_owner() {
    let mut hir_module = minimal_hir_module(InternedPath::new());
    hir_module.blocks[0].statements.push(HirStatement {
        id: HirNodeId(99),
        kind: HirStatementKind::Call {
            target: CallTarget::External(ExternalFunctionId::Synthetic(99_999)),
            args: vec![],
            result: None,
        },
        location: SourceLocation::default(),
    });
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .expect("test HIR should produce function link facts");
    let module = Module {
        executable: ModuleExecutable {
            hir: hir_module,
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: vec![],
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::from("@page.moth"),
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: ModuleRootActivity {
                has_non_trivial_root_body: true,
                ..ModuleRootActivity::default()
            },
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            materialisation_context: None,
        },
    };

    let error = match crate::build_system::test_support::project_compilation_from_test_modules(
        vec![module],
    ) {
        Ok(_) => panic!("missing external package ownership should violate entry assembly"),
        Err(error) => error,
    };
    assert!(error.msg.contains("has no owning package"));
}

#[test]
fn frontend_compilation_retains_project_artefact_interfaces() {
    // WHAT: a successful project artefact keeps its published interface after the frontend
    //       handoff instead of being flattened into a bare `Module`.
    // WHY: R5C1 retains completed artefacts so interface closure, fingerprints and link owners
    //      can consume them after `compile_project_frontend` returns.
    let module = minimal_lane_module(PathBuf::from("@page.moth"), true);
    let frontend = test_frontend_from_project_modules(vec![module]);
    let artifact = frontend
        .project
        .modules
        .artifact(ModuleId::from_index(0))
        .expect("valid module id")
        .expect("project module should be successful");
    assert!(
        artifact
            .module
            .metadata
            .root_activity
            .has_html_artifact_activity(),
        "project artefact root activity is retained"
    );
    assert_eq!(
        artifact.interface.module_origin.role(),
        ModuleRootRole::Normal,
        "artefact interface survives the handoff"
    );

    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("one active project module assembles one entry");
    assert_eq!(compilation.module_count(), 1, "one base module");
    let retained = compilation
        .project
        .modules
        .artifact(ModuleId::from_index(0))
        .expect("valid module id")
        .expect("success-only compilation retains the artefact");
    assert_eq!(
        retained.interface.module_origin.role(),
        ModuleRootRole::Normal,
        "completed public interface remains accessible after success-only conversion"
    );
    assert_eq!(
        compilation.entries().len(),
        1,
        "normal root creates one entry"
    );
}

#[test]
fn source_package_artefacts_are_retained_but_never_project_entries() {
    // WHAT: source-package artefacts are retained immutably (root activity intact) yet never
    //       selected as project entries. Entry selection resolves the project graph's normal
    //       entry identities through the retained mapping rather than mutating package metadata.
    // WHY: R5C1 forbids clearing package `root_activity` to suppress entries; the separate graph
    //      boundaries keep package modules out of project entry selection.
    let active_project = minimal_lane_module(PathBuf::from("src/@page.moth"), true);
    let package_root = minimal_lane_module(PathBuf::from("packages/html/@mod.moth"), true);

    let frontend = test_frontend_with_source_package(active_project, package_root);
    assert_eq!(frontend.source_packages.len(), 1);
    assert!(
        frontend
            .source_packages
            .get(0)
            .expect("package boundary retained")
            .boundary
            .modules
            .artifact(ModuleId::from_index(0))
            .expect("valid package module id")
            .expect("package root should be successful")
            .module
            .metadata
            .root_activity
            .has_html_artifact_activity(),
        "source-package root activity is retained, not cleared"
    );

    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("project and package artefacts should assemble");
    assert_eq!(compilation.module_count(), 2, "both base modules retained");
    assert_eq!(
        compilation.entries().len(),
        1,
        "only the project-boundary module becomes an entry"
    );
    // Package module metadata is untouched: its root activity still marks it as active.
    let package_module = compilation
        .modules()
        .nth(1)
        .expect("package module retained");
    assert!(
        package_module
            .metadata
            .root_activity
            .has_html_artifact_activity(),
        "package module metadata is unchanged by project assembly"
    );
}

fn test_package_registry(packages: Vec<CompiledSourcePackage>) -> CompletedSourcePackageRegistry {
    let mut registry = CompletedSourcePackageRegistry::new();
    for package in packages {
        registry
            .publish(package, &[])
            .expect("test package should publish without dependencies");
    }
    registry
}

fn test_frontend_from_project_modules(modules: Vec<Module>) -> ProjectFrontendCompilation {
    let project = test_graph_boundary(modules, "test", "");
    ProjectFrontendCompilation::new(project, CompletedSourcePackageRegistry::new())
        .expect("test project boundary should validate")
}

fn test_frontend_with_source_package(
    project_module: Module,
    package_module: Module,
) -> ProjectFrontendCompilation {
    let project = test_graph_boundary(vec![project_module], "test", "page");
    let package = test_graph_boundary(vec![package_module], "html", "");
    let package_identity =
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "html");
    ProjectFrontendCompilation::new(
        project,
        test_package_registry(vec![CompiledSourcePackage {
            package_identity,
            root_module_id: ModuleId::from_index(0),
            boundary: package,
        }]),
    )
    .expect("test frontend should validate")
}

fn test_graph_boundary(
    modules: Vec<Module>,
    package_name: &str,
    module_path: &str,
) -> CompiledGraphBoundary {
    let module_count = modules.len();
    let origins = (0..module_count)
        .map(|index| {
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local(package_name),
                if module_path.is_empty() {
                    format!("module_{index}")
                } else {
                    module_path.to_owned()
                },
                ModuleRootRole::Normal,
            )
        })
        .collect::<Vec<_>>();
    let graph = ProjectModuleGraph::from_normal_roots(
        origins
            .iter()
            .cloned()
            .zip((0..module_count).map(|index| {
                let root_path = PathBuf::from(format!("@module_{index}.moth"));
                (root_path.clone(), root_path)
            }))
            .map(|(origin, (root_directory, root_file))| (origin, root_directory, root_file))
            .collect(),
    );
    let mut store = ModuleArtifactStore::new(module_count);
    for (index, module) in modules.into_iter().enumerate() {
        store
            .publish_success(
                ModuleId::from_index(index),
                CompiledModuleArtifact {
                    module,
                    interface: empty_test_interface_for_origin(origins[index].clone()),
                },
            )
            .expect("test store should publish each module");
    }
    CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    }
}

fn empty_test_interface(module_path: String) -> PublicSemanticInterface {
    empty_test_interface_for_origin(StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test"),
        module_path,
        ModuleRootRole::Normal,
    ))
}

fn empty_test_interface_for_origin(
    module_origin: StableModuleOriginIdentity,
) -> PublicSemanticInterface {
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

#[test]
fn dense_lookup_resolves_wave_publication_out_of_module_id_order() {
    let mut store = ModuleArtifactStore::new(3);
    store
        .publish_success(
            ModuleId::from_index(2),
            CompiledModuleArtifact {
                module: minimal_lane_module(PathBuf::from("high/@mod.moth"), true),
                interface: empty_test_interface("high".to_owned()),
            },
        )
        .expect("higher-ID provider should publish first");
    store
        .publish_success(
            ModuleId::from_index(1),
            CompiledModuleArtifact {
                module: minimal_lane_module(PathBuf::from("low/@mod.moth"), true),
                interface: empty_test_interface("low".to_owned()),
            },
        )
        .expect("lower-ID consumer should publish second");
    store
        .mark_diagnosed(ModuleId::from_index(0))
        .expect("diagnosed slot should transition");

    let high = store
        .artifact(ModuleId::from_index(2))
        .expect("valid high module id")
        .expect("high module should be successful");
    assert!(high.module.metadata.entry_point.ends_with("high/@mod.moth"));
    let low = store
        .artifact(ModuleId::from_index(1))
        .expect("valid low module id")
        .expect("low module should be successful");
    assert!(low.module.metadata.entry_point.ends_with("low/@mod.moth"));
    assert!(
        store
            .artifact(ModuleId::from_index(0))
            .expect("valid diagnosed module id")
            .is_none(),
        "diagnosed modules expose no artefact"
    );

    let order = store
        .successful_artefacts_in_module_id_order()
        .map(|artifact| artifact.module.metadata.entry_point.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            PathBuf::from("low/@mod.moth"),
            PathBuf::from("high/@mod.moth")
        ],
        "iteration must be deterministic in ModuleId order, not wave order"
    );
}

#[test]
fn duplicate_declaration_inside_one_materialisation_context_fails_publication() {
    let identity = generated_test_identity("dup").declaration().clone();
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("dup"),
        "dup".to_owned(),
        ModuleRootRole::Normal,
    );
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        origin,
        PathBuf::from("@dup.moth"),
        PathBuf::from("@dup.moth"),
    )]);
    let mut store = ModuleArtifactStore::new(1);
    let mut module = minimal_lane_module(PathBuf::from("@dup.moth"), true);
    module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![identity.clone(), identity]),
    ));

    let error = store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module,
                interface: empty_test_interface("dup".to_owned()),
            },
        )
        .unwrap_err();
    assert!(
        error
            .msg
            .contains("duplicated inside one materialisation context")
    );
    let _ = graph;
}

#[test]
fn materialisation_lookup_resolves_the_exact_template_row() {
    let first_identity = generated_test_identity("first").declaration().clone();
    let second_identity = generated_test_identity("second").declaration().clone();
    let origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("rows"),
        "rows".to_owned(),
        ModuleRootRole::Normal,
    );
    let graph = ProjectModuleGraph::from_normal_roots(vec![(
        origin,
        PathBuf::from("@rows.moth"),
        PathBuf::from("@rows.moth"),
    )]);
    let mut store = ModuleArtifactStore::new(1);
    let mut module = minimal_lane_module(PathBuf::from("@rows.moth"), true);
    module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            first_identity,
            second_identity.clone(),
        ]),
    ));
    store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module,
                interface: empty_test_interface("rows".to_owned()),
            },
        )
        .expect("row context should publish");

    let (_, location) = store
        .materialisation_locations()
        .find(|(identity, _)| **identity == second_identity)
        .expect("the second identity should be published with its own row");
    assert_eq!(
        location.template_index, 1,
        "the row index must be the declaring context's own position, not the publication order"
    );
    assert!(store.materialisation_context_at(location).is_ok());
    let _ = graph;
}

#[test]
fn same_generated_declaration_across_project_and_package_boundaries_fails() {
    let identity = generated_test_identity("shared").declaration().clone();
    let mut project_module = minimal_lane_module(PathBuf::from("@page.moth"), true);
    project_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![identity.clone()]),
    ));
    let project = test_graph_boundary(vec![project_module], "test", "page");

    let mut package_module = minimal_lane_module(PathBuf::from("@mod.moth"), true);
    package_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![identity]),
    ));
    let package = test_graph_boundary(vec![package_module], "html", "");

    let registry = test_package_registry(vec![CompiledSourcePackage {
        package_identity: StablePackageIdentity::source_package(
            PackageOrigin::ProjectLocal,
            "html",
        ),
        root_module_id: ModuleId::from_index(0),
        boundary: package,
    }]);

    let error = match ProjectFrontendCompilation::new(project, registry) {
        Ok(_) => panic!("one declaration identity must not cross boundaries"),
        Err(error) => error,
    };
    assert!(error.msg.contains("both project"));
}

#[test]
fn source_package_boundaries_never_cross_address_overlapping_module_ids() {
    let package_a = test_graph_boundary(
        vec![minimal_lane_module(
            PathBuf::from("packages/a/@mod.moth"),
            true,
        )],
        "a",
        "",
    );
    let package_b = test_graph_boundary(
        vec![minimal_lane_module(
            PathBuf::from("packages/b/@mod.moth"),
            true,
        )],
        "b",
        "",
    );
    let project = test_graph_boundary(
        vec![minimal_lane_module(PathBuf::from("@page.moth"), true)],
        "test",
        "page",
    );
    let frontend = ProjectFrontendCompilation::new(
        project,
        test_package_registry(vec![
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "a",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: package_a,
            },
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "b",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: package_b,
            },
        ]),
    )
    .expect("overlapping package module ids should stay isolated");

    let package_a_module = frontend
        .source_packages
        .iter()
        .next()
        .expect("package a retained")
        .boundary
        .modules
        .artifact(ModuleId::from_index(0))
        .expect("valid package a id")
        .expect("package a root should be successful");
    assert!(
        package_a_module
            .module
            .metadata
            .entry_point
            .ends_with("packages/a/@mod.moth")
    );
    let package_b_module = frontend
        .source_packages
        .iter()
        .nth(1)
        .expect("package b retained")
        .boundary
        .modules
        .artifact(ModuleId::from_index(0))
        .expect("valid package b id")
        .expect("package b root should be successful");
    assert!(
        package_b_module
            .module
            .metadata
            .entry_point
            .ends_with("packages/b/@mod.moth")
    );

    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("overlapping package module ids should assemble");
    assert_eq!(compilation.module_count(), 3);
    let module_paths = compilation
        .modules()
        .map(|module| module.metadata.entry_point.clone())
        .collect::<Vec<_>>();
    assert!(module_paths[1].ends_with("packages/a/@mod.moth"));
    assert!(module_paths[2].ends_with("packages/b/@mod.moth"));
    assert_eq!(
        compilation.entries().len(),
        1,
        "only the project root is an entry"
    );
}

#[test]
fn boundary_outcome_sorting_is_independent_of_wave_order() {
    let mut string_table = StringTable::new();
    let mut boundary = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(Vec::new()),
        modules: ModuleArtifactStore::new(0),
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: vec![
            DiagnosedModule {
                module_id: ModuleId::from_index(2),
                diagnostics: test_module_diagnostics("z.moth", &mut string_table),
            },
            DiagnosedModule {
                module_id: ModuleId::from_index(0),
                diagnostics: test_module_diagnostics("a.moth", &mut string_table),
            },
        ],
        blocked: vec![
            BlockedModule {
                module_id: ModuleId::from_index(1),
                required_provider: BlockedProvider::Module(ModuleId::from_index(0)),
            },
            BlockedModule {
                module_id: ModuleId::from_index(0),
                required_provider: BlockedProvider::Module(ModuleId::from_index(2)),
            },
        ],
    };

    boundary.sort_outcomes();

    let diagnosed_order = boundary
        .diagnosed
        .iter()
        .map(|module| module.module_id.index())
        .collect::<Vec<_>>();
    assert_eq!(diagnosed_order, vec![0, 2]);
    let blocked_order = boundary
        .blocked
        .iter()
        .map(|module| module.module_id.index())
        .collect::<Vec<_>>();
    assert_eq!(blocked_order, vec![0, 1]);
}

fn test_module_diagnostics(module_path: &str, string_table: &mut StringTable) -> ModuleDiagnostics {
    let path = InternedPath::from_single_str(module_path, string_table);
    let name = string_table.intern("missing_name");
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        SourceLocation::new(
            path,
            CharPosition {
                line_number: 1,
                char_column: 1,
            },
            CharPosition {
                line_number: 1,
                char_column: 2,
            },
        ),
        DiagnosticPayload::UnknownName {
            name,
            namespace: NameNamespace::Value,
        },
    );
    let messages = CompilerMessages::from_diagnostic(diagnostic, string_table.clone());
    ModuleDiagnostics::from_messages(messages).expect("user diagnostic should classify")
}

#[test]
fn generated_sidecar_warnings_survive_render_and_success_only_compilation() {
    let mut string_table = StringTable::new();
    let frontend = frontend_with_sidecar_warnings(&mut string_table);
    let messages = frontend.into_render_messages(&mut string_table);
    let rendered_codes = messages
        .warnings()
        .map(|warning| warning.kind.code().to_owned())
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

    let frontend = frontend_with_sidecar_warnings(&mut string_table);
    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("generated sidecar boundaries should assemble");
    let compilation_codes = compilation
        .modules()
        .flat_map(|module| {
            module
                .metadata
                .warnings
                .iter()
                .map(|warning| warning.kind.code().to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compilation_codes.len(),
        2,
        "success-only compilation should retain project and source-package sidecar warnings"
    );
    assert_eq!(
        compilation_codes, rendered_codes,
        "frontend rendering and success-only compilation must retain identical warning codes"
    );
}

#[test]
fn project_and_package_boundaries_may_contain_equal_generated_identities() {
    let shared_identity = generated_test_identity("shared");
    let package_origin = package_origin_identity("sharedpkg", "sharedpkg/@mod.moth");
    let package_facade = OriginFunctionId::new_free(package_origin, "facade".to_owned());

    let project_module = lane_module_with_generated_and_cross_module_calls(
        PathBuf::from("@page.moth"),
        true,
        Some(shared_identity.clone()),
        std::slice::from_ref(&package_facade),
    );
    let mut package_module = lane_module_with_generated_and_cross_module_calls(
        PathBuf::from("packages/sharedpkg/@mod.moth"),
        false,
        Some(shared_identity.clone()),
        &[],
    );
    package_module
        .executable
        .hir
        .function_ids_by_origin
        .insert(package_facade, FunctionId(0));

    let mut project = test_graph_boundary(vec![project_module], "test", "page");
    project.generated = {
        let mut store = BoundaryGeneratedFunctionStore::default();
        store.push_completed_for_test(CompletedGeneratedFunction {
            identity: shared_identity.clone(),
            summary: generated_test_summary(),
            sidecar: lane_sidecar(
                shared_identity.clone(),
                PathBuf::from("@project_shared_generated.moth"),
            ),
        });
        store
    };
    let mut package = test_graph_boundary(vec![package_module], "sharedpkg", "");
    package.generated = {
        let mut store = BoundaryGeneratedFunctionStore::default();
        store.push_completed_for_test(CompletedGeneratedFunction {
            identity: shared_identity.clone(),
            summary: generated_test_summary(),
            sidecar: lane_sidecar(
                shared_identity.clone(),
                PathBuf::from("packages/sharedpkg/@package_shared_generated.moth"),
            ),
        });
        store
    };

    let frontend = ProjectFrontendCompilation::new(
        project,
        test_package_registry(vec![CompiledSourcePackage {
            package_identity: StablePackageIdentity::source_package(
                PackageOrigin::ProjectLocal,
                "sharedpkg",
            ),
            root_module_id: ModuleId::from_index(0),
            boundary: package,
        }]),
    )
    .expect("equal generated identities across boundaries should validate");

    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("equal generated identities across project and package boundaries must assemble");
    let entries = compilation.entries();
    assert_eq!(entries.len(), 1, "one project root becomes one entry");
    let entry = &entries[0];
    assert_eq!(
        entry.linked_modules.len(),
        3,
        "the entry links the package module and both boundary-local sidecars"
    );
    let project_sidecar = entry
        .linked_modules
        .iter()
        .find(|linked| {
            linked
                .module
                .metadata
                .entry_point
                .ends_with("@project_shared_generated.moth")
        })
        .expect("project-boundary sidecar should be linked");
    let package_sidecar = entry
        .linked_modules
        .iter()
        .find(|linked| {
            linked
                .module
                .metadata
                .entry_point
                .ends_with("@package_shared_generated.moth")
        })
        .expect("package-boundary sidecar should be linked");
    let project_name = project_sidecar
        .generated_function_names
        .get(&shared_identity)
        .expect("project boundary owns a symbol for the shared identity");
    let package_name = package_sidecar
        .generated_function_names
        .get(&shared_identity)
        .expect("package boundary owns a symbol for the shared identity");
    assert_ne!(
        project_name, package_name,
        "equal identities in unrelated boundaries must receive distinct symbols"
    );
    assert!(
        entry.all_generated_function_names.contains(project_name)
            && entry.all_generated_function_names.contains(package_name),
        "the entry reserves every generated symbol across boundaries"
    );
}

#[test]
fn package_cannot_resolve_an_unrelated_package_sidecar() {
    let shared_identity = generated_test_identity("shared");
    let package_origin = package_origin_identity("a", "a/@mod.moth");
    let package_facade = OriginFunctionId::new_free(package_origin, "facade".to_owned());

    let project_module = lane_module_with_generated_and_cross_module_calls(
        PathBuf::from("@page.moth"),
        true,
        None,
        std::slice::from_ref(&package_facade),
    );
    let mut package_a_module = lane_module_with_generated_and_cross_module_calls(
        PathBuf::from("packages/a/@mod.moth"),
        false,
        Some(shared_identity.clone()),
        &[],
    );
    package_a_module
        .executable
        .hir
        .function_ids_by_origin
        .insert(package_facade, FunctionId(0));
    let package_a = test_graph_boundary(vec![package_a_module], "a", "");
    let mut package_b = test_graph_boundary(
        vec![minimal_lane_module(
            PathBuf::from("packages/b/@mod.moth"),
            false,
        )],
        "b",
        "",
    );
    package_b.generated = {
        let mut store = BoundaryGeneratedFunctionStore::default();
        store.push_completed_for_test(CompletedGeneratedFunction {
            identity: shared_identity.clone(),
            summary: generated_test_summary(),
            sidecar: lane_sidecar(
                shared_identity.clone(),
                PathBuf::from("packages/b/@generated.moth"),
            ),
        });
        store
    };

    let frontend = ProjectFrontendCompilation::new(
        test_graph_boundary(vec![project_module], "test", "page"),
        test_package_registry(vec![
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "a",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: package_a,
            },
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "b",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: package_b,
            },
        ]),
    )
    .expect("frontend boundaries should validate");

    let error = match ProjectCompilation::from_frontend(frontend) {
        Ok(_) => panic!("a package must not resolve another package's sidecar"),
        Err(error) => error,
    };
    assert!(
        error.msg.contains("in its calling boundary"),
        "unexpected boundary-scoped resolution failure: {error:?}"
    );
}

#[test]
fn independent_packages_publish_equal_generated_identities_in_any_order() {
    let shared_identity = generated_test_identity("shared");
    let facade_a = OriginFunctionId::new_free(
        package_origin_identity("a", "a/@mod.moth"),
        "facade".to_owned(),
    );
    let facade_b = OriginFunctionId::new_free(
        package_origin_identity("b", "b/@mod.moth"),
        "facade".to_owned(),
    );

    let package_module_for = |prefix: &str, facade: &OriginFunctionId| -> Module {
        let mut module = lane_module_with_generated_and_cross_module_calls(
            PathBuf::from(format!("packages/{prefix}/@mod.moth")),
            false,
            Some(shared_identity.clone()),
            &[],
        );
        module
            .executable
            .hir
            .function_ids_by_origin
            .insert(facade.clone(), FunctionId(0));
        module
    };

    let frontend_for = |order: [&str; 2]| -> ProjectFrontendCompilation {
        let (first_prefix, second_prefix) = (order[0], order[1]);
        let project_module = lane_module_with_generated_and_cross_module_calls(
            PathBuf::from("@page.moth"),
            true,
            None,
            &[
                if first_prefix == "a" {
                    facade_a.clone()
                } else {
                    facade_b.clone()
                },
                if second_prefix == "a" {
                    facade_a.clone()
                } else {
                    facade_b.clone()
                },
            ],
        );
        let packages = order
            .iter()
            .map(|prefix| {
                let facade = if *prefix == "a" { &facade_a } else { &facade_b };
                let mut boundary =
                    test_graph_boundary(vec![package_module_for(prefix, facade)], prefix, "");
                boundary.generated = {
                    let mut store = BoundaryGeneratedFunctionStore::default();
                    store.push_completed_for_test(CompletedGeneratedFunction {
                        identity: shared_identity.clone(),
                        summary: generated_test_summary(),
                        sidecar: lane_sidecar(
                            shared_identity.clone(),
                            PathBuf::from(format!("packages/{prefix}/@generated.moth")),
                        ),
                    });
                    store
                };
                CompiledSourcePackage {
                    package_identity: StablePackageIdentity::source_package(
                        PackageOrigin::ProjectLocal,
                        prefix,
                    ),
                    root_module_id: ModuleId::from_index(0),
                    boundary,
                }
            })
            .collect::<Vec<_>>();
        ProjectFrontendCompilation::new(
            test_graph_boundary(vec![project_module], "test", "page"),
            test_package_registry(packages),
        )
        .expect("frontend should validate")
    };

    let first = frontend_for(["a", "b"]);
    let second = frontend_for(["b", "a"]);

    let symbols_by_prefix = |frontend: ProjectFrontendCompilation| {
        let compilation = ProjectCompilation::from_frontend(frontend)
            .expect("two packages may publish equal generated identities in either order");
        assert_eq!(
            compilation.module_count(),
            5,
            "two package base modules plus one sidecar each, beside the project root"
        );
        let entries = compilation.entries();
        assert_eq!(entries.len(), 1);
        let mut symbols = rustc_hash::FxHashMap::default();
        for linked_module in &entries[0].linked_modules {
            if let Some(name) = linked_module.generated_function_names.get(&shared_identity) {
                let prefix = if linked_module
                    .module
                    .metadata
                    .entry_point
                    .starts_with("packages/a/")
                {
                    "a"
                } else {
                    "b"
                };
                symbols.insert(prefix.to_owned(), name.clone());
            }
        }
        assert_eq!(
            symbols.len(),
            2,
            "each package boundary resolves its own local symbol for the shared identity"
        );
        symbols
    };

    let first_symbols = symbols_by_prefix(first);
    let second_symbols = symbols_by_prefix(second);
    assert_eq!(
        first_symbols, second_symbols,
        "reversing package registration order must not change any boundary's assigned symbols"
    );

    // Each package remains coherent when the other boundary is removed.
    for prefix in ["a", "b"] {
        let facade = if prefix == "a" {
            facade_a.clone()
        } else {
            facade_b.clone()
        };
        let project_module = lane_module_with_generated_and_cross_module_calls(
            PathBuf::from("@page.moth"),
            true,
            None,
            &[facade],
        );
        let mut boundary = test_graph_boundary(
            vec![package_module_for(
                prefix,
                if prefix == "a" { &facade_a } else { &facade_b },
            )],
            prefix,
            "",
        );
        boundary.generated = {
            let mut store = BoundaryGeneratedFunctionStore::default();
            store.push_completed_for_test(CompletedGeneratedFunction {
                identity: shared_identity.clone(),
                summary: generated_test_summary(),
                sidecar: lane_sidecar(
                    shared_identity.clone(),
                    PathBuf::from(format!("packages/{prefix}/@generated.moth")),
                ),
            });
            store
        };
        let frontend = ProjectFrontendCompilation::new(
            test_graph_boundary(vec![project_module], "test", "page"),
            test_package_registry(vec![CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    prefix,
                ),
                root_module_id: ModuleId::from_index(0),
                boundary,
            }]),
        )
        .expect("single-package frontend should validate");
        let compilation = ProjectCompilation::from_frontend(frontend)
            .expect("one package alone must stay coherent");
        assert_eq!(compilation.module_count(), 3);
    }
}

fn package_origin_identity(prefix: &str, module_path: &str) -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, prefix),
        module_path.to_owned(),
        ModuleRootRole::Normal,
    )
}

fn lane_module_with_generated_and_cross_module_calls(
    entry_point: PathBuf,
    active_root: bool,
    generated_call: Option<GeneratedFunctionIdentity>,
    cross_module_calls: &[OriginFunctionId],
) -> Module {
    let start_name_path = InternedPath::from_single_str("start_entry", &mut StringTable::new());
    let mut hir_module = minimal_hir_module(start_name_path);
    if let Some(identity) = generated_call {
        hir_module.blocks[0].statements.push(HirStatement {
            id: HirNodeId(7),
            kind: HirStatementKind::Call {
                target: CallTarget::Generated(identity),
                args: vec![],
                result: None,
            },
            location: SourceLocation::default(),
        });
    }
    for (index, origin) in cross_module_calls.iter().enumerate() {
        hir_module.blocks[0].statements.push(HirStatement {
            id: HirNodeId(8 + index as u32),
            kind: HirStatementKind::Call {
                target: CallTarget::CrossModule(origin.clone()),
                args: vec![],
                result: None,
            },
            location: SourceLocation::default(),
        });
    }
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .expect("test HIR should produce function link facts");
    Module {
        executable: ModuleExecutable {
            hir: hir_module,
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: vec![],
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point,
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: if active_root {
                ModuleRootActivity {
                    has_non_trivial_root_body: true,
                    ..ModuleRootActivity::default()
                }
            } else {
                ModuleRootActivity::default()
            },
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            materialisation_context: None,
        },
    }
}

fn lane_sidecar(
    identity: GeneratedFunctionIdentity,
    entry_point: PathBuf,
) -> GeneratedFunctionSidecar {
    let mut module = minimal_lane_module(entry_point, false);
    module
        .executable
        .hir
        .function_ids_by_generated
        .insert(identity.clone(), FunctionId(0));
    GeneratedFunctionSidecar::new(identity, module)
}

#[test]
fn frontend_boundary_rejects_unfinished_module_slots() {
    let first_root = PathBuf::from("@first.moth");
    let second_root = PathBuf::from("@second.moth");
    let graph = ProjectModuleGraph::from_normal_roots(vec![
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "first".to_owned(),
                ModuleRootRole::Normal,
            ),
            first_root.clone(),
            first_root,
        ),
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "second".to_owned(),
                ModuleRootRole::Normal,
            ),
            second_root.clone(),
            second_root,
        ),
    ]);
    let mut store = ModuleArtifactStore::new(2);
    store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module: minimal_lane_module(PathBuf::from("@first.moth"), true),
                interface: empty_test_interface("first".to_owned()),
            },
        )
        .expect("first module should publish");
    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let error =
        match ProjectFrontendCompilation::new(boundary, CompletedSourcePackageRegistry::new()) {
            Ok(_) => panic!("an unfinished module slot must reject the frontend boundary"),
            Err(error) => error,
        };
    assert!(
        error.msg.contains("never reached a completed outcome"),
        "unexpected frontend-boundary rejection: {error:?}"
    );
}

#[test]
fn mixed_outcomes_remain_valid_for_check_and_reject_success_only_compilation() {
    let graph = ProjectModuleGraph::from_normal_roots(vec![
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "success".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("@success.moth"),
            PathBuf::from("@success.moth"),
        ),
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "diagnosed".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("@diagnosed.moth"),
            PathBuf::from("@diagnosed.moth"),
        ),
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "blocked".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("@blocked.moth"),
            PathBuf::from("@blocked.moth"),
        ),
    ]);
    let mut store = ModuleArtifactStore::new(3);
    store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module: minimal_lane_module(PathBuf::from("@success.moth"), true),
                interface: empty_test_interface("success".to_owned()),
            },
        )
        .expect("success module should publish");
    store
        .mark_diagnosed(ModuleId::from_index(1))
        .expect("diagnosed slot should transition");
    store
        .mark_blocked(ModuleId::from_index(2))
        .expect("blocked slot should transition");

    let mut string_table = StringTable::new();
    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: vec![DiagnosedModule {
            module_id: ModuleId::from_index(1),
            diagnostics: test_module_diagnostics("diagnosed.moth", &mut string_table),
        }],
        blocked: vec![BlockedModule {
            module_id: ModuleId::from_index(2),
            required_provider: BlockedProvider::Module(ModuleId::from_index(1)),
        }],
    };

    let frontend = ProjectFrontendCompilation::new(boundary, CompletedSourcePackageRegistry::new())
        .expect("mixed outcomes are a valid retained frontend result for check");
    assert!(frontend.has_diagnosed_or_blocked());

    let error = match ProjectCompilation::from_frontend(frontend) {
        Ok(_) => panic!("diagnosed or blocked modules must reject success-only compilation"),
        Err(error) => error,
    };
    assert!(
        error.msg.contains("boundary with diagnosed ModuleId"),
        "unexpected success-only rejection: {error:?}"
    );
}

fn frontend_with_sidecar_warnings(string_table: &mut StringTable) -> ProjectFrontendCompilation {
    let mut project_sidecar_module =
        minimal_lane_module(PathBuf::from("@generated_project.moth"), true);
    project_sidecar_module
        .metadata
        .warnings
        .push(test_warning_diagnostic(
            "generated_project.moth",
            string_table,
        ));
    let mut project_generated = BoundaryGeneratedFunctionStore::default();
    project_generated.push_completed_for_test(CompletedGeneratedFunction {
        identity: generated_test_identity("generated_project"),
        summary: generated_test_summary(),
        sidecar: GeneratedFunctionSidecar::new(
            generated_test_identity("generated_project"),
            project_sidecar_module,
        ),
    });
    let mut project = test_graph_boundary(
        vec![minimal_lane_module(PathBuf::from("@page.moth"), true)],
        "test",
        "page",
    );
    project.generated = project_generated;

    let mut package_sidecar_module =
        minimal_lane_module(PathBuf::from("@generated_package.moth"), true);
    package_sidecar_module
        .metadata
        .warnings
        .push(test_warning_diagnostic(
            "generated_package.moth",
            string_table,
        ));
    let mut package_generated = BoundaryGeneratedFunctionStore::default();
    package_generated.push_completed_for_test(CompletedGeneratedFunction {
        identity: generated_test_identity("generated_package"),
        summary: generated_test_summary(),
        sidecar: GeneratedFunctionSidecar::new(
            generated_test_identity("generated_package"),
            package_sidecar_module,
        ),
    });
    let mut package = test_graph_boundary(
        vec![minimal_lane_module(
            PathBuf::from("packages/warnpkg/@mod.moth"),
            true,
        )],
        "warnpkg",
        "",
    );
    package.generated = package_generated;

    ProjectFrontendCompilation::new(
        project,
        test_package_registry(vec![CompiledSourcePackage {
            package_identity: StablePackageIdentity::source_package(
                PackageOrigin::ProjectLocal,
                "warnpkg",
            ),
            root_module_id: ModuleId::from_index(0),
            boundary: package,
        }]),
    )
    .expect("warning package frontend should validate")
}

fn test_warning_diagnostic(
    module_path: &str,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let path = InternedPath::from_single_str(module_path, string_table);
    let name = string_table.intern("unused_warning_name");
    CompilerDiagnostic::with_severity(
        DiagnosticKind::Rule(
            crate::compiler_frontend::compiler_messages::RuleDiagnosticKind::UnknownName,
        ),
        DiagnosticSeverity::Warning,
        SourceLocation::new(
            path,
            CharPosition {
                line_number: 1,
                char_column: 1,
            },
            CharPosition {
                line_number: 1,
                char_column: 2,
            },
        ),
        DiagnosticPayload::UnknownName {
            name,
            namespace: NameNamespace::Value,
        },
    )
}

fn generated_test_identity(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
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

fn generated_test_summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

fn minimal_lane_module(entry_point: PathBuf, active_root: bool) -> Module {
    let start_name_path = InternedPath::from_single_str("start_entry", &mut StringTable::new());
    let hir_module = minimal_hir_module(start_name_path);
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .expect("test HIR should produce function link facts");
    Module {
        executable: ModuleExecutable {
            hir: hir_module,
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: vec![],
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point,
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: if active_root {
                ModuleRootActivity {
                    has_non_trivial_root_body: true,
                    ..ModuleRootActivity::default()
                }
            } else {
                ModuleRootActivity::default()
            },
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            materialisation_context: None,
        },
    }
}

fn single_node_graph() -> ProjectModuleGraph {
    ProjectModuleGraph::from_normal_roots(vec![(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            "single".to_owned(),
            ModuleRootRole::Normal,
        ),
        PathBuf::from("@single.moth"),
        PathBuf::from("@single.moth"),
    )])
}

#[test]
fn boundary_validation_rejects_diagnosed_lane_mismatch_in_both_directions() {
    // Record present but the slot is successful.
    let mut string_table = StringTable::new();
    let mut boundary = test_graph_boundary(
        vec![minimal_lane_module(PathBuf::from("@single.moth"), false)],
        "test",
        "single",
    );
    boundary.diagnosed = vec![DiagnosedModule {
        module_id: ModuleId::from_index(0),
        diagnostics: test_module_diagnostics("single.moth", &mut string_table),
    }];
    let error = boundary
        .validate_invariants()
        .expect_err("a diagnosed record must hold the diagnosed slot");
    assert!(error.msg.contains("does not hold the diagnosed store slot"));

    // Slot diagnosed but no record.
    let mut store = ModuleArtifactStore::new(1);
    store
        .mark_diagnosed(ModuleId::from_index(0))
        .expect("slot should transition");
    let boundary = CompiledGraphBoundary {
        structure: single_node_graph(),
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let error = boundary
        .validate_invariants()
        .expect_err("a diagnosed slot must own a diagnosed record");
    assert!(error.msg.contains("has no diagnosed record"));
}

#[test]
fn boundary_validation_rejects_blocked_lane_mismatch_in_both_directions() {
    let mut store = ModuleArtifactStore::new(1);
    store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module: minimal_lane_module(PathBuf::from("@single.moth"), false),
                interface: empty_test_interface("single".to_owned()),
            },
        )
        .expect("slot should publish");
    let boundary = CompiledGraphBoundary {
        structure: single_node_graph(),
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: vec![BlockedModule {
            module_id: ModuleId::from_index(0),
            required_provider: BlockedProvider::Module(ModuleId::from_index(1)),
        }],
    };
    let error = boundary
        .validate_invariants()
        .expect_err("a blocked record must hold the blocked slot");
    assert!(error.msg.contains("does not hold the blocked store slot"));

    let mut store = ModuleArtifactStore::new(1);
    store
        .mark_blocked(ModuleId::from_index(0))
        .expect("slot should transition");
    let boundary = CompiledGraphBoundary {
        structure: single_node_graph(),
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let error = boundary
        .validate_invariants()
        .expect_err("a blocked slot must own a blocked record");
    assert!(error.msg.contains("has no blocked record"));
}

#[test]
fn boundary_validation_rejects_duplicate_and_overlapping_outcome_lanes() {
    let mut string_table = StringTable::new();
    let mut store = ModuleArtifactStore::new(2);
    store
        .mark_diagnosed(ModuleId::from_index(0))
        .expect("slot 0 should transition");
    store
        .mark_diagnosed(ModuleId::from_index(1))
        .expect("slot 1 should transition");
    let graph = ProjectModuleGraph::from_normal_roots(vec![
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "a".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("@a.moth"),
            PathBuf::from("@a.moth"),
        ),
        (
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("test"),
                "b".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("@b.moth"),
            PathBuf::from("@b.moth"),
        ),
    ]);
    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: vec![
            DiagnosedModule {
                module_id: ModuleId::from_index(0),
                diagnostics: test_module_diagnostics("a.moth", &mut string_table),
            },
            DiagnosedModule {
                module_id: ModuleId::from_index(0),
                diagnostics: test_module_diagnostics("duplicate.moth", &mut string_table),
            },
        ],
        blocked: Vec::new(),
    };
    let error = boundary
        .validate_invariants()
        .expect_err("duplicate diagnosed records must be rejected");
    assert!(
        error
            .msg
            .contains("appears more than once in the diagnosed lane")
    );

    let mut store = ModuleArtifactStore::new(2);
    store
        .mark_blocked(ModuleId::from_index(0))
        .expect("slot 0 should transition");
    store
        .mark_blocked(ModuleId::from_index(1))
        .expect("slot 1 should transition");
    let boundary = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(vec![
            (
                StableModuleOriginIdentity::from_portable_path(
                    StablePackageIdentity::project_local("test"),
                    "a".to_owned(),
                    ModuleRootRole::Normal,
                ),
                PathBuf::from("@a.moth"),
                PathBuf::from("@a.moth"),
            ),
            (
                StableModuleOriginIdentity::from_portable_path(
                    StablePackageIdentity::project_local("test"),
                    "b".to_owned(),
                    ModuleRootRole::Normal,
                ),
                PathBuf::from("@b.moth"),
                PathBuf::from("@b.moth"),
            ),
        ]),
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: vec![
            BlockedModule {
                module_id: ModuleId::from_index(0),
                required_provider: BlockedProvider::Module(ModuleId::from_index(2)),
            },
            BlockedModule {
                module_id: ModuleId::from_index(0),
                required_provider: BlockedProvider::Module(ModuleId::from_index(1)),
            },
        ],
    };
    let error = boundary
        .validate_invariants()
        .expect_err("duplicate blocked records must be rejected");
    assert!(
        error
            .msg
            .contains("appears more than once in the blocked lane")
    );

    let mut store = ModuleArtifactStore::new(1);
    store
        .mark_diagnosed(ModuleId::from_index(0))
        .expect("slot should transition");
    let boundary = CompiledGraphBoundary {
        structure: single_node_graph(),
        modules: store,
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: vec![DiagnosedModule {
            module_id: ModuleId::from_index(0),
            diagnostics: test_module_diagnostics("overlap.moth", &mut string_table),
        }],
        blocked: vec![BlockedModule {
            module_id: ModuleId::from_index(0),
            required_provider: BlockedProvider::Module(ModuleId::from_index(1)),
        }],
    };
    let error = boundary
        .validate_invariants()
        .expect_err("diagnosed and blocked lanes must never overlap");
    assert!(error.msg.contains("is both diagnosed and blocked"));
}

#[test]
fn source_package_publication_rejects_unavailable_root_slot() {
    let package_boundary = CompiledGraphBoundary {
        structure: ProjectModuleGraph::from_normal_roots(vec![(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "pkg"),
                "pkg/@mod.moth".to_owned(),
                ModuleRootRole::Normal,
            ),
            PathBuf::from("pkg/@mod.moth"),
            PathBuf::from("pkg/@mod.moth"),
        )]),
        modules: ModuleArtifactStore::new(1),
        generated: BoundaryGeneratedFunctionStore::default(),
        diagnosed: Vec::new(),
        blocked: Vec::new(),
    };
    let mut registry = CompletedSourcePackageRegistry::new();
    let error = registry
        .publish(
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "pkg",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: package_boundary,
            },
            &[],
        )
        .expect_err("an unfinished package boundary must not publish");

    assert!(
        error.msg.contains("never reached a completed outcome"),
        "unexpected publication rejection: {error:?}"
    );
    assert_eq!(
        registry.len(),
        0,
        "a failing publication must not retain the package"
    );
}

#[test]
fn materialisation_rows_iterate_in_deterministic_publication_order() {
    let second_identity = generated_test_identity("second");
    let first_identity = generated_test_identity("first");
    let mut store = ModuleArtifactStore::new(2);

    let mut second_module = minimal_lane_module(PathBuf::from("second/@mod.moth"), false);
    second_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            second_identity.declaration().clone(),
        ]),
    ));
    store
        .publish_success(
            ModuleId::from_index(1),
            CompiledModuleArtifact {
                module: second_module,
                interface: empty_test_interface("second".to_owned()),
            },
        )
        .expect("higher module id publishes first");

    let mut first_module = minimal_lane_module(PathBuf::from("first/@mod.moth"), false);
    first_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            first_identity.declaration().clone(),
        ]),
    ));
    store
        .publish_success(
            ModuleId::from_index(0),
            CompiledModuleArtifact {
                module: first_module,
                interface: empty_test_interface("first".to_owned()),
            },
        )
        .expect("lower module id publishes second");

    let order = store
        .materialisation_locations()
        .map(|(identity, _)| identity.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            second_identity.declaration().clone(),
            first_identity.declaration().clone()
        ],
        "materialisation rows must iterate in publication order, not hash order"
    );
}

#[test]
fn package_registry_materialisation_rows_iterate_in_publication_order() {
    let beta_identity = generated_test_identity("beta");
    let alpha_identity = generated_test_identity("alpha");
    let mut registry = CompletedSourcePackageRegistry::new();

    let mut beta_module = minimal_lane_module(PathBuf::from("packages/beta/@mod.moth"), false);
    beta_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            beta_identity.declaration().clone(),
        ]),
    ));
    registry
        .publish(
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "beta",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: test_graph_boundary(vec![beta_module], "beta", ""),
            },
            &[],
        )
        .expect("beta package publishes first");

    let mut alpha_module = minimal_lane_module(PathBuf::from("packages/alpha/@mod.moth"), false);
    alpha_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            alpha_identity.declaration().clone(),
        ]),
    ));
    registry
        .publish(
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "alpha",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: test_graph_boundary(vec![alpha_module], "alpha", ""),
            },
            &[],
        )
        .expect("alpha package publishes second");

    let order = registry
        .materialisation_locations()
        .map(|(identity, _)| identity.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        vec![
            beta_identity.declaration().clone(),
            alpha_identity.declaration().clone()
        ],
        "package materialisation rows must iterate in publication order"
    );
}

#[test]
fn late_package_duplicate_leaves_registry_unchanged() {
    let shared_identity = generated_test_identity("shared");
    let mut registry = CompletedSourcePackageRegistry::new();

    let mut first_module = minimal_lane_module(PathBuf::from("packages/first/@mod.moth"), false);
    first_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            shared_identity.declaration().clone(),
        ]),
    ));
    registry
        .publish(
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "first",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: test_graph_boundary(vec![first_module], "first", ""),
            },
            &[],
        )
        .expect("first package publishes");

    let mut late_module = minimal_lane_module(PathBuf::from("packages/late/@mod.moth"), false);
    late_module.metadata.materialisation_context = Some(Arc::new(
        ModuleMaterialisationContext::from_identities_for_test(vec![
            shared_identity.declaration().clone(),
        ]),
    ));
    let error = registry
        .publish(
            CompiledSourcePackage {
                package_identity: StablePackageIdentity::source_package(
                    PackageOrigin::ProjectLocal,
                    "late",
                ),
                root_module_id: ModuleId::from_index(0),
                boundary: test_graph_boundary(vec![late_module], "late", ""),
            },
            &[],
        )
        .expect_err("one declaration identity must not cross package boundaries");
    assert!(error.msg.contains("published by source packages"));
    assert_eq!(
        registry.len(),
        1,
        "the failing package row must not be appended"
    );
    assert_eq!(registry.by_prefix("late"), None);
    assert_eq!(
        registry.materialisation_locations().count(),
        1,
        "materialisation rows must remain unchanged"
    );
}

#[test]
fn generated_names_stay_stable_under_sidecar_publication_reordering() {
    let alpha = generated_test_identity("alpha");
    let beta = generated_test_identity("beta");

    let frontend_for_order = |first: GeneratedFunctionIdentity,
                              second: GeneratedFunctionIdentity| {
        let mut project = test_graph_boundary(
            vec![minimal_lane_module(PathBuf::from("@page.moth"), true)],
            "test",
            "page",
        );
        let mut store = BoundaryGeneratedFunctionStore::default();
        store.push_completed_for_test(CompletedGeneratedFunction {
            identity: first.clone(),
            summary: generated_test_summary(),
            sidecar: lane_sidecar(first, PathBuf::from("@generated_first.moth")),
        });
        store.push_completed_for_test(CompletedGeneratedFunction {
            identity: second.clone(),
            summary: generated_test_summary(),
            sidecar: lane_sidecar(second, PathBuf::from("@generated_second.moth")),
        });
        project.generated = store;
        ProjectCompilation::from_frontend(
            ProjectFrontendCompilation::new(project, CompletedSourcePackageRegistry::new())
                .expect("frontend should validate"),
        )
        .expect("sidecar boundaries should assemble")
    };

    let alpha_first = frontend_for_order(alpha.clone(), beta.clone());
    let beta_first = frontend_for_order(beta.clone(), alpha.clone());

    let names_for = |compilation: &ProjectCompilation| -> (String, String) {
        let names = &compilation.entries()[0].generated_function_names;
        (
            names.get(&alpha).expect("alpha name assigned").clone(),
            names.get(&beta).expect("beta name assigned").clone(),
        )
    };
    assert_eq!(
        names_for(&alpha_first),
        names_for(&beta_first),
        "generated symbol names must depend only on stable identity order, not publication order"
    );
}
