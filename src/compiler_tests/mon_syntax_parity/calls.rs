//! Source-only ordinary-call argument observations.
//!
//! WHAT: asserts supplied call values against typed expectations and checks their retained
//! declaration-order parameter slots.
//! WHY: call parsing can be tested at its real source boundary without invoking the function.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, Schema, SchemaType, Value};

use super::source_observation::{
    SourceIdentityExpectation, SourceIdentityExpectationKind, assert_source_matches,
    observe_start_body_call_arguments,
};
use super::support::{MonExpectation, SourceExpectation, assert_fixture, fixture};

#[test]
fn positional_then_named_call_arguments_keep_their_parameter_slots() {
    let _guard = lock_counter_test();
    let source = "consume |first I32, second F64, third Dec2, fourth I32|:\n;\n\nconsume(10, fourth = 40, second = -0.0, third = 1.20)\n";
    let arguments = observe_start_body_call_arguments(source, "consume");

    assert_eq!(arguments.len(), 4);
    let mut arguments_by_slot = arguments.iter().collect::<Vec<_>>();
    arguments_by_slot.sort_by_key(|argument| argument.parameter_slot);
    let expected_arguments = [
        (0, "first", Value::I32(10)),
        (1, "second", Value::F64(-0.0)),
        (2, "third", Value::Decimal("1.20".into())),
        (3, "fourth", Value::I32(40)),
    ];
    for (argument, (expected_slot, expected_name, expected_value)) in
        arguments_by_slot.into_iter().zip(expected_arguments)
    {
        assert_eq!(argument.parameter_slot, expected_slot);
        assert_eq!(argument.parameter_name, expected_name);
        let identity_expectations = if expected_name == "third" {
            vec![SourceIdentityExpectation {
                path: expected_name.to_owned(),
                kind: SourceIdentityExpectationKind::Decimal {
                    coefficient: "120".into(),
                    scale: 2,
                },
            }]
        } else {
            Vec::new()
        };
        assert_source_matches(
            &expected_value,
            &argument.value,
            "positional_then_named_call_arguments_keep_their_parameter_slots",
            expected_name,
            &identity_expectations,
            &[],
        );
    }
}

#[test]
fn ordinary_call_arguments_keep_values_across_layout_boundaries() {
    let _guard = lock_counter_test();
    let source = concat!(
        "consume |first I32, second I32| -> I32:\n",
        "    return first + second\n",
        ";\n\n",
        "consume(\n",
        "    first\u{00a0}= \n",
        "        1,\n",
        "    -- comment between supplied arguments\n",
        "    second =\t2\n",
        ")\n",
    );
    let arguments = observe_start_body_call_arguments(source, "consume");

    assert_eq!(arguments.len(), 2);
    for (argument, expected_slot, expected_name, expected_value) in [
        (&arguments[0], 0, "first", 1),
        (&arguments[1], 1, "second", 2),
    ] {
        assert_eq!(argument.parameter_slot, expected_slot);
        assert_eq!(argument.parameter_name, expected_name);
        assert_source_matches(
            &Value::I32(expected_value),
            &argument.value,
            "ordinary_call_arguments_keep_values_across_layout_boundaries",
            expected_name,
            &[],
            &[],
        );
    }
}

#[test]
fn exact_int64_float32_and_dec2_source_values() {
    let _guard = lock_counter_test();
    let fixture = fixture(
        "exact_int64_float32_and_dec2_source_values",
        "item = Holder(price = 1.20), value = 2147483648, rate = 0.1",
        "Holder = | price Dec2 |\n",
        Schema::record(vec![
            Field::required(
                "item",
                SchemaType::Struct {
                    name: "Holder".into(),
                    fields: vec![Field::required("price", SchemaType::Decimal { scale: 2 })],
                },
            ),
            Field::required("value", SchemaType::I64),
            Field::required("rate", SchemaType::F32),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            (
                "item".into(),
                Value::Record(vec![("price".into(), Value::Decimal("1.20".into()))]),
            ),
            ("value".into(), Value::I64(2_147_483_648)),
            ("rate".into(), Value::F32(0.1_f32 as f64)),
        ])),
    )
    .with_source_override(concat!(
        "Holder = | price Dec2 |\n",
        "ExactValues = | item Holder, value I64, rate F32 |\n",
        "data = ExactValues(item = Holder(price = 1.20), value = 2147483648, rate = 0.1)\n",
    ))
    .with_source_nominal_type("", "ExactValues")
    .with_source_nominal_type("item", "Holder")
    .with_source_decimal("item.price", "120", 2);

    assert_fixture(fixture);
}
