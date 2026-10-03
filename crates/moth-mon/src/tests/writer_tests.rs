use super::*;
use crate::{Field, Limits, Schema, SchemaType, Variant, decode_document};
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

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
        Field::required("real", SchemaType::Float),
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
        ("real".into(), Value::Float(-0.0)),
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
        .find(|(name, _)| name == "real")
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

#[test]
fn explicit_width_and_profile_values_round_trip_through_the_writer() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let explicit = Schema::record(vec![
        Field::required("width_i8", SchemaType::I8),
        Field::required("width_i16", SchemaType::I16),
        Field::required("width_i32", SchemaType::I32),
        Field::required("width_i64", SchemaType::I64),
        Field::required("width_u8", SchemaType::U8),
        Field::required("width_u16", SchemaType::U16),
        Field::required("width_u32", SchemaType::U32),
        Field::required("width_u64", SchemaType::U64),
        Field::required("half", SchemaType::F16),
        Field::required("single", SchemaType::F32),
        Field::required("double", SchemaType::F64),
        Field::required("octet", SchemaType::Byte),
        Field::required("wide", SchemaType::Int),
        Field::required("ratio", SchemaType::Float),
    ])
    .with_profile(profile)
    .prepare()
    .expect("explicit-width schema prepares");

    let value = Value::Record(vec![
        ("width_i8".into(), Value::I8(i8::MIN)),
        ("width_i16".into(), Value::I16(i16::MIN)),
        ("width_i32".into(), Value::I32(i32::MIN)),
        ("width_i64".into(), Value::I64(i64::MIN)),
        ("width_u8".into(), Value::U8(u8::MAX)),
        ("width_u16".into(), Value::U16(u16::MAX)),
        ("width_u32".into(), Value::U32(u32::MAX)),
        ("width_u64".into(), Value::U64(u64::MAX)),
        ("half".into(), Value::F16(-0.0)),
        ("single".into(), Value::F32(-0.0)),
        ("double".into(), Value::F64(-0.0)),
        ("octet".into(), Value::Byte(u8::MAX)),
        ("wide".into(), Value::Int(i64::MAX)),
        ("ratio".into(), Value::Float(f64::from(0.1f32))),
    ]);
    let encoded = encode_document(&value, &explicit).expect("explicit-width values encode");
    assert!(encoded.contains("width_i8 = -128"), "{encoded}");
    assert!(
        encoded.contains("width_u64 = 18446744073709551615"),
        "{encoded}"
    );
    assert!(encoded.contains("half = -0.0"), "{encoded}");
    assert!(encoded.contains("single = -0.0"), "{encoded}");
    assert!(encoded.contains("double = -0.0"), "{encoded}");
    assert!(encoded.contains("wide = 9223372036854775807"), "{encoded}");
    // `Float` is written at the captured Float32 precision, so the carrier's short text round-trips.
    assert!(encoded.contains("ratio = 0.1"), "{encoded}");
    let decoded = decode_document(&encoded, &explicit).expect("encoded document decodes");
    assert_eq!(decoded, value);
    let Value::Record(decoded_fields) = &decoded else {
        panic!("writer produced a non-record");
    };
    for name in ["half", "single", "double"] {
        let number = decoded_fields
            .iter()
            .find(|(field, _)| field == name)
            .and_then(|(_, value)| match value {
                Value::F16(number) | Value::F32(number) | Value::F64(number) => Some(*number),
                _ => None,
            })
            .unwrap_or_else(|| panic!("decoded {name} field is not a fixed binary float"));
        assert_eq!(number.to_bits(), (-0.0f64).to_bits(), "{name}");
    }

    // An inexact programmatic payload materialises at its own precision instead of failing.
    let half_only = Schema::value(SchemaType::F16)
        .prepare()
        .expect("F16 schema prepares");
    assert_eq!(
        encode_value(&Value::F16(0.1), &half_only).expect("F16 payload materialises"),
        "0.1"
    );
    let half_record = schema(vec![Field::required("value", SchemaType::F16)]);
    let rounded_half = encode_document(
        &Value::Record(vec![("value".into(), Value::F16(0.1))]),
        &half_record,
    )
    .expect("F16 payload materialises");
    assert_eq!(rounded_half, "(value = 0.1)");
    assert_eq!(
        decode_document(&rounded_half, &half_record).expect("F16 text decodes"),
        Value::Record(vec![("value".into(), Value::F16(0.099_975_585_937_5))])
    );

    // Profile validity failures stay structured and fail fast.
    let standard = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .prepare()
        .expect("default schema prepares");
    assert_eq!(
        encode_document(
            &Value::Record(vec![("value".into(), Value::Int(i64::from(i32::MAX) + 1))]),
            &standard,
        )
        .unwrap_err()
        .code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        encode_document(
            &Value::Record(vec![("value".into(), Value::Float(1.0e300))]),
            &Schema::record(vec![Field::required("value", SchemaType::Float)])
                .with_profile(profile)
                .prepare()
                .expect("Float32 profile schema prepares"),
        )
        .unwrap_err()
        .code,
        MonErrorCode::NonFiniteFloat
    );
    assert_eq!(
        encode_value(&Value::I8(i8::MIN), &half_only)
            .unwrap_err()
            .code,
        MonErrorCode::TypeMismatch
    );
}

