//! Small deterministic combinations of compatible source and MON syntax.
//!
//! WHAT: varies named-entry spacing, nested record depth, explicit qualifiers and numeric text.
//! WHY: bounded combinations catch cross-boundary interactions without turning into fuzzing.

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompileTimeEvaluationErrorReason, InvalidChoiceVariantReason,
    InvalidExpressionReason, MissingWhitespace, SymbolicSpacingConstruct, SymbolicSpacingError,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{
    Field, MonErrorCode, NumericProfile, PathSegment, Schema, SchemaType, Value, Variant,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth};

use super::support::{
    MonExpectation, SourceExpectation, SourceReason, assert_fixture, fixture, in_declarations,
    in_literal, mon_named_entry_error, mon_numeric_error, mon_span, source_expression_assignment,
    source_invalid_expression, source_invalid_number,
};

#[test]
fn bounded_syntax_perturbations_keep_typed_parity() {
    let _guard = lock_counter_test();
    let profile = NumericProfile::STANDARD;
    let equal_spacing = [" ", "\t", "\n"];
    let comma_spacing = [" ", "\t", "\n"];
    let integer_literals = [("1", 1), ("1_0", 10)];
    let float_literals = [
        ("1.5", 0x3ff8_0000_0000_0000),
        ("1e0", 0x3ff0_0000_0000_0000),
    ];
    let mut fixtures = Vec::new();

    let flat_schema = Schema::record(vec![
        Field::required("first", SchemaType::Int),
        Field::required("second", SchemaType::Float),
    ]);
    for (equal_index, after_equal) in equal_spacing.iter().enumerate() {
        for (comma_index, after_comma) in comma_spacing.iter().enumerate() {
            for (integer_text, integer_value) in integer_literals {
                for (float_text, float_bits) in float_literals {
                    let literal = format!(
                        "first ={after_equal}{integer_text},{after_comma}second ={after_equal}{float_text}"
                    );
                    fixtures.push(
                        fixture(
                            format!("flat_eq_{equal_index}_comma_{comma_index}_{literal:?}"),
                            literal,
                            "",
                            flat_schema.clone(),
                            SourceExpectation::Accept,
                            MonExpectation::Accept(Value::Record(vec![
                                ("first".into(), Value::Int(integer_value)),
                                ("second".into(), Value::Float(f64::from_bits(float_bits))),
                            ])),
                        )
                        .with_profile(profile),
                    );
                }
            }
        }
    }

    let nested_integer = ("1_0", 10);
    for depth in 0..=2 {
        let nested_schema =
            Schema::record(vec![Field::required("value", nested_record_type(depth))]);
        for (equal_index, after_equal) in equal_spacing.iter().enumerate() {
            let nested_literal = nested_record_literal(depth, after_equal, nested_integer.0);
            let literal = format!("value ={after_equal}{nested_literal}");
            fixtures.push(
                fixture(
                    format!("nested_depth_{depth}_eq_{equal_index}_{literal:?}"),
                    literal,
                    "",
                    nested_schema.clone(),
                    SourceExpectation::Accept,
                    MonExpectation::Accept(Value::Record(vec![(
                        "value".into(),
                        nested_record_value(depth, nested_integer.1),
                    )])),
                )
                .with_profile(profile),
            );
        }
    }

    let point_schema = Schema::record(vec![Field::required(
        "point",
        SchemaType::Struct {
            name: "Point".into(),
            fields: vec![Field::required("x", SchemaType::Int)],
        },
    )]);
    for (spacing_index, after_equal) in equal_spacing.iter().take(2).enumerate() {
        for (number_text, number_value) in integer_literals {
            let literal = format!("point = Point(x ={after_equal}{number_text})");
            fixtures.push(
                fixture(
                    format!("point_qualifier_{spacing_index}_{literal:?}"),
                    literal,
                    "Point = | x Int |\n",
                    point_schema.clone(),
                    SourceExpectation::Accept,
                    MonExpectation::Accept(Value::Record(vec![(
                        "point".into(),
                        Value::Record(vec![("x".into(), Value::Int(number_value))]),
                    )])),
                )
                .with_profile(profile)
                .with_source_nominal_type("point", "Point"),
            );
        }
    }

    let choice_declarations = "Theme ::\n    Ready,\n    Selected | value Int |,\n;\n";
    let choice_schema = Schema::record(vec![Field::required(
        "selection",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Ready"),
                Variant::payload("Selected", vec![Field::required("value", SchemaType::Int)]),
            ],
        },
    )]);
    for (spacing_index, after_equal) in equal_spacing.iter().take(2).enumerate() {
        for (number_text, number_value) in integer_literals {
            let literal = format!("selection = Theme::Selected(value ={after_equal}{number_text})");
            fixtures.push(
                fixture(
                    format!("choice_qualifier_{spacing_index}_{literal:?}"),
                    literal,
                    choice_declarations,
                    choice_schema.clone(),
                    SourceExpectation::Accept,
                    MonExpectation::Accept(Value::Record(vec![(
                        "selection".into(),
                        Value::Choice {
                            qualifier: Some("Theme".into()),
                            variant: "Selected".into(),
                            fields: vec![("value".into(), Value::Int(number_value))],
                        },
                    )])),
                )
                .with_profile(profile),
            );
        }
    }
    fixtures.push(
        fixture(
            "choice_unit_qualifier_Theme::Ready",
            "selection = Theme::Ready",
            choice_declarations,
            choice_schema,
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "selection".into(),
                Value::Choice {
                    qualifier: Some("Theme".into()),
                    variant: "Ready".into(),
                    fields: Vec::new(),
                },
            )])),
        )
        .with_profile(profile),
    );

    assert_eq!(
        fixtures.len(),
        54,
        "the deterministic perturbation corpus must remain bounded"
    );
    for fixture in fixtures {
        assert_fixture(fixture);
    }
}

