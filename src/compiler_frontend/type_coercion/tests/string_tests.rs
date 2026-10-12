//! String coercion policy tests for `type_coercion::string`.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::string::fold_expression_kind_to_string;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn signed_value(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("test signed value fits its fixed scalar")
}

fn float_value(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(scalar, value)
        .expect("test float value is finite at its fixed precision")
}

#[test]
fn i32_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::FixedScalar(signed_value(FixedScalar::I32, 42)),
        &table,
    );
    assert_eq!(result.as_deref(), Some("42"));
}

#[test]
fn f64_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::FixedScalar(float_value(FixedScalar::F64, 3.125)),
        &table,
    );
    let Some(s) = result else {
        panic!("expected text for a fixed binary float");
    };
    assert!(s.contains("3.125"), "unexpected float string: {s}");
}

#[test]
fn f64_one_folds_without_trailing_decimal() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::FixedScalar(float_value(FixedScalar::F64, 1.0)),
        &table,
    );
    assert_eq!(result, Some("1".to_string()));
}

#[test]
fn f64_small_value_uses_moth_exponent_form() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::FixedScalar(float_value(FixedScalar::F64, 0.0000001)),
        &table,
    );
    assert_eq!(result, Some("1e-7".to_string()));
}

#[test]
fn f64_large_value_uses_signed_exponent() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::FixedScalar(float_value(FixedScalar::F64, 1e21)),
        &table,
    );
    assert_eq!(result, Some("1e+21".to_string()));
}

#[test]
fn bool_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Bool(true), &table);
    assert_eq!(result.as_deref(), Some("true"));
}

#[test]
fn char_folds_to_text() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Char('x'), &table);
    assert_eq!(result, Some("x".to_string()));
}

#[test]
fn string_slice_folds_to_text() {
    let mut table = StringTable::new();
    let id = table.intern("hello");
    let result = fold_expression_kind_to_string(&ExpressionKind::StringSlice(id), &table);
    assert_eq!(result.as_deref(), Some("hello"));
}

#[test]
fn coerced_scalar_delegates_to_inner_value() {
    let table = StringTable::new();
    let inner_value = Expression::fixed_scalar(
        signed_value(FixedScalar::I32, 42),
        None,
        ValueMode::ImmutableOwned,
    );
    let expression = Expression::coerced(inner_value, builtin_type_ids::STRING);

    let result = fold_expression_kind_to_string(&expression.kind, &table);

    assert_eq!(result, Some("42".to_string()));
}

#[test]
fn non_renderable_expression_kind_returns_none() {
    let table = StringTable::new();
    let result =
        fold_expression_kind_to_string(&ExpressionKind::Runtime(ExpressionRpn::empty()), &table);
    assert!(result.is_none());
}

#[test]
fn cloned_string_table_retains_lookups_after_original_table_is_dropped() {
    let (hello_id, world_id, mut clone) = {
        let mut table = StringTable::new();
        let hello_id = table.intern("hello");
        let world_id = table.intern("world");

        let clone = table.clone();
        assert_eq!(clone.resolve(hello_id), "hello");
        assert_eq!(clone.resolve(world_id), "world");

        (hello_id, world_id, clone)
    };

    assert_eq!(clone.intern("hello"), hello_id);
    assert_eq!(clone.intern("world"), world_id);
}

#[test]
fn fixed_numeric_scalars_fold_like_their_numeric_text_policy() {
    let table = StringTable::new();

    // Integer fixed scalars render exact decimal text, including values above `i64::MAX`.
    let unsigned_max = FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap();
    assert_eq!(
        fold_expression_kind_to_string(&ExpressionKind::FixedScalar(unsigned_max), &table,)
            .as_deref(),
        Some("18446744073709551615")
    );

    // Fixed binary floats render at their own precision: an F16 value renders as the shortest
    // text for its own precision, not by widening to a different float precision first.
    let rounded_f16 =
        FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5).unwrap();
    assert_eq!(
        fold_expression_kind_to_string(&ExpressionKind::FixedScalar(rounded_f16), &table,)
            .as_deref(),
        Some("0.1")
    );

    // `Byte` is outside the numeric text contract, so it never folds into template content.
    let byte = FixedScalarValue::unsigned(FixedScalar::Byte, 200).unwrap();
    assert!(fold_expression_kind_to_string(&ExpressionKind::FixedScalar(byte), &table,).is_none());
}
