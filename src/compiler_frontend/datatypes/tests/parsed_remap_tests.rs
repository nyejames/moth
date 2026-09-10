//! String-ID remapping tests for parsed type references.
//!
//! Parsed syntax owns optional exact spans, while remapping only changes interned names.

use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn authored_span() -> Option<SourceSpan> {
    Some(SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::source_start(),
    ))
}

#[test]
fn parsed_type_ref_named_remaps_name_without_changing_span() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let name_local = local.intern("MyType");
    let span = authored_span();
    let mut parsed = ParsedTypeRef::Named {
        name: name_local,
        span,
    };

    global.intern("preexisting");
    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Named { name, span: actual } => {
            assert_eq!(global.resolve(name), "MyType");
            assert_eq!(actual, span);
        }
        _ => panic!("expected Named type"),
    }
}

#[test]
fn parsed_type_ref_applied_remaps_nested_names() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let box_name = local.intern("Box");
    let string_name = local.intern("String");
    let mut parsed = ParsedTypeRef::Applied {
        base: Box::new(ParsedTypeRef::Named {
            name: box_name,
            span: authored_span(),
        }),
        arguments: vec![ParsedTypeRef::Named {
            name: string_name,
            span: authored_span(),
        }],
        span: authored_span(),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            assert!(
                matches!(*base, ParsedTypeRef::Named { name, .. } if global.resolve(name) == "Box")
            );
            assert!(
                matches!(&arguments[0], ParsedTypeRef::Named { name, .. } if global.resolve(*name) == "String")
            );
        }
        _ => panic!("expected Applied type"),
    }
}

#[test]
fn parsed_type_ref_collection_remaps_element_and_capacity_name() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let element_name = local.intern("Int");
    let capacity_name = local.intern("capacity");
    let mut parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::Named {
            name: element_name,
            span: authored_span(),
        }),
        span: authored_span(),
        fixed_capacity: Some(
            crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity::BareConstant {
                name: capacity_name,
                span: authored_span(),
            },
        ),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Collection {
            element,
            fixed_capacity,
            ..
        } => {
            assert!(
                matches!(*element, ParsedTypeRef::Named { name, .. } if global.resolve(name) == "Int")
            );
            assert!(
                matches!(fixed_capacity, Some(crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity::BareConstant { name, .. }) if global.resolve(name) == "capacity")
            );
        }
        _ => panic!("expected Collection type"),
    }
}

#[test]
fn parsed_type_ref_optional_remaps_inner_name() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let bool_name = local.intern("Bool");
    let mut parsed = ParsedTypeRef::Optional {
        inner: Box::new(ParsedTypeRef::Named {
            name: bool_name,
            span: authored_span(),
        }),
        span: authored_span(),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    assert!(
        matches!(parsed, ParsedTypeRef::Optional { inner, .. } if matches!(*inner, ParsedTypeRef::Named { name, .. } if global.resolve(name) == "Bool"))
    );
}

#[test]
fn parsed_type_ref_map_and_qualified_names_remap_recursively() {
    let mut local = StringTable::new();
    let mut global = StringTable::new();
    let key_name = local.intern("String");
    let path = vec![
        local.intern("io"),
        local.intern("input"),
        local.intern("Input"),
    ];
    let mut parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::Named {
            name: key_name,
            span: authored_span(),
        }),
        value: Box::new(ParsedTypeRef::Qualified {
            path,
            span: authored_span(),
        }),
        span: authored_span(),
    };

    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);

    match parsed {
        ParsedTypeRef::Map { key, value, .. } => {
            assert!(
                matches!(*key, ParsedTypeRef::Named { name, .. } if global.resolve(name) == "String")
            );
            let ParsedTypeRef::Qualified { path, .. } = *value else {
                panic!("expected qualified value")
            };
            assert_eq!(
                path.iter()
                    .map(|id| global.resolve(*id))
                    .collect::<Vec<_>>(),
                vec!["io", "input", "Input"]
            );
        }
        _ => panic!("expected Map type"),
    }
}

#[test]
fn inferred_type_ref_is_unchanged_by_remap() {
    let local = StringTable::new();
    let mut global = StringTable::new();
    let mut parsed = ParsedTypeRef::Inferred;
    let remap = global.merge_from(&local);
    parsed.remap_string_ids(&remap);
    assert!(matches!(parsed, ParsedTypeRef::Inferred));
}
