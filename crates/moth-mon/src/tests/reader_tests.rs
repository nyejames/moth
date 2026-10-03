use super::*;
use crate::{Field, Limits, Schema, SchemaType, Variant};
use moth_lexical::numeric::profile::NumericProfile;

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

    // Rejected reserved names are charged like accepted labels before their path is copied.
    let reserved_label_error = decode_document("__LoOp = 1", &label_schema)
        .expect_err("reserved label names must honor decoded-byte budget");
    assert_eq!(reserved_label_error.code, MonErrorCode::DecodedBudget);
    assert_eq!(reserved_label_error.span, Some(Span::new(0, 6)));
    assert_eq!(reserved_label_error.path, Vec::new());

    // Preparation needs 11 bytes for the schema names; the variant name alone exceeds that.
    let variant_schema = Schema::record(vec![Field::required(
        "v",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![Variant::unit("Ready")],
        },
    )])
    .with_limits(Limits {
        max_decoded_bytes: 11,
        ..Limits::default()
    })
    .prepare()
    .expect("variant budget schema prepares");
    let reserved_variant = "____________if";
    let reserved_variant_error =
        decode_document(&format!("v = Theme::{reserved_variant}"), &variant_schema)
            .expect_err("reserved variant names must honor decoded-byte budget");
    assert_eq!(reserved_variant_error.code, MonErrorCode::DecodedBudget);
    assert_eq!(
        reserved_variant_error.span,
        Some(Span::new(11, 11 + reserved_variant.len()))
    );

    // A rejected qualifier never enters the path, but its detail still copies the spelling.
    let qualifier_schema = Schema::record(vec![Field::required("v", SchemaType::Int)])
        .with_limits(Limits {
            max_decoded_bytes: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("qualifier budget schema prepares");
    let reserved_qualifier = "____________if";
    for source in [
        format!("v = {reserved_qualifier}(inner = 1)"),
        format!("v = {reserved_qualifier}::Ready"),
    ] {
        let error = decode_document(&source, &qualifier_schema)
            .expect_err("reserved qualifiers must honor decoded-byte budget");
        assert_eq!(error.code, MonErrorCode::DecodedBudget, "{source}");
        assert_eq!(
            error.span,
            Some(Span::new(4, 4 + reserved_qualifier.len())),
            "{source}"
        );
        assert_eq!(error.path, vec![PathSegment::Field("v".into())], "{source}");
    }
}
#[test]
fn preserves_float_sign_and_decimal_text_until_conversion() {
    let numeric_schema = schema(vec![
        Field::required("real", SchemaType::Float),
        Field::required("decimal", SchemaType::Decimal { scale: 2 }),
        Field::required("whole", SchemaType::Int),
    ]);
    let value = decode_document("real = -0.0, decimal = 1.2300, whole = 7", &numeric_schema)
        .expect("finite float and lossless decimal should decode");
    let Value::Record(fields) = value else {
        panic!("root schema produced a non-record");
    };
    let float = fields
        .iter()
        .find(|(name, _)| name == "real")
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
        decode_document("real = 1.0, decimal = 1.231, whole = 7", &numeric_schema,)
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
fn accepts_horizontal_whitespace_before_constructor_payloads_but_not_around_separator() {
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
    let nominal_comment = "size = Size -- payload\n(width = 1, height = 2)";
    let error = decode_document(nominal_comment, &struct_schema)
        .expect_err("a comment must not bridge a nominal constructor head to its payload");
    assert_eq!(error.code, MonErrorCode::UnexpectedToken);
    let nominal_start = nominal_comment
        .find("Size")
        .expect("nominal constructor head should exist");
    assert_eq!(
        error.span,
        Some(Span::new(nominal_start, nominal_start + "Size".len()))
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
    let unqualified_comment = "theme = ::Custom -- payload\n(name = \"Gold\")";
    let error = decode_document(unqualified_comment, &choice_schema)
        .expect_err("a comment must not bridge a choice variant head to its payload");
    assert_eq!(error.code, MonErrorCode::MissingComma);
    let opener = unqualified_comment
        .find('(')
        .expect("choice payload opener should exist");
    assert_eq!(error.span, Some(Span::new(opener, opener + 1)));
    // A slash cannot separate a variant from its payload.
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
    let qualified_comment = "theme = Theme::Custom -- payload\n(name = \"Gold\")";
    let error = decode_document(qualified_comment, &choice_schema)
        .expect_err("a comment must not bridge a qualified choice head to its payload");
    assert_eq!(error.code, MonErrorCode::MissingComma);
    let opener = qualified_comment
        .find('(')
        .expect("choice payload opener should exist");
    assert_eq!(error.span, Some(Span::new(opener, opener + 1)));
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
    let map_key = PreparedType::Int {
        width: IntWidth::Bits32,
    };
    let map_value = PreparedType::Int {
        width: IntWidth::Bits32,
    };
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
            Value::Int((ENTRY_COUNT - 1) as i64),
            Value::Int((ENTRY_COUNT - 1) as i64),
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

#[test]
fn decodes_fixed_width_integers_and_byte_with_their_own_ranges() {
    let fixed_schema = schema(vec![
        Field::required("width_i8", SchemaType::I8),
        Field::required("width_i64", SchemaType::I64),
        Field::required("width_u8", SchemaType::U8),
        Field::required("width_u64", SchemaType::U64),
        Field::required("octet", SchemaType::Byte),
    ]);
    assert_eq!(
        decode_document(
            "width_i8 = -128, width_i64 = -9223372036854775808, width_u8 = 1_7, \
             width_u64 = 18446744073709551615, octet = 255",
            &fixed_schema,
        ),
        Ok(Value::Record(vec![
            ("width_i8".into(), Value::I8(i8::MIN)),
            ("width_i64".into(), Value::I64(i64::MIN)),
            ("width_u8".into(), Value::U8(17)),
            ("width_u64".into(), Value::U64(u64::MAX)),
            ("octet".into(), Value::Byte(255)),
        ]))
    );

    // Whole-number spelling is strict: a decimal or exponent spelling is a category failure, not
    // a range failure, for every explicit-width integer and for Byte.
    let i8_schema = schema(vec![Field::required("value", SchemaType::I8)]);
    assert_eq!(
        decode_document("value = 128", &i8_schema).unwrap_err().code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        decode_document("value = 3.0", &i8_schema).unwrap_err().code,
        MonErrorCode::NumericType
    );
    assert_eq!(
        decode_document("value = 1e3", &i8_schema).unwrap_err().code,
        MonErrorCode::NumericType
    );

    // Unsigned widths reject every negative spelling, including `-0`, and check their own maximum.
    let u64_schema = schema(vec![Field::required("value", SchemaType::U64)]);
    assert_eq!(
        decode_document("value = -1", &u64_schema).unwrap_err().code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        decode_document("value = -0", &u64_schema).unwrap_err().code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        decode_document("value = 18446744073709551616", &u64_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );

    let byte_schema = schema(vec![Field::required("value", SchemaType::Byte)]);
    assert_eq!(
        decode_document("value = 0", &byte_schema),
        Ok(Value::Record(vec![("value".into(), Value::Byte(0))]))
    );
    assert_eq!(
        decode_document("value = 256", &byte_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );
    assert_eq!(
        decode_document("value = 2.5", &byte_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericType
    );
}

#[test]
fn fixed_binary_floats_round_once_and_keep_signed_zero() {
    let float_schema = schema(vec![
        Field::required("half", SchemaType::F16),
        Field::required("single", SchemaType::F32),
        Field::required("double", SchemaType::F64),
        Field::required("zero", SchemaType::F16),
        Field::required("whole", SchemaType::F32),
    ]);
    let value = decode_document(
        "half = 0.1, single = 0.1, double = 0.1, zero = -0.0, whole = 3",
        &float_schema,
    )
    .expect("finite fixed-float literals decode");
    assert_eq!(
        value,
        Value::Record(vec![
            ("half".into(), Value::F16(0.099_975_585_937_5)),
            ("single".into(), Value::F32(f64::from(0.1f32))),
            ("double".into(), Value::F64(0.1)),
            ("zero".into(), Value::F16(-0.0)),
            ("whole".into(), Value::F32(3.0)),
        ])
    );
    let Value::Record(fields) = value else {
        panic!("root schema produced a non-record");
    };
    let zero = fields
        .iter()
        .find(|(name, _)| name == "zero")
        .map(|(_, value)| value)
        .expect("zero field");
    assert!(
        matches!(zero, Value::F16(number) if number.to_bits() == (-0.0f64).to_bits()),
        "F16 negative zero keeps its sign bit"
    );

    // Each fixed binary float rounds past its own finite range as a conversion failure.
    let f16_schema = schema(vec![Field::required("value", SchemaType::F16)]);
    for source in ["value = 65520", "value = 70000", "value = 1.0e40"] {
        assert_eq!(
            decode_document(source, &f16_schema).unwrap_err().code,
            MonErrorCode::NonFiniteFloat,
            "source: {source}",
        );
    }
    let f32_schema = schema(vec![Field::required("value", SchemaType::F32)]);
    for source in ["value = 1.0e40", "value = -1.0e40"] {
        assert_eq!(
            decode_document(source, &f32_schema).unwrap_err().code,
            MonErrorCode::NonFiniteFloat,
            "source: {source}",
        );
    }
    assert_eq!(
        decode_document("value = \"0.1\"", &f32_schema)
            .unwrap_err()
            .code,
        MonErrorCode::TypeMismatch
    );
}

#[test]
fn numeric_profile_selects_int_range_and_float_rounding() {
    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };
    let int64 = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .with_profile(int64_profile)
        .prepare()
        .expect("Int64 profile schema prepares");
    assert_eq!(int64.profile(), int64_profile);
    assert_eq!(
        decode_document("value = 9223372036854775807", &int64),
        Ok(Value::Record(vec![("value".into(), Value::Int(i64::MAX),)]))
    );
    assert_eq!(
        decode_document("value = -9223372036854775808", &int64),
        Ok(Value::Record(vec![("value".into(), Value::Int(i64::MIN),)]))
    );
    assert_eq!(
        decode_document("value = 9223372036854775808", &int64)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );

    // The delivered default stays the Int32/Float64 boundary for standalone Rust consumers.
    let standard = schema(vec![Field::required("value", SchemaType::Int)]);
    assert_eq!(standard.profile(), NumericProfile::STANDARD);
    assert_eq!(
        decode_document("value = 2147483648", &standard)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );

    let float32_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let float32 = Schema::record(vec![Field::required("value", SchemaType::Float)])
        .with_profile(float32_profile)
        .prepare()
        .expect("Float32 profile schema prepares");
    assert_eq!(
        decode_document("value = 0.1", &float32),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Float(f64::from(0.1f32)),
        )]))
    );
    assert_eq!(
        decode_document("value = 1.0e40", &float32)
            .unwrap_err()
            .code,
        MonErrorCode::NonFiniteFloat
    );

    // Explicit-width members ignore the captured profile in both directions.
    let explicit_under_float32 = Schema::record(vec![
        Field::required("fixed", SchemaType::F64),
        Field::required("wide", SchemaType::I64),
    ])
    .with_profile(float32_profile)
    .prepare()
    .expect("explicit widths prepare under any profile");
    assert_eq!(
        decode_document(
            "fixed = 0.1, wide = 9223372036854775807",
            &explicit_under_float32
        ),
        Ok(Value::Record(vec![
            ("fixed".into(), Value::F64(0.1)),
            ("wide".into(), Value::I64(i64::MAX)),
        ]))
    );
}

#[test]
fn fixed_width_and_byte_map_keys_keep_distinct_families() {
    let byte_key_schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::Byte),
            value: Box::new(SchemaType::Int),
        },
    )]);
    assert_eq!(
        decode_document("values = {0 = 1, 255 = 2}", &byte_key_schema),
        Ok(Value::Record(vec![(
            "values".into(),
            Value::Map(vec![
                (Value::Byte(0), Value::Int(1)),
                (Value::Byte(255), Value::Int(2)),
            ]),
        )]))
    );
    let duplicate = decode_document("values = {7 = 1, 7 = 2}", &byte_key_schema)
        .expect_err("duplicate Byte keys fail");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        duplicate.path,
        vec![
            PathSegment::Field("values".into()),
            PathSegment::MapKey("7".into()),
        ]
    );
    assert_eq!(
        decode_document("values = {256 = 1}", &byte_key_schema)
            .unwrap_err()
            .code,
        MonErrorCode::NumericRange
    );

    let u64_key_schema = schema(vec![Field::required(
        "values",
        SchemaType::Map {
            key: Box::new(SchemaType::U64),
            value: Box::new(SchemaType::Byte),
        },
    )]);
    assert_eq!(
        decode_document("values = {18446744073709551615 = 255}", &u64_key_schema),
        Ok(Value::Record(vec![(
            "values".into(),
            Value::Map(vec![(Value::U64(u64::MAX), Value::Byte(255))]),
        )]))
    );
    let repeated = decode_document(
        "values = {18446744073709551615 = 1, 18446744073709551615 = 2}",
        &u64_key_schema,
    )
    .expect_err("the same U64 key is a duplicate");
    assert_eq!(repeated.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(
        repeated.path.last(),
        Some(&PathSegment::MapKey(u64::MAX.to_string()))
    );
}

