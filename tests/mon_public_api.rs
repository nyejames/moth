//! WHAT: outside-crate consumer and hardening coverage for the public `moth::mon` API.
//! WHY: proves another crate can round-trip saves with only public imports and
//!      bounded budgets, without compiler internals or engine dependencies.

use moth::mon::{
    Field, Limits, MonErrorCode, PathSegment, PreparedSchema, Schema, SchemaType, Value, Variant,
    decode_document, decode_document_bytes, encode_document, encode_value,
};

#[derive(Debug, PartialEq, Eq)]
struct Window {
    width: i32,
    height: i32,
}

#[derive(Debug, PartialEq, Eq)]
enum Theme {
    Light,
    Custom(String),
}

#[derive(Debug, PartialEq, Eq)]
struct Save {
    format_version: i32,
    title: String,
    window: Window,
    scores: Vec<(String, i32)>,
    theme: Theme,
    note: Option<String>,
    exact_total: String,
}

fn save_schema() -> PreparedSchema {
    Schema::record(vec![
        Field::required("format_version", SchemaType::Int),
        Field::required("title", SchemaType::String),
        Field::with_default(
            "window",
            SchemaType::Struct {
                name: "Window".to_owned(),
                fields: vec![
                    Field::required("width", SchemaType::Int),
                    Field::with_default("height", SchemaType::Int, Value::Int(720)),
                ],
            },
            Value::Record(vec![("width".to_owned(), Value::Int(1280))]),
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
                name: "Theme".to_owned(),
                variants: vec![
                    Variant::unit("Light"),
                    Variant::payload("Custom", vec![Field::required("name", SchemaType::String)]),
                ],
            },
        ),
        Field::required("note", SchemaType::Optional(Box::new(SchemaType::String))),
        Field::required("exact_total", SchemaType::Integer),
    ])
    .prepare()
    .expect("static save schema is valid")
}

fn native_save(value: Value) -> Save {
    let Value::Record(fields) = value else {
        panic!("root is a record");
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|(field_name, _)| field_name == name)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("completed schema result has {name}"))
    };
    let Value::Int(format_version) = field("format_version") else {
        panic!("format_version is Int");
    };
    let Value::String(title) = field("title") else {
        panic!("title is String");
    };
    let Value::Record(window_fields) = field("window") else {
        panic!("window is a record");
    };
    let window_field = |name: &str| {
        window_fields
            .iter()
            .find(|(field_name, _)| field_name == name)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("completed window has {name}"))
    };
    let Value::Int(width) = window_field("width") else {
        panic!("window.width is Int");
    };
    let Value::Int(height) = window_field("height") else {
        panic!("window.height is Int");
    };
    let Value::Map(entries) = field("scores") else {
        panic!("scores is a map");
    };
    let scores = entries
        .iter()
        .map(|(key, value)| {
            let Value::String(key) = key else {
                panic!("score key is String");
            };
            let Value::Int(value) = value else {
                panic!("score value is Int");
            };
            (key.clone(), *value)
        })
        .collect();
    let Value::Choice {
        variant, fields, ..
    } = field("theme")
    else {
        panic!("theme is a choice");
    };
    let theme = match variant.as_str() {
        "Light" => Theme::Light,
        "Custom" => {
            let Some((_, Value::String(name))) =
                fields.iter().find(|(field_name, _)| field_name == "name")
            else {
                panic!("custom theme has name");
            };
            Theme::Custom(name.clone())
        }
        other => panic!("unexpected theme variant {other}"),
    };
    let note = match field("note") {
        Value::None => None,
        Value::String(value) => Some(value.clone()),
        _ => panic!("note is optional String"),
    };
    let Value::Integer(exact_total) = field("exact_total") else {
        panic!("exact_total is Integer");
    };
    Save {
        format_version: *format_version,
        title: title.clone(),
        window: Window {
            width: *width,
            height: *height,
        },
        scores,
        theme,
        note,
        exact_total: exact_total.clone(),
    }
}

#[test]
fn consumer_save_round_trip_drops_source_and_converts_to_native() {
    let schema = save_schema();
    let document = Value::Record(vec![
        ("format_version".into(), Value::Int(1)),
        ("title".into(), Value::String("Strategy".into())),
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
                qualifier: Some("Theme".into()),
                variant: "Custom".into(),
                fields: vec![("name".into(), Value::String("Gold".into()))],
            },
        ),
        ("note".into(), Value::None),
        (
            "exact_total".into(),
            Value::Integer("90071992547409931234567890".into()),
        ),
    ]);
    let encoded = encode_document(&document, &schema).expect("save encodes and fills defaults");
    assert!(
        encoded.contains("height") && encoded.contains("720"),
        "encoding emits completed defaults"
    );

    let window_schema = Schema::value(SchemaType::Struct {
        name: "Window".into(),
        fields: vec![
            Field::required("width", SchemaType::Int),
            Field::with_default("height", SchemaType::Int, Value::Int(720)),
        ],
    })
    .prepare()
    .expect("window schema is valid");
    let fragment = encode_value(
        &Value::Record(vec![("width".into(), Value::Int(1280))]),
        &window_schema,
    )
    .expect("nested values use the same writer");
    assert_eq!(fragment, "(width = 1280, height = 720)");

    let shuffled = Value::Record(vec![
        (
            "exact_total".into(),
            Value::Integer("90071992547409931234567890".into()),
        ),
        ("note".into(), Value::None),
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
                (Value::String("Priya".into()), Value::Int(10)),
                (Value::String("Rob".into()), Value::Int(12)),
            ]),
        ),
        ("title".into(), Value::String("Strategy".into())),
        ("format_version".into(), Value::Int(1)),
    ]);
    let re_encoded =
        encode_document(&shuffled, &schema).expect("field order does not change output");
    assert_eq!(encoded, re_encoded);

    let from_str = decode_document(&encoded, &schema).expect("save decodes");
    let from_bytes =
        decode_document_bytes(encoded.as_bytes(), &schema).expect("bytes decode agrees");
    assert_eq!(from_str, from_bytes);
    drop(encoded);
    let save = native_save(from_str);
    assert_eq!(save.format_version, 1);
    assert_eq!(save.title, "Strategy");
    assert_eq!(
        save.window,
        Window {
            width: 1280,
            height: 720
        }
    );
    assert_eq!(
        save.scores,
        vec![("Priya".to_owned(), 10), ("Rob".to_owned(), 12)]
    );
    assert_eq!(save.theme, Theme::Custom("Gold".to_owned()));
    assert_eq!(save.note, None);
    assert_eq!(save.exact_total, "90071992547409931234567890");
}

