//! Public API coverage for bounded resources and iterative failure cleanup.

use super::*;

#[test]
fn supplied_choice_names_keep_their_completion_budget_and_error_paths() {
    let prepared = |max_decoded_bytes| {
        Schema::record(vec![Field::required(
            "f",
            SchemaType::Collection {
                element: Box::new(SchemaType::Choice {
                    name: "C".into(),
                    variants: vec![Variant::unit("Long")],
                }),
            },
        )])
        .with_limits(Limits {
            max_decoded_bytes,
            ..Limits::default()
        })
        .prepare()
        .expect("choice schema fits the byte budget")
    };
    let choice = || Value::Choice {
        qualifier: None,
        variant: "Long".into(),
        fields: vec![],
    };
    let value = Value::Record(vec![(
        "f".into(),
        Value::Collection(vec![choice(), choice()]),
    )]);

    encode_document(&value, &prepared(9)).expect("field and two variants fit nine bytes");
    let error = encode_document(&value, &prepared(8)).expect_err("second variant exceeds budget");
    assert_eq!(error.code, MonErrorCode::DecodedBudget);
    assert_eq!(
        error.path,
        vec![PathSegment::Field("f".into()), PathSegment::Index(1)]
    );
}

#[test]
fn map_entry_nodes_are_charged_during_parsing() {
    let prepared = |max_nodes| {
        Schema::record(vec![Field::required(
            "m",
            SchemaType::Map {
                key: Box::new(SchemaType::Bool),
                value: Box::new(SchemaType::Bool),
            },
        )])
        .with_limits(Limits {
            max_nodes,
            ..Limits::default()
        })
        .prepare()
        .expect("map schema fits the node budget")
    };

    let input = "(m = {true = false, false = true})";
    let tight = prepared(6);
    for document in [input.to_owned(), format!("{input} tail")] {
        let error = decode_document(&document, &tight).expect_err("second entry exceeds budget");
        assert_eq!(error.code, MonErrorCode::NodeBudget);
        assert_eq!(
            error.path,
            vec![PathSegment::Field("m".into()), PathSegment::Index(1)]
        );
        let start = document.rfind("false").expect("second key exists") as u32;
        assert_eq!(
            error.span,
            Some(moth_mon::Span {
                start,
                end: start + 5
            })
        );
    }

    let first_entry = "(m = {{{true = false}}})";
    let error = decode_document(first_entry, &prepared(5))
        .expect_err("first entry is charged before its value");
    assert_eq!(error.code, MonErrorCode::NodeBudget);
    assert_eq!(
        error.path,
        vec![
            PathSegment::Field("m".into()),
            PathSegment::Index(0),
            PathSegment::Index(0),
            PathSegment::Index(0)
        ]
    );
    let start = first_entry.find("true").expect("first key exists") as u32;
    assert_eq!(
        error.span,
        Some(moth_mon::Span {
            start,
            end: start + 4
        })
    );

    for (document, limit, indexes, detail) in [
        (
            "(m = {{{true = ",
            5,
            vec![0, 0, 0],
            "MON map entry is missing its value",
        ),
        (
            "(m = {true = false, false = ",
            6,
            vec![1],
            "MON value is missing",
        ),
    ] {
        let error = decode_document(document, &prepared(limit))
            .expect_err("a missing value retains its syntax error at the node limit");
        assert_eq!(error.code, MonErrorCode::UnexpectedEnd);
        assert_eq!(error.detail, detail);
        let mut path = vec![PathSegment::Field("m".into())];
        path.extend(indexes.into_iter().map(PathSegment::Index));
        assert_eq!(error.path, path);
        let end = document.len() as u32;
        assert_eq!(error.span, Some(moth_mon::Span { start: end, end }));
    }

    for (prefix, limit, indexes) in [
        ("(m = {{{true = ", 6, vec![0, 0, 0]),
        ("(m = {true = false, false = ", 7, vec![1]),
    ] {
        let schema = prepared(limit);
        let mut path = vec![PathSegment::Field("m".into())];
        path.extend(indexes.into_iter().map(PathSegment::Index));
        let start = prefix.len() as u32;
        for invalid_start in [',', ')', ']', '}', '@', '-', ':'] {
            let error = decode_document(&format!("{prefix}{invalid_start}"), &schema)
                .expect_err("an unsupported literal starter cannot supply an entry value");
            assert_eq!(error.code, MonErrorCode::UnexpectedToken, "{invalid_start}");
            assert_eq!(error.path, path);
            assert_eq!(
                error.span,
                Some(moth_mon::Span {
                    start,
                    end: start + 1
                })
            );
        }
    }

    for document in ["(m = {{{{=}}}})", "(m = {{{{}}}})"] {
        let error = decode_document(document, &prepared(5))
            .expect_err("nested empty containers reach schema validation at the exact limit");
        assert_eq!(error.code, MonErrorCode::MapKind);
    }

    for document in ["(m = {=})", "(m = {true = false})"] {
        decode_document(document, &tight).expect("empty and one-entry maps fit");
    }
    let collection = decode_document("(m = {true, false})", &prepared(5))
        .expect_err("collection remains distinct from a map");
    assert_eq!(collection.code, MonErrorCode::MapKind);

    let exact = prepared(8);
    let value = decode_document(input, &exact).expect("two entries fit exactly eight nodes");
    let encoded = encode_document(&value, &exact).expect("the same node budget permits encoding");
    assert_eq!(
        decode_document(&encoded, &exact).expect("encoded map decodes within the same limit"),
        value
    );
    let trailing = decode_document(&format!("{input} tail"), &exact)
        .expect_err("trailing input remains an error when the map fits");
    assert_eq!(trailing.code, MonErrorCode::TrailingInput);
}

