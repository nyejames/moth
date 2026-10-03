//! Named-entry trivia and boundary parity fixtures.
//!
//! WHAT: owns the named-entry source/MON family.
//! WHY: parser-family cases stay local while using the common fixture assertion contract.
use crate::compiler_frontend::compiler_messages::InvalidExpressionReason;
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, PathSegment, Schema, SchemaType, Value, Variant};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_literal,
    mon_named_entry_error, source_expression_assignment, source_invalid_expression,
};

fn named_entry_fixtures() -> Vec<ParityFixture> {
    let scalar_schema = || Schema::record(vec![Field::required("first", SchemaType::Int)]);
    let mut fixtures = Vec::new();

    for (name, literal) in [
        ("implicit_root_lf", "first\n = 1"),
        ("explicit_root_lf", "(first\n = 1)"),
        ("implicit_root_crlf", "first\r\n = 1"),
        ("implicit_root_cr", "first\r = 1"),
        ("comment_before_equals", "(first -- label comment\r\n= 1)"),
    ] {
        let literal = literal.to_owned();
        fixtures.push(fixture(
            name,
            literal.clone(),
            "first #= 1\n",
            scalar_schema(),
            source_expression_assignment(in_literal(&literal, "=", 1)),
            mon_named_entry_error(&literal, "first", Vec::new()),
        ));
    }
    for (name, literal) in [
        ("later_field_lf", "first = 1, later\n = 2"),
        ("later_field_crlf", "first = 1, later\r\n = 2"),
        ("later_field_cr", "first = 1, later\r = 2"),
        (
            "later_field_line_comment",
            "first = 1, later -- comment\n = 2",
        ),
    ] {
        let literal = literal.to_owned();
        fixtures.push(fixture(
            name,
            literal.clone(),
            "",
            Schema::record(vec![
                Field::required("first", SchemaType::Int),
                Field::required("later", SchemaType::Int),
            ]),
            source_invalid_expression(
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                in_literal(&literal, "later", 1),
            ),
            mon_named_entry_error(&literal, "later", Vec::new()),
        ));
    }

    let nested_after_equals = "outer = (inner =\n1)".to_owned();
    fixtures.push(fixture(
        "nested_record_newline_after_equals",
        nested_after_equals,
        "inner #= 1\n",
        Schema::record(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::Int)],
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "outer".into(),
            Value::Record(vec![("inner".into(), Value::Int(1))]),
        )])),
    ));

    let payload_after_equals = "box = Box(inner =\n1)".to_owned();
    fixtures.push(
        fixture(
            "nominal_payload_newline_after_equals",
            payload_after_equals,
            "Box = | inner Int |\n",
            Schema::record(vec![Field::required(
                "box",
                SchemaType::Struct {
                    name: "Box".into(),
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "box".into(),
                Value::Record(vec![("inner".into(), Value::Int(1))]),
            )])),
        )
        .with_source_nominal_type("box", "Box"),
    );

    let nested_literal = "outer = (inner\n = 1)".to_owned();
    fixtures.push(fixture(
        "nested_record_lf",
        nested_literal.clone(),
        "inner #= 1\n",
        Schema::record(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required("inner", SchemaType::Int)],
            },
        )]),
        source_expression_assignment(in_literal(&nested_literal, "=", 2)),
        mon_named_entry_error(
            &nested_literal,
            "inner",
            vec![PathSegment::Field("outer".into())],
        ),
    ));

    let payload_literal = "box = Box(inner\r\n = 1)".to_owned();
    fixtures.push(fixture(
        "nominal_payload_crlf",
        payload_literal.clone(),
        "inner #= 1\nBox = | inner Int |\n",
        Schema::record(vec![Field::required(
            "box",
            SchemaType::Struct {
                name: "Box".into(),
                fields: vec![Field::required("inner", SchemaType::Int)],
            },
        )]),
        source_expression_assignment(in_literal(&payload_literal, "=", 2)),
        mon_named_entry_error(
            &payload_literal,
            "inner",
            vec![PathSegment::Field("box".into())],
        ),
    ));

    let implicit_positive = "first \t= \n1,\n-- between entries\nlater = 2,\n".to_owned();
    fixtures.push(fixture(
        "horizontal_spacing_newline_after_equals_and_entry_comments",
        implicit_positive,
        "",
        Schema::record(vec![
            Field::required("first", SchemaType::Int),
            Field::required("later", SchemaType::Int),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("first".into(), Value::Int(1)),
            ("later".into(), Value::Int(2)),
        ])),
    ));

    let explicit_positive = "(\nfirst = 1,\nlater \t= \n2,\n)".to_owned();
    fixtures.push(fixture(
        "explicit_root_first_and_later_fields_with_trailing_comma",
        explicit_positive,
        "",
        Schema::record(vec![
            Field::required("first", SchemaType::Int),
            Field::required("later", SchemaType::Int),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("first".into(), Value::Int(1)),
            ("later".into(), Value::Int(2)),
        ])),
    ));

    for (name, literal) in [
        ("implicit_root_one_field", "first = 1"),
        ("explicit_root_one_field", "(first = 1)"),
    ] {
        fixtures.push(fixture(
            name,
            literal,
            "",
            scalar_schema(),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![("first".into(), Value::Int(1))])),
        ));
    }

    let implicit_eof = "first = 1,\n-- final comment at document EOF\n".to_owned();
    fixtures.push(fixture(
        "implicit_root_trailing_comma_and_eof",
        implicit_eof,
        "",
        scalar_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![("first".into(), Value::Int(1))])),
    ));

    for (whitespace_name, whitespace) in [
        ("tab", "\t"),
        ("vertical_tab", "\u{000b}"),
        ("form_feed", "\u{000c}"),
        ("nel", "\u{0085}"),
        ("line_separator", "\u{2028}"),
        ("paragraph_separator", "\u{2029}"),
    ] {
        let first_literal = format!("first{whitespace} = 1");
        fixtures.push(fixture(
            format!("first_field_before_equals_{whitespace_name}"),
            first_literal,
            "",
            scalar_schema(),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![("first".into(), Value::Int(1))])),
        ));

        let later_literal = format!("first = 1, later{whitespace} = 2");
        fixtures.push(fixture(
            format!("later_field_before_equals_{whitespace_name}"),
            later_literal,
            "",
            Schema::record(vec![
                Field::required("first", SchemaType::Int),
                Field::required("later", SchemaType::Int),
            ]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![
                ("first".into(), Value::Int(1)),
                ("later".into(), Value::Int(2)),
            ])),
        ));

        let nested_literal = format!("outer = (inner{whitespace} = 1)");
        fixtures.push(fixture(
            format!("nested_record_before_equals_{whitespace_name}"),
            nested_literal,
            "",
            Schema::record(vec![Field::required(
                "outer",
                SchemaType::Record {
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "outer".into(),
                Value::Record(vec![("inner".into(), Value::Int(1))]),
            )])),
        ));

        let nominal_literal = format!("box = Box(inner{whitespace} = 1)");
        fixtures.push(
            fixture(
                format!("nominal_payload_label_before_equals_{whitespace_name}"),
                nominal_literal,
                "Box = | inner Int |\n",
                Schema::record(vec![Field::required(
                    "box",
                    SchemaType::Struct {
                        name: "Box".into(),
                        fields: vec![Field::required("inner", SchemaType::Int)],
                    },
                )]),
                SourceExpectation::Accept,
                MonExpectation::Accept(Value::Record(vec![(
                    "box".into(),
                    Value::Record(vec![("inner".into(), Value::Int(1))]),
                )])),
            )
            .with_source_nominal_type("box", "Box"),
        );

        let choice_literal = format!("status = Theme::Pair(first{whitespace} = 1, second = 2)");
        fixtures.push(fixture(
            format!("choice_payload_label_before_equals_{whitespace_name}"),
            choice_literal,
            "Theme ::\n    Ready,\n    Pair | first Int, second Int |,\n;\n",
            Schema::record(vec![Field::required(
                "status",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![
                        Variant::unit("Ready"),
                        Variant::payload(
                            "Pair",
                            vec![
                                Field::required("first", SchemaType::Int),
                                Field::required("second", SchemaType::Int),
                            ],
                        ),
                    ],
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "status".into(),
                Value::Choice {
                    qualifier: Some("Theme".into()),
                    variant: "Pair".into(),
                    fields: vec![
                        ("first".into(), Value::Int(1)),
                        ("second".into(), Value::Int(2)),
                    ],
                },
            )])),
        ));
    }

    fixtures
}

#[test]
fn source_and_mon_named_entry_line_boundary_parity() {
    let _guard = lock_counter_test();
    for fixture in named_entry_fixtures() {
        assert_fixture(fixture);
    }
}
