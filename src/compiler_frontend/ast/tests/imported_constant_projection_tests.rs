//! Imported-constant declaration identity regression tests.
//!
//! WHAT: exercises the production append-and-publish boundary after semantic metadata holes and
//! compiler-owned rows.
//! WHY: imported constants must receive their ID from the declaration-table owner and publish that
//! exact ID to constant visibility; reconstructing either offset would let the two owners drift.

use super::*;
use super::{FoldedValueMaterialiser, materialize_public_folded_value};
use crate::compiler_frontend::ast::ast_nodes::DeclarationShell;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind,
    ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity, intern_canonical_builtin,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicFoldedField, PublicFoldedValue,
};
use crate::compiler_frontend::headers::module_symbols::{
    DeclarationId, OrderedSemanticDeclaration, OrderedSemanticDeclarationKind,
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::path::Path;
use std::rc::Rc;

#[test]
fn imported_constant_uses_the_next_table_id_and_publishes_it() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let alias_path = path_fork
        .try_intern_portable_path("Alias", &mut string_table)
        .expect("test path fits");
    let trait_path = path_fork
        .try_intern_portable_path("Trait", &mut string_table)
        .expect("test path fits");
    let start_path = path_fork
        .try_intern_portable_path("start", &mut string_table)
        .expect("test path fits");
    let builtin_path = path_fork
        .try_intern_portable_path("Builtin", &mut string_table)
        .expect("test path fits");
    let imported_path = path_fork
        .try_intern_portable_path("imported", &mut string_table)
        .expect("test path fits");

    let mut declaration_table = Rc::new(
        TopLevelDeclarationTable::from_stage3_order(
            vec![
                metadata_record(0, &alias_path, OrderedSemanticDeclarationKind::TypeAlias),
                metadata_record(1, &trait_path, OrderedSemanticDeclarationKind::Trait),
            ],
            vec![
                Declaration {
                    value: DeclarationValue::Shell(DeclarationShell::Function { receiver: None }),
                    value_mode: ValueMode::ImmutableReference,
                    ..declaration(&start_path, builtin_type_ids::NONE)
                },
                declaration(&builtin_path, builtin_type_ids::NONE),
            ],
            &path_fork,
        )
        .expect("semantic holes and compiler-owned rows should build"),
    );
    let mut resolved_constants = Rc::new(ResolvedConstantSet::default());

    let declaration_id = append_projected_constant(
        &mut declaration_table,
        &mut resolved_constants,
        {
            let mut constants = ConstValueStore::default();
            let folded = constants
                .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
                .expect("fixture bool fits store");
            constants
                .bind_module_constant(imported_path, folded)
                .expect("fixture bool binds");
            let mut declaration = declaration(&imported_path, builtin_type_ids::BOOL);
            declaration.value = DeclarationValue::Default(folded);
            declaration
        },
        &path_fork,
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
    path_fork: PathInternerFork,
}

impl ConsumerFoldedValueMaterialiser {
    fn new() -> Self {
        Self {
            type_environment: TypeEnvironment::new(),
            module_resources: ModuleResourceTable::new(),
            path_fork: PathInternerFork::empty(),
        }
    }
}

impl FoldedValueMaterialiser for ConsumerFoldedValueMaterialiser {
    fn intern_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<ResourceId, InfrastructureFailure> {
        Ok(self.module_resources.intern_origin(origin.clone(), span))
    }

    fn intern_canonical_type(
        &mut self,
        identity: &CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, ConstValueStoreError> {
        match identity {
            CanonicalTypeIdentity::Builtin(builtin) => {
                // The `Error` builtin is seeded by real module compilation, which this fixture
                // never runs, so there is no consumer-local handle to return.
                intern_canonical_builtin(*builtin, &mut self.type_environment).ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "record projection test materialiser has no consumer-local Error builtin",
                    )
                    .into()
                })
            }

            // The marker interns to this fixture environment's one compile-time-only TypeId.
            CanonicalTypeIdentity::AnonymousConstRecord => {
                Ok(self.type_environment.anonymous_const_record_type())
            }

            _ => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "record projection test materialiser interns builtin and marker types only",
            )
            .into()),
        }
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }

    fn path_fork(&mut self) -> &mut PathInternerFork {
        &mut self.path_fork
    }
}