#[test]
fn prepared_schema_is_reusable_across_decodes() {
    let schema = save_schema();
    let first = decode_document(
        r#"format_version = 1, title = "A", window = (width = 10, height = 20), scores = {=}, theme = ::Light, note = none, exact_total = 1"#,
        &schema,
    )
    .expect("first decode succeeds");
    let second = decode_document(
        r#"format_version = 2, title = "B", scores = {"K" = 3}, theme = ::Custom(name = "X"), note = "hi", exact_total = 42"#,
        &schema,
    )
    .expect("second decode reuses the prepared schema");
    let again = decode_document(
        r#"format_version = 1, title = "A", window = (width = 10, height = 20), scores = {=}, theme = ::Light, note = none, exact_total = 1"#,
        &schema,
    )
    .expect("schema remains reusable after success");
    assert_eq!(first, again);
    assert_ne!(first, second);

    let first_save = native_save(first);
    let second_save = native_save(second);
    assert_eq!(first_save.title, "A");
    assert_eq!(second_save.title, "B");
    assert_eq!(
        first_save.window,
        Window {
            width: 10,
            height: 20
        }
    );
    assert_eq!(
        second_save.window,
        Window {
            width: 1280,
            height: 720
        }
    );
}
#[test]
fn record_name_lookup_preserves_unicode_order_and_diagnostics() {
    let schema = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::required("café", SchemaType::Int),
        Field::required("alpha", SchemaType::Int),
        Field::required(
            "panel",
            SchemaType::Struct {
                name: "Panel".into(),
                fields: vec![
                    Field::required("left", SchemaType::Int),
                    Field::required("right", SchemaType::Int),
                ],
            },
        ),
    ])
    .prepare()
    .expect("record schema prepares");
    let expected = Value::Record(vec![
        ("zeta".into(), Value::Int(1)),
        ("café".into(), Value::Int(2)),
        ("alpha".into(), Value::Int(3)),
        (
            "panel".into(),
            Value::Record(vec![
                ("left".into(), Value::Int(4)),
                ("right".into(), Value::Int(5)),
            ]),
        ),
    ]);

    let decoded = decode_document(
        "panel = Panel(4, right = 5), alpha = 3, café = 2, zeta = 1",
        &schema,
    )
    .expect("permuted Unicode names and positional-before-named values route");
    assert_eq!(decoded, expected);

    let programmatic = Value::Record(vec![
        (
            "panel".into(),
            Value::Record(vec![
                ("right".into(), Value::Int(5)),
                ("left".into(), Value::Int(4)),
            ]),
        ),
        ("alpha".into(), Value::Int(3)),
        ("café".into(), Value::Int(2)),
        ("zeta".into(), Value::Int(1)),
    ]);
    let encoded = encode_document(&programmatic, &schema).expect("permuted value encodes");
    let output_positions: Vec<_> = ["zeta", "café", "alpha", "panel", "left", "right"]
        .map(|name| {
            encoded
                .find(name)
                .expect("encoded output contains each field")
        })
        .into();
    assert!(
        output_positions.windows(2).all(|pair| pair[0] < pair[1]),
        "record and nested fields stay in schema order: {encoded}",
    );
    assert_eq!(
        decode_document(&encoded, &schema).expect("encoded document decodes"),
        expected,
    );

    let unknown = decode_document("mystery = 9", &schema).expect_err("unknown fields stay closed");
    assert_eq!(unknown.code, MonErrorCode::UnknownField);
    assert_eq!(unknown.path, vec![PathSegment::Field("mystery".into())]);

    let unknown_programmatic = Value::Record(vec![("mystery".into(), Value::Int(9))]);
    let unknown_programmatic_error = encode_document(&unknown_programmatic, &schema)
        .expect_err("programmatic values reject unknown fields");
    assert_eq!(unknown_programmatic_error.code, MonErrorCode::UnknownField);
    assert_eq!(
        unknown_programmatic_error.path,
        vec![PathSegment::Field("mystery".into())],
    );

    let duplicate = decode_document("zeta = 1, zeta = 2", &schema)
        .expect_err("a record slot cannot be supplied more than once");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateField);
    assert_eq!(duplicate.path, vec![PathSegment::Field("zeta".into())]);

    let duplicate_declaration = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::required("alpha", SchemaType::Int),
        Field::required("zeta", SchemaType::Int),
        Field::required("alpha", SchemaType::Int),
    ])
    .prepare()
    .expect_err("the earliest duplicate declaration remains the diagnostic");
    assert_eq!(duplicate_declaration.code, MonErrorCode::DuplicateField);
    assert_eq!(
        duplicate_declaration.path,
        vec![PathSegment::Field("zeta".into())],
    );

    let overlap = decode_document("panel = Panel(1, left = 2, right = 3)", &schema)
        .expect_err("named fields cannot repeat a positional slot");
    assert_eq!(overlap.code, MonErrorCode::DuplicateField);
    assert_eq!(
        overlap.path,
        vec![
            PathSegment::Field("panel".into()),
            PathSegment::Field("left".into()),
        ],
    );

    let late_positional = decode_document("panel = Panel(left = 1, 2)", &schema)
        .expect_err("positional arguments still precede named arguments");
    assert_eq!(late_positional.code, MonErrorCode::ArgumentOrder);
}