#[test]
fn fixed_width_and_byte_map_keys_emit_in_insertion_order() {
    let schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Byte),
            value: Box::new(SchemaType::U64),
        },
    )]);
    let value = Value::Record(vec![(
        "values".into(),
        Value::Map(vec![
            (Value::Byte(9), Value::U64(u64::MAX)),
            (Value::Byte(1), Value::U64(0)),
            (Value::Byte(255), Value::U64(7)),
        ]),
    )]);
    let encoded = encode_document(&value, &schema).expect("fixed-key map encodes");
    assert!(
        encoded.contains("9 = 18446744073709551615, 1 = 0, 255 = 7"),
        "{encoded}"
    );
    assert_eq!(
        decode_document(&encoded, &schema).expect("fixed-key map decodes"),
        value
    );

    // A duplicate decoded key is still rejected on the completion path.
    let duplicate = encode_document(
        &Value::Record(vec![(
            "values".into(),
            Value::Map(vec![
                (Value::Byte(4), Value::U64(1)),
                (Value::Byte(4), Value::U64(2)),
            ]),
        )]),
        &schema,
    )
    .expect_err("duplicate Byte keys fail");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        duplicate.path.last(),
        Some(&PathSegment::MapKey("4".into()))
    );
}

#[test]
fn encoding_rejects_reserved_names_in_programmatic_values() {
    let record_schema = schema(vec![Field::required("value", SchemaType::Int)]);
    for name in ["__LoOp", "_U64", "dec01", "dec257", "true", "false", "none"] {
        let error = encode_document(
            &Value::Record(vec![(name.into(), Value::Int(1))]),
            &record_schema,
        )
        .expect_err("programmatic record field names cannot bypass reservation");
        assert_eq!(error.code, MonErrorCode::InvalidIdentifier, "{name}");
        assert_eq!(error.path, vec![crate::PathSegment::Field(name.into())]);
    }

    let nested_schema = schema(vec![Field::required(
        "group",
        SchemaType::Record {
            fields: vec![Field::required("known", SchemaType::Int)],
        },
    )]);
    let nested_error = encode_document(
        &Value::Record(vec![(
            "group".into(),
            Value::Record(vec![("_U64".into(), Value::Int(1))]),
        )]),
        &nested_schema,
    )
    .expect_err("nested programmatic names must use the same reservation rule");
    assert_eq!(nested_error.code, MonErrorCode::InvalidIdentifier);
    assert_eq!(
        nested_error.path,
        vec![
            crate::PathSegment::Field("group".into()),
            crate::PathSegment::Field("_U64".into()),
        ]
    );

    let choice_schema = Schema::value(SchemaType::Choice {
        name: "Theme".into(),
        variants: vec![
            Variant::unit("Ready"),
            Variant::payload("Payload", vec![Field::required("value", SchemaType::Int)]),
        ],
    })
    .prepare()
    .expect("valid choice schema prepares");
    for (value, expected_path) in [
        (
            Value::Choice {
                qualifier: Some("_U64".into()),
                variant: "Ready".into(),
                fields: Vec::new(),
            },
            Vec::new(),
        ),
        (
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "dec257".into(),
                fields: Vec::new(),
            },
            vec![crate::PathSegment::Variant("dec257".into())],
        ),
        (
            Value::Choice {
                qualifier: None,
                variant: "Payload".into(),
                fields: vec![("none".into(), Value::Int(1))],
            },
            vec![
                crate::PathSegment::Variant("Payload".into()),
                crate::PathSegment::Field("none".into()),
            ],
        ),
    ] {
        let error = encode_value(&value, &choice_schema)
            .expect_err("programmatic choice names cannot bypass reservation");
        assert_eq!(error.code, MonErrorCode::InvalidIdentifier);
        assert_eq!(error.path, expected_path);
    }
}
