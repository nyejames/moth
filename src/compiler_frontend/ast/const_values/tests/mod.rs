//! Tests for the shared AST const value resolver.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::facts::ConstFactValueKind;
use crate::compiler_frontend::ast::const_values::resolver::{
    ConstResolutionError, ConstValueEnvironment, ConstValueResolver,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, Operator,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn make_resolver<'a>(
    string_table: &'a mut StringTable,
    const_values: &'a ConstValueStore,
    store: &mut TemplateIrStore,
) -> ConstValueResolver<'a> {
    ConstValueResolver::new(
        string_table,
        const_values,
        Rc::new(RefCell::new(std::mem::take(store))),
    )
}

fn make_environment_with(
    path: &str,
    expression: Expression,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> ConstValueEnvironment {
    let mut env = ConstValueEnvironment::default();
    let interned_path = path_fork
        .try_intern_portable_path(path, string_table)
        .expect("test path fits");
    env.insert(interned_path, expression);
    env
}

fn rvalue_item(expression: Expression) -> ExpressionRpnItem {
    ExpressionRpnItem::Operand(expression)
}

fn operator_item(operator: Operator) -> ExpressionRpnItem {
    ExpressionRpnItem::Operator {
        operator,
        span: None,
    }
}

// ------------------------------
//  Literal expression resolves
// ------------------------------

#[test]
fn literal_int_resolves_as_const() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let expression = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 42).expect("42 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("literal should resolve");

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(42)
    ));
}

#[test]
fn literal_string_resolves_as_const() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let string_id = string_table.intern("hello");
    let expression = Expression::string_slice(string_id, None, ValueMode::ImmutableOwned);
    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("literal should resolve");

    assert!(matches!(result.kind, ExpressionKind::StringSlice(_)));
}

// ------------------------------
//  Folded arithmetic resolves
// ------------------------------

#[test]
fn folded_arithmetic_resolves_to_literal() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::fixed_scalar(
                FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )),
            rvalue_item(Expression::fixed_scalar(
                FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::FixedScalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
    );
    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("folded arithmetic should resolve");

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(3)
    ));
}

#[test]
fn folded_arithmetic_with_reference_substitution_resolves() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::reference(
                path_fork
                    .try_intern_portable_path("x", &mut string_table)
                    .expect("test path fits"),
                DataType::FixedScalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            )),
            rvalue_item(Expression::fixed_scalar(
                FixedScalarValue::signed(FixedScalar::I32, 5).expect("5 fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Multiply),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::FixedScalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
    );

    let env = make_environment_with(
        "x",
        Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 3).expect("3 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        ),
        &mut path_fork,
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("substituted arithmetic should resolve");

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(15)
    ));
}

#[test]
fn folded_arithmetic_with_coerced_reference_substitution_resolves() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let reference = Expression::reference(
        path_fork
            .try_intern_portable_path("x", &mut string_table)
            .expect("test path fits"),
        DataType::FixedScalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableReference,
    );
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::coerced(
                reference,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            )),
            rvalue_item(Expression::fixed_scalar(
                FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::FixedScalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
    );

    let env = make_environment_with(
        "x",
        Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 40).expect("40 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        ),
        &mut path_fork,
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("coerced reference arithmetic should resolve");

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(42)
    ));
}

// ------------------------------
//  Reference to known const resolves
// ------------------------------

#[test]
fn reference_to_known_const_resolves() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let path = path_fork
        .try_intern_portable_path("ratio", &mut string_table)
        .expect("test path fits");
    let expression = Expression::reference_with_type_id(path, DataType::FixedScalar(FixedScalar::F64), builtin_type_ids::fixed_scalar(FixedScalar::F64), None, ValueMode::ImmutableReference, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue);

    let env = make_environment_with(
        "ratio",
        Expression::fixed_scalar(
            FixedScalarValue::binary_float(FixedScalar::F64, 2.71).expect("2.71 is a finite F64"),
            None,
            ValueMode::ImmutableOwned,
        ),
        &mut path_fork,
        &mut string_table,
    );
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&expression, &env)
        .expect("reference should resolve");

    assert!(matches!(
        result.kind,
        ExpressionKind::FixedScalar(value)
            if value.as_f64().is_some_and(|resolved| (resolved - 2.71).abs() < f64::EPSILON)
    ));
}

// ------------------------------
//  Forward reference fails
// ------------------------------

#[test]
fn unresolved_reference_fails() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let path = path_fork
        .try_intern_portable_path("unknown", &mut string_table)
        .expect("test path fits");
    let expression = Expression::reference_with_type_id(path, DataType::FixedScalar(FixedScalar::I32), builtin_type_ids::fixed_scalar(FixedScalar::I32), None, ValueMode::ImmutableReference, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue);

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("unresolved reference should fail");

    assert_eq!(error, ConstResolutionError::UnresolvedReference);
}

