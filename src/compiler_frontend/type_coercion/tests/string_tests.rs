//! String coercion policy tests for `type_coercion::string`.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::const_values::store::{ConstStringValue, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::string::fold_expression_to_string;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn signed_value(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("test signed value fits its fixed scalar")
}

fn float_value(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(scalar, value)
        .expect("test float value is finite at its fixed precision")
}

fn fold_fixed_scalar(value: FixedScalarValue) -> Option<String> {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = builder
        .fixed_scalar(&mut constants, value, None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    fold_expression_to_string(view, &constants, &table).expect("string folding cannot fail")
}

#[test]
fn i32_folds_to_string() {
    assert_eq!(
        fold_fixed_scalar(signed_value(FixedScalar::I32, 42)).as_deref(),
        Some("42")
    );
}

#[test]
fn f64_folds_to_string() {
    let result = fold_fixed_scalar(float_value(FixedScalar::F64, 3.125)).expect("float has text");
    assert!(
        result.contains("3.125"),
        "unexpected float string: {result}"
    );
}

#[test]
fn f64_one_folds_without_trailing_decimal() {
    assert_eq!(
        fold_fixed_scalar(float_value(FixedScalar::F64, 1.0)),
        Some("1".to_string())
    );
}

#[test]
fn f64_small_value_uses_moth_exponent_form() {
    assert_eq!(
        fold_fixed_scalar(float_value(FixedScalar::F64, 0.0000001)),
        Some("1e-7".to_string())
    );
}

#[test]
fn f64_large_value_uses_signed_exponent() {
    assert_eq!(
        fold_fixed_scalar(float_value(FixedScalar::F64, 1e21)),
        Some("1e+21".to_string())
    );
}

#[test]
fn bool_folds_to_string() {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    let result = fold_expression_to_string(view, &constants, &table).expect("bool has text");
    assert_eq!(result.as_deref(), Some("true"));
}

#[test]
fn char_folds_to_text() {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = builder
        .char(&mut constants, 'x', None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    let result = fold_expression_to_string(view, &constants, &table).expect("char has text");
    assert_eq!(result, Some("x".to_string()));
}

#[test]
fn string_slice_folds_to_text() {
    let mut table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let id = table.intern("hello");
    let expression = builder
        .string_slice(&mut constants, id, None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    let result = fold_expression_to_string(view, &constants, &table).expect("string has text");
    assert_eq!(result.as_deref(), Some("hello"));
}

#[test]
fn coerced_scalar_delegates_to_inner_value() {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let inner = builder
        .fixed_scalar(
            &mut constants,
            signed_value(FixedScalar::I32, 42),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test literal construction cannot fail");
    let expression = builder
        .coerced(
            inner,
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
        )
        .expect("test coercion construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    let result = fold_expression_to_string(view, &constants, &table).expect("coercion has text");
    assert_eq!(result, Some("42".to_string()));
}

#[test]
fn non_renderable_expression_operation_returns_none() {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = builder
        .option_none_with_type_id(
            &mut constants,
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::BOOL,
            &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
            None,
        )
        .expect("option none construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    assert!(
        fold_expression_to_string(view, &constants, &table)
            .expect("non-scalar renders as absence")
            .is_none()
    );
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
fn structural_string_payload_does_not_observe_anchors_as_text() {
    let table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let text = table_like_text_for_piece_test(&mut table.clone());
    let value = constants
        .insert_resolved(
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(vec![text])),
            None,
        )
        .expect("structural string content construction cannot fail");
    let expression = builder
        .constant(
            value,
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("constant occurrence construction cannot fail");
    let view = builder
        .view_owned(expression)
        .expect("test node is committed");
    assert!(
        fold_expression_to_string(view, &constants, &table)
            .expect("structural anchors cannot be observed as text")
            .is_none()
    );
}

fn table_like_text_for_piece_test(
    table: &mut StringTable,
) -> crate::compiler_frontend::ast::const_values::store::ConstStringPiece {
    let id = table.intern("hello");
    crate::compiler_frontend::ast::const_values::store::ConstStringPiece::Text(id)
}

#[test]
fn fixed_numeric_scalars_fold_like_their_numeric_text_policy() {
    let unsigned_max = FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap();
    assert_eq!(
        fold_fixed_scalar(unsigned_max).as_deref(),
        Some("18446744073709551615")
    );

    let rounded_f16 =
        FixedScalarValue::binary_float(FixedScalar::F16, 0.099_975_585_937_5).unwrap();
    assert_eq!(fold_fixed_scalar(rounded_f16).as_deref(), Some("0.1"));

    assert!(
        fold_fixed_scalar(FixedScalarValue::unsigned(FixedScalar::Byte, 200).unwrap()).is_none()
    );
}