#[test]
fn max_safe_depth_accepts_decodes_and_drops_nested_records() {
    let mut nested_type = SchemaType::I32;
    let mut nested_value = Value::I32(7);
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
    let mut ty = SchemaType::I32;
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
    let mut ty = SchemaType::I32;
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
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required(
        "n",
        SchemaType::Collection {
            element: Box::new(SchemaType::I32),
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
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, decode_document};

    let schema = Schema::record(vec![Field::required("n", SchemaType::I32)])
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
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, Value};

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
fn decimal_scale_errors_project_defaults_without_masking_budgets() {
    for (limits, supplied_code, default_code) in [
        (
            Limits::default(),
            MonErrorCode::NumericScale,
            MonErrorCode::InvalidDefault,
        ),
        (
            Limits {
                max_numeric_digits: 3,
                ..Limits::default()
            },
            MonErrorCode::NumericBudget,
            MonErrorCode::NumericBudget,
        ),
        (
            Limits {
                max_decoded_bytes: 7,
                ..Limits::default()
            },
            MonErrorCode::DecodedBudget,
            MonErrorCode::DecodedBudget,
        ),
    ] {
        let supplied_schema =
            Schema::record(vec![Field::required("n", SchemaType::Decimal { scale: 2 })])
                .with_limits(limits.clone())
                .prepare()
                .expect("the decimal declaration fits each budget");
        let supplied = Value::Record(vec![("n".into(), Value::Decimal("1.239".into()))]);
        let supplied_error = encode_document(&supplied, &supplied_schema)
            .expect_err("inexact decimal encoding never publishes output");
        assert_eq!(supplied_error.code, supplied_code);
        assert_eq!(supplied_error.path, vec![PathSegment::Field("n".into())]);
        assert_eq!(supplied_error.span, None);

        let default_error = Schema::record(vec![Field::with_default(
            "n",
            SchemaType::Decimal { scale: 2 },
            Value::Decimal("1.239".into()),
        )])
        .with_limits(limits)
        .prepare()
        .expect_err("an inexact decimal cannot become a prepared default");
        assert_eq!(default_error.code, default_code);
        assert_eq!(default_error.path, vec![PathSegment::Field("n".into())]);
        assert_eq!(default_error.span, None);
    }
}

#[test]
fn compound_value_errors_project_defaults_and_preserve_supplied_categories() {
    let choice_type = || SchemaType::Choice {
        name: "Theme".into(),
        variants: vec![Variant::payload(
            "Custom",
            vec![Field::required("name", SchemaType::String)],
        )],
    };
    let choice = |qualifier: Option<&str>, variant: &str, fields| Value::Choice {
        qualifier: qualifier.map(str::to_owned),
        variant: variant.into(),
        fields,
    };
    let record_type = || SchemaType::Record {
        fields: vec![Field::required("name", SchemaType::String)],
    };
    let duplicate_fields = || {
        vec![
            ("name".into(), Value::String("first".into())),
            ("name".into(), Value::String("second".into())),
        ]
    };
    let unknown_fields = || vec![("extra".into(), Value::I32(1))];
    for (ty, value, supplied_code, path) in [
        (
            choice_type(),
            choice(Some("Other"), "Custom", vec![]),
            MonErrorCode::QualifierMismatch,
            vec![],
        ),
        (
            choice_type(),
            choice(None, "Missing", vec![]),
            MonErrorCode::UnknownVariant,
            vec![],
        ),
        (
            record_type(),
            Value::Record(unknown_fields()),
            MonErrorCode::UnknownField,
            vec![PathSegment::Field("extra".into())],
        ),
        (
            record_type(),
            Value::Record(duplicate_fields()),
            MonErrorCode::DuplicateField,
            vec![PathSegment::Field("name".into())],
        ),
        (
            choice_type(),
            choice(None, "Custom", unknown_fields()),
            MonErrorCode::UnknownArgument,
            vec![
                PathSegment::Variant("Custom".into()),
                PathSegment::Field("extra".into()),
            ],
        ),
        (
            choice_type(),
            choice(None, "Custom", duplicate_fields()),
            MonErrorCode::DuplicateArgument,
            vec![
                PathSegment::Variant("Custom".into()),
                PathSegment::Field("name".into()),
            ],
        ),
    ] {
        let supplied_schema = Schema::value(ty.clone())
            .prepare()
            .expect("the compound schema is valid");
        let supplied_error = encode_value(&value, &supplied_schema)
            .expect_err("invalid compound values cannot be encoded");
        assert_eq!(supplied_error.code, supplied_code);
        assert_eq!(supplied_error.path, path);
        assert_eq!(supplied_error.span, None);

        let default_error = Schema::record(vec![Field::with_default("nested", ty, value)])
            .prepare()
            .expect_err("invalid compound values cannot become defaults");
        let mut default_path = vec![PathSegment::Field("nested".into())];
        default_path.extend(path);
        assert_eq!(default_error.code, MonErrorCode::InvalidDefault);
        assert_eq!(default_error.path, default_path);
        assert_eq!(default_error.span, None);
    }
}

#[test]
fn inexact_default_failure_releases_a_deep_unvisited_default_tail() {
    let mut tail = Value::I32(1);
    for _ in 0..20_000 {
        tail = Value::Collection(vec![tail]);
    }
    let error = Schema::record(vec![
        Field::with_default(
            "n",
            SchemaType::Decimal { scale: 2 },
            Value::Decimal("1.239".into()),
        ),
        Field::with_default("tail", SchemaType::I32, tail),
    ])
    .prepare()
    .expect_err("the first inexact default rejects preparation before visiting the tail");
    assert_eq!(error.code, MonErrorCode::InvalidDefault);
    assert_eq!(error.path, vec![PathSegment::Field("n".into())]);
}

#[test]
fn failed_decimal_encoding_preserves_schema_defaults_and_successful_output() {
    let schema = Schema::record(vec![
        Field::with_default("prefix", SchemaType::String, Value::String("ready".into())),
        Field::required("n", SchemaType::Decimal { scale: 2 }),
    ])
    .prepare()
    .expect("the reusable decimal schema prepares");
    let valid = Value::Record(vec![("n".into(), Value::Decimal("1.23".into()))]);
    let completed = Value::Record(vec![
        ("prefix".into(), Value::String("ready".into())),
        ("n".into(), Value::Decimal("1.23".into())),
    ]);
    let before = encode_document(&valid, &schema).expect("exact decimal encoding succeeds");
    assert_eq!(
        decode_document(&before, &schema).expect("successful output is a complete document"),
        completed,
    );

    let invalid = Value::Record(vec![("n".into(), Value::Decimal("1.239".into()))]);
    let error = encode_document(&invalid, &schema)
        .expect_err("a later inexact decimal returns only an error, not the completed prefix");
    assert_eq!(error.code, MonErrorCode::NumericScale);
    assert_eq!(error.path, vec![PathSegment::Field("n".into())]);

    let after = encode_document(&valid, &schema)
        .expect("failed encoding leaves the prepared schema reusable");
    assert_eq!(after, before);
    assert_eq!(
        decode_document("n = 1.23", &schema).expect("defaults survive the failed encoding"),
        completed,
    );
}

#[test]
fn exact_numeric_results_fit_their_decoded_byte_budget() {
    use moth_mon::{Field, Limits, MonErrorCode, Schema, SchemaType, Span, Value, decode_document};

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
fn ordinary_error_boundary_retains_structured_failure_without_input() {
    fn load(input: &str, schema: &PreparedSchema) -> Result<Value, Box<dyn std::error::Error>> {
        Ok(decode_document(input, schema)?)
    }

    let schema = Schema::record(vec![Field::required("count", SchemaType::I32)])
        .prepare()
        .expect("the scalar schema prepares");
    let failure = {
        let input = String::from("count = 1.5");
        load(&input, &schema).expect_err("decimal spelling cannot supply an I32")
    };
    let error = failure
        .downcast_ref::<moth_mon::MonError>()
        .expect("ordinary propagation preserves the public error type");
    assert_eq!(error.code, MonErrorCode::NumericType);
    assert_eq!(error.path, vec![PathSegment::Field("count".into())]);
    assert_eq!(error.span, Some(moth_mon::Span { start: 8, end: 11 }));
    assert!(failure.source().is_none());

    // Rendering needs only the owned diagnostic, not a retained input or excerpt.
    let rendered = failure.to_string();
    assert!(rendered.contains("NumericType"));
    assert!(rendered.contains("count"));
    assert!(rendered.contains("8..11"));
    assert!(rendered.contains(&error.detail));
}
