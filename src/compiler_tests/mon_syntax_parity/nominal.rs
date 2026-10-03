//! Nominal record-construction parity fixtures.
//!
//! WHAT: owns explicit qualifier, payload routing and nested nominal-value cases.
//! WHY: equal record shapes do not replace the receiving type's nominal identity.

use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, TypeMismatchContext,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};

use super::support::{
    CallShapeExpectation, MonExpectation, ParityFixture, SourceExpectation, SourceReason,
    assert_fixture, fixture, in_declarations, in_literal, mon_span,
    source_named_argument_not_found,
};

fn box_fields() -> Vec<Field> {
    vec![
        Field::required("first", SchemaType::Int),
        Field::required("second", SchemaType::Int),
    ]
}

fn box_declaration() -> &'static str {
    "Box = |\n    first Int,\n    second Int,\n|\n"
}

fn box_schema() -> Schema {
    Schema::record(vec![Field::required(
        "box",
        SchemaType::Struct {
            name: "Box".into(),
            fields: box_fields(),
        },
    )])
}

fn nominal_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    let expected_box = Value::Record(vec![(
        "box".into(),
        Value::Record(vec![
            ("first".into(), Value::Int(1)),
            ("second".into(), Value::Int(2)),
        ]),
    )]);

    for (name, literal) in [
        (
            "matching_explicit_qualifier",
            "box = Box(first = 1, second = 2)",
        ),
        (
            "positional_then_named_payload_arguments",
            "box = Box(1, second = 2)",
        ),
        (
            "reordered_named_payload_arguments",
            "box = Box(second = 2, first = 1)",
        ),
    ] {
        fixtures.push(
            fixture(
                name,
                literal,
                box_declaration(),
                box_schema(),
                SourceExpectation::Accept,
                MonExpectation::Accept(expected_box.clone()),
            )
            .with_source_nominal_type("box", "Box"),
        );
    }

    for (layout_name, spacing) in [("space", " "), ("tab", "\t")] {
        let literal = format!("box = Box{spacing}(first = 1, second = 2)");
        fixtures.push(
            fixture(
                format!("constructor_head_same_line_{layout_name}"),
                literal,
                box_declaration(),
                box_schema(),
                SourceExpectation::Accept,
                MonExpectation::Accept(expected_box.clone()),
            )
            .with_source_nominal_type("box", "Box"),
        );
    }

    for (layout_name, separator) in [("lf", "\n"), ("crlf", "\r\n"), ("line_comment", " -- c\n")] {
        let literal = format!("box = Box{separator}(first = 1, second = 2)");
        fixtures.push(fixture(
            format!("constructor_head_separated_{layout_name}"),
            literal.clone(),
            box_declaration(),
            box_schema(),
            SourceExpectation::Reject {
                code: "MOTH-RULE-0053",
                reason: SourceReason::CompileTimeEvaluation(
                    CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
                ),
                site: in_literal(&literal, "Box", 1),
            },
            MonExpectation::Reject {
                code: MonErrorCode::UnexpectedToken,
                span: mon_span(&literal, "Box", 1),
                path: vec![PathSegment::Field("box".into())],
            },
        ));
    }

    // Source constructs the declared type; MON checks the qualifier against the supplied schema.
    let wrong_qualifier = "box = Other(first = 1, second = 2)".to_owned();
    fixtures.push(
        fixture(
            "wrong_nominal_qualifier_with_identical_fields",
            wrong_qualifier.clone(),
            concat!(
                "Box = |\n    first Int,\n    second Int,\n|\n",
                "Other = |\n    first Int,\n    second Int,\n|\n",
            ),
            box_schema(),
            SourceExpectation::AcceptObserved(expected_box.clone()),
            MonExpectation::Reject {
                code: MonErrorCode::QualifierMismatch,
                span: mon_span(&wrong_qualifier, "Other", 1),
                path: vec![PathSegment::Field("box".into())],
            },
        )
        .with_source_nominal_type("box", "Other")
        .with_outcome_difference("schema-versus-source-identity"),
    );
    let typed_declarations = concat!(
        "Box = |\n    first Int,\n    second Int,\n|\n",
        "Other = |\n    first Int,\n    second Int,\n|\n",
        "Holder = | box Box |\n",
    );
    let typed_source = format!("{typed_declarations}data = Holder({wrong_qualifier})\n");
    fixtures.push(
        fixture(
            "wrong_nominal_qualifier_rejected_by_typed_source_receiver",
            wrong_qualifier.clone(),
            typed_declarations,
            box_schema(),
            SourceExpectation::Reject {
                code: "MOTH-TYPE-0001",
                reason: SourceReason::TypeMismatch(TypeMismatchContext::ConstructorArgument),
                site: in_declarations(&typed_source, "Other", 2),
            },
            MonExpectation::Reject {
                code: MonErrorCode::QualifierMismatch,
                span: mon_span(&wrong_qualifier, "Other", 1),
                path: vec![PathSegment::Field("box".into())],
            },
        )
        .with_source_override(typed_source),
    );

    let nested_literal = "parent = Parent(child = Child(code = 7))";
    fixtures.push(
        fixture(
            "nested_qualified_nominal_children",
            nested_literal,
            concat!("Child = | code Int |\n", "Parent = | child Child |\n",),
            Schema::record(vec![Field::required(
                "parent",
                SchemaType::Struct {
                    name: "Parent".into(),
                    fields: vec![Field::required(
                        "child",
                        SchemaType::Struct {
                            name: "Child".into(),
                            fields: vec![Field::required("code", SchemaType::Int)],
                        },
                    )],
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "parent".into(),
                Value::Record(vec![(
                    "child".into(),
                    Value::Record(vec![("code".into(), Value::Int(7))]),
                )]),
            )])),
        )
        .with_source_nominal_type("parent", "Parent")
        .with_source_nominal_type("parent.child", "Child"),
    );

    let duplicate = "box = Box(1, first = 2, second = 3)".to_owned();
    fixtures.push(fixture(
        "duplicate_positional_and_named_nominal_slot",
        duplicate.clone(),
        box_declaration(),
        box_schema(),
        SourceExpectation::Reject {
            code: "MOTH-RULE-0054",
            reason: SourceReason::CallShape(CallShapeExpectation::DuplicateArgument {
                parameter_index: 0,
            }),
            site: in_literal(&duplicate, "first", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::DuplicateField,
            span: mon_span(&duplicate, "first", 1),
            path: vec![
                PathSegment::Field("box".into()),
                PathSegment::Field("first".into()),
            ],
        },
    ));

    let unknown = "box = Box(first = 1, mystery = 2)".to_owned();
    fixtures.push(fixture(
        "unknown_nominal_field",
        unknown.clone(),
        box_declaration(),
        box_schema(),
        source_named_argument_not_found(in_literal(&unknown, "mystery", 1)),
        MonExpectation::Reject {
            code: MonErrorCode::UnknownField,
            span: mon_span(&unknown, "mystery", 1),
            path: vec![
                PathSegment::Field("box".into()),
                PathSegment::Field("mystery".into()),
            ],
        },
    ));

    let wrong_arity = "box = Box(1, 2, 3)".to_owned();
    fixtures.push(fixture(
        "nominal_constructor_rejects_extra_positional_argument",
        wrong_arity.clone(),
        box_declaration(),
        box_schema(),
        SourceExpectation::Reject {
            code: "MOTH-RULE-0054",
            reason: SourceReason::CallShape(CallShapeExpectation::ExtraPositional {
                expected_count: 2,
            }),
            site: in_literal(&wrong_arity, "3", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::Arity,
            span: mon_span(&wrong_arity, "3", 1),
            path: vec![PathSegment::Field("box".into())],
        },
    ));

    fixtures
}

#[test]
fn source_and_mon_nominal_construction_parity() {
    let _guard = lock_counter_test();
    for fixture in nominal_fixtures() {
        assert_fixture(fixture);
    }
}
