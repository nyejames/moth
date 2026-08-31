//! Focused unit tests for folded-value projection of directly exported constants.
//!
//! WHAT: exercises the constant folded-value projection owned by the declaration-record
//! projection in `direct_projection`: exact defining-path join, owned backend-neutral value
//! vocabulary, option-present/absent projection, finite-float semantics, and totality
//! failures for missing, duplicate and unsupported folded facts. These are projection
//! invariants integration output cannot inspect, so they own a focused test beside the
//! projection owner.
//!
//! This module reuses shared fixtures from `test_support`.

use super::super::{
    DirectExportSeed, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput,
};
use super::test_support::{
    choice_origin, constant_origin, module_origin, nominal_origins_map, register_struct,
    struct_origin,
};
use crate::compiler_frontend::ast::AstPublicInterfaceProjectionInput;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, Expression, ExpressionKind,
};
use crate::compiler_frontend::ast::{
    ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FiniteFloat, OwnedFoldedString, PublicFoldedValue, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
//  Shared helpers
// ---------------------------------------------------------------------------

/// Build a constant declaration root whose path is the single-component public name.
fn constant_root(
    name: &str,
    type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    string_table: &mut StringTable,
) -> ResolvedPublicTypeRoot {
    ResolvedPublicTypeRoot {
        path: InternedPath::from_single_str(name, string_table),
        kind: ResolvedPublicTypeRootKind::Constant { type_id },
    }
}

/// Build one constant export binding for the given public name.
fn constant_binding(name: &str) -> ExportBinding {
    ExportBinding::new(
        module_origin(),
        name.to_owned(),
        OriginDeclarationId::Constant(constant_origin(name)),
    )
}

/// Run the draft builder over constant-only roots and bindings, returning the projected
/// declaration records. This is the test entry point for the folded-value projection: the
/// builder folds each public constant root's value by exact defining path during the
/// per-binding declaration projection.
fn build_constant_records(
    roots: Vec<ResolvedPublicTypeRoot>,
    bindings: Vec<ExportBinding>,
    module_constants: &[Declaration],
    nominal_origins: &FxHashMap<InternedPath, OriginTypeId>,
    env: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    let root_table = ResolvedPublicTypeRootTable {
        roots,
        receiver_methods: vec![],
        trait_source_facts: FxHashMap::default(),
    };
    let export_seed = DirectExportSeed::new(module_origin(), bindings, FxHashMap::default());
    let projection_input = AstPublicInterfaceProjectionInput {
        root_table,
        trait_roots: vec![],
        trait_environment: Some(Rc::new(TraitEnvironment::new())),
        trait_evidence_environment: Some(Rc::new(TraitEvidenceEnvironment::new())),
    };
    let registry = ExternalPackageRegistry::new();
    let const_values = ConstValueStore::from_test_declarations(module_constants.to_vec(), env)?;
    PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: projection_input,
        public_source_nominal_type_origins: nominal_origins,
        public_source_trait_origins: &FxHashMap::default(),
        type_environment: env,
        external_registry: &registry,
        string_table,
        generic_function_templates: &FxHashMap::default(),
        const_values: &const_values,
        module_resources: None,
    })
    .build()
    .map(|result| result.draft.declarations)
}

// ---------------------------------------------------------------------------
//  Scalar folded values
// ---------------------------------------------------------------------------

#[test]
fn constant_record_owns_scalar_int_folded_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    let value_path = InternedPath::from_single_str("value", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned),
    }];

    let root = constant_root("value", int_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("value")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a scalar int constant");

    assert_eq!(records.len(), 1);
    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(semantics.folded_value, PublicFoldedValue::Int(42));
}

