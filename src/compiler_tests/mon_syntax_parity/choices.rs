//! Choice-data parity fixtures.
//!
//! WHAT: owns qualified choice values, payload routing and checked receiver identity.
//! WHY: source and MON share explicit variant construction while MON also validates a schema.

use crate::compiler_frontend::compiler_messages::{
    InvalidChoiceVariantReason, TypeMismatchContext,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value, Variant};

use super::support::{
    CallShapeExpectation, MonExpectation, ParityFixture, SourceExpectation, SourceReason,
    assert_fixture, fixture, in_declarations, in_literal, mon_span, source_unknown_variant,
};

fn theme_variants() -> Vec<Variant> {
    vec![
        Variant::unit("Ready"),
        Variant::payload(
            "Pair",
            vec![
                Field::required("first", SchemaType::Int),
                Field::required("second", SchemaType::Int),
            ],
        ),
    ]
}

fn theme_declaration() -> &'static str {
    "Theme ::\n    Ready,\n    Pair | first Int, second Int |,\n;\n"
}

fn theme_schema() -> Schema {
    Schema::record(vec![Field::required(
        "status",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: theme_variants(),
        },
    )])
}

fn choice_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    fixtures.push(fixture(
        "qualified_unit_choice_variant",
        "status = Theme::Ready",
        theme_declaration(),
        theme_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "status".into(),
            Value::Choice {
                qualifier: Some("Theme".into()),
                variant: "Ready".into(),
                fields: Vec::new(),
            },
        )])),
    ));

    fixtures.push(fixture(
        "qualified_payload_variant_with_reordered_named_fields",
        "status = Theme::Pair(second = 2, first = 1)",
        theme_declaration(),
        theme_schema(),
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

    let expected_pair = Value::Record(vec![(
        "status".into(),
        Value::Choice {
            qualifier: Some("Theme".into()),
            variant: "Pair".into(),
            fields: vec![
                ("first".into(), Value::Int(1)),
                ("second".into(), Value::Int(2)),
            ],
        },
    )]);

    for (layout_name, spacing) in [("space", " "), ("tab", "\t")] {
        let literal = format!("status = Theme::Pair{spacing}(first = 1, second = 2)");
        fixtures.push(fixture(
            format!("choice_payload_head_same_line_{layout_name}"),
            literal,
            theme_declaration(),
            theme_schema(),
            SourceExpectation::Accept,
            MonExpectation::Accept(expected_pair.clone()),
        ));
    }

    for (layout_name, separator, line_break) in [
        ("lf", "\n", "\n"),
        ("crlf", "\r\n", "\r\n"),
        ("line_comment", " -- c\n", "\n"),
    ] {
        let literal = format!("status = Theme::Pair{separator}(first = 1, second = 2)");
        fixtures.push(fixture(
            format!("choice_payload_head_separated_{layout_name}"),
            literal.clone(),
            theme_declaration(),
            theme_schema(),
            SourceExpectation::Reject {
                code: "MOTH-RULE-0029",
                reason: SourceReason::InvalidChoiceVariant(
                    InvalidChoiceVariantReason::PayloadVariantMissingArguments,
                ),
                site: in_literal(&literal, line_break, 1),
            },
            MonExpectation::Reject {
                code: MonErrorCode::MissingComma,
                span: mon_span(&literal, "(", 1),
                path: Vec::new(),
            },
        ));
    }

    let unknown = "status = Theme::Missing".to_owned();
    fixtures.push(fixture(
        "unknown_choice_variant",
        unknown.clone(),
        theme_declaration(),
        theme_schema(),
        source_unknown_variant(in_literal(&unknown, "Missing", 1)),
        MonExpectation::Reject {
            code: MonErrorCode::UnknownVariant,
            span: mon_span(&unknown, "Missing", 1),
            path: vec![
                PathSegment::Field("status".into()),
                PathSegment::Variant("Missing".into()),
            ],
        },
    ));

    let duplicate = "status = Theme::Pair(first = 1, first = 2, second = 3)".to_owned();
    fixtures.push(fixture(
        "duplicate_choice_payload_fields",
        duplicate.clone(),
        theme_declaration(),
        theme_schema(),
        SourceExpectation::Reject {
            code: "MOTH-RULE-0054",
            reason: SourceReason::CallShape(CallShapeExpectation::DuplicateArgument {
                parameter_index: 0,
            }),
            site: in_literal(&duplicate, "first", 2),
        },
        MonExpectation::Reject {
            code: MonErrorCode::DuplicateArgument,
            span: mon_span(&duplicate, "first", 2),
            path: vec![
                PathSegment::Field("status".into()),
                PathSegment::Variant("Pair".into()),
                PathSegment::Field("first".into()),
            ],
        },
    ));

    // Source builds its declared choice; MON checks the qualifier against the supplied schema.
    let wrong_qualifier = "status = OtherTheme::Ready".to_owned();
    fixtures.push(
        fixture(
            "wrong_choice_qualifier_with_matching_variant_shape",
            wrong_qualifier.clone(),
            "Theme :: Ready;\nOtherTheme :: Ready;\n",
            theme_schema(),
            SourceExpectation::AcceptObserved(Value::Record(vec![(
                "status".into(),
                Value::Choice {
                    qualifier: Some("OtherTheme".into()),
                    variant: "Ready".into(),
                    fields: Vec::new(),
                },
            )])),
            MonExpectation::Reject {
                code: MonErrorCode::QualifierMismatch,
                span: mon_span(&wrong_qualifier, "OtherTheme", 1),
                path: vec![PathSegment::Field("status".into())],
            },
        )
        .with_outcome_difference("schema-versus-source-identity"),
    );
    let typed_declarations = "Theme :: Ready;\nOtherTheme :: Ready;\nHolder = | status Theme |\n";
    let typed_source = format!("{typed_declarations}data = Holder({wrong_qualifier})\n");
    fixtures.push(
        fixture(
            "wrong_choice_qualifier_rejected_by_typed_source_receiver",
            wrong_qualifier.clone(),
            typed_declarations,
            theme_schema(),
            SourceExpectation::Reject {
                code: "MOTH-TYPE-0001",
                reason: SourceReason::TypeMismatch(TypeMismatchContext::ConstructorArgument),
                site: in_declarations(&typed_source, "Ready", 3),
            },
            MonExpectation::Reject {
                code: MonErrorCode::QualifierMismatch,
                span: mon_span(&wrong_qualifier, "OtherTheme", 1),
                path: vec![PathSegment::Field("status".into())],
            },
        )
        .with_source_override(typed_source),
    );

    fixtures
}

#[test]
fn source_and_mon_choice_data_parity() {
    let _guard = lock_counter_test();
    for fixture in choice_fixtures() {
        assert_fixture(fixture);
    }
}
