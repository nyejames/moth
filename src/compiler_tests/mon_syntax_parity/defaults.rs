//! Declared-default and option parity fixtures.
//!
//! WHAT: checks named struct constructor defaults against MON schema completion.
//! WHY: both receivers own field shape, so omission and nested completion are meaningful here.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Span, Value};

use super::support::{
    CallShapeExpectation, MonExpectation, ParityFixture, SourceExpectation, SourceReason,
    assert_fixture, fixture, in_literal,
};

fn settings_fields() -> Vec<Field> {
    vec![
        Field::with_default("name", SchemaType::String, Value::String("guest".into())),
        Field::with_default("retries", SchemaType::U8, Value::U8(3)),
        Field::with_default("enabled", SchemaType::Bool, Value::Bool(true)),
        Field::with_default(
            "tags",
            SchemaType::Collection {
                element: Box::new(SchemaType::String),
            },
            Value::Collection(vec![Value::String("base".into())]),
        ),
        Field::with_default(
            "nickname",
            SchemaType::Optional(Box::new(SchemaType::String)),
            Value::None,
        ),
    ]
}

fn inner_schema() -> SchemaType {
    SchemaType::Struct {
        name: "Inner".into(),
        fields: vec![
            Field::with_default("count", SchemaType::Int, Value::Int(1)),
            Field::with_default(
                "note",
                SchemaType::String,
                Value::String("inner default".into()),
            ),
        ],
    }
}
fn outer_schema() -> SchemaType {
    SchemaType::Struct {
        name: "Outer".into(),
        fields: vec![
            Field::with_default("name", SchemaType::String, Value::String("guest".into())),
            Field::with_default(
                "inner",
                inner_schema(),
                Value::Record(vec![
                    ("count".into(), Value::Int(2)),
                    ("note".into(), Value::String("parent default".into())),
                ]),
            ),
        ],
    }
}