#[test]
fn constant_record_owns_scalar_bool_and_char_folded_values() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let bool_id = env.builtins().bool;
    let char_id = env.builtins().char;

    let bool_path = InternedPath::from_single_str("flag", &mut string_table);
    let char_path = InternedPath::from_single_str("letter", &mut string_table);
    let bool_decl = Declaration {
        id: bool_path,
        value: Expression::bool(true, SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let char_decl = Declaration {
        id: char_path,
        value: Expression::char('A', SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let module_constants = vec![bool_decl, char_decl];

    let bool_root = constant_root("flag", bool_id, &mut string_table);
    let char_root = constant_root("letter", char_id, &mut string_table);
    let records = build_constant_records(
        vec![bool_root, char_root],
        vec![constant_binding("flag"), constant_binding("letter")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for bool and char constants");

    assert_eq!(records.len(), 2);
    let PublicDeclarationSemantics::Constant(bool_sem) = &records[0].semantics else {
        panic!("expected constant semantics for bool");
    };
    assert_eq!(bool_sem.folded_value, PublicFoldedValue::Bool(true));

    let PublicDeclarationSemantics::Constant(char_sem) = &records[1].semantics else {
        panic!("expected constant semantics for char");
    };
    assert_eq!(char_sem.folded_value, PublicFoldedValue::Char('A'));
}

#[test]
fn constant_record_owns_scalar_float_folded_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let float_id = env.builtins().float;

    let value_path = InternedPath::from_single_str("pi", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: Expression::float(3.5, SourceLocation::default(), ValueMode::ImmutableOwned),
    }];

    let root = constant_root("pi", float_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("pi")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a scalar float constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(
        semantics.folded_value,
        PublicFoldedValue::Float(FiniteFloat::new(3.5).unwrap())
    );
}

#[test]
fn constant_record_preserves_negative_zero_exact_bits() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let float_id = env.builtins().float;

    let value_path = InternedPath::from_single_str("zero", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: Expression::float(-0.0, SourceLocation::default(), ValueMode::ImmutableOwned),
    }];

    let root = constant_root("zero", float_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("zero")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a negative-zero float constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    // The folded value must retain the exact negative-zero bits. Ordinary f64 equality
    // treats -0.0 == 0.0, so exact-bit identity must distinguish them.
    assert_ne!(
        semantics.folded_value,
        PublicFoldedValue::Float(FiniteFloat::new(0.0).unwrap()),
        "negative zero must remain distinct from positive zero in folded-value identity"
    );
    assert_eq!(
        semantics.folded_value,
        PublicFoldedValue::Float(FiniteFloat::new(-0.0).unwrap()),
        "negative zero must survive folded-value projection with exact bits"
    );
}

#[test]
fn join_rejects_non_finite_float_value_as_internal_invariant() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let float_id = env.builtins().float;

    let value_path = InternedPath::from_single_str("bad", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        // The AST constructor accepts any f64; projection must reject non-finite input.
        value: Expression::float(
            f64::NAN,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        ),
    }];

    let root = constant_root("bad", float_id, &mut string_table);
    let result = build_constant_records(
        vec![root],
        vec![constant_binding("bad")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("non-finite"),
        "expected a non-finite-float diagnostic, got: {message}"
    );
}

#[test]
fn constant_record_owns_folded_template_string_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let string_id = env.builtins().string;

    let folded_text = string_table.intern("Hello, Moth!");
    let value_path = InternedPath::from_single_str("heading", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: Expression::string_slice(
            folded_text,
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        ),
    }];

    let root = constant_root("heading", string_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("heading")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a folded template string constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(
        semantics.folded_value,
        PublicFoldedValue::String(OwnedFoldedString::Text("Hello, Moth!".to_owned()))
    );
}

#[test]
fn constant_record_owns_const_record_with_ordered_field_names_and_values() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let title_path = InternedPath::from_single_str("title", &mut string_table);
    let year_path = InternedPath::from_single_str("year", &mut string_table);
    let struct_path = InternedPath::from_single_str("Defaults", &mut string_table);
    let string_id = env.builtins().string;
    let int_id = env.builtins().int;
    let (_, struct_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Defaults",
        Box::new([
            FieldDefinition {
                name: title_path,
                type_id: string_id,
                location: SourceLocation::default(),
            },
            FieldDefinition {
                name: year_path,
                type_id: int_id,
                location: SourceLocation::default(),
            },
        ]),
        None,
    );

    let title_text = string_table.intern("Moth");
    let fields = vec![
        Declaration {
            id: InternedPath::from_single_str("title", &mut string_table),
            value: Expression::string_slice(
                title_text,
                SourceLocation::default(),
                ValueMode::ImmutableOwned,
            ),
        },
        Declaration {
            id: InternedPath::from_single_str("year", &mut string_table),
            value: Expression::int(2026, SourceLocation::default(), ValueMode::ImmutableOwned),
        },
    ];

    let struct_instance = Expression::struct_instance(
        struct_path,
        fields,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
        true,
        None,
        struct_type_id,
    );

    let value_path = InternedPath::from_single_str("defaults", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: struct_instance,
    }];

    let struct_origin = struct_origin("Defaults");
    let nominal_origins =
        nominal_origins_map(vec![("Defaults", struct_origin.clone())], &mut string_table);

    let root = constant_root("defaults", struct_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("defaults")],
        &module_constants,
        &nominal_origins,
        &env,
        &string_table,
    )
    .expect("join succeeds for a const record");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::Record(fields) = &semantics.folded_value else {
        panic!("expected a folded record, got {:?}", semantics.folded_value);
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "title");
    assert_eq!(
        fields[0].value,
        PublicFoldedValue::String(OwnedFoldedString::Text("Moth".to_owned()))
    );
    assert_eq!(fields[1].name, "year");
    assert_eq!(fields[1].value, PublicFoldedValue::Int(2026));
}

