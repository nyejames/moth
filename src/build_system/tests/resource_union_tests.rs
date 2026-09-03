//! WHAT: proves exact stable-origin union ownership and ordering for entry/package planning.
//! WHY: inversion tests make unreachable and site-root structural values fail the union if a
//! planner accidentally scans every interned origin or treats SiteRoot as a resource.

use crate::build_system::resource_unions::{
    ResourceOriginUnion, append_entry_module_resources, append_exported_interface_resources,
    append_public_folded_value, append_reachable_resource_uses,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::{ErrorType, SourceLocation};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedValue,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{HirReachability, ReachableResourceUse};
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::module_compilation::artefact::{
    ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ResolvedConstFragment,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, OriginDeclarationId,
    StableModuleOriginIdentity, StablePackageIdentity,
};
use std::path::PathBuf;
use std::sync::Arc;

fn origin(path: &str) -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("resource-union-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_portable_spelling(path.to_owned())
            .expect("test resource path should be valid"),
    )
}

fn module_with_resource_table(resources: ModuleResourceTable) -> Module {
    Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            resource_table: resources,
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: Default::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity: Default::default(),
            doc_fragments: Vec::new(),
            materialisation_context: None,
        },
    }
}

#[test]
fn reachable_resource_use_reads_only_live_origins_through_its_table() {
    let live = origin("assets/live.svg");
    let unreachable = origin("assets/unreachable.svg");
    let mut resources = ModuleResourceTable::new();
    let live_id = resources.intern_origin(live.clone(), SourceLocation::default());
    let _unreachable_id = resources.intern_origin(unreachable.clone(), SourceLocation::default());
    let module = module_with_resource_table(resources);
    let mut reachability = HirReachability::default();
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id: live_id,
            owner: crate::compiler_frontend::hir::ids::FunctionId(0),
            location: SourceLocation::default(),
        });

    let mut union = ResourceOriginUnion::new();
    append_reachable_resource_uses(&mut union, &module, &reachability)
        .expect("the supplied module owns the reachable resource handle");

    assert_eq!(union.origins(), &[live]);
    assert!(!union.origins().contains(&unreachable));
}

#[test]
fn entry_union_includes_const_fragment_resource_without_executable_use() {
    let fragment_resource = origin("assets/fragment.svg");
    let mut module = module_with_resource_table(ModuleResourceTable::new());
    module
        .metadata
        .const_top_level_fragments
        .push(ResolvedConstFragment {
            runtime_insertion_index: 0,
            location: SourceLocation::default(),
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(
                fragment_resource.clone(),
            )]),
        });

    let mut union = ResourceOriginUnion::new();
    append_entry_module_resources(&mut union, &module, &HirReachability::default())
        .expect("an empty executable reachability still permits metadata planning");

    assert_eq!(union.origins(), &[fragment_resource]);
}

#[test]
fn repeated_reachable_origin_is_one_union_member_in_first_seen_order() {
    let first = origin("assets/first.svg");
    let second = origin("assets/second.svg");
    let mut union = ResourceOriginUnion::new();
    union.insert(first.clone());
    union.insert(second.clone());
    union.insert(first.clone());

    assert_eq!(union.origins(), &[first, second]);
}

#[test]
fn site_root_piece_is_not_a_resource_union_member() {
    let live = origin("assets/live.svg");
    let mut union = ResourceOriginUnion::new();
    append_public_folded_value(
        &mut union,
        &PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::SiteRoot,
            OwnedFoldedStringPiece::Resource(live.clone()),
        ])),
    );

    assert_eq!(union.origins(), &[live]);
}

#[test]
fn package_exported_folded_resource_is_selected_but_private_unused_origin_is_not() {
    let exported_resource = origin("assets/exported.svg");
    let private_resource = origin("assets/private.svg");
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("resource-union-tests"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let exported_constant = OriginConstantId::new(module_origin.clone(), "exported".to_owned());
    let private_constant = OriginConstantId::new(module_origin.clone(), "private".to_owned());
    let interface = PublicSemanticInterface {
        module_origin: module_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            module_origin.clone(),
            "exported".to_owned(),
            OriginDeclarationId::Constant(exported_constant.clone()),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: vec![
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Constant(exported_constant.clone()),
                synthetic_interface_provenance: Default::default(),
                semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                    type_identity: CanonicalTypeIdentity::Builtin(
                        crate::compiler_frontend::canonical_type_identity::CanonicalBuiltinType::String,
                    ),
                    folded_value: PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
                        OwnedFoldedStringPiece::Resource(exported_resource.clone()),
                    ])),
                }),
            },
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Constant(private_constant),
                synthetic_interface_provenance: Default::default(),
                semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                    type_identity: CanonicalTypeIdentity::Builtin(
                        crate::compiler_frontend::canonical_type_identity::CanonicalBuiltinType::String,
                    ),
                    folded_value: PublicFoldedValue::String(OwnedFoldedString::Pieces(vec![
                        OwnedFoldedStringPiece::Resource(private_resource.clone()),
                    ])),
                }),
            },
        ],
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let mut union = ResourceOriginUnion::new();
    append_exported_interface_resources(&mut union, &interface)
        .expect("well-formed exports should have declarations");

    assert_eq!(union.origins(), &[exported_resource]);
    assert!(!union.origins().contains(&private_resource));
}

#[test]
fn package_export_join_rejects_export_without_public_declaration() {
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("resource-union-tests"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let missing_constant = OriginConstantId::new(module_origin.clone(), "missing".to_owned());
    let interface = PublicSemanticInterface {
        module_origin: module_origin.clone(),
        export_bindings: vec![ExportBinding::new(
            module_origin,
            "missing".to_owned(),
            OriginDeclarationId::Constant(missing_constant),
        )],
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    };

    let mut union = ResourceOriginUnion::new();
    let error = append_exported_interface_resources(&mut union, &interface)
        .expect_err("an export binding without a declaration is malformed");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("no matching public declaration"));
    assert!(union.is_empty());
}