#[derive(Clone, Copy)]
struct TriviaClass {
    name: &'static str,
    text: &'static str,
    source_line_break: Option<&'static str>,
}

const TRIVIA_CLASSES: [TriviaClass; 13] = [
    TriviaClass {
        name: "none",
        text: "",
        source_line_break: None,
    },
    TriviaClass {
        name: "space",
        text: " ",
        source_line_break: None,
    },
    TriviaClass {
        name: "tab",
        text: "\t",
        source_line_break: None,
    },
    TriviaClass {
        name: "lf",
        text: "\n",
        source_line_break: Some("\n"),
    },
    TriviaClass {
        name: "cr",
        text: "\r",
        source_line_break: Some("\r"),
    },
    TriviaClass {
        name: "crlf",
        text: "\r\n",
        source_line_break: Some("\r\n"),
    },
    TriviaClass {
        name: "line_comment",
        text: " -- matrix comment\n",
        source_line_break: Some("\n"),
    },
    TriviaClass {
        name: "vertical_tab",
        text: "\u{000b}",
        source_line_break: None,
    },
    TriviaClass {
        name: "form_feed",
        text: "\u{000c}",
        source_line_break: None,
    },
    TriviaClass {
        name: "nel",
        text: "\u{0085}",
        source_line_break: None,
    },
    TriviaClass {
        name: "line_separator",
        text: "\u{2028}",
        source_line_break: None,
    },
    TriviaClass {
        name: "paragraph_separator",
        text: "\u{2029}",
        source_line_break: None,
    },
    TriviaClass {
        name: "nbsp",
        text: "\u{00a0}",
        source_line_break: None,
    },
];

