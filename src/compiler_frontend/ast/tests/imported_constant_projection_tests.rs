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
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind,
    ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedField, PublicFoldedValue,
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

/// Test-only import materialiser that delegates resource identity to a real consumer table and
/// interns builtin and anonymous-const-record canonical types through a fresh consumer
/// environment. Nominal origins have no consumer registration in this fixture.
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
        identity: &CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError> {
        match identity {
            CanonicalTypeIdentity::Builtin(builtin) => {
                let type_id = match builtin {
                    CanonicalBuiltinType::Bool => builtin_type_ids::BOOL,
                    CanonicalBuiltinType::Int => builtin_type_ids::INT,
                    CanonicalBuiltinType::Float => builtin_type_ids::FLOAT,
                    CanonicalBuiltinType::Decimal => builtin_type_ids::DECIMAL,
                    CanonicalBuiltinType::String => builtin_type_ids::STRING,
                    CanonicalBuiltinType::Char => builtin_type_ids::CHAR,
                    CanonicalBuiltinType::Range => builtin_type_ids::RANGE,
                    CanonicalBuiltinType::None => builtin_type_ids::NONE,

                    // The Error builtin is seeded by real module compilation, which this fixture
                    // never runs, so there is no consumer-local handle to return.
                    CanonicalBuiltinType::Error => {
                        return Err(CompilerError::compiler_error(
                            "record projection test materialiser has no consumer-local Error builtin",
                        ));
                    }
                };

                Ok(type_id)
            }

            // The marker interns to this fixture environment's one compile-time-only TypeId.
            CanonicalTypeIdentity::AnonymousConstRecord => {
                Ok(self.type_environment.anonymous_const_record_type())
            }

            _ => Err(CompilerError::compiler_error(
                "record projection test materialiser interns builtin and marker types only",
            )),
        }
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
    assert!(consumer_materialiser.module_resources.origins().is_empty());
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

#[test]
fn anonymous_const_record_round_trips_through_public_projection_and_import_materialisation() {
    let mut producer_string_table = StringTable::new();
    let producer_type_environment = TypeEnvironment::new();
    let producer_marker = producer_type_environment.anonymous_const_record_type();

    let nested_count = Declaration {
        id: InternedPath::from_single_str("count", &mut producer_string_table),
        value: Expression::int(7, SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let nested_record = Expression::anonymous_const_record(
        vec![nested_count],
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
        producer_marker,
    );

    let producer_fields = vec![
        Declaration {
            id: InternedPath::from_single_str("year", &mut producer_string_table),
            value: Expression::int(2026, SourceLocation::default(), ValueMode::ImmutableOwned),
        },
        Declaration {
            id: InternedPath::from_single_str("enabled", &mut producer_string_table),
            value: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
        },
        Declaration {
            id: InternedPath::from_single_str("nested", &mut producer_string_table),
            value: nested_record,
        },
    ];
    let constant_path = InternedPath::from_single_str("meta", &mut producer_string_table);
    let module_constant = Declaration {
        id: constant_path.clone(),
        value: Expression::anonymous_const_record(
            producer_fields,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
            producer_marker,
        ),
    };
    let const_values =
        ConstValueStore::from_test_declarations(vec![module_constant], &producer_type_environment)
            .expect("anonymous const record constant should be representable in the value store");

    let producer_module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    );
    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![ResolvedPublicTypeRoot {
            path: constant_path,
            kind: ResolvedPublicTypeRootKind::Constant {
                type_id: producer_marker,
            },
        }],
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };
    let export_seed = DirectExportSeed::new(
        producer_module_origin.clone(),
        vec![ExportBinding::new(
            producer_module_origin.clone(),
            "meta".to_owned(),
            OriginDeclarationId::Constant(OriginConstantId::new(
                producer_module_origin.clone(),
                "meta".to_owned(),
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
        module_resources: None,
    })
    .build()
    .expect("producer public projection should build")
    .draft;

    let PublicDeclarationSemantics::Constant(constant) = &draft.declarations[0].semantics else {
        panic!("expected the projected declaration to be a constant");
    };
    assert_eq!(
        constant.type_identity,
        CanonicalTypeIdentity::AnonymousConstRecord
    );
    let PublicFoldedValue::Record(exported_fields) = &constant.folded_value else {
        panic!("expected the projected constant to retain a folded record");
    };
    let exported_field_names = exported_fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(exported_field_names, ["year", "enabled", "nested"]);
    assert_eq!(
        exported_fields[0].type_identity,
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)
    );
    assert_eq!(
        exported_fields[2].type_identity,
        CanonicalTypeIdentity::AnonymousConstRecord
    );

    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser::new();
    let consumer_marker = consumer_materialiser
        .type_environment
        .anonymous_const_record_type();
    let mut consumer_string_table = StringTable::new();
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &constant.folded_value,
        consumer_marker,
        &mut consumer_string_table,
        &SourceLocation::default(),
    )
    .expect("consumer import projection should materialise an anonymous const record");

    assert_eq!(materialised.type_id, consumer_marker);
    assert_eq!(
        materialised.const_record_state,
        ConstRecordState::ConstRecord
    );
    let ExpressionKind::AnonymousConstRecord {
        fields: imported_fields,
    } = &materialised.kind
    else {
        panic!("expected consumer materialisation to rebuild an anonymous const record");
    };
    let imported_field_names = imported_fields
        .iter()
        .map(|field| field.id.name_str(&consumer_string_table))
        .collect::<Vec<_>>();
    assert_eq!(
        imported_field_names,
        vec![Some("year"), Some("enabled"), Some("nested")]
    );

    assert_eq!(imported_fields[0].value.type_id, builtin_type_ids::INT);
    assert!(matches!(
        imported_fields[0].value.kind,
        ExpressionKind::Int(2026)
    ));
    assert!(matches!(
        imported_fields[1].value.kind,
        ExpressionKind::Bool(true)
    ));

    // The nested anonymous field re-materializes as an anonymous const record, not a struct.
    assert_eq!(imported_fields[2].value.type_id, consumer_marker);
    assert_eq!(
        imported_fields[2].value.const_record_state,
        ConstRecordState::ConstRecord
    );
    let ExpressionKind::AnonymousConstRecord {
        fields: imported_nested_fields,
    } = &imported_fields[2].value.kind
    else {
        panic!("expected the nested imported field to stay an anonymous const record");
    };
    assert_eq!(imported_nested_fields.len(), 1);
    assert_eq!(
        imported_nested_fields[0]
            .id
            .name_str(&consumer_string_table),
        Some("count")
    );
    assert!(matches!(
        imported_nested_fields[0].value.kind,
        ExpressionKind::Int(7)
    ));
}

#[test]
fn named_struct_record_import_keeps_the_struct_instance_path() {
    let mut consumer_string_table = StringTable::new();
    let mut consumer_type_environment = TypeEnvironment::new();
    let title_path = InternedPath::from_single_str("title", &mut consumer_string_table);
    let year_path = InternedPath::from_single_str("year", &mut consumer_string_table);
    let struct_path = InternedPath::from_single_str("Defaults", &mut consumer_string_table);
    let (_, struct_type_id) =
        consumer_type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: struct_path,
            fields: Box::new([
                FieldDefinition {
                    name: title_path,
                    type_id: builtin_type_ids::STRING,
                    location: SourceLocation::default(),
                },
                FieldDefinition {
                    name: year_path,
                    type_id: builtin_type_ids::INT,
                    location: SourceLocation::default(),
                },
            ]),
            generic_parameters: None,
            const_record: false,
        });

    let folded = PublicFoldedValue::Record(vec![
        PublicFoldedField {
            name: "title".to_owned(),
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
            value: PublicFoldedValue::String(OwnedFoldedString::Text("Moth".to_owned())),
        },
        PublicFoldedField {
            name: "year".to_owned(),
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
            value: PublicFoldedValue::Int(2026),
        },
    ]);
    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser {
        type_environment: consumer_type_environment,
        module_resources: ModuleResourceTable::new(),
        template_ir_store: Rc::new(RefCell::new(TemplateIrStore::new())),
    };
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &folded,
        struct_type_id,
        &mut consumer_string_table,
        &SourceLocation::default(),
    )
    .expect("named struct record should keep the struct instance import path");

    assert_eq!(materialised.type_id, struct_type_id);
    let ExpressionKind::StructInstance(fields) = &materialised.kind else {
        panic!("named struct records must keep materialising as struct instances");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].id.name_str(&consumer_string_table), Some("title"));
    assert!(matches!(
        &fields[0].value.kind,
        ExpressionKind::StringSlice(_)
    ));
    assert_eq!(fields[1].id.name_str(&consumer_string_table), Some("year"));
    assert!(matches!(fields[1].value.kind, ExpressionKind::Int(2026)));
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
