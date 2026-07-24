//! String-ID remapping tests for parsed type references and generic parameters.
//!
//! WHAT: verifies that `ParsedTypeRef`, `GenericParameter`, and `GenericParameterList`
//!      can be remapped from local string tables into a merged global table.
//! WHY: per-file frontend preparation produces parsed type syntax using local string
//!      tables; remapping must preserve all nested names and source locations.

use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn make_location(string_table: &mut StringTable) -> SourceLocation {
    let path = InternedPath::from_single_str("test.moth", string_table);
    SourceLocation::new(path, CharPosition::default(), CharPosition::default())
}

fn assert_test_location(location: &SourceLocation, string_table: &StringTable) {
    let scope_components = location
        .scope
        .as_components()
        .iter()
        .map(|id| string_table.resolve(*id))
        .collect::<Vec<_>>();

    assert_eq!(scope_components, vec!["test.moth"]);
}

#[test]
fn parsed_type_ref_named_remaps_name_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let name_local = local.intern("MyType");
    let location = make_location(&mut local);

    let mut parsed = ParsedTypeRef::Named {
        name: name_local,
        location,
    };

    global.intern("preexisting");
    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Named { name, location } => {
            assert_eq!(global.resolve(name), "MyType");
            assert_test_location(&location, &global);
        }
        _ => panic!("expected Named type"),
    }
}

#[test]
fn parsed_type_ref_applied_remaps_recursively() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let box_name = local.intern("Box");
    let string_name = local.intern("String");

    let mut parsed = ParsedTypeRef::Applied {
        base: Box::new(ParsedTypeRef::Named {
            name: box_name,
            location: make_location(&mut local),
        }),
        arguments: vec![ParsedTypeRef::Named {
            name: string_name,
            location: make_location(&mut local),
        }],
        location: make_location(&mut local),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            match *base {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(name), "Box");
                }
                _ => panic!("expected Named base"),
            }
            assert_eq!(arguments.len(), 1);
            match &arguments[0] {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(*name), "String");
                }
                _ => panic!("expected Named argument"),
            }
        }
        _ => panic!("expected Applied"),
    }
}

#[test]
fn parsed_type_ref_collection_remaps_element_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let int_name = local.intern("Int");

    let mut parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::Named {
            name: int_name,
            location: make_location(&mut local),
        }),
        location: make_location(&mut local),
        fixed_capacity: None,
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Collection { element, .. } => match *element {
            ParsedTypeRef::Named { name, .. } => {
                assert_eq!(global.resolve(name), "Int");
            }
            _ => panic!("expected Named element"),
        },
        _ => panic!("expected Collection"),
    }
}

#[test]
fn parsed_type_ref_optional_remaps_inner_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let bool_name = local.intern("Bool");

    let mut parsed = ParsedTypeRef::Optional {
        inner: Box::new(ParsedTypeRef::Named {
            name: bool_name,
            location: make_location(&mut local),
        }),
        location: make_location(&mut local),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Optional { inner, .. } => match *inner {
            ParsedTypeRef::Named { name, .. } => {
                assert_eq!(global.resolve(name), "Bool");
            }
            _ => panic!("expected Named inner"),
        },
        _ => panic!("expected Optional"),
    }
}

#[test]
fn parsed_type_ref_result_remaps_ok_err_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let ok_name = local.intern("OkType");
    let err_name = local.intern("ErrType");

    let mut parsed = ParsedTypeRef::Result {
        ok: Box::new(ParsedTypeRef::Named {
            name: ok_name,
            location: make_location(&mut local),
        }),
        err: Box::new(ParsedTypeRef::Named {
            name: err_name,
            location: make_location(&mut local),
        }),
        location: make_location(&mut local),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Result { ok, err, .. } => {
            match *ok {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(name), "OkType");
                }
                _ => panic!("expected Named ok"),
            }
            match *err {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(name), "ErrType");
                }
                _ => panic!("expected Named err"),
            }
        }
        _ => panic!("expected Result"),
    }
}

#[test]
fn parsed_type_ref_builtin_remaps_location_only() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let location = make_location(&mut local);
    let mut parsed = ParsedTypeRef::BuiltinInt { location };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::BuiltinInt { location } => {
            assert_test_location(&location, &global);
        }
        _ => panic!("expected BuiltinInt"),
    }
}

#[test]
fn parsed_type_ref_inferred_is_unchanged() {
    let local = StringTable::new();
    let mut global = StringTable::new();

    let mut parsed = ParsedTypeRef::Inferred;

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    assert!(matches!(parsed, ParsedTypeRef::Inferred));
}

#[test]
fn generic_parameter_remaps_name_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let t_name = local.intern("T");
    let location = make_location(&mut local);

    let mut param = GenericParameter {
        id: TypeParameterId(0),
        name: t_name,
        location,
        trait_bounds: Vec::new(),
    };

    let remap = global.merge_from(&local);
    param.remap_string_ids(&remap);

    assert_eq!(global.resolve(param.name), "T");
    assert_test_location(&param.location, &global);
}

#[test]
fn generic_parameter_list_remaps_all_parameters() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let t_name = local.intern("T");
    let u_name = local.intern("U");

    let mut list = GenericParameterList {
        parameters: vec![
            GenericParameter {
                id: TypeParameterId(0),
                name: t_name,
                location: make_location(&mut local),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(1),
                name: u_name,
                location: make_location(&mut local),
                trait_bounds: Vec::new(),
            },
        ],
    };

    let remap = global.merge_from(&local);
    list.remap_string_ids(&remap);

    assert_eq!(global.resolve(list.parameters[0].name), "T");
    assert_eq!(global.resolve(list.parameters[1].name), "U");
}

#[test]
fn parsed_type_ref_map_remaps_key_value_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let key_name = local.intern("String");
    let value_name = local.intern("Int");

    let mut parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::Named {
            name: key_name,
            location: make_location(&mut local),
        }),
        value: Box::new(ParsedTypeRef::Named {
            name: value_name,
            location: make_location(&mut local),
        }),
        location: make_location(&mut local),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Map {
            key,
            value,
            location,
        } => {
            match *key {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(name), "String");
                }
                _ => panic!("expected Named key"),
            }
            match *value {
                ParsedTypeRef::Named { name, .. } => {
                    assert_eq!(global.resolve(name), "Int");
                }
                _ => panic!("expected Named value"),
            }
            assert_test_location(&location, &global);
        }
        _ => panic!("expected Map"),
    }
}

#[test]
fn parsed_type_ref_qualified_remaps_all_path_components_and_location() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();

    let root_name = local.intern("io");
    let child_name = local.intern("input");
    let type_name = local.intern("Input");

    let mut parsed = ParsedTypeRef::Qualified {
        path: vec![root_name, child_name, type_name],
        location: make_location(&mut local),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Qualified { path, location } => {
            assert_eq!(path.len(), 3);
            assert_eq!(global.resolve(path[0]), "io");
            assert_eq!(global.resolve(path[1]), "input");
            assert_eq!(global.resolve(path[2]), "Input");
            assert_test_location(&location, &global);
        }
        _ => panic!("expected Qualified type"),
    }
}