#[test]
fn homogeneous_map_keys_reject_mismatched_decoded_literals() {
    for (key_type, source) in [
        (SchemaType::String, "values = {1 = 10}"),
        (SchemaType::Bool, "values = {1 = 10}"),
        (SchemaType::Char, r#"values = {"a" = 10}"#),
        (SchemaType::Int, "values = {true = 10}"),
    ] {
        let schema = schema(vec![Field::required(
            "values",
            SchemaType::Map {
                key: Box::new(key_type.clone()),
                value: Box::new(SchemaType::Int),
            },
        )]);
        let error = decode_document(source, &schema)
            .expect_err("literal must match the homogeneous key family");
        assert_eq!(error.code, MonErrorCode::TypeMismatch, "{key_type:?}");
        assert_eq!(
            error.path,
            vec![PathSegment::Field("values".into()), PathSegment::Index(0)],
        );
    }
}

#[test]
fn decimal_scale_capacity_reaches_two_hundred_and_fifty_six() {
    let wide = Schema::record(vec![Field::required(
        "value",
        SchemaType::Decimal { scale: 256 },
    )])
    .prepare()
    .expect("the widened scale capacity prepares");
    assert_eq!(
        decode_document("value = 1e-256", &wide),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Decimal("1e-256".into()),
        )]))
    );
    assert_eq!(
        decode_document("value = 1e-257", &wide).unwrap_err().code,
        MonErrorCode::NumericScale
    );
    assert_eq!(
        decode_document("value = 1.000000000000000000001", &wide),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Decimal("1.000000000000000000001".into()),
        )]))
    );

    // A scale inside the former 18-digit ceiling keeps decoding unchanged.
    let nineteen = schema(vec![Field::required(
        "value",
        SchemaType::Decimal { scale: 19 },
    )]);
    assert_eq!(
        decode_document("value = 1e-19", &nineteen),
        Ok(Value::Record(vec![(
            "value".into(),
            Value::Decimal("1e-19".into()),
        )]))
    );
    assert_eq!(
        decode_document("value = 1e-20", &nineteen)
            .unwrap_err()
            .code,
        MonErrorCode::NumericScale
    );
}

