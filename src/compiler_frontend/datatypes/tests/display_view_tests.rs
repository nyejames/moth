//! Unit tests for the `TypeDisplayView` read surface of the live `TypeEnvironment`.
//!
//! WHAT: proves that each definition kind reports its spelling class, that choice variants and
//! function parameters keep declaration order, that a function error return stays out of its
//! success returns and that under-applied constructors and empty path tables use fallback names.
//! WHY: the live formatter and the frozen type-display snapshot both spell types from this view, so
//! every classification and fallback here is a spelling fact that diagnostics depend on.
//! OWNERSHIP: tests only. Each test builds its own environment, string table and path table.

use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::display::{
    ChoiceVariantShape, TypeDisplayShape, TypeDisplayView, display_type,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericParameterId, NominalTypeId, TypeConstructor,
    TypeId, builtin_type_ids,
};
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Interns one portable path spelling and returns its id.
fn test_path(paths: &mut PathInternerBuilder, strings: &mut StringTable, spelling: &str) -> PathId {
    paths
        .try_intern_portable_path(spelling, strings)
        .expect("test path fits")
}

/// Builds a field-less struct definition. `register_nominal_struct` assigns the real nominal id.
fn struct_definition(path: PathId, const_record: bool) -> StructTypeDefinition {
    StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record,
    }
}

/// Builds a choice definition with the given variants. `register_nominal_choice` assigns the id.
fn choice_definition(
    path: PathId,
    variants: Box<[ChoiceVariantDefinition]>,
) -> ChoiceTypeDefinition {
    ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants,
        generic_parameters: None,
    }
}

#[test]
fn view_classifies_each_definition_kind() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let settings = test_path(&mut paths, &mut strings, "Settings");
    let status = test_path(&mut paths, &mut strings, "Status");
    let boxed = test_path(&mut paths, &mut strings, "Box");
    let ready = strings.intern("Ready");
    let t_name = strings.intern("T");

    let mut environment = TypeEnvironment::new();
    let bool_type = environment.builtins().bool;
    let string_type = environment.builtins().string;
    let char_type = environment.builtins().char;
    let range_type = environment.builtins().range;
    let none_type = environment.builtins().none;
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let (_, config_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (_, settings_type) = environment.register_nominal_struct(struct_definition(settings, true));
    let (_, status_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([ChoiceVariantDefinition {
            name: ready,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            span: None,
        }]),
    ));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let box_of_i32 = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let number_type = environment.intern_number(NumberScale::new(2).expect("scale 2 fits"));
    let external_type = environment.intern_external(ExternalTypeId(7));
    let collection_type = environment.intern_collection(i32_type, Some(3));
    let map_type = environment.intern_map(string_type, i32_type);
    let option_type = environment.intern_option(i32_type);
    let function_type = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([bool_type]),
        error_return: None,
    });
    let generic_type = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let anonymous_type = environment.anonymous_const_record_type();
    let unknown_type = TypeId(9_999);

    let cases: Vec<(TypeId, TypeDisplayShape<TypeId>)> = vec![
        (bool_type, TypeDisplayShape::Bool),
        (string_type, TypeDisplayShape::String),
        (char_type, TypeDisplayShape::Char),
        (range_type, TypeDisplayShape::Range),
        (none_type, TypeDisplayShape::NoneType),
        (i32_type, TypeDisplayShape::FixedScalar(FixedScalar::I32)),
        (
            number_type,
            TypeDisplayShape::Number(NumberScale::new(2).expect("scale 2 fits")),
        ),
        (
            config_type,
            TypeDisplayShape::Struct {
                path: config,
                const_record: false,
            },
        ),
        (
            settings_type,
            TypeDisplayShape::Struct {
                path: settings,
                const_record: true,
            },
        ),
        (status_type, TypeDisplayShape::Choice { path: status }),
        (
            box_of_i32,
            TypeDisplayShape::GenericInstance {
                base_path: Some(boxed),
            },
        ),
        (
            collection_type,
            TypeDisplayShape::Collection {
                fixed_capacity: Some(3),
            },
        ),
        (map_type, TypeDisplayShape::OrderedMap),
        (option_type, TypeDisplayShape::OptionType),
        (
            function_type,
            TypeDisplayShape::Function { error_return: None },
        ),
        (
            generic_type,
            TypeDisplayShape::GenericParameter { name: t_name },
        ),
        (external_type, TypeDisplayShape::External(7)),
        (anonymous_type, TypeDisplayShape::AnonymousConstRecord),
        (unknown_type, TypeDisplayShape::Unknown),
    ];
    for (type_id, expected) in cases {
        assert_eq!(environment.shape(type_id), expected, "shape of {type_id:?}");
    }
}