#[test]
fn choice_name_lookup_preserves_unicode_order_and_diagnostics() {
    let schema = Schema::record(vec![Field::required(
        "choice",
        SchemaType::Choice {
            name: "Palette".into(),
            variants: vec![
                Variant::payload(
                    "Zebra",
                    vec![
                        Field::required("zeta", SchemaType::Int),
                        Field::required("café", SchemaType::Int),
                        Field::required("alpha", SchemaType::Int),
                    ],
                ),
                Variant::unit("Amber"),
                Variant::unit("Café"),
            ],
        },
    )])
    .prepare()
    .expect("choice schema prepares");
    let expected = Value::Record(vec![(
        "choice".into(),
        Value::Choice {
            qualifier: None,
            variant: "Zebra".into(),
            fields: vec![
                ("zeta".into(), Value::Int(1)),
                ("café".into(), Value::Int(2)),
                ("alpha".into(), Value::Int(3)),
            ],
        },
    )]);

    let decoded = decode_document("choice = ::Zebra(alpha = 3, zeta = 1, café = 2)", &schema)
        .expect("permuted Unicode payload names route");
    assert_eq!(decoded, expected);
    assert_eq!(
        decode_document("choice = ::Café", &schema).expect("Unicode variant name routes"),
        Value::Record(vec![(
            "choice".into(),
            Value::Choice {
                qualifier: None,
                variant: "Café".into(),
                fields: vec![],
            },
        )]),
    );

    let programmatic = Value::Record(vec![(
        "choice".into(),
        Value::Choice {
            qualifier: None,
            variant: "Zebra".into(),
            fields: vec![
                ("alpha".into(), Value::Int(3)),
                ("café".into(), Value::Int(2)),
                ("zeta".into(), Value::Int(1)),
            ],
        },
    )]);
    let encoded = encode_document(&programmatic, &schema).expect("permuted payload encodes");
    let payload_positions: Vec<_> = ["zeta", "café", "alpha"]
        .map(|name| {
            encoded
                .find(name)
                .expect("encoded payload contains each field")
        })
        .into();
    assert!(
        payload_positions.windows(2).all(|pair| pair[0] < pair[1]),
        "choice payload fields stay in schema order: {encoded}",
    );
    assert_eq!(
        decode_document(&encoded, &schema).expect("encoded choice decodes"),
        expected,
    );

    let unknown_variant =
        decode_document("choice = ::Missing", &schema).expect_err("unknown variants stay closed");
    assert_eq!(unknown_variant.code, MonErrorCode::UnknownVariant);
    assert_eq!(
        unknown_variant.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Missing".into()),
        ],
    );

    let unknown_field = decode_document("choice = ::Zebra(mystery = 9)", &schema)
        .expect_err("unknown payload names stay closed");
    assert_eq!(unknown_field.code, MonErrorCode::UnknownArgument);
    assert_eq!(
        unknown_field.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Zebra".into()),
            PathSegment::Field("mystery".into()),
        ],
    );

    let duplicate = decode_document("choice = ::Zebra(zeta = 1, zeta = 2)", &schema)
        .expect_err("a payload slot cannot be supplied twice");
    assert_eq!(duplicate.code, MonErrorCode::DuplicateArgument);
    assert_eq!(
        duplicate.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Zebra".into()),
            PathSegment::Field("zeta".into()),
        ],
    );

    let overlap = decode_document(
        "choice = ::Zebra(1, zeta = 2, café = 3, alpha = 4)",
        &schema,
    )
    .expect_err("named arguments cannot repeat a positional payload slot");
    assert_eq!(overlap.code, MonErrorCode::DuplicateArgument);
    assert_eq!(overlap.path, duplicate.path);

    let programmatic_duplicate = Value::Record(vec![(
        "choice".into(),
        Value::Choice {
            qualifier: None,
            variant: "Zebra".into(),
            fields: vec![
                ("zeta".into(), Value::Int(1)),
                ("zeta".into(), Value::Int(2)),
            ],
        },
    )]);
    let programmatic_duplicate_error = encode_document(&programmatic_duplicate, &schema)
        .expect_err("duplicate payload fields fail");
    assert_eq!(
        programmatic_duplicate_error.code,
        MonErrorCode::DuplicateArgument,
    );
    assert_eq!(programmatic_duplicate_error.path, duplicate.path);

    let late_positional = decode_document("choice = ::Zebra(zeta = 1, 2)", &schema)
        .expect_err("choice positional arguments still precede named arguments");
    assert_eq!(late_positional.code, MonErrorCode::ArgumentOrder);

    let duplicate_variant = Schema::record(vec![Field::required(
        "choice",
        SchemaType::Choice {
            name: "Palette".into(),
            variants: vec![
                Variant::unit("Zebra"),
                Variant::unit("Amber"),
                Variant::unit("Zebra"),
                Variant::unit("Amber"),
            ],
        },
    )])
    .prepare()
    .expect_err("the earliest duplicate variant remains the diagnostic");
    assert_eq!(duplicate_variant.code, MonErrorCode::InvalidSchema);
    assert_eq!(
        duplicate_variant.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Zebra".into()),
        ],
    );
}