#[test]
fn logical_layout_boundary_matrix_keeps_typed_parity() {
    let _guard = lock_counter_test();
    let mut fixtures = Vec::new();

    for trivia in TRIVIA_CLASSES {
        fixtures.extend([
            first_field_before_equals(trivia),
            later_field_before_equals(trivia),
            nested_field_before_equals(trivia),
            nominal_field_before_equals(trivia),
            choice_field_before_equals(trivia),
            after_equals(trivia),
            before_comma(trivia),
            after_comma(trivia),
            after_record_opener(trivia),
            before_record_closer(trivia),
            nominal_head_gap(trivia),
            choice_head_gap(trivia),
        ]);
    }

    assert_eq!(
        fixtures.len(),
        TRIVIA_CLASSES.len() * 12,
        "the explicit boundary table covers each trivia class at every selected position and receiver"
    );
    for fixture in fixtures {
        assert_fixture(fixture);
    }
}

struct NamedSpacingCase {
    context: &'static str,
    prefix: &'static str,
    field: &'static str,
    suffix: &'static str,
    declarations: &'static str,
    equals_occurrence: usize,
    value: &'static str,
    schema: Schema,
    path: Vec<PathSegment>,
}

struct MapSpacingCase {
    position: &'static str,
    prefix: &'static str,
    key: &'static str,
    literal_equals: usize,
    source_equals: usize,
    value: &'static str,
    path: Vec<PathSegment>,
}

