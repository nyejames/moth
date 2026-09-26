//! String coercion policy tests for `type_coercion::string`.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::string::fold_expression_kind_to_string;
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn int_folds_to_string() {
    let table = StringTable::new();
    let result =
        fold_expression_kind_to_string(&ExpressionKind::Int(42), &table, NumericProfile::STANDARD);
    assert_eq!(result.as_deref(), Some("42"));
}

#[test]
fn float_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Float(3.125),
        &table,
        NumericProfile::STANDARD,
    );
    let Some(s) = result else {
        panic!("expected text for Float");
    };
    assert!(s.contains("3.125"), "unexpected float string: {s}");
}

#[test]
fn float_one_folds_without_trailing_decimal() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Float(1.0),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result, Some("1".to_string()));
}

#[test]
fn float_small_value_uses_moth_exponent_form() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Float(0.0000001),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result, Some("1e-7".to_string()));
}

#[test]
fn float_large_value_uses_signed_exponent() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Float(1e21),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result, Some("1e+21".to_string()));
}

#[test]
fn bool_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Bool(true),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result.as_deref(), Some("true"));
}

#[test]
fn char_folds_to_text() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Char('x'),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result, Some("x".to_string()));
}

#[test]
fn string_slice_folds_to_text() {
    let mut table = StringTable::new();
    let id = table.intern("hello");
    let result = fold_expression_kind_to_string(
        &ExpressionKind::StringSlice(id),
        &table,
        NumericProfile::STANDARD,
    );
    assert_eq!(result.as_deref(), Some("hello"));
}

#[test]
fn coerced_scalar_delegates_to_inner_value() {
    let table = StringTable::new();
    let inner_value = Expression::int(42, None, ValueMode::ImmutableOwned);
    let expression = Expression::coerced(inner_value, builtin_type_ids::STRING);

    let result = fold_expression_kind_to_string(&expression.kind, &table, NumericProfile::STANDARD);

    assert_eq!(result, Some("42".to_string()));
}

#[test]
fn non_renderable_expression_kind_returns_none() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(
        &ExpressionKind::Runtime(ExpressionRpn::empty()),
        &table,
        NumericProfile::STANDARD,
    );
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
    use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarValue};

    let table = StringTable::new();

    // Integer fixed scalars render exact decimal text, including values no `Int` could carry.
    let unsigned_max = FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap();
    assert_eq!(
        fold_expression_kind_to_string(
            &ExpressionKind::FixedScalar(unsigned_max),
            &table,
            NumericProfile::STANDARD,
        )
        .as_deref(),
        Some("18446744073709551615")
    );

    // Fixed binary floats render at their own precision, not at the profile's `Float` precision.
    let rounded_f16 =
        FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5).unwrap();
    assert_eq!(
        fold_expression_kind_to_string(
            &ExpressionKind::FixedScalar(rounded_f16),
            &table,
            NumericProfile::STANDARD,
        )
        .as_deref(),
        Some("0.1")
    );

    // `Byte` is outside the numeric text contract, so it never folds into template content.
    let byte = FixedScalarValue::unsigned(FixedScalar::Byte, 200).unwrap();
    assert!(
        fold_expression_kind_to_string(
            &ExpressionKind::FixedScalar(byte),
            &table,
            NumericProfile::STANDARD,
        )
        .is_none()
    );
}