#[test]
fn constant_record_owns_recursive_const_record_fields() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let inner_field_path = InternedPath::from_single_str("inner", &mut string_table);
    let depth_path = InternedPath::from_single_str("depth", &mut string_table);
    let outer_path = InternedPath::from_single_str("Outer", &mut string_table);
    let inner_path = InternedPath::from_single_str("Inner", &mut string_table);
    let none_id = env.builtins().none;
    let int_id = env.builtins().int;
    let (_, outer_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Outer",
        Box::new([FieldDefinition {
            name: inner_field_path,
            type_id: none_id,
            location: SourceLocation::default(),
        }]),
        None,
    );
    let (_, inner_type_id) = register_struct(
        &mut env,
        &mut string_table,
        "Inner",
        Box::new([FieldDefinition {
            name: depth_path,
            type_id: int_id,
            location: SourceLocation::default(),
        }]),
        None,
    );

    let inner_fields = vec![Declaration {
        id: InternedPath::from_single_str("depth", &mut string_table),
        value: Expression::int(7, SourceLocation::default(), ValueMode::ImmutableOwned),
    }];
    let inner_instance = Expression::struct_instance(
        inner_path,
        inner_fields,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
        true,
        None,
        inner_type_id,
    );

    let outer_fields = vec![Declaration {
        id: InternedPath::from_single_str("inner", &mut string_table),
        value: inner_instance,
    }];
    let outer_instance = Expression::struct_instance(
        outer_path,
        outer_fields,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
        true,
        None,
        outer_type_id,
    );

    let value_path = InternedPath::from_single_str("nested", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: outer_instance,
    }];

    let outer_origin = struct_origin("Outer");
    let nominal_origins =
        nominal_origins_map(vec![("Outer", outer_origin.clone())], &mut string_table);

    let root = constant_root("nested", outer_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("nested")],
        &module_constants,
        &nominal_origins,
        &env,
        &string_table,
    )
    .expect("join succeeds for a recursive const record");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::Record(outer_fields) = &semantics.folded_value else {
        panic!("expected a folded record");
    };
    assert_eq!(outer_fields.len(), 1);
    assert_eq!(outer_fields[0].name, "inner");
    let PublicFoldedValue::Record(inner_fields) = &outer_fields[0].value else {
        panic!("expected a nested folded record");
    };
    assert_eq!(inner_fields.len(), 1);
    assert_eq!(inner_fields[0].name, "depth");
    assert_eq!(inner_fields[0].value, PublicFoldedValue::Int(7));
}

