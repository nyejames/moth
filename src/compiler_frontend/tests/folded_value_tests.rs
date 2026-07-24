//! Focused unit tests for folded-value projection of directly exported constants.
//!
//! WHAT: exercises the constant folded-value join and conversion owned by
//! `folded_value`: exact defining-path join, owned backend-neutral value vocabulary,
//! option-present/absent projection, finite-float semantics, and totality failures for
//! missing, duplicate, extra and unsupported folded facts. These are projection/join
//! invariants integration output cannot inspect, so they own a focused test beside the
//! projection owner.
//!
//! This module is a child of `public_interface_draft_tests` and reuses its shared fixtures.

use super::super::{
    DefinedPublicTraitSurface, FoldedValueJoinContext, PublicDeclarationRecord,
    PublicDeclarationSemantics, join_declaration_records,
};
use super::{
    choice_origin, constant_origin, immutable, module_origin, nominal_origins_map, register_struct,
    struct_origin, type_surface,
};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, Expression, ExpressionKind,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity, CanonicalTypeProjectionContext,
    CollectionTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::NominalTypeId;
use crate::compiler_frontend::defined_public_type_surface::{
    DefinedPublicConstantTypeSurface, DefinedPublicTypeSurface, TransientNominalOriginResolver,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FiniteFloat, FoldedValueGenericParameterResolver, PublicFoldedValue,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;

// ---------------------------------------------------------------------------
//  Shared helpers
// ---------------------------------------------------------------------------

/// Wraps `join_declaration_records` with a folded-value context that carries module
/// constants and the shared type environment for tests that exercise constant folded-value
/// projection.
fn join_with_constants(
    export_bindings: &[ExportBinding],
    type_surface: DefinedPublicTypeSurface,
    trait_surfaces: Vec<DefinedPublicTraitSurface>,
    env: &TypeEnvironment,
    string_table: &StringTable,
    module_constants: &[Declaration],
    nominal_origins: &FxHashMap<InternedPath, OriginTypeId>,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    let nominal_resolver = TransientNominalOriginResolver::new(env, nominal_origins);
    let generic_resolver = FoldedValueGenericParameterResolver;
    let registry = ExternalPackageRegistry::new();
    let projection_context =
        CanonicalTypeProjectionContext::new(&nominal_resolver, &generic_resolver, &registry);
    let folded_value_context = FoldedValueJoinContext {
        module_constants,
        type_environment: env,
        string_table,
        projection_context: &projection_context,
    };
    join_declaration_records(
        export_bindings,
        type_surface,
        trait_surfaces,
        &folded_value_context,
    )
}

/// Builds one constant export binding and matching type-surface entry for the given public
/// name, canonical type identity and exact defining path. The defining path is the exact
/// `InternedPath` used to join the surface to a finalized module constant declaration.
fn constant_binding_and_surface(
    name: &str,
    type_identity: CanonicalTypeIdentity,
    defining_path: InternedPath,
) -> (ExportBinding, DefinedPublicConstantTypeSurface) {
    let origin = constant_origin(name);
    let binding = ExportBinding::new(
        module_origin(),
        name.to_owned(),
        OriginDeclarationId::Constant(origin.clone()),
    );
    let surface = DefinedPublicConstantTypeSurface {
        origin,
        type_identity,
        defining_path,
    };
    (binding, surface)
}

fn default_location() -> SourceLocation {
    SourceLocation::default()
}

// ---------------------------------------------------------------------------
//  Scalar folded values
// ---------------------------------------------------------------------------

#[test]
fn constant_record_owns_scalar_int_folded_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let value_path = InternedPath::from_single_str("value", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: Expression::int(42, default_location(), immutable()),
    }];

    let (binding, surface) = constant_binding_and_surface(
        "value",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    let bool_path = InternedPath::from_single_str("flag", &mut string_table);
    let char_path = InternedPath::from_single_str("letter", &mut string_table);
    let bool_decl = Declaration {
        id: bool_path.clone(),
        value: Expression::bool(true, default_location(), immutable()),
    };
    let char_decl = Declaration {
        id: char_path.clone(),
        value: Expression::char('A', default_location(), immutable()),
    };
    let module_constants = vec![bool_decl, char_decl];

    let (bool_binding, bool_surface) = constant_binding_and_surface(
        "flag",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Bool),
        bool_path,
    );
    let (char_binding, char_surface) = constant_binding_and_surface(
        "letter",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Char),
        char_path,
    );
    let type_surface = type_surface(
        vec![],
        vec![],
        vec![],
        vec![bool_surface, char_surface],
        vec![],
    );

    let records = join_with_constants(
        &[bool_binding, char_binding],
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    let value_path = InternedPath::from_single_str("pi", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: Expression::float(3.5, default_location(), immutable()),
    }];

    let (binding, surface) = constant_binding_and_surface(
        "pi",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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
fn constant_record_normalizes_negative_zero_float_to_positive_zero() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let value_path = InternedPath::from_single_str("zero", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: Expression::float(-0.0, default_location(), immutable()),
    }];

    let (binding, surface) = constant_binding_and_surface(
        "zero",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    )
    .expect("join succeeds for a negative-zero float constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    // Negative zero normalizes to positive zero: both produce the same canonical FiniteFloat.
    assert_eq!(
        semantics.folded_value,
        PublicFoldedValue::Float(FiniteFloat::new(0.0).unwrap())
    );
}

