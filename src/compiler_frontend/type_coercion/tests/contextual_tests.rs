//! Contextual coercion tests.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_declared_type;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::profile::{FloatPrecision, NumericProfile};

fn int_literal(value: i64) -> Expression {
    Expression::int(value, None, ValueMode::ImmutableOwned)
}

fn uint_literal(value: u64) -> Expression {
    Expression::uint(value, None, ValueMode::ImmutableOwned)
}

fn float_literal(value: f64) -> Expression {
    Expression::float(value, None, ValueMode::ImmutableOwned)
}

#[test]
fn float_declaration_from_int_literal_becomes_float() {
    let env = TypeEnvironment::new();
    let expr = int_literal(1);
    let result = coerce_expression_to_declared_type(
        expr,
        env.builtins().float,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(result.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(result.kind, ExpressionKind::Float(v) if (v - 1.0).abs() < f64::EPSILON),
        "constant int should fold to float literal"
    );
}

#[test]
fn float_declaration_from_int_expression_becomes_coerced() {
    // Simulate a runtime Int expression (non-constant)
    let env = TypeEnvironment::new();
    let runtime_expr = Expression::new(
        ExpressionKind::Runtime(ExpressionRpn::empty()),
        None,
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );
    let result = coerce_expression_to_declared_type(
        runtime_expr,
        env.builtins().float,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(result.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(
            result.kind,
            ExpressionKind::Coerced {
                to_type,
                ..
            } if to_type == builtin_type_ids::FLOAT
        ),
        "runtime int should become Coerced node with canonical Float TypeId"
    );
}

#[test]
fn float_declaration_from_float_is_unchanged() {
    let env = TypeEnvironment::new();
    let expr = float_literal(1.5);
    let result = coerce_expression_to_declared_type(
        expr,
        env.builtins().float,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(result.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(result.kind, ExpressionKind::Float(_)),
        "float should not be wrapped in Coerced"
    );
}

#[test]
fn int_declaration_from_int_is_unchanged() {
    let env = TypeEnvironment::new();
    let expr = int_literal(42);
    let result = coerce_expression_to_declared_type(
        expr,
        env.builtins().int,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(result.type_id, builtin_type_ids::INT);
    assert!(matches!(result.kind, ExpressionKind::Int(42)));
}

#[test]
fn float_declaration_rejects_bool_unchanged() {
    // Bool → Float is not coercible; the expression should be returned unchanged.
    let env = TypeEnvironment::new();
    let expr = Expression::bool(true, None, ValueMode::ImmutableOwned);
    let result = coerce_expression_to_declared_type(
        expr,
        env.builtins().float,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    // No coercion applied — type stays Bool.
    assert_eq!(result.type_id, builtin_type_ids::BOOL);
}

#[test]
fn int_to_float_coercion_rounds_at_the_profile_precision() {
    let env = TypeEnvironment::new();

    let float32_result = coerce_expression_to_declared_type(
        int_literal(16777217),
        env.builtins().float,
        &env,
        FloatPrecision::Bits32,
    );
    assert_eq!(float32_result.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(float32_result.kind, ExpressionKind::Float(v) if v == 16777216.0),
        "Int -> Float32 must round 16777217 to its nearest Float32 value"
    );

    let float64_result = coerce_expression_to_declared_type(
        int_literal(16777217),
        env.builtins().float,
        &env,
        FloatPrecision::Bits64,
    );
    assert_eq!(float64_result.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(float64_result.kind, ExpressionKind::Float(v) if v == 16777217.0),
        "Int -> Float64 must keep 16777217 exactly"
    );
}

#[test]
fn uint_to_float_coercion_rounds_directly_at_the_profile_precision() {
    let env = TypeEnvironment::new();

    let small = coerce_expression_to_declared_type(
        uint_literal(3),
        env.builtins().float,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(small.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(small.kind, ExpressionKind::Float(v) if v == 3.0),
        "small Uint values convert exactly"
    );

    // `9007199791611905` must round directly to Float32 bits `0x5a000001`
    // (exact integer value `9007200328482816`), never through an f64 intermediate
    // which would double-round to `0x5a000000`.
    let direct = coerce_expression_to_declared_type(
        uint_literal(9_007_199_791_611_905),
        env.builtins().float,
        &env,
        FloatPrecision::Bits32,
    );
    assert_eq!(direct.type_id, builtin_type_ids::FLOAT);
    let ExpressionKind::Float(bits) = direct.kind else {
        panic!("Uint -> Float32 must produce a Float literal");
    };
    assert_eq!(
        (bits as f32).to_bits(),
        0x5a00_0001_u32,
        "Uint64 9007199791611905 must round directly to Float32 bits 0x5a000001"
    );
}

#[test]
fn uint_to_float_coercion_rejects_transitive_int_compatibility() {
    let env = TypeEnvironment::new();

    // A Uint expression into an Int declaration stays incompatible even though
    // both convert to Float: there is no search through Float.
    let into_int = coerce_expression_to_declared_type(
        uint_literal(3),
        env.builtins().int,
        &env,
        NumericProfile::STANDARD.float_precision,
    );
    assert_eq!(into_int.type_id, builtin_type_ids::UINT);
    assert!(
        matches!(into_int.kind, ExpressionKind::Uint(3)),
        "Uint into Int must stay unconverted"
    );
}

#[test]
fn option_declaration_from_inner_expression_becomes_coerced() {
    let mut env = TypeEnvironment::new();
    let option_string = env.intern_option(env.builtins().string);
    let mut string_table = StringTable::new();
    let expr =
        Expression::string_slice(string_table.intern("Ana"), None, ValueMode::ImmutableOwned);

    let result = coerce_expression_to_declared_type(
        expr,
        option_string,
        &env,
        NumericProfile::STANDARD.float_precision,
    );

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

    let result = coerce_expression_to_declared_type(
        expr,
        option_string,
        &env,
        NumericProfile::STANDARD.float_precision,
    );

    assert_eq!(result.type_id, option_string);
    assert!(matches!(result.kind, ExpressionKind::OptionNone));
}