#[test]
fn constant_record_owns_choice_with_stable_variant_name() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let choice_path = InternedPath::from_single_str("Status", &mut string_table);
    let variants: Box<[ChoiceVariantDefinition]> = Box::new([
        ChoiceVariantDefinition {
            name: string_table.intern("Active"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: SourceLocation::default(),
        },
        ChoiceVariantDefinition {
            name: string_table.intern("Inactive"),
            tag: 1,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: SourceLocation::default(),
        },
    ]);
    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: choice_path.clone(),
        variants,
        generic_parameters: None,
    });

    let choice_expr = Expression::choice_construct(ChoiceConstructInput {
        nominal_path: choice_path,
        tag: 1,
        fields: vec![],
        diagnostic_type: DataType::Inferred,
        type_id: choice_type_id,
        location: SourceLocation::default(),
        value_mode: ValueMode::ImmutableOwned,
    });

    let value_path = InternedPath::from_single_str("state", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: choice_expr,
    }];

    let choice_origin = choice_origin("Status");
    let nominal_origins =
        nominal_origins_map(vec![("Status", choice_origin.clone())], &mut string_table);

    let root = constant_root("state", choice_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("state")],
        &module_constants,
        &nominal_origins,
        &env,
        &string_table,
    )
    .expect("join succeeds for a choice constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::Choice {
        variant_name,
        fields,
        ..
    } = &semantics.folded_value
    else {
        panic!("expected a folded choice, got {:?}", semantics.folded_value);
    };
    assert_eq!(variant_name, "Inactive");
    assert!(fields.is_empty());
}

#[test]
fn constant_record_owns_collection_of_folded_values() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let items = vec![
        Expression::int(10, SourceLocation::default(), ValueMode::ImmutableOwned),
        Expression::int(20, SourceLocation::default(), ValueMode::ImmutableOwned),
        Expression::int(30, SourceLocation::default(), ValueMode::ImmutableOwned),
    ];
    let collection_type_id = env.intern_collection(env.builtins().int, None);
    let collection_expr = Expression::new(
        ExpressionKind::Collection(items),
        SourceLocation::default(),
        collection_type_id,
        DataType::Inferred,
        ValueMode::ImmutableOwned,
    );

    let value_path = InternedPath::from_single_str("scores", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: collection_expr,
    }];

    let root = constant_root("scores", collection_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("scores")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a collection constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::Collection(values) = &semantics.folded_value else {
        panic!("expected a folded collection");
    };
    assert_eq!(values.len(), 3);
    assert_eq!(values[0], PublicFoldedValue::Int(10));
    assert_eq!(values[1], PublicFoldedValue::Int(20));
    assert_eq!(values[2], PublicFoldedValue::Int(30));
}

// ---------------------------------------------------------------------------
//  Option folded values
// ---------------------------------------------------------------------------

#[test]
fn constant_record_owns_option_some_value() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let int_id = env.builtins().int;
    let option_type_id = env.intern_option(int_id);

    let inner = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned);
    let coerced = Expression::coerced(inner, option_type_id);

    let value_path = InternedPath::from_single_str("maybe_value", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: coerced,
    }];

    let root = constant_root("maybe_value", option_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("maybe_value")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for an option-present constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::OptionSome(inner) = &semantics.folded_value else {
        panic!(
            "expected a folded OptionSome value, got {:?}",
            semantics.folded_value
        );
    };
    assert_eq!(inner.as_ref(), &PublicFoldedValue::Int(42));
}