#[test]
fn join_rejects_non_finite_float_value_as_internal_invariant() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let value_path = InternedPath::from_single_str("bad", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        // The AST constructor accepts any f64; projection must reject non-finite input.
        value: Expression::float(f64::NAN, default_location(), immutable()),
    }];

    let (binding, surface) = constant_binding_and_surface(
        "bad",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Float),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let result = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("non-finite Float value"),
        "expected a non-finite rejection diagnostic, got: {message}"
    );
}

// ---------------------------------------------------------------------------
//  String, record, choice and collection folded values
// ---------------------------------------------------------------------------

#[test]
fn constant_record_owns_folded_template_string_value() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let folded_text = string_table.intern("Hello, Moth!");
    let value_path = InternedPath::from_single_str("heading", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: Expression::string_slice(folded_text, default_location(), immutable()),
    }];

    let (binding, surface) = constant_binding_and_surface(
        "heading",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::String),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    )
    .expect("join succeeds for a folded template string constant");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(
        semantics.folded_value,
        PublicFoldedValue::String("Hello, Moth!".to_owned())
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
                location: default_location(),
            },
            FieldDefinition {
                name: year_path,
                type_id: int_id,
                location: default_location(),
            },
        ]),
        None,
    );

    let title_text = string_table.intern("Moth");
    let fields = vec![
        Declaration {
            id: InternedPath::from_single_str("title", &mut string_table),
            value: Expression::string_slice(title_text, default_location(), immutable()),
        },
        Declaration {
            id: InternedPath::from_single_str("year", &mut string_table),
            value: Expression::int(2026, default_location(), immutable()),
        },
    ];

    let struct_instance = Expression::struct_instance(
        struct_path,
        fields,
        default_location(),
        immutable(),
        true,
        None,
        struct_type_id,
    );

    let value_path = InternedPath::from_single_str("defaults", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: struct_instance,
    }];

    let struct_origin = struct_origin("Defaults");
    let nominal_origins =
        nominal_origins_map(vec![("Defaults", struct_origin.clone())], &mut string_table);

    let (binding, surface) = constant_binding_and_surface(
        "defaults",
        CanonicalTypeIdentity::SourceNominal(struct_origin),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &nominal_origins,
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
        PublicFoldedValue::String("Moth".to_owned())
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
            location: default_location(),
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
            location: default_location(),
        }]),
        None,
    );

    let inner_fields = vec![Declaration {
        id: InternedPath::from_single_str("depth", &mut string_table),
        value: Expression::int(7, default_location(), immutable()),
    }];
    let inner_instance = Expression::struct_instance(
        inner_path,
        inner_fields,
        default_location(),
        immutable(),
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
        default_location(),
        immutable(),
        true,
        None,
        outer_type_id,
    );

    let value_path = InternedPath::from_single_str("nested", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: outer_instance,
    }];

    let outer_origin = struct_origin("Outer");
    let nominal_origins =
        nominal_origins_map(vec![("Outer", outer_origin.clone())], &mut string_table);

    let (binding, surface) = constant_binding_and_surface(
        "nested",
        CanonicalTypeIdentity::SourceNominal(outer_origin),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &nominal_origins,
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
            location: default_location(),
        },
        ChoiceVariantDefinition {
            name: string_table.intern("Inactive"),
            tag: 1,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: default_location(),
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
        location: default_location(),
        value_mode: immutable(),
    });

    let value_path = InternedPath::from_single_str("state", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: choice_expr,
    }];

    let choice_origin = choice_origin("Status");
    let nominal_origins =
        nominal_origins_map(vec![("Status", choice_origin.clone())], &mut string_table);

    let (binding, surface) = constant_binding_and_surface(
        "state",
        CanonicalTypeIdentity::SourceNominal(choice_origin),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &nominal_origins,
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
        Expression::int(10, default_location(), immutable()),
        Expression::int(20, default_location(), immutable()),
        Expression::int(30, default_location(), immutable()),
    ];
    let collection_type_id = env.intern_collection(env.builtins().int, None);
    let collection_expr = Expression::new(
        ExpressionKind::Collection(items),
        default_location(),
        collection_type_id,
        DataType::Inferred,
        immutable(),
    );

    let value_path = InternedPath::from_single_str("scores", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: collection_expr,
    }];

    let (binding, surface) = constant_binding_and_surface(
        "scores",
        CanonicalTypeIdentity::Collection(CollectionTypeIdentity::new(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
            None,
        )),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    let inner = Expression::int(42, default_location(), immutable());
    let coerced = Expression::coerced(inner, option_type_id);

    let value_path = InternedPath::from_single_str("maybe_value", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: coerced,
    }];

    let (binding, surface) = constant_binding_and_surface(
        "maybe_value",
        CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Int,
        ))),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    // Inner: Int -> Int?
    let inner = Expression::int(7, default_location(), immutable());
    let inner_option = Expression::coerced(inner, inner_option_id);
    // Outer: Int? -> Int??
    let outer_option = Expression::coerced(inner_option, outer_option_id);

    let value_path = InternedPath::from_single_str("doubly_maybe", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: outer_option,
    }];

    let (binding, surface) = constant_binding_and_surface(
        "doubly_maybe",
        CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::Option(Box::new(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        )))),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    // `none` for `Int?`. The const classifier currently rejects a standalone `none` as a
    // module constant initializer, so this exercises the projection arm directly through the
    // join boundary rather than the full parser path.
    let none_expr =
        Expression::option_none_with_type_id(int_id, DataType::Int, &mut env, default_location());

    let value_path = InternedPath::from_single_str("absent", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: none_expr,
    }];

    let (binding, surface) = constant_binding_and_surface(
        "absent",
        CanonicalTypeIdentity::Option(Box::new(CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Int,
        ))),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
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

    // Two module constants whose last path component is the same leaf name "value" but
    // whose exact defining paths differ in the parent component. Building each path with
    // `from_single_str` + `push_str` produces true two-component `InternedPath` values so
    // a last-component/name-based implementation would collide; only the exact public path
    // is selected and the private same-leaf path is an expected extra that is ignored.
    let mut public_path = InternedPath::from_single_str("scope", &mut string_table);
    public_path.push_str("value", &mut string_table);
    let mut private_path = InternedPath::from_single_str("other", &mut string_table);
    private_path.push_str("value", &mut string_table);

    let module_constants = vec![
        Declaration {
            id: public_path.clone(),
            value: Expression::int(1, default_location(), immutable()),
        },
        Declaration {
            id: private_path.clone(),
            value: Expression::int(2, default_location(), immutable()),
        },
    ];

    let (binding, surface) = constant_binding_and_surface(
        "value",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        public_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    )
    .expect("join succeeds when two constants share a leaf name but differ in exact path");

    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    // The public surface selects the exact public path, so the value is 1, not 2.
    assert_eq!(semantics.folded_value, PublicFoldedValue::Int(1));
}