#[test]
fn duplicate_declarations_keep_the_earliest_error() {
    // Duplicate detection indexes the declarations before the walk, so it must not report a
    // repeat that an earlier declaration's own error already outranks, and the earliest
    // declaration-order repeat must stay the diagnostic.
    let field_error_first = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::required("9bad", SchemaType::Int),
        Field::required("zeta", SchemaType::Int),
    ])
    .prepare()
    .expect_err("the earlier invalid field name remains the diagnostic");
    assert_eq!(field_error_first.code, MonErrorCode::InvalidIdentifier);

    let duplicate_field_first = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::required("zeta", SchemaType::Int),
        Field::required("9bad", SchemaType::Int),
    ])
    .prepare()
    .expect_err("a repeat outranks a later invalid field name");
    assert_eq!(duplicate_field_first.code, MonErrorCode::DuplicateField);
    assert_eq!(
        duplicate_field_first.path,
        vec![PathSegment::Field("zeta".into())],
    );
    let duplicate_field_before_child_error = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::required(
            "zeta",
            SchemaType::Unsupported {
                name: "Missing".into(),
            },
        ),
    ])
    .prepare()
    .expect_err("the duplicate check precedes the child schema validation");
    assert_eq!(
        duplicate_field_before_child_error.code,
        MonErrorCode::DuplicateField,
    );

    let duplicate_field_before_default_error = Schema::record(vec![
        Field::required("zeta", SchemaType::Int),
        Field::with_default(
            "zeta",
            SchemaType::Int,
            Value::String("not an integer".into()),
        ),
    ])
    .prepare()
    .expect_err("the duplicate check precedes default validation");
    assert_eq!(
        duplicate_field_before_default_error.code,
        MonErrorCode::DuplicateField,
    );

    let variant_error_first = Schema::record(vec![Field::required(
        "choice",
        SchemaType::Choice {
            name: "Palette".into(),
            variants: vec![
                Variant::unit("Zebra"),
                Variant::unit("9bad"),
                Variant::unit("Zebra"),
            ],
        },
    )])
    .prepare()
    .expect_err("the earlier invalid variant name remains the diagnostic");
    assert_eq!(variant_error_first.code, MonErrorCode::InvalidIdentifier);
    assert_eq!(
        variant_error_first.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("9bad".into()),
        ],
    );

    let duplicate_variant_first = Schema::record(vec![Field::required(
        "choice",
        SchemaType::Choice {
            name: "Palette".into(),
            variants: vec![
                Variant::unit("Zebra"),
                Variant::payload("Zebra", vec![Field::required("9bad", SchemaType::Int)]),
                Variant::unit("9bad"),
            ],
        },
    )])
    .prepare()
    .expect_err("a repeat outranks a later invalid variant name");
    assert_eq!(duplicate_variant_first.code, MonErrorCode::InvalidSchema);
    assert_eq!(
        duplicate_variant_first.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Zebra".into()),
        ],
    );
}

#[test]
fn duplicate_declarations_respect_the_node_budget_prefix() {
    // Every declaration pays one node before its own duplicate check, so a repeat the
    // remaining node budget cannot reach must keep the budget error.
    let field_repeat = |max_nodes: usize| {
        Schema::record(vec![
            Field::required("zeta", SchemaType::Int),
            Field::required("zeta", SchemaType::Int),
        ])
        .with_limits(Limits {
            max_nodes,
            ..Limits::default()
        })
        .prepare()
    };

    let exhausted = field_repeat(3).expect_err("the field repeat sits past the remaining nodes");
    assert_eq!(exhausted.code, MonErrorCode::NodeBudget);

    let reachable = field_repeat(4).expect_err("the field repeat stays inside the node budget");
    assert_eq!(reachable.code, MonErrorCode::DuplicateField);
    assert_eq!(reachable.path, vec![PathSegment::Field("zeta".into())]);

    let variant_repeat = |max_nodes: usize| {
        Schema::record(vec![Field::required(
            "choice",
            SchemaType::Choice {
                name: "Palette".into(),
                variants: vec![Variant::unit("Zebra"), Variant::unit("Zebra")],
            },
        )])
        .with_limits(Limits {
            max_nodes,
            ..Limits::default()
        })
        .prepare()
    };

    let exhausted =
        variant_repeat(4).expect_err("the variant repeat sits past the remaining nodes");
    assert_eq!(exhausted.code, MonErrorCode::NodeBudget);

    let reachable = variant_repeat(5).expect_err("the variant repeat stays inside the node budget");
    assert_eq!(reachable.code, MonErrorCode::InvalidSchema);
    assert_eq!(
        reachable.path,
        vec![
            PathSegment::Field("choice".into()),
            PathSegment::Variant("Zebra".into()),
        ],
    );
}

#[test]
fn first_error_is_deterministic() {
    let schema = save_schema();
    let input = r#"format_version = 1 title = "x", scores = {=}, theme = ::Light, note = none, exact_total = 1"#;
    let first = decode_document(input, &schema).expect_err("missing comma fails");
    let second = decode_document(input, &schema).expect_err("same input fails identically");
    assert_eq!(first.code, MonErrorCode::MissingComma);
    assert_eq!(first.code, second.code);
    assert_eq!(first.span, second.span);
    assert_eq!(first.path, second.path);
    assert!(first.span.is_some());
}

#[test]
fn invalid_utf8_bytes_report_stable_span() {
    let schema = save_schema();
    let error = decode_document_bytes(&[0xff], &schema).expect_err("invalid UTF-8 fails");
    assert_eq!(error.code, MonErrorCode::InvalidUtf8);
    assert!(error.span.is_some());
    assert_eq!(error.span.unwrap().start, 0);
    assert!(error.path.is_empty());
}