#[test]
fn explicit_width_numerics_keep_the_existing_budgets() {
    let digit_limited = Schema::record(vec![Field::required("value", SchemaType::U64)])
        .with_limits(Limits {
            max_numeric_digits: 2,
            ..Limits::default()
        })
        .prepare()
        .expect("digit budget schema prepares");
    assert_eq!(
        decode_document("value = 18446744073709551615", &digit_limited)
            .unwrap_err()
            .code,
        MonErrorCode::NumericBudget
    );

    // The field label costs one byte at preparation; the literal's retained magnitude scratch is
    // then charged before it is reserved, exactly as it is for the profile numerics.
    let byte_limited = Schema::record(vec![Field::required("v", SchemaType::F16)])
        .with_limits(Limits {
            max_decoded_bytes: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("decoded budget schema prepares");
    assert_eq!(
        decode_document("v = 0.1", &byte_limited).unwrap_err().code,
        MonErrorCode::DecodedBudget
    );

    let key_limited = Schema::record(vec![Field::required(
        "v",
        SchemaType::Map {
            key: Box::new(SchemaType::U64),
            value: Box::new(SchemaType::Byte),
        },
    )])
    .with_limits(Limits {
        max_decoded_bytes: 2,
        ..Limits::default()
    })
    .prepare()
    .expect("key budget schema prepares");
    assert_eq!(
        decode_document("v = {18446744073709551615 = 1}", &key_limited)
            .unwrap_err()
            .code,
        MonErrorCode::DecodedBudget
    );
}
fn reserved_name_error(source: &str, name: &str, path: Vec<PathSegment>, schema: &PreparedSchema) {
    let error = decode_document(source, schema).expect_err("reserved user name must fail");
    let start = source
        .find(name)
        .expect("the expected name should occur in the source");
    assert_eq!(error.code, MonErrorCode::InvalidIdentifier, "{source}");
    assert_eq!(
        error.span,
        Some(Span::new(start, start + name.len())),
        "{source}"
    );
    assert_eq!(error.path, path, "{source}");
}

#[test]
fn rejects_reserved_labels_qualifiers_and_variants_but_keeps_literal_data() {
    let scalar_schema = schema(vec![Field::required("value", SchemaType::Int)]);
    let reserved_names = [
        "if", "__LoOp", "_U64", "dec01", "dec257", "true", "false", "none",
    ];

    for name in reserved_names {
        let source = format!("{name} = 1");
        reserved_name_error(
            &source,
            name,
            vec![PathSegment::Field(name.to_owned())],
            &scalar_schema,
        );
    }

    let struct_schema = schema(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    for name in reserved_names {
        let source = format!("value = {name}(inner = 1)");
        reserved_name_error(
            &source,
            name,
            vec![PathSegment::Field("value".into())],
            &struct_schema,
        );
    }

    let choice_schema = schema(vec![Field::required(
        "value",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![Variant::unit("Ready")],
        },
    )]);
    for name in reserved_names {
        let source = format!("value = {name}::Ready");
        reserved_name_error(
            &source,
            name,
            vec![PathSegment::Field("value".into())],
            &choice_schema,
        );

        let source = format!("value = Theme::{name}");
        reserved_name_error(
            &source,
            name,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Variant(name.to_owned()),
            ],
            &choice_schema,
        );
    }

    let data_schema = schema(vec![
        Field::required("text", SchemaType::String),
        Field::required(
            "lookup",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        ),
    ]);
    assert_eq!(
        decode_document(
            r#"text = "if", lookup = {"true" = 1, "if" = 2}"#,
            &data_schema
        ),
        Ok(Value::Record(vec![
            ("text".into(), Value::String("if".into())),
            (
                "lookup".into(),
                Value::Map(vec![
                    (Value::String("true".into()), Value::Int(1)),
                    (Value::String("if".into()), Value::Int(2)),
                ]),
            ),
        ]))
    );

    let ordinary_schema = schema(vec![
        Field::required("DecBox", SchemaType::Int),
        Field::required("configuration", SchemaType::Int),
        Field::required("_configuration", SchemaType::Int),
    ]);
    assert_eq!(
        decode_document(
            "DecBox = 1, configuration = 2, _configuration = 3",
            &ordinary_schema
        ),
        Ok(Value::Record(vec![
            ("DecBox".into(), Value::Int(1)),
            ("configuration".into(), Value::Int(2)),
            ("_configuration".into(), Value::Int(3)),
        ]))
    );
}

#[test]
fn named_entries_reject_a_line_break_before_equals_and_accept_other_layout() {
    let fields_schema = schema(vec![
        Field::required("first", SchemaType::Int),
        Field::required("later", SchemaType::Int),
    ]);
    let first_schema = schema(vec![Field::required("first", SchemaType::Int)]);

    for source in [
        "first\n = 1",
        "(first\n = 1)",
        "(first -- label comment\r\n = 1)",
    ] {
        let start = source.find("first").expect("first label should exist");
        let error =
            decode_document(source, &first_schema).expect_err("label and '=' must share a line");
        assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{source:?}");
        assert_eq!(
            error.span,
            Some(Span::new(start, start + "first".len())),
            "{source:?}"
        );
        assert!(error.path.is_empty(), "{source:?}");
    }

    let nested_schema = schema(vec![Field::required(
        "outer",
        SchemaType::Record {
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let nested_source = "outer = (inner\n = 1)";
    let nested_error =
        decode_document(nested_source, &nested_schema).expect_err("nested label break must fail");
    let inner_start = nested_source
        .find("inner")
        .expect("nested label should exist");
    assert_eq!(nested_error.code, MonErrorCode::UnexpectedToken);
    assert_eq!(
        nested_error.span,
        Some(Span::new(inner_start, inner_start + "inner".len()))
    );
    assert_eq!(nested_error.path, vec![PathSegment::Field("outer".into())]);

    let payload_schema = schema(vec![Field::required(
        "box",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let payload_source = "box = Box(inner\r\n = 1)";
    let payload_error = decode_document(payload_source, &payload_schema)
        .expect_err("nominal payload label break must fail");
    let inner_start = payload_source
        .find("inner")
        .expect("payload label should exist");
    assert_eq!(payload_error.code, MonErrorCode::UnexpectedToken);
    assert_eq!(
        payload_error.span,
        Some(Span::new(inner_start, inner_start + "inner".len()))
    );
    assert_eq!(payload_error.path, vec![PathSegment::Field("box".into())]);

    let source = "first \t= \n1,\n-- between fields\nlater = 2,\n";
    assert_eq!(
        decode_document(source, &fields_schema),
        Ok(Value::Record(vec![
            ("first".into(), Value::Int(1)),
            ("later".into(), Value::Int(2)),
        ]))
    );
    let source = "(\nfirst = 1,\nlater \t= \n2,\n)";
    assert_eq!(
        decode_document(source, &fields_schema),
        Ok(Value::Record(vec![
            ("first".into(), Value::Int(1)),
            ("later".into(), Value::Int(2)),
        ]))
    );
}

#[test]
fn named_entries_accept_all_horizontal_whitespace_before_equals() {
    let first_schema = schema(vec![Field::required("first", SchemaType::Int)]);

    for whitespace in [
        "\t", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let source = format!("first{whitespace}= 1");
        assert_eq!(
            decode_document(&source, &first_schema),
            Ok(Value::Record(vec![("first".into(), Value::Int(1))])),
            "{source:?}"
        );
    }
}

#[test]
fn equals_spacing_errors_report_missing_sides_at_equals() {
    let named_schema = schema(vec![Field::required("first", SchemaType::Int)]);
    let named_cases = [
        ("first= 1", "before"),
        ("first =1", "after"),
        ("first=1", "before and after"),
    ];
    for (source, missing_sides) in named_cases {
        let error = decode_document(source, &named_schema)
            .expect_err("named entry spacing must match the shared syntax rule");
        let equals = source.find('=').expect("the entry has an equals sign");
        assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{source:?}");
        assert_eq!(
            error.span,
            Some(Span::new(equals, equals + 1)),
            "{source:?}"
        );
        assert_eq!(
            error.detail,
            format!(
                "MON named entry '=' requires whitespace on both sides; missing whitespace {missing_sides} '='"
            ),
            "{source:?}"
        );
        assert_eq!(
            error.path,
            vec![PathSegment::Field("first".into())],
            "{source:?}"
        );
    }

    let map_schema = schema(vec![Field::required(
        "items",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);
    let map_cases = [
        (
            "items = {\"a\"= 1}",
            1,
            "before",
            vec![PathSegment::Field("items".into())],
        ),
        (
            "items = {\"a\" =1}",
            1,
            "after",
            vec![PathSegment::Field("items".into())],
        ),
        (
            "items = {\"a\" = 1, \"b\"=2}",
            2,
            "before and after",
            vec![PathSegment::Field("items".into()), PathSegment::Index(1)],
        ),
    ];
    for (source, equals_occurrence, missing_sides, expected_path) in map_cases {
        let error = decode_document(source, &map_schema)
            .expect_err("map entry spacing must match the shared syntax rule");
        let equals = source
            .match_indices('=')
            .nth(equals_occurrence)
            .map(|(start, _)| start)
            .expect("the expected map entry equals sign exists");
        assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{source:?}");
        assert_eq!(
            error.span,
            Some(Span::new(equals, equals + 1)),
            "{source:?}"
        );
        assert_eq!(
            error.detail,
            format!(
                "MON map entry '=' requires whitespace on both sides; missing whitespace {missing_sides} '='"
            ),
            "{source:?}"
        );
        assert_eq!(error.path, expected_path, "{source:?}");
    }
}

#[test]
fn constructor_payload_heads_only_accept_horizontal_whitespace() {
    let nominal_schema = schema(vec![Field::required(
        "box",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let expected_nominal = Value::Record(vec![(
        "box".into(),
        Value::Record(vec![("inner".into(), Value::Int(1))]),
    )]);

    for whitespace in [
        " ", "\t", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let source = format!("box = Box{whitespace}(inner = 1)");
        assert_eq!(
            decode_document(&source, &nominal_schema),
            Ok(expected_nominal.clone()),
            "{source:?}"
        );
    }

    for separator in ["\n", "\r", "\r\n", " -- c\n"] {
        let source = format!("box = Box{separator}(inner = 1)");
        let error = decode_document(&source, &nominal_schema)
            .expect_err("a line break or comment must separate the constructor head");
        assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{source:?}");
        let start = source.find("Box").expect("nominal head should exist");
        assert_eq!(
            error.span,
            Some(Span::new(start, start + "Box".len())),
            "{source:?}"
        );
    }

    let choice_schema = schema(vec![Field::required(
        "status",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Ready"),
                Variant::payload("Pair", vec![Field::required("inner", SchemaType::Int)]),
            ],
        },
    )]);
    let expected_choice = Value::Record(vec![(
        "status".into(),
        Value::Choice {
            qualifier: Some("Theme".into()),
            variant: "Pair".into(),
            fields: vec![("inner".into(), Value::Int(1))],
        },
    )]);

    for whitespace in [
        " ", "\t", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let source = format!("status = Theme::Pair{whitespace}(inner = 1)");
        assert_eq!(
            decode_document(&source, &choice_schema),
            Ok(expected_choice.clone()),
            "{source:?}"
        );
    }

    for separator in ["\n", "\r", "\r\n", " -- c\n"] {
        let source = format!("status = Theme::Pair{separator}(inner = 1)");
        let error = decode_document(&source, &choice_schema)
            .expect_err("a line break or comment must separate the choice head");
        assert_eq!(error.code, MonErrorCode::MissingComma, "{source:?}");
        let start = source.find('(').expect("payload opener should exist");
        assert_eq!(error.span, Some(Span::new(start, start + 1)), "{source:?}");
    }

    assert_eq!(
        decode_document("box = Box(\ninner =\n1\n)", &nominal_schema),
        Ok(expected_nominal)
    );
    assert_eq!(
        decode_document("status = Theme::Pair(\ninner =\n1\n)", &choice_schema),
        Ok(expected_choice)
    );
}

#[test]
fn named_entries_reject_lf_cr_and_crlf_before_equals() {
    let first_schema = schema(vec![Field::required("first", SchemaType::Int)]);

    for line_break in ["\n", "\r", "\r\n"] {
        let source = format!("first{line_break} = 1");
        let error =
            decode_document(&source, &first_schema).expect_err("label and equals share a line");
        assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{source:?}");
        let start = source.find("first").expect("label should exist");
        assert_eq!(
            error.span,
            Some(Span::new(start, start + "first".len())),
            "{source:?}"
        );
    }
}

#[test]
fn literal_and_constructor_error_spans_end_at_their_syntax() {
    let integer_schema = schema(vec![Field::required("value", SchemaType::Int)]);
    for (separator, suffix) in [
        (" ", ","),
        ("\t", ","),
        ("\u{00a0}", ","),
        (" ", ")"),
        ("\t", ")"),
        ("\u{00a0}", ")"),
    ] {
        let source = if suffix == "," {
            format!("value = Theme::Ready{separator},")
        } else {
            format!("(value = Theme::Ready{separator})")
        };
        let error =
            decode_document(&source, &integer_schema).expect_err("choice cannot inhabit Int");
        assert_eq!(error.code, MonErrorCode::TypeMismatch, "{source:?}");
        assert_eq!(
            error.span,
            Some(Span::new(
                source.find("Theme::Ready").unwrap(),
                source.find("Theme::Ready").unwrap() + "Theme::Ready".len(),
            )),
            "{source:?}"
        );
    }

    for source in [
        "value = Theme::Pair (inner = 1),",
        "value = Box (inner = 1),",
    ] {
        let error =
            decode_document(source, &integer_schema).expect_err("constructor cannot inhabit Int");
        assert_eq!(error.code, MonErrorCode::TypeMismatch, "{source:?}");
        let start = source
            .find("Theme::Pair")
            .or_else(|| source.find("Box"))
            .unwrap();
        let end = source.find(')').unwrap() + 1;
        assert_eq!(error.span, Some(Span::new(start, end)), "{source:?}");
    }
}

#[test]
fn boolean_and_none_keywords_only_start_literals_without_same_line_payloads() {
    for (keyword, value_schema) in [
        ("none", SchemaType::Optional(Box::new(SchemaType::String))),
        ("true", SchemaType::Bool),
        ("false", SchemaType::Bool),
    ] {
        let value_schema = schema(vec![Field::required("value", value_schema)]);
        for (separator, expected_code, expected_token) in [
            ("", MonErrorCode::InvalidIdentifier, keyword),
            ("\n", MonErrorCode::MissingComma, "("),
        ] {
            let source = format!("value = {keyword}{separator}()");
            let error = decode_document(&source, &value_schema)
                .expect_err("a keyword followed by parentheses is not a literal value");
            assert_eq!(error.code, expected_code, "{source:?}");
            let start = source.find(expected_token).unwrap();
            assert_eq!(
                error.span,
                Some(Span::new(start, start + expected_token.len())),
                "{source:?}"
            );
        }
    }
}