// ------------------------------
//  Function call fails
// ------------------------------

#[test]
fn function_call_fails_const_resolution() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let expression = Expression::function_call(
        path_fork
            .try_intern_portable_path("foo", &mut string_table)
            .expect("test path fits"),
        vec![],
        vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
        None,
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("function call should fail");

    assert_eq!(error, ConstResolutionError::CallInConstContext);
}

// ------------------------------
//  Mutable declaration fails
// ------------------------------

#[test]
fn mutable_declaration_fails_private_const_resolution() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let declaration = Declaration {
        id: path_fork
            .try_intern_portable_path("value", &mut string_table)
            .expect("test path fits"),
        value: Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 fits I32"),
            None,
            ValueMode::MutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let error = resolver
        .resolve_private_top_level_declaration(&declaration, &env)
        .expect_err("mutable declaration should fail");

    assert_eq!(error, ConstResolutionError::MutableDeclaration);
}

// ------------------------------
//  Fact value kinds
// ------------------------------

#[test]
fn fact_value_kind_from_literal_is_literal() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::Literal),
        ConstFactValueKind::Literal
    );
}

#[test]
fn fact_value_kind_from_runtime_is_non_const() {
    assert_eq!(
        ConstFactValueKind::from_const_value_kind(ConstValueKind::NonConst),
        ConstFactValueKind::NonConst
    );
}

// ------------------------------
//  Coerced expression resolution
// ------------------------------

#[test]
fn coerced_expression_resolves_inner_value() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let inner = Expression::fixed_scalar(
        FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32"),
        None,
        ValueMode::ImmutableOwned,
    );
    let coerced = Expression::coerced(inner, builtin_type_ids::fixed_scalar(FixedScalar::F64));

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let result = resolver
        .resolve_expression(&coerced, &env)
        .expect("coerced literal should resolve");

    // The fast path preserves the Coerced wrapper because it is_compile_time_constant.
    assert!(matches!(result.kind, ExpressionKind::Coerced { .. }));
}

// ------------------------------
//  Runtime RPN with non-const reference fails
// ------------------------------

#[test]
fn runtime_rpn_with_unresolved_reference_fails() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let rpn = ExpressionRpn {
        items: vec![
            rvalue_item(Expression::reference(
                path_fork
                    .try_intern_portable_path("missing", &mut string_table)
                    .expect("test path fits"),
                DataType::FixedScalar(FixedScalar::I32),
                None,
                ValueMode::ImmutableReference,
            )),
            rvalue_item(Expression::fixed_scalar(
                FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 fits I32"),
                None,
                ValueMode::ImmutableOwned,
            )),
            operator_item(Operator::Add),
        ],
    };
    let expression = Expression::runtime_with_type_id(
        rpn,
        DataType::FixedScalar(FixedScalar::I32),
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        None,
        ValueMode::ImmutableOwned,
    );

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let error = resolver
        .resolve_expression(&expression, &env)
        .expect_err("unresolved reference in RPN should fail");

    assert_eq!(error, ConstResolutionError::UnresolvedReference);
}

// ------------------------------
//  Body-local declaration resolution
// ------------------------------

#[test]
fn body_local_immutable_literal_resolves() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let declaration = Declaration {
        id: path_fork
            .try_intern_portable_path("local", &mut string_table)
            .expect("test path fits"),
        value: Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 99).expect("99 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let resolved = resolver
        .resolve_body_local_declaration(&declaration, &env)
        .expect("body-local immutable literal should resolve");

    assert_eq!(resolved.value_kind, ConstFactValueKind::Literal);
    assert_eq!(resolved.expression.span, None);
    assert!(matches!(
        resolved.expression.kind,
        ExpressionKind::FixedScalar(value) if value.as_i64() == Some(99)
    ));
}

#[test]
fn body_local_mutable_declaration_fails() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut store = TemplateIrStore::new();
    let const_values = ConstValueStore::default();
    let declaration = Declaration {
        id: path_fork
            .try_intern_portable_path("local", &mut string_table)
            .expect("test path fits"),
        value: Expression::fixed_scalar(
            FixedScalarValue::signed(FixedScalar::I32, 99).expect("99 fits I32"),
            None,
            ValueMode::MutableOwned,
        ),
        binding_span: None,
        config_qualifier: None,
    };

    let env = ConstValueEnvironment::default();
    let mut resolver = make_resolver(&mut string_table, &const_values, &mut store);

    let error = resolver
        .resolve_body_local_declaration(&declaration, &env)
        .expect_err("body-local mutable declaration should fail");

    assert_eq!(error, ConstResolutionError::MutableDeclaration);
}
