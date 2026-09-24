use super::*;
use crate::compiler_frontend::mon::{Field, Limits, Schema, SchemaType, Variant};

fn schema(fields: Vec<Field>) -> PreparedSchema {
    Schema::record(fields)
        .with_limits(Limits::default())
        .prepare()
        .expect("test schema prepares")
}

#[test]
fn decodes_implicit_and_explicit_roots() {
    let root_schema = schema(vec![
        Field::required("title", SchemaType::String),
        Field::required("count", SchemaType::Int),
    ]);
    let expected = Value::Record(vec![
        ("title".into(), Value::String("x".into())),
        ("count".into(), Value::Int(2)),
    ]);
    assert_eq!(
        decode_document("title = \"x\", count = 2", &root_schema),
        Ok(expected.clone())
    );
    assert_eq!(
        decode_document("(count = 2, title = \"x\",)", &root_schema),
        Ok(expected)
    );
    let empty_schema = schema(Vec::new());
    assert_eq!(
        decode_document("", &empty_schema),
        Ok(Value::Record(Vec::new()))
    );
    assert_eq!(
        decode_document("-- comment only\n", &empty_schema),
        Ok(Value::Record(Vec::new()))
    );
    assert_eq!(
        decode_document("title = \"x\" (count = 2)", &root_schema)
            .unwrap_err()
            .code,
        MonErrorCode::MissingComma
    );
}
#[test]
fn rejects_nonliteral_names_and_missing_commas() {
    let schema = schema(vec![Field::required("value", SchemaType::Int)]);
    assert_eq!(
        decode_document("value = other", &schema).unwrap_err().code,
        MonErrorCode::UnexpectedToken
    );
    assert_eq!(
        decode_document("value = 1 value2 = 2", &schema)
            .unwrap_err()
            .code,
        MonErrorCode::MissingComma
    );
}

#[test]
fn decodes_exact_integer_and_unicode_escape() {
    let schema = schema(vec![
        Field::required("exact", SchemaType::Integer),
        Field::required("text", SchemaType::String),
    ]);
    let value = decode_document(
        "exact = 90071992547409931234567890, text = \"A\\u{1f600}\"",
        &schema,
    )
    .expect("exact MON values decode");
    assert_eq!(
        value,
        Value::Record(vec![
            (
                "exact".into(),
                Value::Integer("90071992547409931234567890".into()),
            ),
            ("text".into(), Value::String("A😀".into())),
        ])
    );
}