#[test]
fn missing_equals_spacing_is_rejected_by_source_and_mon() {
    let _guard = lock_counter_test();
    let named_cases = [
        NamedSpacingCase {
            context: "first",
            prefix: "",
            field: "first",
            suffix: "",
            declarations: "",
            equals_occurrence: 1,
            value: "1",
            schema: Schema::record(vec![Field::required("first", SchemaType::Int)]),
            path: vec![PathSegment::Field("first".into())],
        },
        NamedSpacingCase {
            context: "later",
            prefix: "first = 10, ",
            field: "later",
            suffix: "",
            declarations: "",
            equals_occurrence: 2,
            value: "2",
            schema: Schema::record(vec![
                Field::required("first", SchemaType::Int),
                Field::required("later", SchemaType::Int),
            ]),
            path: vec![PathSegment::Field("later".into())],
        },
        NamedSpacingCase {
            context: "nested",
            prefix: "outer = (",
            field: "inner",
            suffix: ")",
            declarations: "",
            equals_occurrence: 2,
            value: "3",
            schema: Schema::record(vec![Field::required(
                "outer",
                SchemaType::Record {
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            path: vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("inner".into()),
            ],
        },
        NamedSpacingCase {
            context: "nominal",
            prefix: "box = Box(",
            field: "inner",
            suffix: ")",
            declarations: "Box = | inner Int |\n",
            equals_occurrence: 2,
            value: "4",
            schema: Schema::record(vec![Field::required(
                "box",
                SchemaType::Struct {
                    name: "Box".into(),
                    fields: vec![Field::required("inner", SchemaType::Int)],
                },
            )]),
            path: vec![
                PathSegment::Field("box".into()),
                PathSegment::Field("inner".into()),
            ],
        },
        NamedSpacingCase {
            context: "choice_payload",
            prefix: "state = Theme::Pair(",
            field: "inner",
            suffix: ")",
            declarations: "Theme ::\n    Pair | inner Int |,\n;\n",
            equals_occurrence: 2,
            value: "5",
            schema: Schema::record(vec![Field::required(
                "state",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![Variant::payload(
                        "Pair",
                        vec![Field::required("inner", SchemaType::Int)],
                    )],
                },
            )]),
            path: vec![
                PathSegment::Field("state".into()),
                PathSegment::Variant("Pair".into()),
                PathSegment::Field("inner".into()),
            ],
        },
    ];
    let mut fixtures = Vec::new();

    for case in named_cases {
        for spelling in SHARED_EQUALS_SPELLINGS {
            fixtures.push(named_spacing_fixture(&case, spelling));
        }
    }

    let map_cases = [
        MapSpacingCase {
            position: "first",
            prefix: "",
            key: "\"alpha\"",
            literal_equals: 2,
            source_equals: 5,
            value: "1",
            path: vec![PathSegment::Field("items".into())],
        },
        MapSpacingCase {
            position: "later",
            prefix: "\"alpha\" = 10, ",
            key: "\"beta\"",
            literal_equals: 3,
            source_equals: 6,
            value: "2",
            path: vec![PathSegment::Field("items".into()), PathSegment::Index(1)],
        },
    ];
    for case in map_cases {
        for spelling in SHARED_EQUALS_SPELLINGS
            .iter()
            .chain(&MAP_KEY_GAP_EQUALS_SPELLINGS)
        {
            fixtures.push(map_spacing_fixture(&case, *spelling));
        }
    }

    assert_eq!(
        fixtures.len(),
        5 * SHARED_EQUALS_SPELLINGS.len()
            + 2 * (SHARED_EQUALS_SPELLINGS.len() + MAP_KEY_GAP_EQUALS_SPELLINGS.len()),
        "each named-entry context and map-entry position covers every missing-side spelling"
    );
    for fixture in fixtures {
        assert_fixture(fixture);
    }
}

/// One malformed `=` spelling between an entry label or key and its value.
#[derive(Clone, Copy)]
struct EqualsSpelling {
    name: &'static str,
    separator: &'static str,
    missing: MissingWhitespace,
}

/// A line break after a compact `=` still leaves the left side unspaced.
const SHARED_EQUALS_SPELLINGS: [EqualsSpelling; 4] = [
    EqualsSpelling {
        name: "before",
        separator: "= ",
        missing: MissingWhitespace::Before,
    },
    EqualsSpelling {
        name: "after",
        separator: " =",
        missing: MissingWhitespace::After,
    },
    EqualsSpelling {
        name: "both",
        separator: "=",
        missing: MissingWhitespace::Both,
    },
    EqualsSpelling {
        name: "before_with_line_break_value",
        separator: "=\n",
        missing: MissingWhitespace::Before,
    },
];

/// Map keys may reach `=` across a line break or comment, which still leaves `=` needing
/// whitespace after it.
const MAP_KEY_GAP_EQUALS_SPELLINGS: [EqualsSpelling; 2] = [
    EqualsSpelling {
        name: "after_line_break_key",
        separator: "\n=",
        missing: MissingWhitespace::After,
    },
    EqualsSpelling {
        name: "after_comment_key",
        separator: " -- gap\n=",
        missing: MissingWhitespace::After,
    },
];

fn named_spacing_fixture(
    case: &NamedSpacingCase,
    spelling: EqualsSpelling,
) -> super::support::ParityFixture {
    let entry = format!("{}{}{}", case.field, spelling.separator, case.value);
    let literal = format!("{}{entry}{}", case.prefix, case.suffix);
    fixture(
        format!("{}_named_spacing_missing_{}", case.context, spelling.name),
        literal.clone(),
        case.declarations,
        case.schema.clone(),
        source_equals_spacing_error(
            spelling.missing,
            in_literal(&literal, "=", case.equals_occurrence),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&literal, "=", case.equals_occurrence),
            path: case.path.clone(),
        },
    )
}

fn map_spacing_fixture(
    case: &MapSpacingCase,
    spelling: EqualsSpelling,
) -> super::support::ParityFixture {
    let entry = format!("{}{}{}", case.key, spelling.separator, case.value);
    let map_literal = format!("{{{}{entry}}}", case.prefix);
    let literal = format!("items = {map_literal}");
    let source = format!(
        "MapHolder = | items {{String = Int}} |\n\
         data = MapHolder(items = {map_literal})\n"
    );
    let schema = Schema::record(vec![Field::required(
        "items",
        SchemaType::Map {
            key: Box::new(SchemaType::String),
            value: Box::new(SchemaType::Int),
        },
    )]);

    fixture(
        format!(
            "map_{}_entry_spacing_missing_{}",
            case.position, spelling.name
        ),
        literal.clone(),
        "",
        schema,
        source_equals_spacing_error(
            spelling.missing,
            in_declarations(&source, "=", case.source_equals),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&literal, "=", case.literal_equals),
            path: case.path.clone(),
        },
    )
    .with_source_override(source)
    .with_source_nominal_type("", "MapHolder")
}

