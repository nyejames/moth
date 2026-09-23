use super::*;
use crate::compiler_frontend::mon::{Field, Limits, Schema, SchemaType, Variant, decode_document};

fn schema(fields: Vec<Field>) -> PreparedSchema {
    Schema::record(fields)
        .prepare()
        .expect("test schema prepares")
}

#[test]
fn compact_writer_completes_defaults_and_orders_fields() {
    let schema = schema(vec![
        Field::with_default("second", SchemaType::Int, Value::Int(2)),
        Field::required("first", SchemaType::String),
    ]);
    let value = Value::Record(vec![("first".into(), Value::String("x".into()))]);
    let encoded = encode_document(&value, &schema).expect("value encodes");
    assert_eq!(encoded, "(second = 2, first = \"x\")");
    let decoded = decode_document(&encoded, &schema).expect("encoded document decodes");
    assert_eq!(
        decoded,
        Value::Record(vec![
            ("second".into(), Value::Int(2)),
            ("first".into(), Value::String("x".into())),
        ])
    );
}

#[test]
fn exact_numbers_unicode_signed_zero_and_empty_containers_round_trip() {
    let schema = schema(vec![
        Field::required("integer", SchemaType::Integer),
        Field::required("decimal", SchemaType::Decimal { scale: 3 }),
        Field::required("float", SchemaType::Float),
        Field::required("boundary", SchemaType::Float),
        Field::required("text", SchemaType::String),
        Field::required("letter", SchemaType::Char),
        Field::required("quote", SchemaType::Char),
        Field::required("slash", SchemaType::Char),
        Field::required("control", SchemaType::Char),
        Field::required(
            "items",
            SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            },
        ),
        Field::required(
            "map",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        ),
    ]);
    let value = Value::Record(vec![
        (
            "integer".into(),
            Value::Integer("90071992547409931234567890".into()),
        ),
        ("decimal".into(), Value::Decimal("1.230".into())),
        ("float".into(), Value::Float(-0.0)),
        ("boundary".into(), Value::Float(f64::MIN_POSITIVE)),
        ("text".into(), Value::String("\"\\\n\r\t\u{0001}".into())),
        ("letter".into(), Value::Char('😀')),
        ("quote".into(), Value::Char('\'')),
        ("slash".into(), Value::Char('\\')),
        ("control".into(), Value::Char('\n')),
        ("items".into(), Value::Collection(Vec::new())),
        ("map".into(), Value::Map(Vec::new())),
    ]);
    let encoded = encode_document(&value, &schema).expect("value encodes");
    assert!(encoded.contains("-0.0"));
    assert!(encoded.contains("\\\""));
    assert!(encoded.contains("\\\\"));
    assert!(encoded.contains("\\n"));
    assert!(encoded.contains("\\r"));
    assert!(encoded.contains("\\t"));
    assert!(encoded.contains("\\u{1}"));
    assert!(encoded.contains("items = {}"));
    assert!(encoded.contains("map = {=}"));
    let decoded = decode_document(&encoded, &schema).expect("encoded document decodes");
    assert_eq!(decoded, value);
    let Value::Record(decoded_fields) = &decoded else {
        panic!("writer produced a non-record");
    };
    let decoded_float = decoded_fields
        .iter()
        .find(|(name, _)| name == "float")
        .and_then(|(_, value)| match value {
            Value::Float(value) => Some(*value),
            _ => None,
        })
        .expect("decoded signed zero field");
    assert_eq!(decoded_float.to_bits(), (-0.0f64).to_bits());
    let decoded_boundary = decoded_fields
        .iter()
        .find(|(name, _)| name == "boundary")
        .and_then(|(_, value)| match value {
            Value::Float(value) => Some(*value),
            _ => None,
        })
        .expect("decoded finite boundary field");
    assert_eq!(decoded_boundary.to_bits(), f64::MIN_POSITIVE.to_bits());
}

#[test]
fn choices_maps_and_pretty_output_have_same_semantics() {
    let schema = schema(vec![
        Field::required(
            "theme",
            SchemaType::Choice {
                name: "Theme".into(),
                variants: vec![
                    Variant::unit("Light"),
                    Variant::payload("Custom", vec![Field::required("name", SchemaType::String)]),
                ],
            },
        ),
        Field::required(
            "scores",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        ),
    ]);
    let value = Value::Record(vec![
        (
            "theme".into(),
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "Custom".into(),
                fields: vec![("name".into(), Value::String("Gold".into()))],
            },
        ),
        (
            "scores".into(),
            Value::Map(vec![
                (Value::String("a".into()), Value::Int(1)),
                (Value::String("b".into()), Value::Int(2)),
            ]),
        ),
    ]);
    let compact = encode_document(&value, &schema).expect("compact encoding");
    let pretty = encode_document_with_options(&value, &schema, WriteOptions::PRETTY)
        .expect("pretty encoding");
    assert_eq!(
        decode_document(&compact, &schema).unwrap(),
        decode_document(&pretty, &schema).unwrap()
    );
    assert_eq!(
        compact
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>(),
        pretty
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>(),
    );
    assert!(pretty.contains('\n'));
    assert!(compact.contains("::Custom(name = \"Gold\")"));
    assert!(compact.contains("\"a\" = 1, \"b\" = 2"));
}