#[test]
fn join_consumes_one_value_when_alias_binding_shares_constant_origin() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let value_path = InternedPath::from_single_str("canonical", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: Expression::int(9, default_location(), immutable()),
    }];

    let origin = constant_origin("canonical");
    // Two export bindings for the same stable constant origin with different public alias
    // spellings. Both are retained in the export-bindings list, but only one declaration
    // record is produced and the module constant value is consumed once.
    let primary_binding = ExportBinding::new(
        module_origin(),
        "canonical".to_owned(),
        OriginDeclarationId::Constant(origin.clone()),
    );
    let alias_binding = ExportBinding::new(
        module_origin(),
        "alias_name".to_owned(),
        OriginDeclarationId::Constant(origin.clone()),
    );
    let surface = DefinedPublicConstantTypeSurface {
        origin,
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        defining_path: value_path,
    };
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let records = join_with_constants(
        &[primary_binding, alias_binding],
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    )
    .expect("join succeeds for an aliased constant origin");

    // Exactly one declaration record for the shared origin.
    assert_eq!(records.len(), 1);
    let PublicDeclarationSemantics::Constant(semantics) = &records[0].semantics else {
        panic!("expected constant semantics");
    };
    assert_eq!(semantics.folded_value, PublicFoldedValue::Int(9));
}