#[test]
fn structural_string_round_trips_through_public_projection_and_import_materialisation() {
    let mut producer_string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
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
    producer_resources.intern_origin(producer_offset_origin, None);
    let producer_resource_id =
        producer_resources.intern_origin(producer_resource_origin.clone(), None);
    let prefix = producer_string_table.intern("assets/");
    let constant_path = path_fork
        .try_intern_portable_path("logo", &mut producer_string_table)
        .expect("test path fits");
    let producer_pieces = vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(producer_resource_id),
        ConstStringPiece::SiteRoot,
    ];
    let mut const_values = ConstValueStore::default();
    let structural_id = const_values
        .insert_resolved(
            producer_string_type_id,
            ConstValuePayload::String(ConstStringValue::Pieces(producer_pieces)),
            None,
        )
        .expect("structural string constant should be representable in the value store");
    const_values
        .bind_module_constant(constant_path, structural_id)
        .expect("structural string constant binds");

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
        path_fork: &path_fork,
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
    let mut consumer_constants = ConstValueStore::default();
    let _path_fork = PathInternerFork::empty();
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &constant.folded_value,
        consumer_string_type_id,
        &mut consumer_constants,
        &mut consumer_string_table,
        None,
    )
    .expect("consumer import projection should materialise structural string pieces");
    let ConstValuePayload::String(ConstStringValue::Pieces(consumer_pieces)) = consumer_constants
        .payload(materialised)
        .expect("materialised string has content")
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
    let mut path_fork = PathInternerFork::empty();
    let producer_type_environment = TypeEnvironment::new();
    let producer_marker = producer_type_environment.anonymous_const_record_type();
    let mut const_values = ConstValueStore::default();
    let year_id = const_values
        .insert_resolved(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 2026).expect("2026 fits I32"),
            ),
            None,
        )
        .expect("fixture year fits store");
    let enabled_id = const_values
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("fixture enabled fits store");
    let count_id = const_values
        .insert_resolved(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32"),
            ),
            None,
        )
        .expect("fixture count fits store");
    let count_path = path_fork
        .try_intern_portable_path("count", &mut producer_string_table)
        .expect("test path fits");
    let nested_id = {
        use crate::compiler_frontend::ast::const_values::store::{
            ConstValueField, ConstValueMetadata,
        };
        use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
        let mut metadata =
            ConstValueMetadata::resolved(producer_marker, None, ValueMode::ImmutableOwned);
        metadata.const_record_state = ConstRecordState::ConstRecord;
        metadata.value_kind = ConstValueKind::Composite;
        metadata.hir_visible = false;
        const_values
            .insert_value(
                metadata,
                ConstValuePayload::Record(vec![ConstValueField {
                    name: count_path,
                    value: count_id,
                }]),
            )
            .expect("fixture nested record fits store")
    };
    let year_path = path_fork
        .try_intern_portable_path("year", &mut producer_string_table)
        .expect("test path fits");
    let enabled_path = path_fork
        .try_intern_portable_path("enabled", &mut producer_string_table)
        .expect("test path fits");
    let nested_path = path_fork
        .try_intern_portable_path("nested", &mut producer_string_table)
        .expect("test path fits");
    let constant_path = path_fork
        .try_intern_portable_path("meta", &mut producer_string_table)
        .expect("test path fits");
    {
        use crate::compiler_frontend::ast::const_values::store::{
            ConstValueField, ConstValueMetadata,
        };
        use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
        let mut metadata =
            ConstValueMetadata::resolved(producer_marker, None, ValueMode::ImmutableOwned);
        metadata.const_record_state = ConstRecordState::ConstRecord;
        metadata.value_kind = ConstValueKind::Composite;
        metadata.hir_visible = false;
        let root_id = const_values
            .insert_value(
                metadata,
                ConstValuePayload::Record(vec![
                    ConstValueField {
                        name: year_path,
                        value: year_id,
                    },
                    ConstValueField {
                        name: enabled_path,
                        value: enabled_id,
                    },
                    ConstValueField {
                        name: nested_path,
                        value: nested_id,
                    },
                ]),
            )
            .expect("anonymous const record constant should be representable in the value store");
        const_values
            .bind_module_constant(constant_path, root_id)
            .expect("anonymous const record binds");
    }

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
        path_fork: &path_fork,
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
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(FixedScalar::I32))
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
    let mut consumer_constants = ConstValueStore::default();
    let _path_fork = PathInternerFork::empty();
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &constant.folded_value,
        consumer_marker,
        &mut consumer_constants,
        &mut consumer_string_table,
        None,
    )
    .expect("consumer import projection should materialise an anonymous const record");

    assert_eq!(
        consumer_constants
            .metadata(materialised)
            .expect("materialised record has content")
            .type_id,
        consumer_marker
    );
    assert_eq!(
        consumer_constants
            .metadata(materialised)
            .expect("materialised record has content")
            .const_record_state,
        ConstRecordState::ConstRecord
    );
    let ConstValuePayload::Record(imported_fields) = consumer_constants
        .payload(materialised)
        .expect("materialised record has content")
    else {
        panic!("expected consumer materialisation to rebuild an anonymous const record");
    };
    let imported_field_names = imported_fields
        .iter()
        .map(|field| {
            consumer_materialiser
                .path_fork
                .component(field.name)
                .map(|id| consumer_string_table.resolve(id))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        imported_field_names,
        vec![Some("year"), Some("enabled"), Some("nested")]
    );

    assert_eq!(
        consumer_constants
            .metadata(imported_fields[0].value)
            .expect("imported field has content")
            .type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert!(matches!(
        consumer_constants.payload(imported_fields[0].value),
        Some(ConstValuePayload::FixedScalar(value))
            if *value == FixedScalarValue::signed(FixedScalar::I32, 2026).expect("2026 fits I32")
    ));
    assert!(matches!(
        consumer_constants.payload(imported_fields[1].value),
        Some(ConstValuePayload::Bool(true))
    ));

    // The nested anonymous field re-materializes as an anonymous const record, not a struct.
    assert_eq!(
        consumer_constants
            .metadata(imported_fields[2].value)
            .expect("nested field has content")
            .type_id,
        consumer_marker
    );
    assert_eq!(
        consumer_constants
            .metadata(imported_fields[2].value)
            .expect("nested field has content")
            .const_record_state,
        ConstRecordState::ConstRecord
    );
    let ConstValuePayload::Record(imported_nested_fields) = consumer_constants
        .payload(imported_fields[2].value)
        .expect("nested field has content")
    else {
        panic!("expected the nested imported field to stay an anonymous const record");
    };
    assert_eq!(imported_nested_fields.len(), 1);
    assert_eq!(
        consumer_materialiser
            .path_fork
            .component(imported_nested_fields[0].name)
            .map(|id| consumer_string_table.resolve(id)),
        Some("count")
    );
    assert!(matches!(
        consumer_constants.payload(imported_nested_fields[0].value),
        Some(ConstValuePayload::FixedScalar(value))
            if *value == FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32")
    ));
}

#[test]
fn fixed_scalar_round_trips_through_import_materialisation_with_identity() {
    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser::new();
    let mut consumer_string_table = StringTable::new();
    let mut consumer_constants = ConstValueStore::default();

    // `U64::MAX` must keep its exact bits: a narrower carrier cannot represent it.
    let u64_max =
        FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64::MAX fits U64");
    let u64_type = builtin_type_ids::fixed_scalar(FixedScalar::U64);
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &PublicFoldedValue::FixedScalar(u64_max),
        u64_type,
        &mut consumer_constants,
        &mut consumer_string_table,
        None,
    )
    .expect("a fixed-scalar folded value should import");
    assert_eq!(
        consumer_constants
            .metadata(materialised)
            .expect("materialised scalar has content")
            .type_id,
        u64_type
    );
    assert!(matches!(
        consumer_constants.payload(materialised),
        Some(ConstValuePayload::FixedScalar(value)) if *value == u64_max
    ));

    // -0.0 stays negative through the import path: `==` would hide a sign flip.
    let neg_zero_f16 =
        FixedScalarValue::binary_float(FixedScalar::F16, -0.0).expect("-0.0 is a finite F16 value");
    let f16_type = builtin_type_ids::fixed_scalar(FixedScalar::F16);
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &PublicFoldedValue::FixedScalar(neg_zero_f16),
        f16_type,
        &mut consumer_constants,
        &mut consumer_string_table,
        None,
    )
    .expect("a signed-zero folded value should import");
    assert_eq!(
        consumer_constants
            .metadata(materialised)
            .expect("materialised scalar has content")
            .type_id,
        f16_type
    );
    let Some(ConstValuePayload::FixedScalar(imported)) = consumer_constants.payload(materialised)
    else {
        panic!("expected the import to rebuild a fixed-scalar value");
    };
    assert_eq!(
        f64::to_bits(imported.as_f64().expect("F16 reads as f64")),
        0x8000_0000_0000_0000
    );

    // Equal numeric payloads with distinct scalars keep distinct identities.
    let u8_five = FixedScalarValue::unsigned(FixedScalar::U8, 5).expect("5 fits U8");
    let u16_five = FixedScalarValue::unsigned(FixedScalar::U16, 5).expect("5 fits U16");
    let byte_five = FixedScalarValue::unsigned(FixedScalar::Byte, 5).expect("5 fits Byte");
    for (scalar, value) in [
        (FixedScalar::U8, u8_five),
        (FixedScalar::U16, u16_five),
        (FixedScalar::Byte, byte_five),
    ] {
        let expected = builtin_type_ids::fixed_scalar(scalar);
        let materialised = materialize_public_folded_value(
            &mut consumer_materialiser,
            &PublicFoldedValue::FixedScalar(value),
            expected,
            &mut consumer_constants,
            &mut consumer_string_table,
            None,
        )
        .expect("each scalar identity should import");
        assert_eq!(
            consumer_constants
                .metadata(materialised)
                .expect("materialised scalar has content")
                .type_id,
            expected
        );
        assert!(matches!(
            consumer_constants.payload(materialised),
            Some(ConstValuePayload::FixedScalar(round_tripped)) if *round_tripped == value
        ));
    }
    assert_ne!(u8_five, u16_five);
    assert_ne!(u8_five, byte_five);
}

