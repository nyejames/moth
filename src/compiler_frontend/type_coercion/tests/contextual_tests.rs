//! Contextual coercion tests.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::contextual::{
    coerce_expression_to_declared_type, coerce_expression_to_explicit_type_boundary,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn fixed_scalar_id(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn signed_value(scalar: FixedScalar, value: i64) -> FixedScalarValue {
    FixedScalarValue::signed(scalar, value).expect("test signed value fits its fixed scalar")
}

fn unsigned_value(scalar: FixedScalar, value: u64) -> FixedScalarValue {
    FixedScalarValue::unsigned(scalar, value).expect("test unsigned value fits its fixed scalar")
}

fn float_value(scalar: FixedScalar, value: f64) -> FixedScalarValue {
    FixedScalarValue::binary_float(scalar, value)
        .expect("test float value is finite at its fixed precision")
}

fn fixed_expression(value: FixedScalarValue) -> Expression {
    Expression::fixed_scalar(value, None, ValueMode::ImmutableOwned)
}

fn assert_type_mismatch(diagnostic: CompilerDiagnostic, expected: TypeId, found: TypeId) {
    match diagnostic.payload {
        DiagnosticPayload::TypeMismatch {
            expected: expected_id,
            found: found_id,
            context,
        } => {
            assert_eq!(expected_id, expected);
            assert_eq!(found_id, found);
            assert_eq!(context, TypeMismatchContext::Declaration);
        }
        other => panic!("expected a TypeMismatch diagnostic, got {other:?}"),
    }
}

#[test]
fn same_type_integer_declaration_is_unchanged() {
    let env = TypeEnvironment::new();
    let result = coerce_expression_to_declared_type(
        fixed_expression(signed_value(FixedScalar::I32, 42)),
        fixed_scalar_id(FixedScalar::I32),
        &env,
    );

    assert_eq!(result.type_id, fixed_scalar_id(FixedScalar::I32));
    let ExpressionKind::FixedScalar(value) = result.kind else {
        panic!("an exactly typed integer must not be wrapped or retyped");
    };
    assert_eq!(value, signed_value(FixedScalar::I32, 42));
}

#[test]
fn same_type_binary_float_declaration_is_unchanged() {
    let env = TypeEnvironment::new();
    let result = coerce_expression_to_declared_type(
        fixed_expression(float_value(FixedScalar::F64, 1.5)),
        fixed_scalar_id(FixedScalar::F64),
        &env,
    );

    assert_eq!(result.type_id, fixed_scalar_id(FixedScalar::F64));
    let ExpressionKind::FixedScalar(value) = result.kind else {
        panic!("an exactly typed float must not be wrapped or retyped");
    };
    assert_eq!(value, float_value(FixedScalar::F64, 1.5));
}

#[test]
fn numeric_declarations_never_convert_implicitly() {
    let env = TypeEnvironment::new();

    // An `I32` value stays `I32` in an `F64` declaration. The literal keeps its exact integer
    // value; only an explicit `cast` converts it.
    let result = coerce_expression_to_declared_type(
        fixed_expression(signed_value(FixedScalar::I32, 16_777_217)),
        fixed_scalar_id(FixedScalar::F64),
        &env,
    );
    assert_eq!(result.type_id, fixed_scalar_id(FixedScalar::I32));
    let ExpressionKind::FixedScalar(value) = result.kind else {
        panic!("a binary-float declaration must not rewrite an integer value");
    };
    assert_eq!(value, signed_value(FixedScalar::I32, 16_777_217));

    // An `F64` value stays exact in an `F32` declaration; the destination never rounds it
    // implicitly.
    let result = coerce_expression_to_declared_type(
        fixed_expression(float_value(FixedScalar::F64, 16_777_217.0)),
        fixed_scalar_id(FixedScalar::F32),
        &env,
    );
    assert_eq!(result.type_id, fixed_scalar_id(FixedScalar::F64));
    let ExpressionKind::FixedScalar(value) = result.kind else {
        panic!("a narrower float declaration must not rewrite an F64 value");
    };
    assert_eq!(value, float_value(FixedScalar::F64, 16_777_217.0));

    // A `U64` value stays `U64` in an `I32` declaration: integer receivers never search for a
    // compatible signed or unsigned domain.
    let result = coerce_expression_to_declared_type(
        fixed_expression(unsigned_value(FixedScalar::U64, 9_007_199_791_611_905)),
        fixed_scalar_id(FixedScalar::I32),
        &env,
    );
    assert_eq!(result.type_id, fixed_scalar_id(FixedScalar::U64));
    let ExpressionKind::FixedScalar(value) = result.kind else {
        panic!("an integer declaration must not rewrite an unsigned value");
    };
    assert_eq!(
        value,
        unsigned_value(FixedScalar::U64, 9_007_199_791_611_905)
    );
}

#[test]
fn bool_into_binary_float_declaration_is_unchanged() {
    // Bool → binary float is not coercible; the expression is returned unchanged.
    let env = TypeEnvironment::new();
    let result = coerce_expression_to_declared_type(
        Expression::bool(true, None, ValueMode::ImmutableOwned),
        fixed_scalar_id(FixedScalar::F64),
        &env,
    );

    assert_eq!(result.type_id, builtin_type_ids::BOOL);
    assert!(matches!(result.kind, ExpressionKind::Bool(true)));
}

#[test]
fn numeric_explicit_type_boundary_rejects_implicit_conversion() {
    let env = TypeEnvironment::new();
    let i32_id = fixed_scalar_id(FixedScalar::I32);
    let u32_id = fixed_scalar_id(FixedScalar::U32);
    let u64_id = fixed_scalar_id(FixedScalar::U64);
    let f32_id = fixed_scalar_id(FixedScalar::F32);
    let f64_id = fixed_scalar_id(FixedScalar::F64);

    // Constant and runtime values, unsigned values and floats all require an explicit `cast` at
    // an exact receiver: the removed implicit promotions, integer-domain searches and implicit
    // destination rounding must not reappear.
    let cases = [
        (
            fixed_expression(signed_value(FixedScalar::I32, 1)),
            f64_id,
            i32_id,
        ),
        (
            Expression::new(
                ExpressionKind::Runtime(ExpressionRpn::empty()),
                None,
                i32_id,
                DataType::FixedScalar(FixedScalar::I32),
                ValueMode::ImmutableOwned,
            ),
            f64_id,
            i32_id,
        ),
        (
            fixed_expression(unsigned_value(FixedScalar::U32, 1)),
            f64_id,
            u32_id,
        ),
        (
            fixed_expression(unsigned_value(FixedScalar::U64, 1)),
            f64_id,
            u64_id,
        ),
        (
            fixed_expression(unsigned_value(FixedScalar::U64, 1)),
            i32_id,
            u64_id,
        ),
        (
            fixed_expression(float_value(FixedScalar::F64, 16_777_217.0)),
            f32_id,
            f64_id,
        ),
    ];

    for (expression, expected, found) in cases {
        let error = coerce_expression_to_explicit_type_boundary(
            expression,
            expected,
            &env,
            TypeMismatchContext::Declaration,
        )
        .expect_err("a numeric value must not cross to another exact type implicitly");
        assert_type_mismatch(error, expected, found);
    }
}

#[test]
fn option_declaration_from_inner_expression_becomes_coerced() {
    let mut env = TypeEnvironment::new();
    let option_string = env.intern_option(env.builtins().string);
    let mut string_table = StringTable::new();
    let expr =
        Expression::string_slice(string_table.intern("Ana"), None, ValueMode::ImmutableOwned);

    let result = coerce_expression_to_declared_type(expr, option_string, &env);

    assert_eq!(result.type_id, option_string);
    assert!(
        matches!(
            result.kind,
            ExpressionKind::Coerced {
                to_type,
                ..
            } if to_type == option_string
        ),
        "inner value should become an explicit option coercion"
    );
}

#[test]
fn option_declaration_from_option_expression_is_unchanged() {
    let mut env = TypeEnvironment::new();
    let string_type = env.builtins().string;
    let option_string = env.intern_option(string_type);
    let expr =
        Expression::option_none_with_type_id(string_type, DataType::StringSlice, &mut env, None);

    let result = coerce_expression_to_declared_type(expr, option_string, &env);

    assert_eq!(result.type_id, option_string);
    assert!(matches!(result.kind, ExpressionKind::OptionNone));
}
