use crate::{Field, Limits, MonErrorCode, PathSegment, Schema, SchemaType, Value, Variant};

#[test]
fn malformed_schema_and_defaults_fail_during_preparation() {
    let invalid_default = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I32,
        Value::String("not an I32".into()),
    )])
    .prepare()
    .expect_err("schema defaults are checked before use");
    assert_eq!(invalid_default.code, MonErrorCode::InvalidDefault);

    let duplicate_map_key_default = Schema::record(vec![Field::with_default(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::I32),
            value: Box::new(SchemaType::I32),
        },
        Value::Map(vec![
            (Value::I32(1), Value::I32(2)),
            (Value::I32(1), Value::I32(3)),
        ]),
    )])
    .prepare()
    .expect_err("prepared map defaults reject duplicate keys before copying");
    assert_eq!(duplicate_map_key_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        duplicate_map_key_default.path,
        vec![
            PathSegment::Field("values".into()),
            PathSegment::MapKey("1".into()),
        ],
    );

    let invalid_key = Schema::record(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Collection {
                element: Box::new(SchemaType::I32),
            }),
            value: Box::new(SchemaType::I32),
        },
    )])
    .prepare()
    .expect_err("only supported scalar families may be map keys");
    assert_eq!(invalid_key.code, MonErrorCode::InvalidMapKey);

    let duplicate_variant = Schema::record(vec![Field::required(
        "value",
        SchemaType::Choice {
            name: "Choice".into(),
            variants: vec![Variant::unit("Same"), Variant::unit("Same")],
        },
    )])
    .prepare()
    .expect_err("choice variant names must be unique");
    assert_eq!(duplicate_variant.code, MonErrorCode::InvalidSchema);
}

#[test]
fn preparation_checks_decimal_capacity_defaults_and_key_families() {
    // Exact-decimal scale capacity reaches the shared 256 owner and stops there.
    assert!(
        Schema::value(SchemaType::Decimal { scale: 256 })
            .prepare()
            .is_ok()
    );
    let too_wide = Schema::value(SchemaType::Decimal { scale: 257 })
        .prepare()
        .expect_err("a scale above the capacity fails");
    assert_eq!(too_wide.code, MonErrorCode::NumericScale);

    // An I64 default carries its exact scalar, so a value beyond the I32 range prepares.
    let above_int32 = i64::from(i32::MAX) + 1;
    assert!(
        Schema::record(vec![Field::with_default(
            "value",
            SchemaType::I64,
            Value::I64(above_int32),
        )])
        .prepare()
        .is_ok()
    );

    let non_finite = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::F64,
        Value::F64(f64::INFINITY),
    )])
    .prepare()
    .expect_err("a non-finite F64 default fails");
    assert_eq!(non_finite.code, MonErrorCode::InvalidDefault);
    let above_float32 = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::F32,
        Value::F32(1.0e300),
    )])
    .prepare()
    .expect_err("an F32 default outside the single-precision range fails");
    assert_eq!(above_float32.code, MonErrorCode::InvalidDefault);

    // Fixed defaults must match their own width family.
    let mismatched_family = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::I8,
        Value::I16(1),
    )])
    .prepare()
    .expect_err("a fixed default must match its own width");
    assert_eq!(mismatched_family.code, MonErrorCode::InvalidDefault);

    // Fixed integers and Byte are key families; fixed binary floats are not.
    assert!(
        Schema::record(vec![Field::required(
            "values",
            SchemaType::Map {
                key: Box::new(SchemaType::U64),
                value: Box::new(SchemaType::Byte),
            },
        )])
        .prepare()
        .is_ok()
    );
    let float_key = Schema::record(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::F32),
            value: Box::new(SchemaType::I32),
        },
    )])
    .prepare()
    .expect_err("fixed binary floats are not map-key families");
    assert_eq!(float_key.code, MonErrorCode::InvalidMapKey);

    // Prepared default maps keep fixed-width key identity in their duplicate check.
    let duplicate_byte_keys = Schema::record(vec![Field::with_default(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Byte),
            value: Box::new(SchemaType::I32),
        },
        Value::Map(vec![
            (Value::Byte(1), Value::I32(2)),
            (Value::Byte(1), Value::I32(3)),
        ]),
    )])
    .prepare()
    .expect_err("duplicate Byte keys in a default fail");
    assert_eq!(duplicate_byte_keys.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        duplicate_byte_keys.path,
        vec![
            PathSegment::Field("values".into()),
            PathSegment::MapKey("1".into()),
        ],
    );
}