#[test]
fn folded_range_import_preserves_each_endpoint_type_and_value() {
    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser::new();
    let mut consumer_string_table = StringTable::new();
    let mut consumer_constants = ConstValueStore::default();
    let cases = [
        (
            FixedScalarValue::unsigned(FixedScalar::U16, u64::from(u16::MAX))
                .expect("U16 maximum fits U16"),
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 maximum fits U64"),
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F16, -0.0)
                .expect("negative zero is finite"),
            FixedScalarValue::binary_float(FixedScalar::F32, f64::from(f32::from_bits(1)))
                .expect("smallest F32 subnormal is finite"),
        ),
    ];

    for (start, end) in cases {
        let folded = PublicFoldedValue::Range {
            start: Box::new(PublicFoldedValue::FixedScalar(start)),
            end: Box::new(PublicFoldedValue::FixedScalar(end)),
        };
        let imported = materialize_public_folded_value(
            &mut consumer_materialiser,
            &folded,
            builtin_type_ids::RANGE,
            &mut consumer_constants,
            &mut consumer_string_table,
            None,
        )
        .expect("folded numeric range should import");
        let ConstValuePayload::Range {
            start: imported_start,
            end: imported_end,
        } = consumer_constants
            .payload(imported)
            .expect("imported range has content")
        else {
            panic!("range import must preserve its endpoint values");
        };

        for (endpoint, expected) in [(*imported_start, start), (*imported_end, end)] {
            assert_eq!(
                consumer_constants
                    .metadata(endpoint)
                    .expect("range endpoint has content")
                    .type_id,
                builtin_type_ids::fixed_scalar(expected.scalar())
            );
            assert!(matches!(
                consumer_constants.payload(endpoint),
                Some(ConstValuePayload::FixedScalar(value)) if *value == expected
            ));
        }
    }
}