#[test]
fn constant_record_owns_nested_option_some_value() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let int_id = env.builtins().int;
    let inner_option_id = env.intern_option(int_id);
    let outer_option_id = env.intern_option(inner_option_id);

    let inner = Expression::int(7, SourceLocation::default(), ValueMode::ImmutableOwned);
    let inner_option = Expression::coerced(inner, inner_option_id);
    let outer_option = Expression::coerced(inner_option, outer_option_id);

    let value_path = InternedPath::from_single_str("doubly_maybe", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: outer_option,
    }];

    let root = constant_root("doubly_maybe", outer_option_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("doubly_maybe")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for a nested option-present constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    let PublicFoldedValue::OptionSome(outer) = &semantics.folded_value else {
        panic!(
            "expected a folded outer OptionSome, got {:?}",
            semantics.folded_value
        );
    };
    let PublicFoldedValue::OptionSome(inner) = outer.as_ref() else {
        panic!("expected a folded inner OptionSome, got {:?}", outer);
    };
    assert_eq!(inner.as_ref(), &PublicFoldedValue::Int(7));
}

#[test]
fn constant_record_projects_option_none_value() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let int_id = env.builtins().int;
    let option_type_id = env.intern_option(int_id);

    // `none` for `Int?`. The const classifier currently rejects a standalone `none` as a
    // module constant initializer, so this exercises the projection arm directly through the
    // builder boundary rather than the full parser path.
    let none_expr = Expression::option_none_with_type_id(
        int_id,
        DataType::Int,
        &mut env,
        SourceLocation::default(),
    );

    let value_path = InternedPath::from_single_str("absent", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: none_expr,
    }];

    let root = constant_root("absent", option_type_id, &mut string_table);
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("absent")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds for an option-absent constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(semantics.folded_value, PublicFoldedValue::OptionNone);
}

// ---------------------------------------------------------------------------
//  Exact defining-path join regressions
// ---------------------------------------------------------------------------

#[test]
fn join_allows_two_module_constants_sharing_a_leaf_name_with_distinct_paths() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // Two module constants whose last path component is the same leaf name "value" but
    // whose exact defining paths differ in the parent component. The public constant root
    // selects the exact public path, so the private same-leaf path is an unmatched extra
    // ignored by the fold.
    let mut public_path = InternedPath::from_single_str("scope", &mut string_table);
    public_path.push_str("value", &mut string_table);
    let mut private_path = InternedPath::from_single_str("other", &mut string_table);
    private_path.push_str("value", &mut string_table);

    let module_constants = vec![
        Declaration {
            id: public_path.clone(),
            value: Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned),
        },
        Declaration {
            id: private_path,
            value: Expression::int(2, SourceLocation::default(), ValueMode::ImmutableOwned),
        },
    ];

    let root = ResolvedPublicTypeRoot {
        path: public_path,
        kind: ResolvedPublicTypeRootKind::Constant { type_id: int_id },
    };
    let records = build_constant_records(
        vec![root],
        vec![constant_binding("value")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("join succeeds when two constants share a leaf name but differ in exact path");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    // The root selects the exact public path, so the value is 1, not 2.
    assert_eq!(semantics.folded_value, PublicFoldedValue::Int(1));
}

// ---------------------------------------------------------------------------
//  Folded-value totality failures
// ---------------------------------------------------------------------------

#[test]
fn join_rejects_constant_binding_without_matching_module_constant() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // A constant root whose defining path has no matching finalized module constant
    // declaration: the folded value cannot be projected.
    let root = constant_root("missing", int_id, &mut string_table);
    let result = build_constant_records(
        vec![root],
        vec![constant_binding("missing")],
        &[],
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("no matching finalized module constant declaration"),
        "expected a missing-module-constant diagnostic, got: {message}"
    );
}

