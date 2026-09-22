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