#[test]
fn default_expansion_budget_is_distinct_from_syntax() {
    let tight = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(1),
    )])
    .with_limits(Limits {
        max_default_expansions: 0,
        ..Limits::default()
    })
    .prepare()
    .expect("tight schema prepares");
    let error = decode_document("", &tight).expect_err("default expansion exceeds budget");
    assert_eq!(error.code, MonErrorCode::DefaultBudget);

    let normal = Schema::record(vec![Field::with_default(
        "value",
        SchemaType::Int,
        Value::Int(1),
    )])
    .prepare()
    .expect("normal schema prepares");
    assert_eq!(
        decode_document("", &normal).expect("default applies"),
        Value::Record(vec![("value".into(), Value::Int(1))])
    );
}

#[test]
fn optional_and_default_rules_are_exact() {
    let schema = save_schema();
    let missing = decode_document(
        r#"format_version = 1, title = "T", scores = {=}, theme = ::Light, exact_total = 1"#,
        &schema,
    )
    .expect_err("omitted optional without a default fails");
    assert_eq!(missing.code, MonErrorCode::MissingField);
    assert_eq!(missing.path, vec![PathSegment::Field("note".into())]);

    let with_none = decode_document(
        r#"format_version = 1, title = "T", scores = {=}, theme = ::Light, note = none, exact_total = 1"#,
        &schema,
    )
    .expect("explicit none succeeds and window uses its default");
    let save = native_save(with_none);
    assert_eq!(save.note, None);
    assert_eq!(
        save.window,
        Window {
            width: 1280,
            height: 720
        }
    );

    let parent = Schema::record(vec![Field::with_default(
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
    )])
    .prepare()
    .expect("parent default schema prepares");
    let partial = decode_document("layout = (width = 9)", &parent)
        .expect_err("partial nested record does not deep-merge");
    assert_eq!(partial.code, MonErrorCode::MissingField);
}

#[test]
fn malformed_input_reports_syntax_not_budget() {
    let schema = Schema::record(vec![
        Field::required("a", SchemaType::Int),
        Field::required("b", SchemaType::Int),
    ])
    .prepare()
    .expect("schema prepares");
    let error = decode_document("a = 1 b = 2", &schema).expect_err("missing comma fails");
    assert_eq!(error.code, MonErrorCode::MissingComma);
    assert!(error.span.is_some());
}

#[test]
fn wide_input_hits_node_budget() {
    let schema = Schema::record(vec![Field::required(
        "values",
        SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        },
    )])
    .with_limits(Limits {
        max_nodes: 6,
        ..Limits::default()
    })
    .prepare()
    .expect("wide schema prepares");
    let error = decode_document("values = {1, 2, 3, 4, 5, 6, 7, 8}", &schema)
        .expect_err("wide input exceeds node budget");
    assert_eq!(error.code, MonErrorCode::NodeBudget);
}

#[test]
fn deep_input_hits_depth_budget() {
    let schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .with_limits(Limits {
            max_depth: 1,
            ..Limits::default()
        })
        .prepare()
        .expect("shallow schema prepares");
    let error = decode_document("value = (nested = 1)", &schema)
        .expect_err("deep input exceeds depth budget");
    assert_eq!(error.code, MonErrorCode::DepthBudget);
}

#[test]
fn failure_then_success_shows_no_leaked_state() {
    let schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
        .prepare()
        .expect("schema prepares");
    let bad = Value::Record(vec![("value".into(), Value::String("bad".into()))]);
    let encode_error = encode_document(&bad, &schema).expect_err("wrong value type fails encoding");
    assert_eq!(encode_error.code, MonErrorCode::TypeMismatch);

    let good = Value::Record(vec![("value".into(), Value::Int(1))]);
    let encoded = encode_document(&good, &schema).expect("encoding succeeds after a failure");
    let decoded = decode_document(&encoded, &schema).expect("encoded output is valid");
    assert_eq!(decoded, good);

    let decode_error =
        decode_document("value = bad", &schema).expect_err("bare names are not literals");
    assert_eq!(decode_error.code, MonErrorCode::UnexpectedToken);
    let again =
        encode_document(&good, &schema).expect("encoding still succeeds after decode failure");
    assert_eq!(again, encoded);
}

#[test]
fn nested_scalar_encoding_quotes_strings() {
    let string_schema = Schema::value(SchemaType::String)
        .prepare()
        .expect("string schema prepares");
    assert_eq!(
        encode_value(&Value::String("looks = MON".into()), &string_schema)
            .expect("string data encodes quoted"),
        "\"looks = MON\""
    );
    let optional_schema = Schema::value(SchemaType::Optional(Box::new(SchemaType::String)))
        .prepare()
        .expect("optional schema prepares");
    assert_eq!(
        encode_value(&Value::None, &optional_schema).expect("explicit none encodes"),
        "none"
    );
}

#[test]
fn max_depth_policy_has_an_implementation_safe_ceiling() {
    let schema = Schema::value(SchemaType::Int)
        .with_limits(Limits {
            max_depth: Limits::MAX_SAFE_DEPTH + 1,
            ..Limits::default()
        })
        .prepare()
        .expect_err("unsafe recursion policies are rejected");
    assert_eq!(schema.code, MonErrorCode::DepthBudget);
}

#[test]
fn encoder_preserves_decoded_node_budget() {
    let schema = Schema::record(vec![Field::required(
        "items",
        SchemaType::Collection {
            element: Box::new(SchemaType::Record {
                fields: vec![Field::required("value", SchemaType::Int)],
            }),
        },
    )])
    .with_limits(Limits {
        max_nodes: 6,
        ..Limits::default()
    })
    .prepare()
    .expect("schema types fit the node budget");
    let decoded = decode_document("items = {(value = 1), (value = 2)}", &schema)
        .expect("collection records fit the node budget");

    let encoded = encode_document(&decoded, &schema)
        .expect("encoding a decoded value uses the same node budget");
    let round_trip =
        decode_document(&encoded, &schema).expect("encoded output remains valid under the schema");
    assert_eq!(round_trip, decoded);
}