fn source_equals_spacing_error(
    missing: MissingWhitespace,
    site: super::support::SourceSite,
) -> SourceExpectation {
    SourceExpectation::Reject {
        code: "MOTH-SYNTAX-0031",
        reason: SourceReason::CommonSyntaxMistake(
            CommonSyntaxMistakeReason::InvalidSymbolicSpacing {
                error: SymbolicSpacingError {
                    construct: SymbolicSpacingConstruct::Assignment,
                    missing,
                },
            },
        ),
        site,
    }
}

fn first_field_before_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("first{} = 1", trivia.text);
    let schema = Schema::record(vec![Field::required("first", SchemaType::Int)]);
    let (source, mon) = if trivia.source_line_break.is_some() {
        (
            source_expression_assignment(in_literal(&literal, "=", 1)),
            mon_named_entry_error(&literal, "first", Vec::new()),
        )
    } else {
        (
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![("first".into(), Value::Int(1))])),
        )
    };
    fixture(
        format!("first_field_before_equals_{}", trivia.name),
        literal,
        "first #= 0\n",
        schema,
        source,
        mon,
    )
}

fn later_field_before_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("first = 0, later{} = 1", trivia.text);
    let schema = Schema::record(vec![
        Field::required("first", SchemaType::Int),
        Field::required("later", SchemaType::Int),
    ]);
    let (source, mon) = if trivia.source_line_break.is_some() {
        (
            source_invalid_expression(
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                in_literal(&literal, "later", 1),
            ),
            mon_named_entry_error(&literal, "later", Vec::new()),
        )
    } else {
        (
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![
                ("first".into(), Value::Int(0)),
                ("later".into(), Value::Int(1)),
            ])),
        )
    };
    fixture(
        format!("later_field_before_equals_{}", trivia.name),
        literal,
        "",
        schema,
        source,
        mon,
    )
}

fn nested_field_before_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("outer = (inner{} = 1)", trivia.text);
    let schema = Schema::record(vec![Field::required(
        "outer",
        SchemaType::Record {
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let (source, mon) = if trivia.source_line_break.is_some() {
        (
            source_expression_assignment(in_literal(&literal, "=", 2)),
            mon_named_entry_error(&literal, "inner", vec![PathSegment::Field("outer".into())]),
        )
    } else {
        (
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "outer".into(),
                Value::Record(vec![("inner".into(), Value::Int(1))]),
            )])),
        )
    };
    fixture(
        format!("nested_field_before_equals_{}", trivia.name),
        literal,
        "inner #= 0\n",
        schema,
        source,
        mon,
    )
}

fn nominal_field_before_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("box = Box(inner{} = 1)", trivia.text);
    let schema = Schema::record(vec![Field::required(
        "box",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let (source, mon) = if trivia.source_line_break.is_some() {
        (
            source_expression_assignment(in_literal(&literal, "=", 2)),
            mon_named_entry_error(&literal, "inner", vec![PathSegment::Field("box".into())]),
        )
    } else {
        (
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "box".into(),
                Value::Record(vec![("inner".into(), Value::Int(1))]),
            )])),
        )
    };
    fixture(
        format!("nominal_field_before_equals_{}", trivia.name),
        literal,
        "inner #= 0\nBox = | inner Int |\n",
        schema,
        source,
        mon,
    )
    .with_source_nominal_type("box", "Box")
}

