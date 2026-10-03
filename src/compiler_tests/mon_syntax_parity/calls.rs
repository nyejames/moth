//! Source-only ordinary-call argument observations.
//!
//! WHAT: asserts supplied call values against typed expectations and checks their retained
//! declaration-order parameter slots.
//! WHY: call parsing can be tested at its real source boundary without invoking the function.

use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

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
    let source = "consume |first Int, second Float, third Dec2, fourth Int|:\n;\n\nconsume(10, fourth = 40, second = -0.0, third = 1.20)\n";
    let arguments = observe_start_body_call_arguments(source, NumericProfile::STANDARD, "consume");

    assert_eq!(arguments.len(), 4);
    let mut arguments_by_slot = arguments.iter().collect::<Vec<_>>();
    arguments_by_slot.sort_by_key(|argument| argument.parameter_slot);
    let expected_arguments = [
        (0, "first", Value::Int(10)),
        (1, "second", Value::Float(-0.0)),
        (2, "third", Value::Decimal("1.20".into())),
        (3, "fourth", Value::Int(40)),
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
        "consume |first Int, second Int| -> Int:\n",
        "    return first + second\n",
        ";\n\n",
        "consume(\n",
        "    first\u{00a0}= \n",
        "        1,\n",
        "    -- comment between supplied arguments\n",
        "    second =\t2\n",
        ")\n",
    );
    let arguments = observe_start_body_call_arguments(source, NumericProfile::STANDARD, "consume");

    assert_eq!(arguments.len(), 2);
    for (argument, expected_slot, expected_name, expected_value) in [
        (&arguments[0], 0, "first", 1),
        (&arguments[1], 1, "second", 2),
    ] {
        assert_eq!(argument.parameter_slot, expected_slot);
        assert_eq!(argument.parameter_name, expected_name);
        assert_source_matches(
            &Value::Int(expected_value),
            &argument.value,
            "ordinary_call_arguments_keep_values_across_layout_boundaries",
            expected_name,
            &[],
            &[],
        );
    }
}

#[test]
fn profiled_fixture_uses_source_and_mon_profiles_and_decimal_identity() {
    let _guard = lock_counter_test();
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let fixture = fixture(
        "profiled_int64_float32_and_dec2_source_values",
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
            Field::required("value", SchemaType::Int),
            Field::required("rate", SchemaType::Float),
        ]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![
            (
                "item".into(),
                Value::Record(vec![("price".into(), Value::Decimal("1.20".into()))]),
            ),
            ("value".into(), Value::Int(2_147_483_648)),
            ("rate".into(), Value::Float(0.1_f32 as f64)),
        ])),
    )
    .with_profile(profile)
    .with_source_nominal_type("item", "Holder")
    .with_source_decimal("item.price", "120", 2);

    assert_fixture(fixture);
}