#[test]
fn default_maps_use_the_same_node_cost_as_authored_maps() {
    use moth::mon::{
        Field, Limits, MonErrorCode, PreparedSchema, Schema, SchemaType, Value, decode_document,
        encode_document,
    };

    fn prepared(max_nodes: usize) -> PreparedSchema {
        Schema::record(vec![Field::required(
            "items",
            SchemaType::Collection {
                element: Box::new(SchemaType::Record {
                    fields: vec![Field::with_default(
                        "m",
                        SchemaType::Map {
                            key: Box::new(SchemaType::Int),
                            value: Box::new(SchemaType::Int),
                        },
                        Value::Map(vec![(Value::Int(1), Value::Int(2))]),
                    )],
                }),
            },
        )])
        .with_limits(Limits {
            max_nodes,
            ..Limits::default()
        })
        .prepare()
        .expect("schema preparation fits the selected budget")
    }

    let omitted = "items = {(), (), (), ()}";
    let explicit = "items = {(m = {1 = 2}), (m = {1 = 2}), \
                    (m = {1 = 2}), (m = {1 = 2})}";

    let tight = prepared(18);
    for input in [omitted, explicit] {
        let error = decode_document(input, &tight)
            .expect_err("the same completed maps need the same node budget");
        assert_eq!(error.code, MonErrorCode::NodeBudget);
    }

    let sufficient = prepared(22);
    let from_defaults =
        decode_document(omitted, &sufficient).expect("defaulted maps fit the complete node budget");
    let from_explicit =
        decode_document(explicit, &sufficient).expect("explicit maps fit the complete node budget");
    assert_eq!(from_defaults, from_explicit);

    let encoded = encode_document(&from_defaults, &sufficient)
        .expect("the completed decoded value remains encodable");
    assert_eq!(
        decode_document(&encoded, &sufficient).expect("encoded maps decode"),
        from_defaults,
    );
}

#[test]
fn default_maps_inside_choice_payload_share_the_same_node_cost() {
    use moth::mon::{
        Field, Limits, MonErrorCode, PreparedSchema, Schema, SchemaType, Value, Variant,
        decode_document, encode_document,
    };

    fn prepared(max_nodes: usize) -> PreparedSchema {
        Schema::record(vec![Field::required(
            "items",
            SchemaType::Collection {
                element: Box::new(SchemaType::Choice {
                    name: "Pick".to_owned(),
                    variants: vec![Variant::payload(
                        "With",
                        vec![Field::required(
                            "settings",
                            SchemaType::Record {
                                fields: vec![Field::with_default(
                                    "m",
                                    SchemaType::Map {
                                        key: Box::new(SchemaType::Int),
                                        value: Box::new(SchemaType::Int),
                                    },
                                    Value::Map(vec![
                                        (Value::Int(1), Value::Int(2)),
                                        (Value::Int(3), Value::Int(4)),
                                    ]),
                                )],
                            },
                        )],
                    )],
                }),
            },
        )])
        .with_limits(Limits {
            max_nodes,
            ..Limits::default()
        })
        .prepare()
        .expect("schema preparation fits the selected budget")
    }

    let omitted = "items = {::With(settings = ()), ::With(settings = ()), ::With(settings = ())}";
    let explicit = "items = {::With(settings = (m = {1 = 2, 3 = 4})), \
                    ::With(settings = (m = {1 = 2, 3 = 4})), \
                    ::With(settings = (m = {1 = 2, 3 = 4}))}";

    let tight = prepared(26);
    for input in [omitted, explicit] {
        let error = decode_document(input, &tight)
            .expect_err("the same completed choice maps need the same node budget");
        assert_eq!(error.code, MonErrorCode::NodeBudget);
    }
    let programmatic = Value::Record(vec![(
        "items".into(),
        Value::Collection(vec![
            Value::Choice {
                qualifier: None,
                variant: "With".into(),
                fields: vec![("settings".into(), Value::Record(vec![]))],
            },
            Value::Choice {
                qualifier: None,
                variant: "With".into(),
                fields: vec![("settings".into(), Value::Record(vec![]))],
            },
            Value::Choice {
                qualifier: None,
                variant: "With".into(),
                fields: vec![("settings".into(), Value::Record(vec![]))],
            },
        ]),
    )]);
    let error = encode_document(&programmatic, &tight)
        .expect_err("programmatic choice defaults need the same node budget");
    assert_eq!(error.code, MonErrorCode::NodeBudget);

    let sufficient = prepared(29);
    let from_defaults = decode_document(omitted, &sufficient)
        .expect("defaulted choice maps fit the complete node budget");
    let from_explicit = decode_document(explicit, &sufficient)
        .expect("explicit choice maps fit the complete node budget");
    assert_eq!(from_defaults, from_explicit);

    let encoded_programmatic = encode_document(&programmatic, &sufficient)
        .expect("programmatic choice defaults fit the complete node budget");
    assert_eq!(
        decode_document(&encoded_programmatic, &sufficient)
            .expect("encoded programmatic choice maps decode"),
        from_defaults,
    );

    let encoded = encode_document(&from_defaults, &sufficient)
        .expect("the completed decoded choice value remains encodable");
    assert_eq!(
        decode_document(&encoded, &sufficient).expect("encoded choice maps decode"),
        from_defaults,
    );
}

#[test]
fn input_and_output_byte_limits_are_distinct() {
    use moth::mon::{
        Field, Limits, MonErrorCode, Schema, SchemaType, Value, decode_document, encode_document,
    };

    let schema = Schema::record(vec![Field::required("v", SchemaType::String)])
        .with_limits(Limits {
            max_input_bytes: 8,
            max_output_bytes: 1024,
            ..Limits::default()
        })
        .prepare()
        .expect("schema prepares despite the tight input limit");

    let value = Value::Record(vec![("v".into(), Value::String("hello".into()))]);
    let encoded = encode_document(&value, &schema).expect("output fits the larger output budget");
    assert!(
        encoded.len() > 8,
        "encoded text exceeds the input budget: {encoded:?}"
    );
    let error =
        decode_document(&encoded, &schema).expect_err("the same text exceeds the input budget");
    assert_eq!(error.code, MonErrorCode::InputBudget);
}