#[test]
fn view_marks_only_generic_parameters_as_generic() {
    let mut strings = StringTable::new();
    let t_name = strings.intern("T");
    let mut environment = TypeEnvironment::new();
    let generic_type = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let cases = [
        (generic_type, true),
        (i32_type, false),
        (TypeId(9_999), false),
    ];
    for (type_id, expected) in cases {
        assert_eq!(
            environment.is_generic_parameter(type_id),
            expected,
            "generic flag of {type_id:?}"
        );
    }
}

#[test]
fn view_lists_choice_variants_in_declaration_order() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let status = test_path(&mut paths, &mut strings, "Status");
    let ready = strings.intern("Ready");
    let failed = strings.intern("Failed");
    let idle = strings.intern("Idle");
    let mut environment = TypeEnvironment::new();
    let (_, status_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([
            ChoiceVariantDefinition {
                name: ready,
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                span: None,
            },
            ChoiceVariantDefinition {
                name: failed,
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: Box::new([]),
                },
                span: None,
            },
            ChoiceVariantDefinition {
                name: idle,
                tag: 2,
                payload: ChoiceVariantPayloadDefinition::Unit,
                span: None,
            },
        ]),
    ));
    let variants: Vec<ChoiceVariantShape> = environment.choice_variants(status_type).collect();
    assert_eq!(
        variants,
        vec![
            ChoiceVariantShape {
                name: ready,
                has_record_payload: false,
            },
            ChoiceVariantShape {
                name: failed,
                has_record_payload: true,
            },
            ChoiceVariantShape {
                name: idle,
                has_record_payload: false,
            },
        ]
    );
}

#[test]
fn view_lists_function_parameters_in_order() {
    let mut environment = TypeEnvironment::new();
    let string_type = environment.builtins().string;
    let bool_type = environment.builtins().bool;
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let function_type = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([string_type, i32_type]),
        returns: Box::new([bool_type]),
        error_return: None,
    });
    let parameters: Vec<TypeId> = environment.function_parameter_ids(function_type).collect();
    assert_eq!(parameters, vec![string_type, i32_type]);
}

#[test]
fn view_keeps_error_return_out_of_success_returns() {
    let mut environment = TypeEnvironment::new();
    let string_type = environment.builtins().string;
    let bool_type = environment.builtins().bool;
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let function_type = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([bool_type]),
        error_return: Some(string_type),
    });
    let returns: Vec<TypeId> = environment.function_return_ids(function_type).collect();
    assert_eq!(returns, vec![bool_type]);
    assert_eq!(
        environment.shape(function_type),
        TypeDisplayShape::Function {
            error_return: Some(string_type),
        }
    );
}

#[test]
fn display_uses_arity_fallbacks_for_under_applied_constructors() {
    let strings = StringTable::new();
    let paths = PathInternerBuilder::new().freeze();
    let mut environment = TypeEnvironment::new();
    let collection = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([]),
    );
    let map = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([]),
    );
    let option = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([]),
    );
    assert_eq!(
        display_type(collection, &environment, &strings, &paths),
        "Collection"
    );
    assert_eq!(display_type(map, &environment, &strings, &paths), "Map");
    assert_eq!(
        display_type(option, &environment, &strings, &paths),
        "Option"
    );
}

#[test]
fn display_uses_fallback_names_when_path_table_is_empty() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let settings = test_path(&mut paths, &mut strings, "Settings");
    let status = test_path(&mut paths, &mut strings, "Status");
    let boxed = test_path(&mut paths, &mut strings, "Box");
    let ready = strings.intern("Ready");
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let mut environment = TypeEnvironment::new();
    let (_, struct_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (_, const_type) = environment.register_nominal_struct(struct_definition(settings, true));
    let (_, choice_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([ChoiceVariantDefinition {
            name: ready,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            span: None,
        }]),
    ));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let instance = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let empty_paths = PathInternerBuilder::new().freeze();
    assert_eq!(
        display_type(struct_type, &environment, &strings, &empty_paths),
        "<anonymous struct>"
    );
    assert_eq!(
        display_type(const_type, &environment, &strings, &empty_paths),
        "const record <anonymous struct>"
    );
    assert_eq!(
        display_type(choice_type, &environment, &strings, &empty_paths),
        "<choice>::{Ready}"
    );
    assert_eq!(
        display_type(instance, &environment, &strings, &empty_paths),
        "<generic> of I32"
    );
}
