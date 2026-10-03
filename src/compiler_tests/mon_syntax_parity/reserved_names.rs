//! Reserved-name and word-role parity fixtures.
//!
//! WHAT: owns the reserved-name source/MON family.
//! WHY: parser-family cases stay local while using the common fixture assertion contract.
use crate::compiler_frontend::compiler_messages::InvalidExpressionReason;
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, Variant};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_declarations,
    in_literal, mon_identifier_error, source_expression_assignment, source_invalid_expression,
    source_named_argument_not_found, source_none_literal_context, source_reserved_keyword,
    source_unexpected_token, source_unknown_name, source_unknown_variant,
    source_value_block_outside_receiver,
};
fn reserved_name_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    let scalar_schema = || Schema::record(vec![Field::required("ordinary", SchemaType::Int)]);

    for name in ["__LoOp", "_U64", "dec01", "dec257", "if", "U64", "Dec"] {
        let literal = format!("{name} = 1");
        fixtures.push(fixture(
            format!("reserved_label_use_site_{name}"),
            literal.clone(),
            "",
            scalar_schema(),
            match name {
                "if" => source_value_block_outside_receiver(in_literal(&literal, "if", 1)),
                "U64" => source_unexpected_token(in_literal(&literal, name, 1)),
                _ => source_reserved_keyword(in_literal(&literal, name, 1)),
            },
            mon_identifier_error(&literal, name, vec![PathSegment::Field(name.to_owned())]),
        ));
    }

    for name in ["true", "false", "none"] {
        let literal = format!("{name} = 1");
        fixtures.push(fixture(
            format!("literal_word_label_{name}"),
            literal.clone(),
            "",
            scalar_schema(),
            if name == "none" {
                source_none_literal_context(in_literal(&literal, name, 1))
            } else {
                source_expression_assignment(in_literal(&literal, "=", 1))
            },
            mon_identifier_error(&literal, name, vec![PathSegment::Field(name.to_owned())]),
        ));
    }

    for name in ["__LoOp", "_U64", "dec01", "dec257", "true", "false", "none"] {
        let literal = format!("value = {name}(inner = 1)");
        fixtures.push(fixture(
            format!("reserved_nominal_use_site_qualifier_{name}"),
            literal.clone(),
            "Box = | inner Int |\n",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Struct {
                    name: "Box".into(),
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            match name {
                "true" | "false" => source_invalid_expression(
                    InvalidExpressionReason::ExpectedOperatorBeforeExpression,
                    in_literal(&literal, "(", 1),
                ),
                "none" => source_none_literal_context(in_literal(&literal, name, 1)),
                _ => source_unknown_name(in_literal(&literal, name, 1)),
            },
            mon_identifier_error(&literal, name, vec![PathSegment::Field("value".into())]),
        ));
    }

    for name in ["__LoOp", "_U64", "dec01", "dec257", "true", "false", "none"] {
        let literal = format!("value = {name}::Ready");
        fixtures.push(fixture(
            format!("reserved_choice_use_site_qualifier_{name}"),
            literal.clone(),
            "Theme :: Ready;\n",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::unit("Ready")],
                },
            )]),
            match name {
                "true" | "false" => source_unexpected_token(in_literal(&literal, "::", 1)),
                "none" => source_none_literal_context(in_literal(&literal, name, 1)),
                _ => source_unknown_name(in_literal(&literal, name, 1)),
            },
            mon_identifier_error(&literal, name, vec![PathSegment::Field("value".into())]),
        ));
    }

    for name in ["__LoOp", "_U64", "dec01", "dec257", "true", "false", "none"] {
        let literal = format!("value = Theme::{name}");
        fixtures.push(fixture(
            format!("reserved_variant_use_site_{name}"),
            literal.clone(),
            "Theme :: Ready;\n",
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::unit("Ready")],
                },
            )]),
            match name {
                "true" | "false" | "none" => source_unexpected_token(in_literal(&literal, name, 1)),
                _ => source_unknown_variant(in_literal(&literal, name, 1)),
            },
            mon_identifier_error(
                &literal,
                name,
                vec![
                    PathSegment::Field("value".into()),
                    PathSegment::Variant(name.to_owned()),
                ],
            ),
        ));
    }

    let payload_literal = "value = Box(__LoOp = 1)".to_owned();
    fixtures.push(fixture(
        "reserved_payload_use_site_label",
        payload_literal.clone(),
        "Box = | inner Int |\n",
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "Box".into(),
                fields: vec![Field::required("inner", SchemaType::Int)],
            },
        )]),
        source_named_argument_not_found(in_literal(&payload_literal, "__LoOp", 1)),
        mon_identifier_error(
            &payload_literal,
            "__LoOp",
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("__LoOp".into()),
            ],
        ),
    ));

    for name in [
        "__LoOp", "_U64", "dec01", "dec257", "Dec", "true", "false", "none",
    ] {
        let declarations = format!("{name} = | inner Int |\n");
        let expected_source = match name {
            "true" | "false" => {
                source_expression_assignment(in_declarations(&declarations, "=", 1))
            }
            "none" => source_unexpected_token(in_declarations(&declarations, name, 1)),
            _ => source_reserved_keyword(in_declarations(&declarations, name, 1)),
        };
        fixtures.push(fixture(
            format!("reserved_struct_declaration_{name}"),
            "value = 1",
            declarations,
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Struct {
                    name: name.into(),
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            expected_source,
            MonExpectation::SchemaRejected {
                code: MonErrorCode::InvalidSchema,
                path: vec![PathSegment::Field("value".into())],
            },
        ));
    }

    for name in [
        "__LoOp", "_U64", "dec01", "dec257", "Dec", "true", "false", "none",
    ] {
        let declarations = format!("Theme :: {name};\n");
        let expected_source = match name {
            "true" | "false" | "none" => {
                source_unexpected_token(in_declarations(&declarations, name, 1))
            }
            _ => source_reserved_keyword(in_declarations(&declarations, name, 1)),
        };
        fixtures.push(fixture(
            format!("reserved_variant_declaration_{name}"),
            "value = Theme::Ready",
            declarations,
            Schema::record(vec![Field::required(
                "value",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::unit(name)],
                },
            )]),
            expected_source,
            MonExpectation::SchemaRejected {
                code: MonErrorCode::InvalidSchema,
                path: vec![
                    PathSegment::Field("value".into()),
                    PathSegment::Variant(name.into()),
                ],
            },
        ));
    }

    let payload_declaration_literal = "value = Box(inner = 1)";
    let payload_declarations = "Box = | __LoOp Int |\n";
    fixtures.push(fixture(
        "reserved_payload_declaration___LoOp",
        payload_declaration_literal,
        payload_declarations,
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "Box".into(),
                fields: vec![Field::required("__LoOp", SchemaType::Int)],
            },
        )]),
        source_reserved_keyword(in_declarations(payload_declarations, "__LoOp", 1)),
        MonExpectation::SchemaRejected {
            code: MonErrorCode::InvalidSchema,
            path: vec![PathSegment::Field("value".into())],
        },
    ));

    let control_literal = "DecBox = 1, configuration = 2, _configuration = 3".to_owned();
    fixtures.push(fixture(
        "nonreserved_identifier_controls",
        control_literal,
        "",
        Schema::record(vec![
            Field::required("DecBox", SchemaType::Int),
            Field::required("configuration", SchemaType::Int),
            Field::required("_configuration", SchemaType::Int),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("DecBox".into(), Value::Int(1)),
            ("configuration".into(), Value::Int(2)),
            ("_configuration".into(), Value::Int(3)),
        ])),
    ));

    let word_literal =
        "words = WordRoles(truth = true, falsehood = false, absent = none)".to_owned();
    let word_fields = vec![
        Field::required("truth", SchemaType::Bool),
        Field::required("falsehood", SchemaType::Bool),
        Field::required("absent", SchemaType::Optional(Box::new(SchemaType::Int))),
    ];
    fixtures.push(
        fixture(
            "literal_words_keep_value_roles",
            word_literal,
            "WordRoles = |\n    truth Bool,\n    falsehood Bool,\n    absent Int?,\n|\n",
            Schema::record(vec![Field::required(
                "words",
                SchemaType::Struct {
                    name: "WordRoles".into(),
                    fields: word_fields,
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "words".into(),
                Value::Record(vec![
                    ("truth".into(), Value::Bool(true)),
                    ("falsehood".into(), Value::Bool(false)),
                    ("absent".into(), Value::None),
                ]),
            )])),
        )
        .with_source_nominal_type("words", "WordRoles")
        .with_source_optional("words.absent"),
    );
    let quoted_literal = r#"text = "if", truth = "true""#;
    fixtures.push(fixture(
        "quoted_reserved_spellings_remain_string_data",
        quoted_literal,
        "",
        Schema::record(vec![
            Field::required("text", SchemaType::String),
            Field::required("truth", SchemaType::String),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("text".into(), Value::String("if".into())),
            ("truth".into(), Value::String("true".into())),
        ])),
    ));

    let map_value = r#"{"if" = 1, "true" = 2}"#;
    let map_literal = format!("map = {map_value}");
    let map_expected = Value::Record(vec![(
        "map".into(),
        Value::Map(vec![
            (Value::String("if".into()), Value::Int(1)),
            (Value::String("true".into()), Value::Int(2)),
        ]),
    )]);
    let string_keyed_map = fixture(
        "quoted_reserved_spellings_remain_string_map_keys",
        map_literal,
        "",
        Schema::record(vec![Field::required(
            "map",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        )]),
        SourceExpectation::AcceptObserved(map_expected.clone()),
        MonExpectation::Accept(map_expected),
    );
    let string_keyed_map = string_keyed_map
        .with_source_override(format!(
            "MapHolder = | map {{String = Int}} |\ndata = MapHolder(map = {map_value})\n"
        ))
        .with_source_nominal_type("", "MapHolder");
    fixtures.push(string_keyed_map);

    fixtures
}
#[test]
fn source_and_mon_reserved_name_and_word_role_parity() {
    let _guard = lock_counter_test();
    for fixture in reserved_name_fixtures() {
        assert_fixture(fixture);
    }
}