#[test]
fn encoded_duplicate_map_key_reports_the_key_path() {
    let schema = Schema::value(SchemaType::Map {
        key: Box::new(SchemaType::String),
        value: Box::new(SchemaType::Int),
    })
    .prepare()
    .expect("map schema prepares");
    let error = encode_value(
        &Value::Map(vec![
            (Value::String("same".into()), Value::Int(1)),
            (Value::String("same".into()), Value::Int(2)),
        ]),
        &schema,
    )
    .expect_err("duplicate map keys must fail");

    assert_eq!(error.code, MonErrorCode::DuplicateMapKey);
    assert_eq!(error.path, vec![PathSegment::MapKey("same".into())]);
}

#[test]
fn encoded_duplicate_key_path_respects_decoded_byte_budget() {
    let schema = Schema::value(SchemaType::Map {
        key: Box::new(SchemaType::String),
        value: Box::new(SchemaType::Int),
    })
    .with_limits(Limits {
        max_decoded_bytes: 12,
        ..Limits::default()
    })
    .prepare()
    .expect("map schema prepares");
    let error = encode_value(
        &Value::Map(vec![
            (Value::String("same".into()), Value::Int(1)),
            (Value::String("same".into()), Value::Int(2)),
        ]),
        &schema,
    )
    .expect_err("duplicate diagnostic path bytes are budgeted before allocation");

    assert_eq!(error.code, MonErrorCode::DecodedBudget);
}
#[test]
fn max_safe_depth_accepts_decodes_and_drops_nested_records() {
    let mut nested_type = SchemaType::Int;
    let mut nested_value = Value::Int(7);
    let mut nested_literal = "7".to_owned();
    for _ in 0..Limits::MAX_SAFE_DEPTH - 1 {
        nested_type = SchemaType::Record {
            fields: vec![Field::required("next", nested_type)],
        };
        nested_value = Value::Record(vec![("next".to_owned(), nested_value)]);
        nested_literal = format!("(next = {nested_literal})");
    }

    let schema = Schema::record(vec![Field::required("value", nested_type)])
        .with_limits(Limits {
            max_depth: Limits::MAX_SAFE_DEPTH,
            ..Limits::default()
        })
        .prepare()
        .expect("schema at the safe depth ceiling prepares");
    let source = format!("value = {nested_literal}");
    let decoded = decode_document(&source, &schema)
        .expect("nested document at the safe depth ceiling decodes");
    let expected = Value::Record(vec![("value".to_owned(), nested_value)]);
    assert_eq!(decoded, expected);

    let encoded = encode_document(&decoded, &schema)
        .expect("the writer accepts a value at the safe depth ceiling");
    let round_trip = decode_document(&encoded, &schema)
        .expect("encoded output remains valid at the safe depth ceiling");
    assert_eq!(round_trip, decoded);

    drop(decoded);
    drop(round_trip);
    drop(expected);
    drop(schema);
}
#[test]
fn rejecting_a_deep_schema_releases_its_unvisited_tail_iteratively() {
    let mut ty = SchemaType::Int;
    for _ in 0..20_000 {
        ty = SchemaType::Optional(Box::new(ty));
    }

    let error = Schema::value(ty)
        .prepare()
        .expect_err("the default depth limit rejects the schema");
    assert_eq!(error.code, MonErrorCode::DepthBudget);
}

#[test]
fn rejecting_a_deep_default_releases_its_unvisited_tail_iteratively() {
    let mut ty = SchemaType::Int;
    for _ in 0..32 {
        ty = SchemaType::Collection {
            element: Box::new(ty),
        };
    }

    let mut value = Value::String("wrong leaf".into());
    for _ in 0..20_000 {
        value = Value::Collection(vec![value]);
    }

    let error = Schema::record(vec![Field::with_default("nested", ty, value)])
        .with_limits(Limits {
            max_depth: Limits::MAX_SAFE_DEPTH,
            ..Limits::default()
        })
        .prepare()
        .expect_err("the nested default does not match its schema");
    assert_eq!(error.code, MonErrorCode::InvalidDefault);
}

#[test]
fn retained_numeric_scratch_respects_decoded_byte_budget() {
    use moth::mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required(
        "n",
        SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        },
    )])
    .with_limits(Limits {
        max_decoded_bytes: 64,
        ..Limits::default()
    })
    .prepare()
    .expect("the small schema fits its limits");

    // Each number is in i32 range. Input size, depth, node count and the
    // per-literal digit limit remain comfortably within their budgets.
    let numbers = vec!["1234567890"; 256].join(",");
    let input = format!("n = {{{numbers}}}");
    let error = decode_document(&input, &schema)
        .expect_err("retained numeric normalization must be budgeted");
    assert_eq!(error.code, MonErrorCode::DecodedBudget);
}

#[test]
fn malformed_numeric_scratch_is_budgeted_before_syntax() {
    use moth::mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required("n", SchemaType::Int)])
        .with_limits(Limits {
            max_decoded_bytes: 64,
            ..Limits::default()
        })
        .prepare()
        .expect("the small schema fits its limits");

    // One digit stays under the numeric-digit limit while the malformed suffix
    // makes normalization reserve the full token length.
    let token = format!("1{}", "_".repeat(100));
    let error = decode_document(&format!("n = {token}"), &schema)
        .expect_err("malformed long numeric scratch must be budgeted");
    assert_eq!(error.code, MonErrorCode::DecodedBudget);
}

