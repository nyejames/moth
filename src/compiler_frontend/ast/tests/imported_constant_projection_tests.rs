//! Imported-constant declaration identity regression tests.
//!
//! WHAT: exercises the production append-and-publish boundary after semantic metadata holes and
//! compiler-owned rows.
//! WHY: imported constants must receive their ID from the declaration-table owner and publish that
//! exact ID to constant visibility; reconstructing either offset would let the two owners drift.

use super::*;
use super::{FoldedValueMaterialiser, materialize_public_folded_value};
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind,
    ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedValue,
};
use crate::compiler_frontend::headers::module_symbols::{
    CompilerOwnedDeclaration, CompilerOwnedDeclarationKind, DeclarationId,
    OrderedSemanticDeclaration, OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::public_interface::{
    DirectExportSeed, PublicDeclarationSemantics, PublicInterfaceDraftBuilder,
    PublicInterfaceDraftBuilderInput,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, ModuleRootRole, OriginConstantId, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

#[test]
fn imported_constant_uses_the_next_table_id_and_publishes_it() {
    let mut string_table = StringTable::new();
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);
    let trait_path = InternedPath::from_single_str("Trait", &mut string_table);
    let start_path = InternedPath::from_single_str("start", &mut string_table);
    let builtin_path = InternedPath::from_single_str("Builtin", &mut string_table);
    let imported_path = InternedPath::from_single_str("imported", &mut string_table);

    let mut declaration_table = Rc::new(
        TopLevelDeclarationTable::from_stage3_order(
            vec![
                metadata_record(0, &alias_path, OrderedSemanticDeclarationKind::TypeAlias),
                metadata_record(1, &trait_path, OrderedSemanticDeclarationKind::Trait),
            ],
            vec![
                compiler_owned(
                    CompilerOwnedDeclarationKind::Start,
                    declaration(
                        &start_path,
                        DataType::Function(Box::new(None), Default::default()),
                    ),
                ),
                compiler_owned(
                    CompilerOwnedDeclarationKind::Builtin,
                    declaration(&builtin_path, DataType::Inferred),
                ),
            ],
        )
        .expect("semantic holes and compiler-owned rows should build"),
    );
    let mut resolved_constants = Rc::new(ResolvedConstantSet::default());

    let declaration_id = append_projected_constant(
        &mut declaration_table,
        &mut resolved_constants,
        declaration(&imported_path, DataType::Bool),
    )
    .expect("imported constant should append through the production owner");

    assert_eq!(declaration_id.index(), 4);
    assert_eq!(
        declaration_table.declaration_id_by_path(&imported_path),
        Some(declaration_id)
    );
    assert!(resolved_constants.contains(declaration_id));
}

/// Test-only import materialiser that delegates resource identity to a real consumer table.
struct ConsumerFoldedValueMaterialiser {
    type_environment: TypeEnvironment,
    module_resources: ModuleResourceTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
}

impl ConsumerFoldedValueMaterialiser {
    fn new() -> Self {
        Self {
            type_environment: TypeEnvironment::new(),
            module_resources: ModuleResourceTable::new(),
            template_ir_store: Rc::new(RefCell::new(TemplateIrStore::new())),
        }
    }
}

impl FoldedValueMaterialiser for ConsumerFoldedValueMaterialiser {
    fn intern_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        location: &SourceLocation,
    ) -> Result<ResourceId, CompilerError> {
        Ok(self
            .module_resources
            .intern_origin(origin.clone(), location.clone()))
    }

    fn intern_canonical_type(
        &mut self,
        _identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError> {
        Err(CompilerError::compiler_error(
            "structural-string test materialiser does not intern canonical types",
        ))
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }

    fn template_ir_store(&self) -> Rc<RefCell<TemplateIrStore>> {
        Rc::clone(&self.template_ir_store)
    }
}

#[test]
fn structural_string_round_trips_through_public_projection_and_import_materialisation() {
    let mut producer_string_table = StringTable::new();
    let producer_type_environment = TypeEnvironment::new();
    let producer_string_type_id = producer_type_environment.builtins().string;
    let producer_module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    );
    let producer_resource_origin = StableResourceOriginId::module_owned(
        producer_module_origin.clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("relative resource path should be portable"),
    );
    let producer_offset_origin = StableResourceOriginId::module_owned(
        producer_module_origin.clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/offset.css"))
            .expect("relative resource path should be portable"),
    );
    let mut producer_resources = ModuleResourceTable::new();

    // Keep the producer handle nonzero so the empty consumer table must mint a different local ID.
    producer_resources.intern_origin(producer_offset_origin, SourceLocation::default());
    let producer_resource_id = producer_resources
        .intern_origin(producer_resource_origin.clone(), SourceLocation::default());
    let prefix = producer_string_table.intern("assets/");
    let constant_path = InternedPath::from_single_str("logo", &mut producer_string_table);
    let producer_pieces = vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(producer_resource_id),
        ConstStringPiece::SiteRoot,
    ];
    let module_constant = Declaration {
        id: constant_path.clone(),
        value: Expression::structural_string(producer_pieces, SourceLocation::default()),
    };
    let const_values =
        ConstValueStore::from_test_declarations(vec![module_constant], &producer_type_environment)
            .expect("structural string constant should be representable in the value store");

    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![ResolvedPublicTypeRoot {
            path: constant_path,
            kind: ResolvedPublicTypeRootKind::Constant {
                type_id: producer_string_type_id,
            },
        }],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };
    let export_seed = DirectExportSeed::new(
        producer_module_origin.clone(),
        vec![ExportBinding::new(
            producer_module_origin.clone(),
            "logo".to_owned(),
            OriginDeclarationId::Constant(OriginConstantId::new(
                producer_module_origin.clone(),
                "logo".to_owned(),
            )),
        )],
        FxHashMap::default(),
    );
    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(Rc::new(TraitEvidenceEnvironment::new())),
    };
    let registry = ExternalPackageRegistry::new();
    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: &FxHashMap::default(),
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: &producer_type_environment,
        external_registry: &registry,
        string_table: &producer_string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &const_values,
        module_resources: Some(&producer_resources),
    })
    .build()
    .expect("producer public projection should build")
    .draft;

    let PublicDeclarationSemantics::Constant(constant) = &draft.declarations[0].semantics else {
        panic!("expected the projected declaration to be a constant");
    };
    let PublicFoldedValue::String(OwnedFoldedString::Pieces(projected_pieces)) =
        &constant.folded_value
    else {
        panic!("expected the projected constant to retain structural string pieces");
    };
    assert_eq!(
        projected_pieces,
        &vec![
            OwnedFoldedStringPiece::Text("assets/".to_owned()),
            OwnedFoldedStringPiece::Resource(producer_resource_origin.clone()),
            OwnedFoldedStringPiece::SiteRoot,
        ]
    );

    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser::new();
    assert!(consumer_materialiser.module_resources.is_empty());
    let consumer_string_type_id = consumer_materialiser.type_environment.builtins().string;
    let mut consumer_string_table = StringTable::new();
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &constant.folded_value,
        consumer_string_type_id,
        &mut consumer_string_table,
        &SourceLocation::default(),
    )
    .expect("consumer import projection should materialise structural string pieces");
    let ExpressionKind::StructuralString {
        pieces: consumer_pieces,
    } = &materialised.kind
    else {
        panic!("expected consumer materialisation to retain structural string pieces");
    };
    let [
        ConstStringPiece::Text(consumer_text),
        ConstStringPiece::Resource(consumer_resource),
        ConstStringPiece::SiteRoot,
    ] = consumer_pieces.as_slice()
    else {
        panic!("consumer materialisation changed structural string piece order");
    };
    assert_eq!(consumer_string_table.resolve(*consumer_text), "assets/");
    assert_ne!(*consumer_resource, producer_resource_id);
    let consumer_origin = consumer_materialiser
        .module_resources
        .try_origin(*consumer_resource)
        .expect("consumer resource handle should resolve in its own table")
        .origin
        .clone();
    assert_eq!(consumer_origin, producer_resource_origin);
}

fn metadata_record(
    index: usize,
    path: &InternedPath,
    kind: OrderedSemanticDeclarationKind,
) -> OrderedSemanticDeclaration {
    OrderedSemanticDeclaration {
        declaration_id: DeclarationId::from_index(index),
        header_index: index,
        path: path.clone(),
        kind,
        declaration: None,
    }
}

fn declaration(path: &InternedPath, data_type: DataType) -> Declaration {
    Declaration {
        id: path.clone(),
        value: Expression::no_value(
            SourceLocation::default(),
            data_type,
            ValueMode::ImmutableOwned,
        ),
    }
}

fn compiler_owned(
    kind: CompilerOwnedDeclarationKind,
    declaration: Declaration,
) -> CompilerOwnedDeclaration {
    CompilerOwnedDeclaration { kind, declaration }
}