fn default_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    let settings_declarations = concat!(
        "Settings = |\n",
        "    name String = \"guest\",\n",
        "    retries U8 = 3,\n",
        "    enabled Bool = true,\n",
        "    tags {String} = {\"base\"},\n",
        "    nickname String? = none,\n",
        "|\n"
    );
    let settings = fixture(
        "missing_fields_insert_matching_declared_defaults",
        "settings = Settings()",
        settings_declarations,
        Schema::record(vec![Field::required(
            "settings",
            SchemaType::Struct {
                name: "Settings".into(),
                fields: settings_fields(),
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "settings".into(),
            Value::Record(vec![
                ("name".into(), Value::String("guest".into())),
                ("retries".into(), Value::U8(3)),
                ("enabled".into(), Value::Bool(true)),
                (
                    "tags".into(),
                    Value::Collection(vec![Value::String("base".into())]),
                ),
                ("nickname".into(), Value::None),
            ]),
        )])),
    )
    .with_source_nominal_type("settings", "Settings")
    .with_source_optional("settings.nickname");
    fixtures.push(settings);

    let required_declarations = "Required = | nickname String? |\n";
    let required_literal = "required = Required()";
    fixtures.push(fixture(
        "optional_field_without_default_is_required",
        required_literal,
        required_declarations,
        Schema::record(vec![Field::required(
            "required",
            SchemaType::Struct {
                name: "Required".into(),
                fields: vec![Field::required(
                    "nickname",
                    SchemaType::Optional(Box::new(SchemaType::String)),
                )],
            },
        )]),
        SourceExpectation::Reject {
            code: "MOTH-RULE-0054",
            reason: SourceReason::CallShape(CallShapeExpectation::MissingArgument {
                parameter_index: 0,
            }),
            site: in_literal(required_literal, "Required", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::MissingField,
            span: Span { start: 11, end: 21 },
            path: vec![
                PathSegment::Field("required".into()),
                PathSegment::Field("nickname".into()),
            ],
        },
    ));

    let nested_declarations = concat!(
        "Inner = | count Int = 1, note String = \"inner default\" |\n",
        "Outer = | name String = \"guest\", ",
        "inner Inner = Inner(count = 2, note = \"parent default\") |\n",
    );
    let nested_literal = "outer = Outer(inner = Inner(count = 9))";
    let nested_source = format!(
        "{nested_declarations}\
         Data = | outer Outer |\n\
         data = Data({nested_literal})\n"
    );
    fixtures.push(
        fixture(
            "supplied_nested_records_are_not_deep_merged",
            nested_literal,
            nested_declarations,
            Schema::record(vec![Field::required("outer", outer_schema())]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "outer".into(),
                Value::Record(vec![
                    ("name".into(), Value::String("guest".into())),
                    (
                        "inner".into(),
                        Value::Record(vec![
                            ("count".into(), Value::Int(9)),
                            ("note".into(), Value::String("inner default".into())),
                        ]),
                    ),
                ]),
            )])),
        )
        .with_source_override(nested_source)
        .with_source_nominal_type("", "Data")
        .with_source_nominal_type("outer", "Outer")
        .with_source_nominal_type("outer.inner", "Inner"),
    );

    let explicit_none_literal = "profile = Profile(nickname = none)";
    let profile_declarations = "Profile = | nickname String?, label String = \"member\" |\n";
    let profile_schema = || {
        Schema::record(vec![Field::required(
            "profile",
            SchemaType::Struct {
                name: "Profile".into(),
                fields: vec![
                    Field::required(
                        "nickname",
                        SchemaType::Optional(Box::new(SchemaType::String)),
                    ),
                    Field::with_default(
                        "label",
                        SchemaType::String,
                        Value::String("member".into()),
                    ),
                ],
            },
        )])
    };
    for (name, literal, nickname) in [
        (
            "explicit_none_fills_optional_field_beside_inserted_default",
            explicit_none_literal,
            Value::None,
        ),
        (
            "present_optional_value_keeps_its_optional_layer",
            "profile = Profile(nickname = \"Priya\")",
            Value::String("Priya".into()),
        ),
    ] {
        fixtures.push(
            fixture(
                name,
                literal,
                profile_declarations,
                profile_schema(),
                SourceExpectation::Accept,
                MonExpectation::Accept(Value::Record(vec![(
                    "profile".into(),
                    Value::Record(vec![
                        ("nickname".into(), nickname),
                        ("label".into(), Value::String("member".into())),
                    ]),
                )])),
            )
            .with_source_nominal_type("profile", "Profile")
            .with_source_optional("profile.nickname"),
        );
    }

    let optional_default_declarations = "Profile = | nickname String? = \"guest\" |\n";
    let optional_default_schema = || {
        Schema::record(vec![Field::required(
            "profile",
            SchemaType::Struct {
                name: "Profile".into(),
                fields: vec![Field::with_default(
                    "nickname",
                    SchemaType::Optional(Box::new(SchemaType::String)),
                    Value::String("guest".into()),
                )],
            },
        )])
    };
    for (name, literal, nickname) in [
        (
            "same_field_optional_default_omission_uses_present_default",
            "profile = Profile()",
            Value::String("guest".into()),
        ),
        (
            "same_field_optional_default_explicit_none_remains_absent",
            "profile = Profile(nickname = none)",
            Value::None,
        ),
        (
            "same_field_optional_default_present_string_replaces_default",
            "profile = Profile(nickname = \"Priya\")",
            Value::String("Priya".into()),
        ),
        (
            "same_field_optional_default_empty_string_replaces_default",
            "profile = Profile(nickname = \"\")",
            Value::String("".into()),
        ),
    ] {
        fixtures.push(
            fixture(
                name,
                literal,
                optional_default_declarations,
                optional_default_schema(),
                SourceExpectation::Accept,
                MonExpectation::Accept(Value::Record(vec![(
                    "profile".into(),
                    Value::Record(vec![("nickname".into(), nickname)]),
                )])),
            )
            .with_source_nominal_type("profile", "Profile")
            .with_source_optional("profile.nickname"),
        );
    }

    let rob_source_override = concat!(
        "Profile = | nickname String? = \"guest\" |\n",
        "rob #String? = \"Rob\"\n",
        "data #= (profile = Profile(nickname = rob))\n",
    );
    fixtures.push(
        fixture(
            "same_field_optional_default_source_override_uses_typed_rob_binding",
            "profile = Profile(nickname = \"Rob\")",
            optional_default_declarations,
            optional_default_schema(),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "profile".into(),
                Value::Record(vec![("nickname".into(), Value::String("Rob".into()))]),
            )])),
        )
        .with_source_override(rob_source_override)
        .with_source_nominal_type("profile", "Profile")
        .with_source_optional("profile.nickname"),
    );

    fixtures
}

#[test]
fn named_struct_receivers_apply_exact_declared_defaults() {
    let _guard = lock_counter_test();
    for fixture in default_fixtures() {
        assert_fixture(fixture);
    }
}