#[test]
fn schema_numeric_scratch_is_budgeted_before_syntax() {
    use moth::mon::{Field, Limits, MonErrorCode, Schema, SchemaType, Value};

    let token = format!("1{}", "_".repeat(100));

    let programmatic_schema = Schema::value(SchemaType::Integer)
        .with_limits(Limits {
            max_decoded_bytes: token.len(),
            ..Limits::default()
        })
        .prepare()
        .expect("the programmatic schema fits the output-copy budget");
    let programmatic_error = encode_value(&Value::Integer(token.clone()), &programmatic_schema)
        .expect_err("numeric normalization scratch must be charged before parsing");
    assert_eq!(programmatic_error.code, MonErrorCode::DecodedBudget);

    let default_error = Schema::record(vec![Field::with_default(
        "n",
        SchemaType::Decimal { scale: 0 },
        Value::Decimal(token.clone()),
    )])
    .with_limits(Limits {
        max_decoded_bytes: token.len() + 1,
        ..Limits::default()
    })
    .prepare()
    .expect_err("default normalization scratch must be charged before parsing");
    assert_eq!(default_error.code, MonErrorCode::DecodedBudget);
}

#[test]
fn exact_numeric_results_fit_their_decoded_byte_budget() {
    use moth::mon::{
        Field, Limits, MonErrorCode, Schema, SchemaType, Span, Value, decode_document,
    };

    let integer_schema = Schema::record(vec![Field::required("x", SchemaType::Integer)])
        .with_limits(Limits {
            max_decoded_bytes: 6,
            ..Limits::default()
        })
        .prepare()
        .expect("the integer schema fits the decoded-byte budget");
    assert_eq!(
        decode_document("x = 123", &integer_schema).expect("exact integer fits its budget"),
        Value::Record(vec![("x".into(), Value::Integer("123".into()))]),
    );

    let decimal_schema =
        Schema::record(vec![Field::required("x", SchemaType::Decimal { scale: 2 })])
            .with_limits(Limits {
                max_decoded_bytes: 10,
                ..Limits::default()
            })
            .prepare()
            .expect("the decimal schema fits the decoded-byte budget");
    assert_eq!(
        decode_document("x = -1.2300", &decimal_schema)
            .expect("signed exact decimal fits its budget"),
        Value::Record(vec![("x".into(), Value::Decimal("-1.2300".into()))]),
    );

    // The parsed and validated key plus unsigned scratch consume eight bytes;
    // the restored sign is byte nine.
    let sign_charge_schema =
        Schema::record(vec![Field::required("x", SchemaType::Decimal { scale: 2 })])
            .with_limits(Limits {
                max_decoded_bytes: 8,
                ..Limits::default()
            })
            .prepare()
            .expect("the tight-budget schema prepares");
    let sign_charge_error = decode_document("x = -1.2300", &sign_charge_schema)
        .expect_err("the restored sign must be charged before insertion");
    assert_eq!(sign_charge_error.code, MonErrorCode::DecodedBudget);
    assert_eq!(sign_charge_error.span, Some(Span { start: 4, end: 11 }),);
    assert_eq!(sign_charge_error.path, vec![PathSegment::Field("x".into())],);
}

#[test]
fn decimal_default_scale_failure_keeps_numeric_scale_code() {
    use moth::mon::{Field, MonErrorCode, Schema, SchemaType, Value};

    let error = Schema::record(vec![Field::with_default(
        "amount",
        SchemaType::Decimal { scale: 2 },
        Value::Decimal("1.239".into()),
    )])
    .prepare()
    .expect_err("an out-of-scale default must be rejected");

    assert_eq!(error.code, MonErrorCode::NumericScale);
    assert_eq!(error.path, vec![PathSegment::Field("amount".into())],);
    assert_eq!(
        error.detail,
        "Decimal value has effective scale 3, above declared scale 2",
    );
}

#[test]
fn decimal_exponents_preserve_scale_and_exact_text() {
    use moth::mon::{Field, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required(
        "amount",
        SchemaType::Decimal { scale: 2 },
    )])
    .prepare()
    .expect("the Decimal schema prepares");
    assert_eq!(
        decode_document("amount = 1_2e-2", &schema).expect("in-scale exponent decodes"),
        Value::Record(vec![("amount".into(), Value::Decimal("12e-2".into()))]),
    );

    let reader_error = decode_document("amount = 1e-30", &schema)
        .expect_err("an exponent beyond the Decimal scale must fail");
    assert_eq!(reader_error.code, MonErrorCode::NumericScale);

    let default_error = Schema::record(vec![Field::with_default(
        "amount",
        SchemaType::Decimal { scale: 2 },
        Value::Decimal("1e-30".into()),
    )])
    .prepare()
    .expect_err("the same out-of-scale exponent default must fail");
    assert_eq!(default_error.code, MonErrorCode::NumericScale);
    assert_eq!(
        default_error.detail,
        "Decimal value has effective scale 30, above declared scale 2",
    );
}

#[test]
fn extreme_float_exponent_reports_non_finite_float() {
    use moth::mon::{Field, Limits, MonErrorCode, Schema, SchemaType, Value, decode_document};

    let schema = Schema::record(vec![Field::required("v", SchemaType::Float)])
        .with_limits(Limits {
            max_numeric_digits: 10_000,
            ..Limits::default()
        })
        .prepare()
        .expect("the exponent schema fits its limits");

    let value = decode_document("v = 1e+21", &schema).expect("large finite exponent decodes");
    assert_eq!(value, Value::Record(vec![("v".into(), Value::Float(1e21))]));
    let error = decode_document("v = 1e10000", &schema)
        .expect_err("a non-finite exponent must fail as float range");
    assert_eq!(error.code, MonErrorCode::NonFiniteFloat);
}