#[test]
fn writer_rejects_bad_values_and_output_budget_without_partial_result() {
    let schema = schema(vec![Field::required("value", SchemaType::Int)]);
    let wrong = encode_document(
        &Value::Record(vec![("value".into(), Value::String("bad".into()))]),
        &schema,
    )
    .unwrap_err();
    assert_eq!(wrong.code, MonErrorCode::TypeMismatch);

    let bounded_string_schema = Schema::value(SchemaType::String)
        .with_limits(Limits {
            max_decoded_bytes: 2,
            ..Limits::default()
        })
        .prepare()
        .expect("bounded string schema prepares");
    let decoded_error =
        encode_value(&Value::String("too long".into()), &bounded_string_schema).unwrap_err();
    assert_eq!(decoded_error.code, MonErrorCode::DecodedBudget);

    let tiny = Schema::record(vec![Field::required("value", SchemaType::String)])
        .with_limits(Limits {
            max_output_bytes: 2,
            ..Limits::default()
        })
        .prepare()
        .unwrap();
    let budget = encode_document(
        &Value::Record(vec![("value".into(), Value::String("long".into()))]),
        &tiny,
    )
    .unwrap_err();
    assert_eq!(budget.code, MonErrorCode::OutputBudget);
}

#[test]
fn nested_fragments_compose_into_a_complete_document() {
    let nested = Schema::value(SchemaType::Struct {
        name: "Window".into(),
        fields: vec![
            Field::required("width", SchemaType::Int),
            Field::with_default("height", SchemaType::Int, Value::Int(720)),
        ],
    })
    .prepare()
    .expect("nested schema prepares");
    let fragment = encode_value(
        &Value::Record(vec![("width".into(), Value::Int(1280))]),
        &nested,
    )
    .expect("nested value encodes");
    assert_eq!(fragment, "(width = 1280, height = 720)");

    let root = schema(vec![Field::required(
        "window",
        SchemaType::Struct {
            name: "Window".into(),
            fields: vec![
                Field::required("width", SchemaType::Int),
                Field::required("height", SchemaType::Int),
            ],
        },
    )]);
    let document = format!("window = {fragment}");
    let decoded = decode_document(&document, &root).expect("composed document decodes");
    assert_eq!(
        decoded,
        Value::Record(vec![(
            "window".into(),
            Value::Record(vec![
                ("width".into(), Value::Int(1280)),
                ("height".into(), Value::Int(720)),
            ]),
        )])
    );
}