// ---------------------------------------------------------------------------
//  Folded-value totality failures
// ---------------------------------------------------------------------------

#[test]
fn join_rejects_constant_binding_without_matching_module_constant() {
    let string_table = StringTable::new();
    let env = TypeEnvironment::new();

    // A public constant surface with a defining path that has no matching finalized module
    // constant declaration: the folded value cannot be projected.
    let defining_path = InternedPath::from_single_str("missing", &mut StringTable::new());
    let (binding, surface) = constant_binding_and_surface(
        "missing",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        defining_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let result = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &[],
        &FxHashMap::default(),
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

    // Two module constant declarations with the exact same defining path: a silent overwrite
    // must not happen.
    let dup_path = InternedPath::from_single_str("dup", &mut string_table);
    let decl = Declaration {
        id: dup_path,
        value: Expression::int(1, default_location(), immutable()),
    };
    // Rebuild the same path so the second declaration shares the exact interned identity.
    let duplicate_path = InternedPath::from_single_str("dup", &mut string_table);
    let duplicate = Declaration {
        id: duplicate_path,
        value: Expression::int(2, default_location(), immutable()),
    };
    let module_constants = vec![decl, duplicate];

    let binding_path = InternedPath::from_single_str("dup", &mut string_table);
    let (binding, surface) = constant_binding_and_surface(
        "dup",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        binding_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let result = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("two module constant declarations share the exact defining path"),
        "expected a duplicate-path diagnostic, got: {message}"
    );
}

#[test]
fn join_rejects_extra_constant_surface_without_export_binding() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    // A public constant surface whose origin has no export binding: an unconsumed public
    // folded fact must fail deterministically rather than leak silently.
    let orphan_path = InternedPath::from_single_str("orphan", &mut string_table);
    let surface = DefinedPublicConstantTypeSurface {
        origin: constant_origin("orphan"),
        type_identity: CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        defining_path: orphan_path,
    };
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let result = join_with_constants(
        &[],
        type_surface,
        vec![],
        &env,
        &string_table,
        &[],
        &FxHashMap::default(),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("constant type-surface entries have no matching export binding"),
        "expected an unconsumed-public-fact diagnostic, got: {message}"
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
        default_location(),
        int_id,
        DataType::Int,
        immutable(),
    );

    let value_path = InternedPath::from_single_str("bad", &mut string_table);
    let module_constants = vec![Declaration {
        id: value_path.clone(),
        value: reference_expr,
    }];

    let (binding, surface) = constant_binding_and_surface(
        "bad",
        CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int),
        value_path,
    );
    let type_surface = type_surface(vec![], vec![], vec![], vec![surface], vec![]);

    let result = join_with_constants(
        std::slice::from_ref(&binding),
        type_surface,
        vec![],
        &env,
        &string_table,
        &module_constants,
        &FxHashMap::default(),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("Reference expression reached conversion"),
        "expected an unsupported-shape diagnostic, got: {message}"
    );
}
