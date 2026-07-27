//! Focused invariant tests for the compiled `Module` lane container.

use crate::build_system::build::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleExternalImport, ModuleLinkFacts,
    ModuleRootActivity, ProjectCompilation,
};
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalFunctionId, ExternalPackageId, ExternalPackageRegistry,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use crate::compiler_frontend::validated_generic_template_metadata::ValidatedGenericTemplateStore;

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
        return_aliases: vec![],
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
    let mut hir_module = minimal_hir_module(start_name_path);
    hir_module.blocks[0].statements.push(HirStatement {
        id: HirNodeId(1),
        kind: HirStatementKind::Expr(HirExpression {
            id: HirValueId(1),
            kind: HirExpressionKind::MapLiteral(vec![]),
            ty: NONE,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
        }),
        location: SourceLocation::new(
            source_scope,
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

    let entry_point = PathBuf::from("src/#page.moth");
    let mut module = Module {
        executable: ModuleExecutable {
            hir: hir_module,
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts,
        metadata: ModuleCompilerMetadata {
            entry_point: entry_point.clone(),
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: ModuleRootActivity::default(),
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            validated_generic_templates: ValidatedGenericTemplateStore::default(),
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
            entry_point: PathBuf::from("#page.moth"),
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: ModuleRootActivity {
                has_non_trivial_root_body: true,
                ..ModuleRootActivity::default()
            },
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            validated_generic_templates: ValidatedGenericTemplateStore::default(),
        },
    };

    let error = match ProjectCompilation::from_successful_modules(vec![module]) {
        Ok(_) => panic!("missing external package ownership should violate entry assembly"),
        Err(error) => error,
    };
    assert!(error.msg.contains("has no owning package"));
}

#[test]
fn pre_provider_handoff_discards_validated_generic_template_store_before_remap() {
    use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
    use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
    use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
    use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StablePackageIdentity};
    use crate::compiler_frontend::semantic_identity::{
        OriginFunctionId, StableModuleOriginIdentity,
    };
    use crate::compiler_frontend::symbols::interned_path::InternedPath;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
    use crate::compiler_frontend::validated_generic_template_metadata::{
        ValidatedGenericTemplateArtefact, ValidatedGenericTemplateStore,
    };

    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("identity", &mut string_table);
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    );
    let origin = OriginFunctionId::new_free(module_origin, "identity".to_owned());

    let template = GenericFunctionTemplate {
        function_path: path.to_owned(),
        source_file: InternedPath::new(),
        generic_parameter_list_id: GenericParameterListId(0),
        signature: FunctionSignature::default(),
        body_tokens: FileTokens::new(path, vec![]),
        declaration_location: SourceLocation::default(),
    };

    let store =
        ValidatedGenericTemplateStore::from_artefacts(vec![ValidatedGenericTemplateArtefact {
            origin: origin.clone(),
            template,
        }]);

    let mut module = Module {
        executable: ModuleExecutable {
            hir: minimal_hir_module(InternedPath::new()),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: vec![],
            functions: collect_module_function_link_facts(&minimal_hir_module(InternedPath::new()))
                .expect("test HIR should produce function link facts"),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::from("src/#page.moth"),
            warnings: vec![],
            const_top_level_fragments: vec![],
            root_activity: ModuleRootActivity::default(),
            doc_fragments: vec![],
            rendered_path_usages: vec![],
            validated_generic_templates: store,
        },
    };

    // The store is carried in metadata with one artefact keyed by the exact origin.
    assert_eq!(module.metadata.validated_generic_templates.len(), 1);
    assert_eq!(
        module.metadata.validated_generic_templates.artefacts()[0].origin,
        origin
    );

    // The pre-provider project-compilation handoff discards the unconsumed store before
    // string-table remap so its donor-local `StringId`s never reach backends. This mirrors the
    // production boundary in `compile_single_file_frontend` and `compile_directory_frontend`.
    module.metadata.discard_validated_generic_templates();
    assert!(
        module.metadata.validated_generic_templates.is_empty(),
        "the pre-provider handoff discards the store before backend remap"
    );

    // Remap runs only after the store is empty, so no unremappable template state remains.
    let mut merged_string_table = StringTable::new();
    merged_string_table.intern("prefix");
    let remap = merged_string_table.merge_from(&string_table);
    module.remap_string_ids(&remap);

    assert!(
        module.metadata.validated_generic_templates.is_empty(),
        "remap must not resurrect the discarded store"
    );
}