#[test]
fn join_rejects_duplicate_module_constant_defining_paths() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // Two module constant declarations with the exact same defining path: a silent overwrite
    // must not happen.
    let dup_path = InternedPath::from_single_str("dup", &mut string_table);
    let decl = Declaration {
        id: dup_path,
        value: Expression::int(1, SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let duplicate_path = InternedPath::from_single_str("dup", &mut string_table);
    let duplicate = Declaration {
        id: duplicate_path,
        value: Expression::int(2, SourceLocation::default(), ValueMode::ImmutableOwned),
    };
    let module_constants = vec![decl, duplicate];

    let root = constant_root("dup", int_id, &mut string_table);
    let result = build_constant_records(
        vec![root],
        vec![constant_binding("dup")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("two finalized module constants share the defining path"),
        "expected a duplicate-path diagnostic, got: {message}"
    );
}

#[test]
fn join_rejects_extra_constant_root_without_export_binding() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // A public constant root whose name has no export binding: an unconsumed public fact must
    // fail deterministically rather than leak silently.
    let orphan_root = constant_root("orphan", int_id, &mut string_table);
    let result = build_constant_records(
        vec![orphan_root],
        vec![],
        &[],
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("no matching export binding"),
        "expected an unconsumed-root diagnostic, got: {message}"
    );
}

#[test]
fn join_rejects_unsupported_expression_shape_in_folded_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    let reference_expr = Expression::new(
        ExpressionKind::Reference(InternedPath::from_single_str(
            "other_constant",
            &mut string_table,
        )),
        SourceLocation::default(),
        int_id,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );

    let value_path = InternedPath::from_single_str("bad", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path,
        value: reference_expr,
    }];

    let root = constant_root("bad", int_id, &mut string_table);
    let result = build_constant_records(
        vec![root],
        vec![constant_binding("bad")],
        &module_constants,
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("reached ConstValueStore without a folded value"),
        "expected an unsupported-shape diagnostic, got: {message}"
    );
}

#[test]
fn public_structural_string_preserves_resource_identity_and_piece_order() {
    let mut string_table = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let origin = StableResourceOriginId::module_owned(
        module_origin(),
        PortableResourcePath::from_relative_logical_path(std::path::Path::new("assets/logo.svg"))
            .expect("relative resource path should be portable"),
    );
    let resource = resources.intern_origin(origin.clone(), SourceLocation::default());
    let prefix = string_table.intern("assets/");
    let folded = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(prefix),
        ConstStringPiece::Resource(resource),
        ConstStringPiece::SiteRoot,
    ]);

    let projected = owned_folded_string_from_const_string(&folded, &resources, &string_table)
        .expect("structural string should project");
    assert_eq!(
        projected,
        OwnedFoldedString::Pieces(vec![
            crate::compiler_frontend::folded_value::OwnedFoldedStringPiece::Text(
                "assets/".to_owned(),
            ),
            crate::compiler_frontend::folded_value::OwnedFoldedStringPiece::Resource(origin),
            crate::compiler_frontend::folded_value::OwnedFoldedStringPiece::SiteRoot,
        ])
    );
    assert_eq!(projected.into_text(), None);
    assert_eq!(
        OwnedFoldedString::Text("plain".to_owned()).into_text(),
        Some("plain".to_owned())
    );
}

#[test]
fn text_is_available_for_a_piece_list_that_carries_only_text() {
    let mut string_table = StringTable::new();
    let resources = ModuleResourceTable::new();
    let head = string_table.intern("docs/");
    let tail = string_table.intern("intro.html");
    let folded = ConstStringValue::Pieces(vec![
        ConstStringPiece::Text(head),
        ConstStringPiece::Text(tail),
    ]);

    let projected = owned_folded_string_from_const_string(&folded, &resources, &string_table)
        .expect("an all-text piece list needs no resource table entry");

    // Availability must match `require_concrete_text`: only a Resource or SiteRoot piece withholds
    // text, so a piece list carrying only text concatenates in authored order.
    assert_eq!(projected.into_text(), Some("docs/intro.html".to_owned()));
}
