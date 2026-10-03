//! Numeric spelling parity: category, separator and exponent grammar.
//!
//! WHAT: exercises the same authored number bytes through a typed nominal-field receiver and MON.
//! WHY: whole, decimal and exponent spellings stay distinct until their receiver is known.

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, InvalidExpressionReason, TypeMismatchContext,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, SourceReason, assert_fixture, fixture,
    in_literal, mon_numeric_error, source_invalid_number, source_type_mismatch,
};

fn box_schema(number_type: SchemaType) -> Schema {
    Schema::record(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("number", number_type)],
        },
    )])
}

fn box_expected(number: Value) -> Value {
    Value::Record(vec![(
        "value".into(),
        Value::Record(vec![("number".into(), number)]),
    )])
}

fn accepted_number(
    name: &str,
    spelling: &str,
    type_name: &str,
    schema_type: SchemaType,
    value: Value,
) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    fixture(
        name,
        literal,
        format!("Box = | number {type_name} |\n"),
        box_schema(schema_type),
        SourceExpectation::Accept,
        MonExpectation::Accept(box_expected(value)),
    )
    .with_source_nominal_type("value", "Box")
}

fn rejected_number(
    name: &str,
    spelling: &str,
    type_name: &str,
    schema_type: SchemaType,
    source: SourceExpectation,
    mon: MonExpectation,
) -> ParityFixture {
    fixture(
        name,
        format!("value = Box(number = {spelling})"),
        format!("Box = | number {type_name} |\n"),
        box_schema(schema_type),
        source,
        mon,
    )
    .with_source_nominal_type("value", "Box")
}

fn numeric_spelling_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = vec![
        accepted_number(
            "grouped_whole_spelling_into_f64",
            "1_000",
            "F64",
            SchemaType::F64,
            Value::F64(f64::from_bits(0x408f_4000_0000_0000)), // Exact 1000.0.
        ),
        accepted_number(
            "separators_in_integer_and_fraction_runs",
            "1_0.0_1",
            "F64",
            SchemaType::F64,
            Value::F64(f64::from_bits(0x4024_051e_b851_eb85)), // Nearest binary64 to 10.01.
        ),
        accepted_number(
            "lowercase_positive_separated_exponent",
            "1.25e+0_2",
            "F64",
            SchemaType::F64,
            Value::F64(f64::from_bits(0x405f_4000_0000_0000)), // Exact 125.0.
        ),
        accepted_number(
            "lowercase_negative_separated_exponent",
            "1.25e-0_2",
            "F64",
            SchemaType::F64,
            Value::F64(f64::from_bits(0x3f89_9999_9999_999a)), // Nearest binary64 to 0.0125.
        ),
    ];

    for (name, spelling, reason) in [
        (
            "separator_at_integer_end",
            "1_",
            NumberLiteralErrorReason::EndsWithSeparator,
        ),
        (
            "doubled_integer_separator",
            "1__0",
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "separator_before_decimal_point",
            "1_.0",
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "separator_after_decimal_point",
            "1._0",
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "separator_after_exponent_marker",
            "1e_2",
            NumberLiteralErrorReason::InvalidSeparatorPlacement,
        ),
        (
            "uppercase_exponent_marker",
            "1E3",
            NumberLiteralErrorReason::UppercaseExponentMarker,
        ),
        (
            "missing_fraction_digits",
            "1.",
            NumberLiteralErrorReason::MissingFractionalDigits,
        ),
        (
            "missing_exponent_digits",
            "1e",
            NumberLiteralErrorReason::MissingExponentDigits,
        ),
        (
            "missing_signed_exponent_digits",
            "1e+",
            NumberLiteralErrorReason::MissingExponentDigits,
        ),
    ] {
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            "F64",
            SchemaType::F64,
            source_invalid_number(reason, in_literal(&literal, spelling, 1)),
            mon_numeric_error(
                &literal,
                spelling,
                1,
                MonErrorCode::NumericSyntax,
                vec![
                    PathSegment::Field("value".into()),
                    PathSegment::Field("number".into()),
                ],
            ),
        ));
    }

    let leading_plus = "value = Box(number = +1)";
    fixtures.push(rejected_number(
        "leading_plus_is_not_a_literal_sign",
        "+1",
        "F64",
        SchemaType::F64,
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0031",
            reason: SourceReason::CommonSyntaxMistake(
                CommonSyntaxMistakeReason::UnsupportedUnaryPlus,
            ),
            site: in_literal(leading_plus, "+", 1),
        },
        mon_numeric_error(
            leading_plus,
            "+",
            1,
            MonErrorCode::UnexpectedToken,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    ));

    let trailing_junk = "value = Box(number = 1tail)";
    fixtures.push(rejected_number(
        "trailing_identifier_after_number",
        "1tail",
        "F64",
        SchemaType::F64,
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0023",
            reason: SourceReason::InvalidExpression(
                InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            ),
            site: in_literal(trailing_junk, "tail", 1),
        },
        mon_numeric_error(
            trailing_junk,
            "1tail",
            1,
            MonErrorCode::NumericSyntax,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    ));

    for (name, spelling) in [
        ("decimal_spelling_into_int", "1.0"),
        ("exponent_spelling_into_int", "1e0"),
    ] {
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            "Int",
            SchemaType::Int,
            source_type_mismatch(
                TypeMismatchContext::ConstructorArgument,
                in_literal(&literal, spelling, 1),
            ),
            mon_numeric_error(
                &literal,
                spelling,
                1,
                MonErrorCode::NumericType,
                vec![
                    PathSegment::Field("value".into()),
                    PathSegment::Field("number".into()),
                ],
            ),
        ));
    }

    fixtures
}

#[test]
fn source_and_mon_numeric_spelling_categories_and_errors_match() {
    let _guard = lock_counter_test();
    for fixture in numeric_spelling_fixtures() {
        assert_fixture(fixture);
    }
}