fn choice_field_before_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("state = Theme::Pair(inner{} = 1)", trivia.text);
    let schema = Schema::record(vec![Field::required(
        "state",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Ready"),
                Variant::payload("Pair", vec![Field::required("inner", SchemaType::Int)]),
            ],
        },
    )]);
    let (source, mon) = if trivia.source_line_break.is_some() {
        (
            source_expression_assignment(in_literal(&literal, "=", 2)),
            mon_named_entry_error(
                &literal,
                "inner",
                vec![
                    PathSegment::Field("state".into()),
                    PathSegment::Variant("Pair".into()),
                ],
            ),
        )
    } else {
        (
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "state".into(),
                Value::Choice {
                    qualifier: Some("Theme".into()),
                    variant: "Pair".into(),
                    fields: vec![("inner".into(), Value::Int(1))],
                },
            )])),
        )
    };
    fixture(
        format!("choice_field_before_equals_{}", trivia.name),
        literal,
        "inner #= 0\nTheme ::\n    Ready,\n    Pair | inner Int |,\n;\n",
        schema,
        source,
        mon,
    )
}

fn after_equals(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("first = {}1", trivia.text);
    fixture(
        format!("after_equals_{}", trivia.name),
        literal,
        "",
        Schema::record(vec![Field::required("first", SchemaType::Int)]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![("first".into(), Value::Int(1))])),
    )
}

fn comma_boundary_schema() -> Schema {
    Schema::record(vec![
        Field::required("first", SchemaType::Int),
        Field::required("later", SchemaType::Int),
    ])
}

fn comma_boundary_value() -> Value {
    Value::Record(vec![
        ("first".into(), Value::Int(1)),
        ("later".into(), Value::Int(2)),
    ])
}

fn before_comma(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("first = 1{}, later = 2", trivia.text);
    fixture(
        format!("before_comma_{}", trivia.name),
        literal,
        "",
        comma_boundary_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(comma_boundary_value()),
    )
}

fn after_comma(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("first = 1,{}later = 2", trivia.text);
    fixture(
        format!("after_comma_{}", trivia.name),
        literal,
        "",
        comma_boundary_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(comma_boundary_value()),
    )
}

fn nested_record_boundary_schema() -> Schema {
    Schema::record(vec![Field::required(
        "outer",
        SchemaType::Record {
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )])
}

fn nested_record_boundary_value() -> Value {
    Value::Record(vec![(
        "outer".into(),
        Value::Record(vec![("inner".into(), Value::Int(1))]),
    )])
}

fn after_record_opener(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("outer = ({}inner = 1)", trivia.text);
    fixture(
        format!("after_record_opener_{}", trivia.name),
        literal,
        "",
        nested_record_boundary_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(nested_record_boundary_value()),
    )
}

fn before_record_closer(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("outer = (inner = 1{})", trivia.text);
    fixture(
        format!("before_record_closer_{}", trivia.name),
        literal,
        "",
        nested_record_boundary_schema(),
        SourceExpectation::Accept,
        MonExpectation::Accept(nested_record_boundary_value()),
    )
}

fn nominal_head_gap(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("box = Box{}(inner = 1)", trivia.text);
    let schema = Schema::record(vec![Field::required(
        "box",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("inner", SchemaType::Int)],
        },
    )]);
    let expected = Value::Record(vec![(
        "box".into(),
        Value::Record(vec![("inner".into(), Value::Int(1))]),
    )]);
    let (source, mon) = match trivia.source_line_break {
        None => (SourceExpectation::Accept, MonExpectation::Accept(expected)),
        Some(_) => (
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
        ),
    };
    fixture(
        format!("nominal_head_gap_{}", trivia.name),
        literal,
        "Box = | inner Int |\n",
        schema,
        source,
        mon,
    )
    .with_source_nominal_type("box", "Box")
}

