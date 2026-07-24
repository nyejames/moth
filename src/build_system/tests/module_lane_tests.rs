//! Focused invariant tests for the compiled `Module` lane container.

use crate::build_system::build::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleExternalImport, ModuleLinkFacts,
    ModuleRootActivity,
};
use crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids::NONE;
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
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
    module.start_function = FunctionId(0);
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::EntryStart);
    module
        .side_table
        .bind_function_name(FunctionId(0), start_name_path);
    module
}

#[test]
fn remap_string_ids_routes_hir_through_executable_and_leaves_link_facts_untouched() {
    // WHAT: a module's interned HIR name remaps through the executable lane while the link-facts
    //       lane (which carries only resolved runtime asset identities and package IDs) is not
    //       remapped, and the metadata entry path is preserved.
    // WHY: the lane container must remap HIR and type identity exactly once and must never treat
    //      link-fact runtime asset paths or the resolved entry path as interned string state.

    let mut local_string_table = StringTable::new();
    let start_name_path = InternedPath::from_single_str("start_entry", &mut local_string_table);

    let hir_module = minimal_hir_module(start_name_path);

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
    let link_facts = ModuleLinkFacts {
        external_package_registry: Arc::new(ExternalPackageRegistry::new()),
        module_external_imports: vec![ModuleExternalImport {
            package_id: ExternalPackageId(11),
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: asset_path.clone(),
                asset_kind: String::from("js"),
            }),
            required_runtime_imports: vec![],
        }],
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

    // The link-facts lane carries no interned string IDs: the runtime asset path and package ID
    // are preserved unchanged through remap.
    let import = &module.link_facts.module_external_imports[0];
    assert_eq!(import.package_id, ExternalPackageId(11));
    assert_eq!(
        import.runtime_asset.as_ref().unwrap().canonical_source_path,
        asset_path
    );

    // The metadata entry path is a PathBuf, not interned, so it is preserved.
    assert_eq!(module.metadata.entry_point, entry_point);
}

#[test]
fn legacy_handoff_discards_validated_generic_template_store_before_remap() {
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
            module_external_imports: vec![],
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

    // The legacy flat-module handoff discards the unconsumed store before string-table remap
    // so its donor-local `StringId`s never reach backends. This mirrors the production
    // boundary in `compile_single_file_frontend` and `compile_directory_frontend`.
    module.metadata.discard_validated_generic_templates();
    assert!(
        module.metadata.validated_generic_templates.is_empty(),
        "the legacy handoff discards the store before backend remap"
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