#[test]
fn writer_rejects_qualifier_numeric_float_depth_and_node_errors() {
    let choice_schema = schema(vec![Field::required(
        "theme",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Light"),
                Variant::payload("Custom", vec![Field::required("name", SchemaType::String)]),
            ],
        },
    )]);
    let qualifier_error = encode_document(
        &Value::Record(vec![(
            "theme".into(),
            Value::Choice {
                qualifier: Some("Other".into()),
                variant: "Light".into(),
                fields: Vec::new(),
            },
        )]),
        &choice_schema,
    )
    .unwrap_err();
    assert_eq!(qualifier_error.code, MonErrorCode::QualifierMismatch);

    let integer_schema = schema(vec![Field::required("value", SchemaType::Integer)]);
    let numeric_error = encode_document(
        &Value::Record(vec![("value".into(), Value::Integer("1e3".into()))]),
        &integer_schema,
    )
    .unwrap_err();
    assert_eq!(numeric_error.code, MonErrorCode::NumericType);

    let float_schema = schema(vec![Field::required("value", SchemaType::Float)]);
    let float_error = encode_document(
        &Value::Record(vec![("value".into(), Value::Float(f64::NAN))]),
        &float_schema,
    )
    .unwrap_err();
    assert_eq!(float_error.code, MonErrorCode::NonFiniteFloat);
    let depth_error = Schema::value(SchemaType::Collection {
        element: Box::new(SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        }),
    })
    .with_limits(Limits {
        max_depth: 1,
        ..Limits::default()
    })
    .prepare()
    .unwrap_err();
    assert_eq!(depth_error.code, MonErrorCode::DepthBudget);

    let malformed_depth_schema = Schema::value(SchemaType::Int)
        .with_limits(Limits {
            max_depth: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("scalar schema prepares");
    let malformed_depth_error = encode_value(
        &Value::Collection(vec![Value::Collection(vec![Value::Int(1)])]),
        &malformed_depth_schema,
    )
    .unwrap_err();
    assert_eq!(malformed_depth_error.code, MonErrorCode::TypeMismatch);
    let map_schema = Schema::value(SchemaType::Map {
        key: Box::new(SchemaType::String),
        value: Box::new(SchemaType::Int),
    })
    .prepare()
    .expect("map schema prepares");
    let duplicate_map_error = encode_value(
        &Value::Map(vec![
            (Value::String("same".into()), Value::Int(1)),
            (Value::String("same".into()), Value::Int(2)),
        ]),
        &map_schema,
    )
    .unwrap_err();
    assert_eq!(duplicate_map_error.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        duplicate_map_error.path,
        vec![PathSegment::MapKey("same".into())]
    );

    let arity_error = encode_document(
        &Value::Record(vec![(
            "theme".into(),
            Value::Choice {
                qualifier: None,
                variant: "Custom".into(),
                fields: Vec::new(),
            },
        )]),
        &choice_schema,
    )
    .unwrap_err();
    assert_eq!(arity_error.code, MonErrorCode::Arity);

    let node_schema = Schema::value(SchemaType::Collection {
        element: Box::new(SchemaType::Int),
    })
    .with_limits(Limits {
        max_nodes: 2,
        ..Limits::default()
    })
    .prepare()
    .expect("schema preparation fits node budget");
    let node_error = encode_value(
        &Value::Collection(vec![Value::Int(1), Value::Int(2)]),
        &node_schema,
    )
    .unwrap_err();
    assert_eq!(node_error.code, MonErrorCode::NodeBudget);
}

#[test]
fn nested_scalar_encoder_returns_complete_literal() {
    let string_schema = Schema::value(SchemaType::String)
        .prepare()
        .expect("string schema prepares");
    assert_eq!(
        encode_value(&Value::String("looks = MON".into()), &string_schema).unwrap(),
        "\"looks = MON\""
    );

    let optional_schema = Schema::value(SchemaType::Optional(Box::new(SchemaType::String)))
        .prepare()
        .expect("optional schema prepares");
    assert_eq!(
        encode_value(&Value::None, &optional_schema).unwrap(),
        "none"
    );
}

#[test]
fn finite_float_boundaries_materialise_and_round_trip() {
    let schema = schema(vec![Field::required(
        "values",
        SchemaType::Collection {
            element: Box::new(SchemaType::Float),
        },
    )]);
    let bits_of = |value: &Value| {
        let Value::Record(fields) = value else {
            panic!("float boundary document decoded to a non-record");
        };
        let Some((_, Value::Collection(values))) = fields.iter().find(|(name, _)| name == "values")
        else {
            panic!("decoded float boundary field is not a collection");
        };
        values
            .iter()
            .map(|value| match value {
                Value::Float(number) => number.to_bits(),
                _ => panic!("decoded float boundary element is not a Float"),
            })
            .collect::<Vec<_>>()
    };

    // Decoder materialisation from literal source text, without the writer.
    let materialised = decode_document(
        "values = {1.7976931348623157e+308, -1.7976931348623157e+308, \
             2.2250738585072014e-308, -2.2250738585072014e-308, \
             5e-324, -5e-324, -0.0, 0}",
        &schema,
    )
    .expect("float boundary literals decode");
    assert_eq!(
        bits_of(&materialised),
        [
            f64::MAX.to_bits(),
            (-f64::MAX).to_bits(),
            f64::MIN_POSITIVE.to_bits(),
            (-f64::MIN_POSITIVE).to_bits(),
            f64::from_bits(1).to_bits(),
            f64::from_bits(0x8000_0000_0000_0001).to_bits(),
            (-0.0f64).to_bits(),
            0.0f64.to_bits(),
        ]
    );

    // Writer-to-reader round trip, including neighbours of each boundary.
    let boundaries = [
        f64::MAX,
        -f64::MAX,
        f64::from_bits(f64::MAX.to_bits() - 1),
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1),
        f64::from_bits(f64::MIN_POSITIVE.to_bits() + 1),
        f64::from_bits(1),
        f64::from_bits(0x8000_0000_0000_0001),
        -0.0,
        0.0,
        1.0,
        f64::from_bits(1.0f64.to_bits() + 1),
    ];
    let value = Value::Record(vec![(
        "values".into(),
        Value::Collection(
            boundaries
                .iter()
                .map(|value| Value::Float(*value))
                .collect(),
        ),
    )]);
    let encoded = encode_document(&value, &schema).expect("float boundaries encode");
    let round_tripped = decode_document(&encoded, &schema).expect("float boundaries decode");
    assert_eq!(bits_of(&round_tripped), boundaries.map(f64::to_bits));
}
