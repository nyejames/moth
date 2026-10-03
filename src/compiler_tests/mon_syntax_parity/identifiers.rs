//! Identifier-shape parity fixtures.
//!
//! WHAT: owns common identifier character-shape cases and multibyte diagnostic boundaries.
//! WHY: source and MON share the lexical name law without sharing their value grammars.

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, DiagnosticOperator, InvalidExpressionReason, MissingWhitespace,
    SymbolicSpacingConstruct, SymbolicSpacingError,
};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, SourceReason, assert_fixture, fixture,
    in_literal, mon_span, source_invalid_expression, source_unknown_name,
};

fn identifier_shape_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    let accepted_literal = "ascii9 = 1, _leading = 2, __field_name = 3, é2 = 4, λ7 = 5";
    fixtures.push(fixture(
        "ascii_underscores_unicode_starts_and_alphanumeric_continuations",
        accepted_literal,
        "",
        Schema::record(vec![
            Field::required("ascii9", SchemaType::Int),
            Field::required("_leading", SchemaType::Int),
            Field::required("__field_name", SchemaType::Int),
            Field::required("é2", SchemaType::Int),
            Field::required("λ7", SchemaType::Int),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            ("ascii9".into(), Value::Int(1)),
            ("_leading".into(), Value::Int(2)),
            ("__field_name".into(), Value::Int(3)),
            ("é2".into(), Value::Int(4)),
            ("λ7".into(), Value::Int(5)),
        ])),
    ));

    let invalid_start = "1name = 1".to_owned();
    fixtures.push(fixture(
        "digit_cannot_start_identifier",
        invalid_start.clone(),
        "",
        Schema::record(vec![Field::required("name", SchemaType::Int)]),
        source_invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            in_literal(&invalid_start, "name", 1),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::NumericSyntax,
            span: mon_span(&invalid_start, "1name", 1),
            path: Vec::new(),
        },
    ));

    let combining_mark = "e\u{301} = 1".to_owned();
    fixtures.push(fixture(
        "combining_mark_is_not_identifier_continuation",
        combining_mark.clone(),
        "",
        Schema::record(vec![Field::required("e", SchemaType::Int)]),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0007",
            reason: SourceReason::InvalidCharacter('\u{301}'),
            site: in_literal(&combining_mark, "\u{301}", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&combining_mark, "e", 1),
            path: Vec::new(),
        },
    ));

    let punctuation = "name-hyphen = 1".to_owned();
    fixtures.push(fixture(
        "hyphen_is_not_identifier_continuation",
        punctuation.clone(),
        "",
        Schema::record(vec![Field::required("name", SchemaType::Int)]),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0031",
            reason: SourceReason::CommonSyntaxMistake(
                CommonSyntaxMistakeReason::InvalidSymbolicSpacing {
                    error: SymbolicSpacingError {
                        construct: SymbolicSpacingConstruct::BinaryOperator {
                            operator: DiagnosticOperator::Subtract,
                        },
                        missing: MissingWhitespace::Both,
                    },
                },
            ),
            site: in_literal(&punctuation, "-", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&punctuation, "name", 1),
            path: Vec::new(),
        },
    ));

    let multibyte_name = "value = éclair".to_owned();
    fixtures.push(fixture(
        "multibyte_name_source_site_and_mon_byte_span",
        multibyte_name.clone(),
        "",
        Schema::record(vec![Field::required("value", SchemaType::Int)]),
        source_unknown_name(in_literal(&multibyte_name, "éclair", 1)),
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&multibyte_name, "éclair", 1),
            path: vec![PathSegment::Field("value".into())],
        },
    ));

    fixtures
}

#[test]
fn source_and_mon_identifier_shape_parity() {
    let _guard = lock_counter_test();
    for fixture in identifier_shape_fixtures() {
        assert_fixture(fixture);
    }
}
