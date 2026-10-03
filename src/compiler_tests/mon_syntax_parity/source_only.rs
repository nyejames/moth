//! Source expressions that MON literal documents deliberately do not evaluate.
//!
//! WHAT: proves source arithmetic, constant references, templates and calls retain typed values.
//! WHY: the MON reader accepts self-contained literal data only, before source evaluation.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};

use super::source_observation::{assert_source_matches, observe_start_body_call_arguments};
use super::support::{MonExpectation, SourceExpectation, assert_fixture, fixture, mon_span};

#[test]
fn source_expressions_and_nominal_construction_are_not_mon_calls() {
    let _guard = lock_counter_test();

    let arithmetic = "result = 2 + 3";
    let reference = "result = base";
    let template = "result = [:Hello[name]]";
    let call_literal = "value = measure(x = 1)";
    let call_declarations = "measure |x Int| -> Int:\n    return x\n;\n\n";
    let call_source = concat!(
        "measure |x Int| -> Int:\n",
        "    return x\n",
        ";\n\n",
        "data #= (value = 1)\n",
        "measure(x = 1)\n",
    );
    let fixtures = vec![
        fixture(
            "source_only_arithmetic_expression",
            arithmetic,
            "",
            Schema::record(vec![Field::required("result", SchemaType::Int)]),
            SourceExpectation::AcceptObserved(Value::Record(vec![(
                "result".into(),
                Value::Int(5),
            )])),
            MonExpectation::Reject {
                code: MonErrorCode::MissingComma,
                span: mon_span(arithmetic, "+", 1),
                path: Vec::new(),
            },
        )
        .with_outcome_difference("literal-data-versus-source-syntax"),
        fixture(
            "source_only_constant_reference",
            reference,
            "base #Int = 12\n",
            Schema::record(vec![Field::required("result", SchemaType::Int)]),
            SourceExpectation::AcceptObserved(Value::Record(vec![(
                "result".into(),
                Value::Int(12),
            )])),
            MonExpectation::Reject {
                code: MonErrorCode::UnexpectedToken,
                span: mon_span(reference, "base", 1),
                path: vec![PathSegment::Field("result".into())],
            },
        )
        .with_outcome_difference("literal-data-versus-source-syntax"),
        fixture(
            "source_only_template_expression",
            template,
            "name #= \"Moth\"\n",
            Schema::record(vec![Field::required("result", SchemaType::String)]),
            SourceExpectation::AcceptObserved(Value::Record(vec![(
                "result".into(),
                Value::String("HelloMoth".into()),
            )])),
            MonExpectation::Reject {
                code: MonErrorCode::UnexpectedToken,
                span: mon_span(template, "[", 1),
                path: vec![PathSegment::Field("result".into())],
            },
        )
        .with_outcome_difference("literal-data-versus-source-syntax"),
        fixture(
            "qualified_nominal_constructor_is_record_data",
            "point = Point(x = 1)",
            "Point = | x Int |\n",
            Schema::record(vec![Field::required(
                "point",
                SchemaType::Struct {
                    name: "Point".into(),
                    fields: vec![Field::required("x", SchemaType::Int)],
                },
            )]),
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "point".into(),
                Value::Record(vec![("x".into(), Value::Int(1))]),
            )])),
        )
        .with_source_nominal_type("point", "Point"),
        fixture(
            "source_only_ordinary_function_call",
            call_literal,
            call_declarations,
            Schema::record(vec![Field::required("value", SchemaType::Int)]),
            SourceExpectation::AcceptObserved(Value::Record(vec![("value".into(), Value::Int(1))])),
            MonExpectation::Reject {
                code: MonErrorCode::TypeMismatch,
                span: mon_span(call_literal, "measure(x = 1)", 1),
                path: vec![PathSegment::Field("value".into())],
            },
        )
        .with_source_override(call_source)
        .with_outcome_difference("literal-data-versus-source-syntax"),
    ];

    for fixture in fixtures {
        assert_fixture(fixture);
    }

    let arguments = observe_start_body_call_arguments(
        call_source,
        crate::mon::NumericProfile::STANDARD,
        "measure",
    );
    assert_eq!(arguments.len(), 1, "call literal {call_literal:?}");
    assert_eq!(
        arguments[0].parameter_slot, 0,
        "call literal {call_literal:?}"
    );
    assert_eq!(
        arguments[0].parameter_name, "x",
        "call literal {call_literal:?}"
    );
    assert_source_matches(
        &Value::Int(1),
        &arguments[0].value,
        "source_only_ordinary_function_call",
        "x",
        &[],
        &[],
    );
}
