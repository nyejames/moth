//! String coercion policy tests for `type_coercion::string`.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::string::{
    FoldedStringPiece, fold_expression_kind_to_string,
};
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn int_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Int(42), &table);
    let Some(FoldedStringPiece::Text(s)) = result else {
        panic!("expected Text piece for Int");
    };
    assert_eq!(s, "42");
}

#[test]
fn float_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Float(3.125), &table);
    let Some(FoldedStringPiece::Text(s)) = result else {
        panic!("expected Text piece for Float");
    };
    assert!(s.contains("3.125"), "unexpected float string: {s}");
}

#[test]
fn float_one_folds_without_trailing_decimal() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Float(1.0), &table);
    assert_eq!(result, Some(FoldedStringPiece::Text("1".to_string())));
}

#[test]
fn float_small_value_uses_moth_exponent_form() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Float(0.0000001), &table);
    assert_eq!(result, Some(FoldedStringPiece::Text("1e-7".to_string())));
}

#[test]
fn float_large_value_uses_signed_exponent() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Float(1e21), &table);
    assert_eq!(result, Some(FoldedStringPiece::Text("1e+21".to_string())));
}

#[test]
fn bool_folds_to_string() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Bool(true), &table);
    let Some(FoldedStringPiece::Text(s)) = result else {
        panic!("expected Text piece for Bool");
    };
    assert_eq!(s, "true");
}

#[test]
fn char_folds_to_char_piece() {
    let table = StringTable::new();
    let result = fold_expression_kind_to_string(&ExpressionKind::Char('x'), &table);
    assert!(matches!(result, Some(FoldedStringPiece::Char('x'))));
}

#[test]
fn string_slice_folds_to_text() {
    let mut table = StringTable::new();
    let id = table.intern("hello");
    let result = fold_expression_kind_to_string(&ExpressionKind::StringSlice(id), &table);
    let Some(FoldedStringPiece::Text(s)) = result else {
        panic!("expected Text piece for StringSlice");
    };
    assert_eq!(s, "hello");
}

#[test]
fn coerced_scalar_delegates_to_inner_value() {
    let table = StringTable::new();
    let inner_value = Expression::int(42, SourceLocation::default(), ValueMode::ImmutableOwned);
    let expression = Expression::coerced(inner_value, builtin_type_ids::STRING);

    let result = fold_expression_kind_to_string(&expression.kind, &table);

    assert_eq!(result, Some(FoldedStringPiece::Text("42".to_string())));
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