#[test]
fn named_struct_record_import_keeps_the_struct_instance_path() {
    let mut consumer_string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut consumer_type_environment = TypeEnvironment::new();
    let title_path = path_fork
        .try_intern_portable_path("title", &mut consumer_string_table)
        .expect("test path fits");
    let year_path = path_fork
        .try_intern_portable_path("year", &mut consumer_string_table)
        .expect("test path fits");
    let struct_path = path_fork
        .try_intern_portable_path("Defaults", &mut consumer_string_table)
        .expect("test path fits");
    let (_, struct_type_id) =
        consumer_type_environment.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: struct_path,
            fields: Box::new([
                FieldDefinition {
                    name: title_path,
                    type_id: builtin_type_ids::STRING,
                    span: None,
                },
                FieldDefinition {
                    name: year_path,
                    type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    span: None,
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
            type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::FixedScalar(
                FixedScalar::I32,
            )),
            value: PublicFoldedValue::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 2026).expect("2026 fits I32"),
            ),
        },
    ]);
    let mut consumer_materialiser = ConsumerFoldedValueMaterialiser {
        type_environment: consumer_type_environment,
        module_resources: ModuleResourceTable::new(),
        path_fork,
    };
    let mut consumer_constants = ConstValueStore::default();
    let materialised = materialize_public_folded_value(
        &mut consumer_materialiser,
        &folded,
        struct_type_id,
        &mut consumer_constants,
        &mut consumer_string_table,
        None,
    )
    .expect("named struct record should keep the struct instance import path");

    assert_eq!(
        consumer_constants
            .metadata(materialised)
            .expect("materialised record has content")
            .type_id,
        struct_type_id
    );
    let ConstValuePayload::Record(fields) = consumer_constants
        .payload(materialised)
        .expect("materialised record has content")
    else {
        panic!("named struct records must keep materialising as struct instances");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(
        consumer_materialiser
            .path_fork
            .component(fields[0].name)
            .map(|id| consumer_string_table.resolve(id)),
        Some("title")
    );
    assert!(matches!(
        consumer_constants.payload(fields[0].value),
        Some(ConstValuePayload::String(ConstStringValue::Text(_)))
    ));
    assert_eq!(
        consumer_materialiser
            .path_fork
            .component(fields[1].name)
            .map(|id| consumer_string_table.resolve(id)),
        Some("year")
    );
    assert!(matches!(
        consumer_constants.payload(fields[1].value),
        Some(ConstValuePayload::FixedScalar(value))
            if *value == FixedScalarValue::signed(FixedScalar::I32, 2026).expect("2026 fits I32")
    ));
}

fn metadata_record(
    index: usize,
    path: &PathId,
    kind: OrderedSemanticDeclarationKind,
) -> OrderedSemanticDeclaration {
    OrderedSemanticDeclaration {
        declaration_id: DeclarationId::from_index(index),
        header_index: index,
        path: *path,
        kind,
        declaration: None,
    }
}

fn declaration(path: &PathId, type_id: TypeId) -> Declaration {
    Declaration {
        id: *path,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    }
}
