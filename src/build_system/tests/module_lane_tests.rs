//! Focused invariant tests for the compiled `Module` lane container.

use crate::build_system::build::{
    CompiledModuleArtifact, Module, ModuleCompilerMetadata, ModuleExecutable, ModuleExternalImport,
    ModuleLinkFacts, ModuleRootActivity, ProjectCompilation, ProjectFrontendCompilation,
};
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, ReactiveInvalidationFact, ReactiveInvalidationKind,
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

    let error = match ProjectCompilation::from_successful_modules(vec![module]) {
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
    let artifact = CompiledModuleArtifact {
        module,
        interface: crate::compiler_frontend::public_interface::PublicSemanticInterface {
            module_origin:
                crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity::from_portable_path(
                    crate::compiler_frontend::semantic_identity::StablePackageIdentity::project_local(
                        "test",
                    ),
                    String::new(),
                    crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
                ),
            export_bindings: Vec::new(),
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        },
    };

    let frontend = ProjectFrontendCompilation::new(vec![artifact], Vec::new(), Vec::new());
    let project_artifacts = frontend.project_artifacts();
    assert_eq!(project_artifacts.len(), 1, "one project artefact retained");
    assert!(
        project_artifacts[0]
            .module
            .metadata
            .root_activity
            .has_html_artifact_activity(),
        "project artefact root activity is retained"
    );
    assert_eq!(
        project_artifacts[0].interface.module_origin.role(),
        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        "artefact interface survives the handoff"
    );

    let compilation = ProjectCompilation::from_frontend(frontend)
        .expect("one active project module assembles one entry");
    assert_eq!(compilation.module_count(), 1, "one base module");
    assert_eq!(
        compilation.entries().len(),
        1,
        "normal root creates one entry"
    );
}

#[test]
fn source_package_artefacts_are_retained_but_never_project_entries() {
    // WHAT: source-package artefacts are retained immutably (root activity intact) yet never
    //       selected as project entries. Entry selection uses the project-boundary count rather
    //       than mutating package module metadata.
    // WHY: R5C1 forbids clearing package `root_activity` to suppress entries; the boundary index
    //      alone must keep package modules out of project entry selection.
    let active_project = minimal_lane_module(PathBuf::from("src/@page.moth"), true);
    let package_root = minimal_lane_module(PathBuf::from("packages/html/@mod.moth"), true);

    let frontend = ProjectFrontendCompilation::new(
        vec![CompiledModuleArtifact {
            module: active_project,
            interface: crate::compiler_frontend::public_interface::PublicSemanticInterface {
                module_origin:
                    crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity::from_portable_path(
                        crate::compiler_frontend::semantic_identity::StablePackageIdentity::project_local(
                            "test",
                        ),
                        String::from("page"),
                        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
                    ),
                export_bindings: Vec::new(),
                export_diagnostic_provenance: Vec::new(),
                binding_exports: Vec::new(),
                declarations: Vec::new(),
                reusable_evidence: Vec::new(),
                concrete_call_summaries: Vec::new(),
            },
        }],
        vec![CompiledModuleArtifact {
            module: package_root,
            interface: crate::compiler_frontend::public_interface::PublicSemanticInterface {
                module_origin:
                    crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity::from_portable_path(
                        crate::compiler_frontend::semantic_identity::StablePackageIdentity::project_local(
                            "html",
                        ),
                        String::new(),
                        crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
                    ),
                export_bindings: Vec::new(),
                export_diagnostic_provenance: Vec::new(),
                binding_exports: Vec::new(),
                declarations: Vec::new(),
                reusable_evidence: Vec::new(),
                concrete_call_summaries: Vec::new(),
            },
        }],
        Vec::new(),
    );

    assert_eq!(frontend.source_package_artifacts().len(), 1);
    assert!(
        frontend.source_package_artifacts()[0]
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