fn choice_head_gap(trivia: TriviaClass) -> super::support::ParityFixture {
    let literal = format!("status = Theme::Pair{}(inner = 1)", trivia.text);
    let schema = Schema::record(vec![Field::required(
        "status",
        SchemaType::Choice {
            name: "Theme".into(),
            variants: vec![
                Variant::unit("Ready"),
                Variant::payload("Pair", vec![Field::required("inner", SchemaType::Int)]),
            ],
        },
    )]);
    let expected = Value::Record(vec![(
        "status".into(),
        Value::Choice {
            qualifier: Some("Theme".into()),
            variant: "Pair".into(),
            fields: vec![("inner".into(), Value::Int(1))],
        },
    )]);
    let (source, mon) = match trivia.source_line_break {
        None => (SourceExpectation::Accept, MonExpectation::Accept(expected)),
        Some(line_break) => (
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
        ),
    };
    fixture(
        format!("choice_head_gap_{}", trivia.name),
        literal,
        "Theme ::\n    Ready,\n    Pair | inner Int |,\n;\n",
        schema,
        source,
        mon,
    )
}

#[test]
fn nested_numeric_receivers_preserve_profiled_and_fixed_values() {
    let _guard = lock_counter_test();
    let fixtures = nested_numeric_profile_fixtures()
        .into_iter()
        .chain([
            nested_fixed_numeric_fixture(),
            nested_narrow_integer_rejection(),
        ])
        .collect::<Vec<_>>();

    assert_eq!(
        fixtures.len(),
        6,
        "four profile-sensitive cases, one fixed-width stress case and one nested rejection"
    );
    for fixture in fixtures {
        assert_fixture(fixture);
    }
}

fn nested_numeric_profile_fixtures() -> Vec<super::support::ParityFixture> {
    super::integer_destinations::profiles()
        .into_iter()
        .map(|(profile_name, profile)| {
            let (minimum_text, minimum) = match profile.int_width {
                IntWidth::Bits32 => ("-2147483648", i64::from(i32::MIN)),
                IntWidth::Bits64 => ("-9223372036854775808", i64::MIN),
            };
            let expected_float = match profile.float_precision {
                FloatPrecision::Bits32 => f64::from(f32::from_bits(0x3dcc_cccd)),
                FloatPrecision::Bits64 => f64::from_bits(0x3fb9_9999_9999_999a),
            };
            fixture(
                format!("nested_numeric_profile_{profile_name}"),
                format!("value = ProfilePacket(profile_int = {minimum_text}, profile_float = 0.1)"),
                "ProfilePacket = | profile_int Int, profile_float Float |\n",
                Schema::record(vec![Field::required(
                    "value",
                    SchemaType::Struct {
                        name: "ProfilePacket".into(),
                        fields: vec![
                            Field::required("profile_int", SchemaType::Int),
                            Field::required("profile_float", SchemaType::Float),
                        ],
                    },
                )]),
                SourceExpectation::Accept,
                MonExpectation::Accept(Value::Record(vec![(
                    "value".into(),
                    Value::Record(vec![
                        ("profile_int".into(), Value::Int(minimum)),
                        ("profile_float".into(), Value::Float(expected_float)),
                    ]),
                )])),
            )
            .with_profile(profile)
            .with_source_nominal_type("value", "ProfilePacket")
        })
        .collect()
}