#[test]
fn preparation_rejects_malformed_and_reserved_names_in_schema_shapes() {
    let invalid_schemas = vec![
        (
            "reserved root field",
            Schema::record(vec![Field::required("__LoOp", SchemaType::I32)]),
        ),
        (
            "fixed-width nominal name",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Struct {
                    name: "_U64".into(),
                    fields: vec![Field::required("inner", SchemaType::I32)],
                },
            )]),
        ),
        (
            "Dec-family choice name",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "dec01".into(),
                    variants: vec![Variant::unit("Ready")],
                },
            )]),
        ),
        (
            "Dec-family variant",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::unit("dec257")],
                },
            )]),
        ),
        (
            "nested record field",
            Schema::record(vec![Field::required(
                "outer",
                SchemaType::Record {
                    fields: vec![Field::required("true", SchemaType::I32)],
                },
            )]),
        ),
        (
            "choice payload field",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::payload(
                        "Ready",
                        vec![Field::required("none", SchemaType::I32)],
                    )],
                },
            )]),
        ),
        (
            "malformed field",
            Schema::record(vec![Field::required("not-a-name", SchemaType::I32)]),
        ),
    ];

    for (case, schema) in invalid_schemas {
        let error = match schema.prepare() {
            Ok(_) => panic!("{case} should fail schema preparation"),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            MonErrorCode::InvalidSchema,
            "{case}: {}",
            error.detail
        );
    }
}

#[test]
fn reserved_schema_field_names_are_charged_before_their_diagnostic() {
    let error = Schema::record(vec![Field::required("____________if", SchemaType::I32)])
        .with_limits(Limits {
            max_decoded_bytes: 4,
            ..Limits::default()
        })
        .prepare()
        .expect_err("an over-budget reserved field name must fail before its detail is built");
    assert_eq!(error.code, MonErrorCode::DecodedBudget);
    assert_eq!(error.path, Vec::new());
}

#[test]
fn preparation_charges_each_schema_name_exactly_once() {
    // outer + a + c + T + V + p = 10 name bytes across a nested record and a choice payload.
    let schema_with_budget = |max_decoded_bytes| {
        Schema::record(vec![
            Field::required(
                "outer",
                SchemaType::Record {
                    fields: vec![Field::required("a", SchemaType::I32)],
                },
            ),
            Field::required(
                "c",
                SchemaType::Choice {
                    name: "T".into(),
                    variants: vec![Variant::payload(
                        "V",
                        vec![Field::required("p", SchemaType::I32)],
                    )],
                },
            ),
        ])
        .with_limits(Limits {
            max_decoded_bytes,
            ..Limits::default()
        })
        .prepare()
    };
    assert!(schema_with_budget(10).is_ok());
    let error = schema_with_budget(9).expect_err("one byte below the name total must fail");
    assert_eq!(error.code, MonErrorCode::DecodedBudget);
}

#[test]
fn defaults_reject_reserved_names_with_the_invalid_default_projection() {
    let invalid_record_default = Schema::record(vec![Field::with_default(
        "group",
        SchemaType::Record {
            fields: vec![Field::required("known", SchemaType::I32)],
        },
        Value::Record(vec![("__LoOp".into(), Value::I32(1))]),
    )])
    .prepare()
    .expect_err("default record field names must use the shared reservation policy");
    assert_eq!(invalid_record_default.code, MonErrorCode::InvalidDefault);
    assert_eq!(
        invalid_record_default.path,
        vec![
            PathSegment::Field("group".into()),
            PathSegment::Field("__LoOp".into()),
        ]
    );

    let choice_type = SchemaType::Choice {
        name: "Theme".into(),
        variants: vec![
            Variant::unit("Ready"),
            Variant::payload("Payload", vec![Field::required("value", SchemaType::I32)]),
        ],
    };
    let invalid_choice_defaults = [
        (
            Value::Choice {
                qualifier: Some("_U64".into()),
                variant: "Ready".into(),
                fields: Vec::new(),
            },
            vec![PathSegment::Field("choice".into())],
        ),
        (
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "true".into(),
                fields: Vec::new(),
            },
            vec![
                PathSegment::Field("choice".into()),
                PathSegment::Variant("true".into()),
            ],
        ),
        (
            Value::Choice {
                qualifier: None,
                variant: "Payload".into(),
                fields: vec![("none".into(), Value::I32(1))],
            },
            vec![
                PathSegment::Field("choice".into()),
                PathSegment::Variant("Payload".into()),
                PathSegment::Field("none".into()),
            ],
        ),
    ];
    for (default, expected_path) in invalid_choice_defaults {
        let error = Schema::record(vec![Field::with_default(
            "choice",
            choice_type.clone(),
            default,
        )])
        .prepare()
        .expect_err("reserved qualifier, variant and payload names must fail defaults");
        assert_eq!(error.code, MonErrorCode::InvalidDefault);
        assert_eq!(error.path, expected_path);
    }
}
