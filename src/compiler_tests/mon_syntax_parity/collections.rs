//! Typed collection and map parity fixtures.
//!
//! WHAT: covers scalar and record collections, nesting, typed map keys, and duplicate/schema
//!       rejection boundaries.
//! WHY: source map literals need their declared struct receiver in the start body, while MON
//!      validates the same literal against the explicit root schema.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, SourceReason, assert_fixture, fixture,
    in_declarations, mon_span,
};

fn map_source(map_literal: &str, key_type: &str, value_type: &str) -> String {
    format!(
        "MapHolder = | items {{{key_type} = {value_type}}} |\n\
         data = MapHolder(items = {map_literal})\n"
    )
}

fn map_fixture(
    name: &str,
    map_literal: String,
    source: String,
    key_type: SchemaType,
    value_type: SchemaType,
    expected_source: SourceExpectation,
    expected_mon: MonExpectation,
) -> ParityFixture {
    fixture(
        name,
        format!("items = {map_literal}"),
        "",
        Schema::record(vec![Field::required(
            "items",
            SchemaType::Map {
                key: Box::new(key_type),
                value: Box::new(value_type),
            },
        )]),
        expected_source,
        expected_mon,
    )
    .with_source_override(source)
    .with_source_nominal_type("", "MapHolder")
}

fn schema_rejected_map_fixture(
    name: &str,
    key_type_name: &str,
    key_type: SchemaType,
    map_value: &str,
) -> ParityFixture {
    let declarations = format!("MapHolder = | items {{{key_type_name} = Int}} |\n");
    fixture(
        name,
        format!("items = {map_value}"),
        declarations.clone(),
        Schema::record(vec![Field::required(
            "items",
            SchemaType::Map {
                key: Box::new(key_type),
                value: Box::new(SchemaType::Int),
            },
        )]),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0016-MAP",
            reason: SourceReason::InvalidMapKeyType,
            site: in_declarations(&declarations, "{", 1),
        },
        MonExpectation::SchemaRejected {
            code: MonErrorCode::InvalidMapKey,
            path: vec![PathSegment::Field("items".into())],
        },
    )
}

fn map_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    let nested_map = r#"{"alpha" = {1, 2}, "beta" = {3}}"#;
    fixtures.push(map_fixture(
        "typed_string_map_with_nested_collections",
        nested_map.to_owned(),
        map_source(nested_map, "String", "{Int}"),
        SchemaType::String,
        SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        },
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "items".into(),
            Value::Map(vec![
                (
                    Value::String("alpha".into()),
                    Value::Collection(vec![Value::Int(1), Value::Int(2)]),
                ),
                (
                    Value::String("beta".into()),
                    Value::Collection(vec![Value::Int(3)]),
                ),
            ]),
        )])),
    ));

    let fixed_map = "{5 = 10, 255 = 20}";
    let fixed_source = "MapHolder = | items {U8 = Int} |\n\
entries {U8 = Int} = {5 = 10, 255 = 20}\n\
data = MapHolder(items = entries)\n"
        .to_owned();
    fixtures.push(map_fixture(
        "fixed_u8_map_keys_keep_their_declared_identity",
        fixed_map.into(),
        fixed_source,
        SchemaType::U8,
        SchemaType::Int,
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "items".into(),
            Value::Map(vec![
                (Value::U8(5), Value::Int(10)),
                (Value::U8(255), Value::Int(20)),
            ]),
        )])),
    ));

    let numeric_map = "{10 = 1, 1_0 = 2}";
    let numeric_source = map_source(numeric_map, "Int", "Int");
    let numeric_root = format!("items = {numeric_map}");
    fixtures.push(map_fixture(
        "numeric_separator_equivalent_keys_are_duplicate",
        numeric_map.into(),
        numeric_source.clone(),
        SchemaType::Int,
        SchemaType::Int,
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0033",
            reason: SourceReason::DuplicateMapKey,
            site: in_declarations(&numeric_source, "1_0", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::DuplicateMapKey,
            span: mon_span(&numeric_root, "1_0", 1),
            path: vec![
                PathSegment::Field("items".into()),
                PathSegment::MapKey("10".into()),
            ],
        },
    ));

    let tab_key = "a\t";
    let escaped_map = format!(r#"{{"a\t" = 1, "{tab_key}" = 2}}"#);
    let escaped_source = map_source(&escaped_map, "String", "Int");
    let escaped_root = format!("items = {escaped_map}");
    let second_spelling = format!("\"{tab_key}\"");
    fixtures.push(map_fixture(
        "escaped_and_literal_tab_string_keys_are_duplicate",
        escaped_map,
        escaped_source.clone(),
        SchemaType::String,
        SchemaType::Int,
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0033",
            reason: SourceReason::DuplicateMapKey,
            site: in_declarations(&escaped_source, &second_spelling, 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::DuplicateMapKey,
            span: mon_span(&escaped_root, &second_spelling, 1),
            path: vec![
                PathSegment::Field("items".into()),
                PathSegment::MapKey("a\t".into()),
            ],
        },
    ));

    fixtures.push(schema_rejected_map_fixture(
        "binary_float_map_key_schema_is_rejected",
        "F32",
        SchemaType::F32,
        "{1.0 = 2}",
    ));
    fixtures.push(schema_rejected_map_fixture(
        "decimal_map_key_schema_is_rejected",
        "Dec2",
        SchemaType::Decimal { scale: 2 },
        "{1.0 = 2}",
    ));

    fixtures
}

fn collection_fixtures() -> Vec<ParityFixture> {
    let cells = SchemaType::Collection {
        element: Box::new(SchemaType::Struct {
            name: "Cell".into(),
            fields: vec![Field::required("value", SchemaType::Int)],
        }),
    };
    let nested = SchemaType::Collection {
        element: Box::new(SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        }),
    };

    let literal =
        "scalars = {1, -2, 3}, cells = {Cell(value = 4), Cell(value = 5)}, nested = {{1, 2}, {3}}";
    let source = format!(
        "Cell = | value Int |\n\
         CollectionHolder = | scalars {{Int}}, cells {{Cell}}, nested {{{{Int}}}} |\n\
         data = CollectionHolder({literal})\n"
    );
    let fixture = fixture(
        "scalar_record_and_nested_collections",
        literal,
        "",
        Schema::record(vec![
            Field::required(
                "scalars",
                SchemaType::Collection {
                    element: Box::new(SchemaType::Int),
                },
            ),
            Field::required("cells", cells),
            Field::required("nested", nested),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            (
                "scalars".into(),
                Value::Collection(vec![Value::Int(1), Value::Int(-2), Value::Int(3)]),
            ),
            (
                "cells".into(),
                Value::Collection(vec![
                    Value::Record(vec![("value".into(), Value::Int(4))]),
                    Value::Record(vec![("value".into(), Value::Int(5))]),
                ]),
            ),
            (
                "nested".into(),
                Value::Collection(vec![
                    Value::Collection(vec![Value::Int(1), Value::Int(2)]),
                    Value::Collection(vec![Value::Int(3)]),
                ]),
            ),
        ])),
    )
    .with_source_override(source)
    .with_source_nominal_type("", "CollectionHolder")
    .with_source_nominal_type("cells[0]", "Cell")
    .with_source_nominal_type("cells[1]", "Cell");

    vec![fixture]
}

#[test]
fn typed_collections_and_maps_match_mon_data() {
    let _guard = lock_counter_test();
    for fixture in collection_fixtures().into_iter().chain(map_fixtures()) {
        assert_fixture(fixture);
    }
}