fn nested_fixed_numeric_fixture() -> super::support::ParityFixture {
    fixture(
        "nested_fixed_numeric_representations",
        concat!(
            "value = FixedPacket(",
            "signed_minimum = I64Box(number = -9223372036854775808), ",
            "unsigned_maximum = U64Box(number = 18446744073709551615), ",
            "fixed = FloatBox(half = 1.0004882812500000000001, ",
            "single = 1.0000000596046449, negative_zero = -0.0), ",
            "price = PriceBox(number = 1.20))",
        ),
        concat!(
            "I64Box = | number I64 |\n",
            "U64Box = | number U64 |\n",
            "FloatBox = | half F16, single F32, negative_zero F32 |\n",
            "PriceBox = | number Dec2 |\n",
            "FixedPacket = | signed_minimum I64Box, unsigned_maximum U64Box, ",
            "fixed FloatBox, price PriceBox |\n",
        ),
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "FixedPacket".into(),
                fields: vec![
                    Field::required(
                        "signed_minimum",
                        SchemaType::Struct {
                            name: "I64Box".into(),
                            fields: vec![Field::required("number", SchemaType::I64)],
                        },
                    ),
                    Field::required(
                        "unsigned_maximum",
                        SchemaType::Struct {
                            name: "U64Box".into(),
                            fields: vec![Field::required("number", SchemaType::U64)],
                        },
                    ),
                    Field::required(
                        "fixed",
                        SchemaType::Struct {
                            name: "FloatBox".into(),
                            fields: vec![
                                Field::required("half", SchemaType::F16),
                                Field::required("single", SchemaType::F32),
                                Field::required("negative_zero", SchemaType::F32),
                            ],
                        },
                    ),
                    Field::required(
                        "price",
                        SchemaType::Struct {
                            name: "PriceBox".into(),
                            fields: vec![Field::required(
                                "number",
                                SchemaType::Decimal { scale: 2 },
                            )],
                        },
                    ),
                ],
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "value".into(),
            Value::Record(vec![
                (
                    "signed_minimum".into(),
                    Value::Record(vec![("number".into(), Value::I64(i64::MIN))]),
                ),
                (
                    "unsigned_maximum".into(),
                    Value::Record(vec![("number".into(), Value::U64(u64::MAX))]),
                ),
                (
                    "fixed".into(),
                    Value::Record(vec![
                        (
                            "half".into(),
                            Value::F16(f64::from_bits(0x3ff0_0400_0000_0000)),
                        ),
                        (
                            "single".into(),
                            Value::F32(f64::from(f32::from_bits(0x3f80_0001))),
                        ),
                        (
                            "negative_zero".into(),
                            Value::F32(f64::from_bits(0x8000_0000_0000_0000)),
                        ),
                    ]),
                ),
                (
                    "price".into(),
                    Value::Record(vec![("number".into(), Value::Decimal("1.20".into()))]),
                ),
            ]),
        )])),
    )
    .with_source_nominal_type("value", "FixedPacket")
    .with_source_nominal_type("value.signed_minimum", "I64Box")
    .with_source_nominal_type("value.unsigned_maximum", "U64Box")
    .with_source_nominal_type("value.fixed", "FloatBox")
    .with_source_nominal_type("value.price", "PriceBox")
    .with_source_decimal("value.price.number", "120", 2)
}

fn nested_narrow_integer_rejection() -> super::support::ParityFixture {
    let literal = "outer = (small = I8Box(number = 128))";
    fixture(
        "nested_i8_out_of_range",
        literal,
        "I8Box = | number I8 |\n",
        Schema::record(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required(
                    "small",
                    SchemaType::Struct {
                        name: "I8Box".into(),
                        fields: vec![Field::required("number", SchemaType::I8)],
                    },
                )],
            },
        )]),
        source_invalid_number(
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I8),
            in_literal(literal, "128", 1),
        ),
        mon_numeric_error(
            literal,
            "128",
            1,
            MonErrorCode::NumericRange,
            vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("small".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    )
}

fn nested_record_type(depth: usize) -> SchemaType {
    if depth == 0 {
        return SchemaType::Int;
    }

    SchemaType::Record {
        fields: vec![Field::required("next", nested_record_type(depth - 1))],
    }
}

fn nested_record_literal(depth: usize, after_equal: &str, number: &str) -> String {
    let mut literal = number.to_owned();
    for _ in 0..depth {
        literal = format!("(next ={after_equal}{literal})");
    }
    literal
}

fn nested_record_value(depth: usize, number: i64) -> Value {
    if depth == 0 {
        return Value::Int(number);
    }

    Value::Record(vec![(
        "next".into(),
        nested_record_value(depth - 1, number),
    )])
}
