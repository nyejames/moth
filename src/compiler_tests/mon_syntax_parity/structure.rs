//! Anonymous record and grouped-expression parity fixtures.
//!
//! WHAT: owns const-record shape, malformed entries and the source/MON grouping boundary.
//! WHY: parentheses are shared syntax, but only MON documents frame a record at the root.

use crate::compiler_frontend::compiler_messages::InvalidExpressionReason;
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::mon::{
    Field, MonErrorCode, PathSegment, Schema, SchemaType, Span, Value, decode_document,
};

use super::source_observation::{assert_source_matches, observe_data_value};
use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, SourceReason, assert_fixture, fixture,
    in_literal, mon_span, source_invalid_expression,
};

fn structure_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();

    fixtures.push(fixture(
        "empty_const_record",
        "()",
        "",
        Schema::record(Vec::new()),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(Vec::new())),
    ));

    fixtures.push(fixture(
        "single_field_const_record",
        "(answer = 42)",
        "",
        Schema::record(vec![Field::required("answer", SchemaType::Int)]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![("answer".into(), Value::Int(42))])),
    ));

    fixtures.push(fixture(
        "recursive_nested_const_records",
        "outer = (inner = (leaf = 7))",
        "",
        Schema::record(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required(
                    "inner",
                    SchemaType::Record {
                        fields: vec![Field::required("leaf", SchemaType::Int)],
                    },
                )],
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "outer".into(),
            Value::Record(vec![(
                "inner".into(),
                Value::Record(vec![("leaf".into(), Value::Int(7))]),
            )]),
        )])),
    ));

    let duplicate = "value = 1, value = 2".to_owned();
    fixtures.push(fixture(
        "duplicate_record_labels",
        duplicate.clone(),
        "",
        Schema::record(vec![Field::required("value", SchemaType::Int)]),
        SourceExpectation::Reject {
            code: "MOTH-RULE-0002",
            reason: SourceReason::DuplicateDeclaration,
            site: in_literal(&duplicate, "value", 2),
        },
        MonExpectation::Reject {
            code: MonErrorCode::DuplicateField,
            span: mon_span(&duplicate, "value", 2),
            path: vec![PathSegment::Field("value".into())],
        },
    ));

    let missing_value = "value = , other = 1".to_owned();
    fixtures.push(fixture(
        "missing_record_field_value",
        missing_value.clone(),
        "",
        Schema::record(vec![
            Field::required("value", SchemaType::Int),
            Field::required("other", SchemaType::Int),
        ]),
        source_invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            in_literal(&missing_value, ",", 1),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::UnexpectedToken,
            span: mon_span(&missing_value, ",", 1),
            path: vec![PathSegment::Field("value".into())],
        },
    ));

    let missing_comma = "first = 1 second = 2".to_owned();
    fixtures.push(fixture(
        "missing_comma_between_record_fields",
        missing_comma.clone(),
        "",
        Schema::record(vec![
            Field::required("first", SchemaType::Int),
            Field::required("second", SchemaType::Int),
        ]),
        SourceExpectation::Reject {
            code: "MOTH-SYNTAX-0001",
            reason: SourceReason::ExpectedToken(TokenTag::COMMA),
            site: in_literal(&missing_comma, "second", 1),
        },
        MonExpectation::Reject {
            code: MonErrorCode::MissingComma,
            span: mon_span(&missing_comma, "s", 2),
            path: Vec::new(),
        },
    ));

    let trailing = "(first = 1) tail".to_owned();
    fixtures.push(fixture(
        "trailing_input_after_explicit_mon_root",
        trailing.clone(),
        "",
        Schema::record(vec![Field::required("first", SchemaType::Int)]),
        source_invalid_expression(
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            in_literal(&trailing, "tail", 1),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::TrailingInput,
            span: mon_span(&trailing, "tail", 1),
            path: Vec::new(),
        },
    ));

    let positional = "(first = 1, 2)".to_owned();
    fixtures.push(fixture(
        "anonymous_records_have_no_positional_fields",
        positional.clone(),
        "",
        Schema::record(vec![Field::required("first", SchemaType::Int)]),
        source_invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            in_literal(&positional, "2", 1),
        ),
        MonExpectation::Reject {
            code: MonErrorCode::ArgumentOrder,
            span: mon_span(&positional, "2", 1),
            path: Vec::new(),
        },
    ));

    fixtures
}

#[test]
fn source_and_mon_record_structure_parity() {
    let _guard = lock_counter_test();
    for fixture in structure_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn grouped_source_scalar_is_not_mon_document_data() {
    let _guard = lock_counter_test();
    let case = "grouped_source_scalar_is_not_mon_document_data";
    let source = "data #= ((1))\n";
    let (build_result, path_fork, string_table) =
        crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result_with_profile(
            source,
            crate::mon::NumericProfile::STANDARD,
        )
        .unwrap_or_else(|diagnostic| {
            panic!(
                "source rejected grouped scalar case '{case}' with {} {:?}",
                diagnostic.identity().code,
                diagnostic.payload
            )
        });
    let observed = observe_data_value(&build_result, &path_fork, &string_table)
        .unwrap_or_else(|| panic!("source case '{case}' had no observable data value"));
    assert_source_matches(&Value::Int(1), &observed, case, "", &[], &[]);

    // Source parentheses group an expression; MON's framed root record requires named entries.
    let schema = Schema::record(Vec::new())
        .prepare()
        .expect("empty MON record schema should prepare");
    let error = decode_document("(1)", &schema)
        .expect_err("a positional root entry must not become a MON record field");
    assert_eq!(
        error.code,
        MonErrorCode::ArgumentOrder,
        "MON error for '{case}'"
    );
    assert_eq!(
        error.span,
        Some(Span { start: 1, end: 2 }),
        "MON span for '{case}'"
    );
    assert!(error.path.is_empty(), "MON path for '{case}'");
}