#[test]
fn preserves_parser_paths_and_charges_exact_numbers_before_copying() {
    let nested_schema = schema(vec![Field::required(
        "outer",
        SchemaType::Record {
            fields: vec![Field::required("text", SchemaType::String)],
        },
    )]);
    let nested_error = decode_document(r#"outer = (text = "\u{D800}")"#, &nested_schema)
        .expect_err("invalid nested Unicode must fail");
    assert_eq!(nested_error.code, MonErrorCode::InvalidCharacter);
    assert_eq!(
        nested_error.path,
        vec![
            PathSegment::Field("outer".into()),
            PathSegment::Field("text".into()),
        ]
    );
    let truncated_error = decode_document("outer = (text =", &nested_schema)
        .expect_err("truncated nested field must fail");
    assert_eq!(truncated_error.code, MonErrorCode::UnexpectedEnd);
    assert_eq!(
        truncated_error.path,
        vec![
            PathSegment::Field("outer".into()),
            PathSegment::Field("text".into()),
        ]
    );

    let collection_schema = schema(vec![Field::required(
        "values",
        SchemaType::Collection {
            element: Box::new(SchemaType::String),
        },
    )]);
    let collection_error = decode_document(r#"values = {"\u{D800}"}"#, &collection_schema)
        .expect_err("invalid collection Unicode must fail");
    assert_eq!(collection_error.code, MonErrorCode::InvalidCharacter);
    assert_eq!(
        collection_error.path,
        vec![PathSegment::Field("values".into()), PathSegment::Index(0),]
    );

    let map_schema = schema(vec![Field::required(
        "scores",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let map_error = decode_document(r#"scores = {"x" ="#, &map_schema)
        .expect_err("truncated map value must fail");
    assert_eq!(map_error.code, MonErrorCode::UnexpectedEnd);
    assert_eq!(
        map_error.path,
        vec![PathSegment::Field("scores".into()), PathSegment::Index(0)]
    );
    let map_tail_error = decode_document(r#"scores = {"x" = 1, "y""#, &map_schema)
        .expect_err("truncated map entry must fail");
    assert_eq!(map_tail_error.code, MonErrorCode::UnexpectedEnd);
    assert_eq!(
        map_tail_error.path,
        vec![PathSegment::Field("scores".into()), PathSegment::Index(1)]
    );
    let choice_schema = schema(vec![Field::required(
        "choice",
        SchemaType::Choice {
            name: "Widget".into(),
            variants: vec![Variant::payload(
                "Text",
                vec![Field::required("value", SchemaType::String)],
            )],
        },
    )]);
    let choice_error = decode_document(r#"choice = ::Text("\u{D800}")"#, &choice_schema)
        .expect_err("invalid choice payload Unicode must fail");
    assert_eq!(choice_error.code, MonErrorCode::InvalidCharacter);
    assert_eq!(
        choice_error.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Text".into()),
        ]
    );

    let exact_schema = Schema::record(vec![Field::required("x", SchemaType::Integer)])
        .with_limits(Limits {
            max_decoded_bytes: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("exact-number budget schema prepares");
    let exact_error = decode_document("x = 123", &exact_schema)
        .expect_err("exact numeric output must honor decoded-byte budget");
    assert_eq!(exact_error.code, MonErrorCode::DecodedBudget);
    assert_eq!(exact_error.path, vec![PathSegment::Field("x".into())]);

    let label_schema = Schema::record(Vec::new())
        .with_limits(Limits {
            max_decoded_bytes: 0,
            ..Limits::default()
        })
        .prepare()
        .expect("label budget schema prepares");
    let label_error = decode_document("long_name = 1", &label_schema)
        .expect_err("input-derived path names must honor decoded-byte budget");
    assert_eq!(label_error.code, MonErrorCode::DecodedBudget);
}
#[test]
fn preserves_float_sign_and_decimal_text_until_conversion() {
    let numeric_schema = schema(vec![
        Field::required("float", SchemaType::Float),
        Field::required("decimal", SchemaType::Decimal { scale: 2 }),
        Field::required("whole", SchemaType::Int),
    ]);
    let value = decode_document("float = -0.0, decimal = 1.2300, whole = 7", &numeric_schema)
        .expect("finite float and lossless decimal should decode");
    let Value::Record(fields) = value else {
        panic!("root schema produced a non-record");
    };
    let float = fields
        .iter()
        .find(|(name, _)| name == "float")
        .map(|(_, value)| value)
        .expect("float field");
    assert!(matches!(float, Value::Float(value) if value.to_bits() == (-0.0f64).to_bits()));
    assert_eq!(
        fields
            .iter()
            .find(|(name, _)| name == "decimal")
            .map(|(_, value)| value),
        Some(&Value::Decimal("1.2300".into()))
    );
    let int_schema = schema(vec![Field::required("whole", SchemaType::Int)]);
    assert_eq!(
        decode_document("whole = 2147483648", &int_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        decode_document("float = 1.0, decimal = 1.231, whole = 7", &numeric_schema,)
            .unwrap_err()
            .code,
        MonErrorCode::NumericScale
    );
}
#[test]
fn decodes_defaults_containers_choices_and_comments() {
    let data_schema = schema(vec![
        Field::with_default(
            "layout",
            SchemaType::Struct {
                name: "Layout".into(),
                fields: vec![
                    Field::required("width", SchemaType::Int),
                    Field::with_default("height", SchemaType::Int, Value::Int(720)),
                ],
            },
            Value::Record(vec![("width".into(), Value::Int(1280))]),
        ),
        Field::required(
            "items",
            SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            },
        ),
        Field::required(
            "scores",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        ),
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
        Field::required("note", SchemaType::Optional(Box::new(SchemaType::String))),
    ]);
    let value = decode_document(
        "-- save data\nitems = {1, 2,}, scores = {\"Priya\" = 10, \"Rob\" = 12},\n\
             theme = ::Custom(name = \"Gold\"), note = none,",
        &data_schema,
    )
    .expect("literal data should decode");

    assert_eq!(
        value,
        Value::Record(vec![
            (
                "layout".into(),
                Value::Record(vec![
                    ("width".into(), Value::Int(1280)),
                    ("height".into(), Value::Int(720)),
                ]),
            ),
            (
                "items".into(),
                Value::Collection(vec![Value::Int(1), Value::Int(2)]),
            ),
            (
                "scores".into(),
                Value::Map(vec![
                    (Value::String("Priya".into()), Value::Int(10)),
                    (Value::String("Rob".into()), Value::Int(12)),
                ]),
            ),
            (
                "theme".into(),
                Value::Choice {
                    qualifier: None,
                    variant: "Custom".into(),
                    fields: vec![("name".into(), Value::String("Gold".into()))],
                },
            ),
            ("note".into(), Value::None),
        ])
    );

    let optional_schema = schema(vec![Field::required(
        "note",
        SchemaType::Optional(Box::new(SchemaType::String)),
    )]);
    assert_eq!(
        decode_document("", &optional_schema).unwrap_err().code,
        MonErrorCode::MissingField
    );

    let no_merge_schema = schema(vec![Field::with_default(
        "layout",
        SchemaType::Record {
            fields: vec![
                Field::required("width", SchemaType::Int),
                Field::required("height", SchemaType::Int),
            ],
        },
        Value::Record(vec![
            ("width".into(), Value::Int(1)),
            ("height".into(), Value::Int(2)),
        ]),
    )]);
    assert_eq!(
        decode_document("layout = (width = 9)", &no_merge_schema)
            .unwrap_err()
            .code,
        MonErrorCode::MissingField
    );
}

#[test]
fn rejects_numeric_categories_and_container_kind_changes() {
    let int_schema = schema(vec![Field::required("value", SchemaType::Int)]);
    assert_eq!(
        decode_document("1", &int_schema).unwrap_err().code,
        MonErrorCode::RootNotRecord
    );
    assert_eq!(
        decode_document("value = 3.0", &int_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericType
    );
    assert_eq!(
        decode_document("value = 1e3", &int_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericType
    );
    assert_eq!(
        decode_document("value = 1E3", &int_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericSyntax
    );

    let map_schema = schema(vec![Field::required(
        "value",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    assert_eq!(
        decode_document("value = {}", &map_schema).unwrap_err().code,
        MonErrorCode::MapKind
    );

    let collection_schema = schema(vec![Field::required(
        "value",
        SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        },
    )]);
    assert_eq!(
        decode_document("value = {=}", &collection_schema)
            .unwrap_err()
            .code,
        MonErrorCode::MapKind
    );
}
#[test]
fn accepts_trivia_before_constructor_payloads_but_not_around_separator() {
    let struct_schema = schema(vec![Field::required(
        "size",
        SchemaType::Struct {
            name: "Size".into(),
            fields: vec![
                Field::required("width", SchemaType::Int),
                Field::required("height", SchemaType::Int),
            ],
        },
    )]);
    assert_eq!(
        decode_document("size = Size (width = 1, height = 2)", &struct_schema),
        Ok(Value::Record(vec![(
            "size".into(),
            Value::Record(vec![
                ("width".into(), Value::Int(1)),
                ("height".into(), Value::Int(2)),
            ]),
        )]))
    );
    assert_eq!(
        decode_document(
            "size = Size -- payload\n(width = 1, height = 2)",
            &struct_schema
        ),
        Ok(Value::Record(vec![(
            "size".into(),
            Value::Record(vec![
                ("width".into(), Value::Int(1)),
                ("height".into(), Value::Int(2)),
            ]),
        )]))
    );

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
    assert_eq!(
        decode_document("theme = Theme::Custom (name = \"Gold\")", &choice_schema),
        Ok(Value::Record(vec![(
            "theme".into(),
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "Custom".into(),
                fields: vec![("name".into(), Value::String("Gold".into()))],
            },
        )]))
    );
    assert_eq!(
        decode_document(
            "theme = ::Custom -- payload\n(name = \"Gold\")",
            &choice_schema
        ),
        Ok(Value::Record(vec![(
            "theme".into(),
            Value::Choice {
                qualifier: None,
                variant: "Custom".into(),
                fields: vec![("name".into(), Value::String("Gold".into()))],
            },
        )]))
    );
    // `payload_opens_after_trivia` only accepts whitespace/comments, never `/`.
    assert_eq!(
        decode_document("theme = ::Custom / (name = \"Gold\")", &choice_schema)
            .unwrap_err()
            .code,
        MonErrorCode::MissingComma
    );
    assert_eq!(
        decode_document("theme = Theme ::Custom(name = \"Gold\")", &choice_schema)
            .unwrap_err()
            .code,
        MonErrorCode::UnexpectedToken
    );
    assert_eq!(
        decode_document("theme = Theme:: Custom(name = \"Gold\")", &choice_schema)
            .unwrap_err()
            .code,
        MonErrorCode::InvalidIdentifier
    );
    assert_eq!(
        decode_document(
            "theme = Theme::Custom -- payload\n(name = \"Gold\")",
            &choice_schema
        ),
        Ok(Value::Record(vec![(
            "theme".into(),
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "Custom".into(),
                fields: vec![("name".into(), Value::String("Gold".into()))],
            },
        )]))
    );
}

#[test]
fn typed_map_keys_keep_distinctions_and_first_duplicate_wins() {
    let int_key_schema = schema(vec![Field::required(
        "counts",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
    )]);
    assert_eq!(
        decode_document("counts = {1 = 10, 2 = 20}", &int_key_schema),
        Ok(Value::Record(vec![(
            "counts".into(),
            Value::Map(vec![
                (Value::Int(1), Value::Int(10)),
                (Value::Int(2), Value::Int(20)),
            ]),
        )]))
    );
    let duplicate = decode_document("counts = {1 = 10, 1 = 20}", &int_key_schema)
        .expect_err("duplicate int keys must fail");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(duplicate.span, Some(Span::new(18, 19)));
    assert_eq!(
        duplicate.path,
        vec![
            PathSegment::Field("counts".into()),
            PathSegment::MapKey("1".into()),
        ]
    );

    let repeated = decode_document("counts = {1 = 10, 2 = 20, 1 = 30}", &int_key_schema)
        .expect_err("the first repeated key must fail even after distinct keys");
    assert_eq!(repeated.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(repeated.path.last(), Some(&PathSegment::MapKey("1".into())));

    // Exercise the budgeted index path directly with an exhausted budget: the
    // per-key index charge fails before any further validation work.
    let tight_schema = schema(vec![Field::required(
        "counts",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let map_key = PreparedType::Int;
    let map_value = PreparedType::Int;
    let mut budget = BudgetState::new(tight_schema.limits());
    budget
        .charge_nodes(tight_schema.limits().max_nodes, None)
        .expect("budget starts exhausted");
    let mut path = vec![PathSegment::Field("counts".into())];
    let key_span = Span::new(10, 11);
    let value_span = Span::new(14, 16);
    let entries = vec![(
        RawValue {
            span: key_span,
            kind: RawKind::Number(NumericRaw {
                text: "1",
                normalized: "1".into(),
                kind: NumericLiteralKind::WholeNumber,
            }),
        },
        RawValue {
            span: value_span,
            kind: RawKind::Number(NumericRaw {
                text: "10",
                normalized: "10".into(),
                kind: NumericLiteralKind::WholeNumber,
            }),
        },
    )];
    assert_eq!(
        validate_map(entries, &map_key, &map_value, &mut budget, &mut path, 0)
            .unwrap_err()
            .code,
        MonErrorCode::NodeBudget
    );
}

#[test]
fn routes_qualified_struct_arguments_and_preserves_nested_map_paths() {
    let struct_schema = schema(vec![Field::required(
        "size",
        SchemaType::Struct {
            name: "Size".into(),
            fields: vec![
                Field::required("width", SchemaType::Int),
                Field::required("height", SchemaType::Int),
            ],
        },
    )]);
    assert_eq!(
        decode_document("size = Size(1280, height = 720)", &struct_schema),
        Ok(Value::Record(vec![(
            "size".into(),
            Value::Record(vec![
                ("width".into(), Value::Int(1280)),
                ("height".into(), Value::Int(720)),
            ]),
        )]))
    );
    assert_eq!(
        decode_document("size = (1280, height = 720)", &struct_schema)
            .unwrap_err()
            .code,
        MonErrorCode::ArgumentOrder
    );

    let nested_schema = schema(vec![Field::required(
        "outer",
        SchemaType::Record {
            fields: vec![Field::required(
                "scores",
                SchemaType::Map {
                    key: Box::new(SchemaType::String),
                    value: Box::new(SchemaType::Int),
                },
            )],
        },
    )]);
    let error = decode_document(r#"outer = (scores = {"x" = 1, "x" = 2})"#, &nested_schema)
        .expect_err("duplicate nested map key must fail");
    assert_eq!(error.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        error.path,
        vec![
            PathSegment::Field("outer".into()),
            PathSegment::Field("scores".into()),
            PathSegment::MapKey("x".into()),
        ]
    );

    let value_error = decode_document(r#"outer = (scores = {"x" = "bad"})"#, &nested_schema)
        .expect_err("map value type mismatch must fail");
    assert_eq!(value_error.code, MonErrorCode::TypeMismatch);
    assert_eq!(
        value_error.path,
        vec![
            PathSegment::Field("outer".into()),
            PathSegment::Field("scores".into()),
            PathSegment::MapKey("x".into()),
        ]
    );
}

#[test]
fn rejects_duplicate_keys_and_invalid_unicode_at_their_spans() {
    let map_schema = schema(vec![Field::required(
        "scores",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let duplicate = decode_document(r#"scores = {"x" = 1, "x" = 2}"#, &map_schema)
        .expect_err("duplicate map keys must fail");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(duplicate.span, Some(Span::new(19, 22)));

    let text_schema = schema(vec![
        Field::required("text", SchemaType::String),
        Field::required("letter", SchemaType::Char),
    ]);
    assert_eq!(
        decode_document(r#"text = "\u{D800}", letter = 'a'"#, &text_schema)
            .unwrap_err()
            .code,
        MonErrorCode::InvalidCharacter
    );
    assert_eq!(
        decode_document(r#"text = "\0", letter = 'a'"#, &text_schema)
            .unwrap_err()
            .code,
        MonErrorCode::InvalidEscape
    );
    let decoded = decode_document(
        r#"text = "line
break", letter = '\u{1F600}'"#,
        &text_schema,
    )
    .expect("MON preserves string newlines and decodes scalar escapes");
    assert_eq!(
        decoded,
        Value::Record(vec![
            ("text".into(), Value::String("line\nbreak".into())),
            ("letter".into(), Value::Char('😀')),
        ])
    );
}

#[test]
fn enforces_decoder_budgets_and_schema_eligibility() {
    let numeric_schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .with_limits(Limits {
            max_numeric_digits: 2,
            ..Limits::default()
        })
        .prepare()
        .expect("numeric budget schema prepares");
    assert_eq!(
        decode_document("value = 123", &numeric_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericBudget
    );

    let depth_schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .with_limits(Limits {
            max_depth: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("depth budget schema prepares");
    assert_eq!(
        decode_document("value = (nested = 1)", &depth_schema)
            .unwrap_err()
            .code,
        MonErrorCode::DepthBudget
    );

    let default_schema = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(1),
    )])
    .with_limits(Limits {
        max_default_expansions: 0,
        ..Limits::default()
    })
    .prepare()
    .expect("default budget schema prepares");
    assert_eq!(
        decode_document("", &default_schema).unwrap_err().code,
        MonErrorCode::DefaultBudget
    );

    let unsupported = Schema::record(vec![Field::required(
        "resource",
        SchemaType::Collection {
            element: Box::new(SchemaType::Unsupported {
                name: "Resource".into(),
            }),
        },
    )])
    .prepare()
    .expect_err("unsupported schema members must fail during preparation");
    assert_eq!(unsupported.code, MonErrorCode::UnsupportedSchema);

    let depth_limited_schema = Schema::value(SchemaType::Collection {
        element: Box::new(SchemaType::Int),
    })
    .with_limits(Limits {
        max_depth: 0,
        ..Limits::default()
    })
    .prepare()
    .expect_err("schema traversal depth must be bounded");
    assert_eq!(depth_limited_schema.code, MonErrorCode::DepthBudget);

    let node_limited_schema = Schema::record(Vec::new())
        .with_limits(Limits {
            max_nodes: 0,
            ..Limits::default()
        })
        .prepare()
        .expect_err("prepared schema nodes must be bounded");
    assert_eq!(node_limited_schema.code, MonErrorCode::NodeBudget);

    let name_budget_schema = Schema::record(vec![Field::required("long_name", SchemaType::Int)])
        .with_limits(Limits {
            max_decoded_bytes: 4,
            ..Limits::default()
        })
        .prepare()
        .expect_err("schema identifier copies must honor decoded-byte budget");
    assert_eq!(name_budget_schema.code, MonErrorCode::DecodedBudget);

    let nested_default_budget = Schema::record(vec![Field::with_default(
        "layout",
        SchemaType::Record {
            fields: vec![Field::with_default(
                "height",
                SchemaType::Int,
                Value::Int(720),
            )],
        },
        Value::Record(Vec::new()),
    )])
    .with_limits(Limits {
        max_default_expansions: 0,
        ..Limits::default()
    })
    .prepare()
    .expect_err("nested prepared defaults must honor expansion budget");
    assert_eq!(nested_default_budget.code, MonErrorCode::DefaultBudget);

    let default_bytes_budget = Schema::record(vec![Field::with_default(
        "layout",
        SchemaType::Record {
            fields: vec![Field::with_default(
                "text",
                SchemaType::String,
                Value::String("large".into()),
            )],
        },
        Value::Record(Vec::new()),
    )])
    .with_limits(Limits {
        max_decoded_bytes: 0,
        ..Limits::default()
    })
    .prepare()
    .expect_err("prepared default copies must honor decoded-byte budget");
    assert_eq!(default_bytes_budget.code, MonErrorCode::DecodedBudget);

    let numeric_default_budget = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Integer,
        Value::Integer("123".into()),
    )])
    .with_limits(Limits {
        max_numeric_digits: 2,
        ..Limits::default()
    })
    .prepare()
    .expect_err("numeric defaults must check digit budget before parsing");
    assert_eq!(numeric_default_budget.code, MonErrorCode::NumericBudget);
    let nested_optional = Schema::record(vec![Field::required(
        "value",
        SchemaType::Optional(Box::new(SchemaType::Optional(Box::new(SchemaType::Int)))),
    )])
    .prepare()
    .expect_err("nested optional schemas must be rejected");
    assert_eq!(nested_optional.code, MonErrorCode::InvalidSchema);

    let mut choice_default_schema = Schema::record(vec![Field::with_default(
        "theme",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![Variant::payload(
                "V",
                vec![Field::required("p", SchemaType::String)],
            )],
        },
        Value::Choice {
            qualifier: None,
            variant: "V".into(),
            fields: vec![("p".into(), Value::String("x".into()))],
        },
    )])
    .prepare()
    .expect("choice default schema prepares");
    choice_default_schema.limits.max_decoded_bytes = 8;
    let choice_default_error = decode_document("", &choice_default_schema)
        .expect_err("choice default payload copy must preserve its variant path");
    assert_eq!(choice_default_error.code, MonErrorCode::DecodedBudget);
    assert_eq!(
        choice_default_error.path,
        vec![
            PathSegment::Field("theme".into()),
            PathSegment::Variant("V".into()),
            PathSegment::Field("p".into()),
        ]
    );

    let optional_none = Schema::record(vec![Field::required(
        "value",
        SchemaType::Optional(Box::new(SchemaType::None)),
    )])
    .prepare()
    .expect_err("optional none schemas must be rejected");
    assert_eq!(optional_none.code, MonErrorCode::InvalidSchema);
}

#[test]
fn rejects_expressions_in_nested_literal_contexts() {
    let text_schema = schema(vec![Field::required("text", SchemaType::String)]);
    for (source, expected) in [
        ("text = external_name", MonErrorCode::UnexpectedToken),
        ("text = module.value", MonErrorCode::UnexpectedToken),
        ("text = $\"template\"", MonErrorCode::UnexpectedToken),
    ] {
        assert_eq!(
            decode_document(source, &text_schema).unwrap_err().code,
            expected,
            "source: {source}",
        );
    }

    let int_schema = schema(vec![Field::required("value", SchemaType::Int)]);
    for source in ["value = 1 + 2", "value = 1 as Int"] {
        assert_eq!(
            decode_document(source, &int_schema).unwrap_err().code,
            MonErrorCode::MissingComma,
            "source: {source}",
        );
    }

    let record_schema = schema(vec![Field::required(
        "record",
        SchemaType::Record {
            fields: vec![Field::required("value", SchemaType::Int)],
        },
    )]);
    let collection_schema = schema(vec![Field::required(
        "items",
        SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        },
    )]);
    let map_schema = schema(vec![Field::required(
        "lookup",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let choice_schema = schema(vec![Field::required(
        "theme",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![Variant::payload(
                "Payload",
                vec![Field::required("value", SchemaType::Int)],
            )],
        },
    )]);

    for (expression, expected) in [
        ("external_name", MonErrorCode::UnexpectedToken),
        ("constant_reference", MonErrorCode::UnexpectedToken),
        ("1 + 2", MonErrorCode::MissingComma),
        ("foldable_call()", MonErrorCode::TypeMismatch),
        ("1 as Int", MonErrorCode::MissingComma),
        ("$\"template\"", MonErrorCode::UnexpectedToken),
        ("module.value", MonErrorCode::UnexpectedToken),
    ] {
        for (context, schema, source) in [
            (
                "record",
                &record_schema,
                format!("record = (value = {expression})"),
            ),
            (
                "collection",
                &collection_schema,
                format!("items = {{0, {expression}}}"),
            ),
            ("map", &map_schema, format!("lookup = {{0 = {expression}}}")),
            (
                "choice payload",
                &choice_schema,
                format!("theme = ::Payload(value = {expression})"),
            ),
        ] {
            assert_eq!(
                decode_document(&source, schema).unwrap_err().code,
                expected,
                "{context} input: {source}",
            );
        }
    }
}

#[test]
fn duplicate_map_keys_compare_decoded_string_and_integer_values() {
    let string_schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let escaped_duplicate = decode_document(r#"values = {"x" = 1, "\u{78}" = 2}"#, &string_schema)
        .expect_err("different escapes decode to the same string key");
    assert_eq!(escaped_duplicate.code, MonErrorCode::DuplicateMapKey);

    let integer_schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let separated_duplicate = decode_document("values = {1000 = 1, 1_000 = 2}", &integer_schema)
        .expect_err("numeric separators do not change the decoded integer key");
    assert_eq!(separated_duplicate.code, MonErrorCode::DuplicateMapKey);
}

#[test]
fn choice_schema_rejections_cover_nominality_and_payload_routing() {
    let choice_schema = schema(vec![Field::required(
        "theme",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Ready"),
                Variant::payload("Text", vec![Field::required("value", SchemaType::String)]),
            ],
        },
    )]);
    for (source, expected) in [
        ("theme = Other::Ready", MonErrorCode::QualifierMismatch),
        ("theme = ::Missing", MonErrorCode::UnknownVariant),
        ("theme = ::Ready()", MonErrorCode::Arity),
        ("theme = ::Text", MonErrorCode::Arity),
        (
            "theme = ::Text(value = \"first\", \"second\")",
            MonErrorCode::ArgumentOrder,
        ),
        (
            "theme = ::Text(\"first\", value = \"second\")",
            MonErrorCode::DuplicateArgument,
        ),
    ] {
        assert_eq!(
            decode_document(source, &choice_schema).unwrap_err().code,
            expected,
            "source: {source}",
        );
    }
}

#[test]
fn wide_unique_map_decodes_in_insertion_order() {
    const ENTRY_COUNT: usize = 2_048;
    let schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Int),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let entries = (0..ENTRY_COUNT)
        .map(|value| format!("{value} = {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    let source = format!("values = {{{entries}}}");
    let decoded = decode_document(&source, &schema).expect("wide unique map decodes");
    let Value::Record(fields) = decoded else {
        panic!("document root is a record");
    };
    let Some((_, Value::Map(entries))) = fields.first() else {
        panic!("values field is a map");
    };
    assert_eq!(entries.len(), ENTRY_COUNT);
    assert_eq!(entries.first(), Some(&(Value::Int(0), Value::Int(0))));
    assert_eq!(
        entries.last(),
        Some(&(
            Value::Int((ENTRY_COUNT - 1) as i32),
            Value::Int((ENTRY_COUNT - 1) as i32),
        )),
    );
}
#[test]
fn rejects_trailing_roots_and_closed_record_violations() {
    let schema = schema(vec![Field::with_default(
        "setting",
        SchemaType::Int,
        Value::Int(0),
    )]);

    let trailing = decode_document("(setting = 1) (setting = 2)", &schema).unwrap_err();
    assert_eq!(trailing.code, MonErrorCode::TrailingInput);

    let unknown = decode_document("seting = 2", &schema).unwrap_err();
    assert_eq!(unknown.code, MonErrorCode::UnknownField);
    assert_eq!(unknown.path, vec![PathSegment::Field("seting".into())],);

    let duplicate = decode_document("setting = 1, setting = 2", &schema).unwrap_err();
    assert_eq!(duplicate.code, MonErrorCode::DuplicateField);
    assert_eq!(duplicate.span, Some(Span::new(13, 20)));
    assert_eq!(duplicate.path, vec![PathSegment::Field("setting".into())],);
}

#[test]
fn rejects_bare_names_as_map_keys() {
    let schema = schema(vec![Field::required(
        "scores",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let error = decode_document("scores = {player = 10}", &schema)
        .expect_err("bare names cannot become implicit map-key strings");
    assert_eq!(error.code, MonErrorCode::InvalidMapKey);
    assert_eq!(error.span, Some(Span::new(10, 16)));
    assert_eq!(
        error.path,
        vec![PathSegment::Field("scores".into()), PathSegment::Index(0),],
    );
}
