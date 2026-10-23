//! Contextual coercion tests.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CoercionOperation, ExpressionOp, OwnedExpr, SuccessShape,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::InvalidValueReceiverView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
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

fn fixed_expression(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: FixedScalarValue,
) -> OwnedExpr {
    builder
        .fixed_scalar(constants, value, None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail")
}

fn bool_expression(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: bool,
) -> OwnedExpr {
    builder
        .bool(constants, value, None, ValueMode::ImmutableOwned)
        .expect("test literal construction cannot fail")
}

fn assert_type_mismatch(
    failure: crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure,
    expected: TypeId,
    found: TypeId,
    environment: &TypeEnvironment,
) {
    let diagnostic = match failure {
        crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure::Diagnostic(
            diagnostic,
        ) => diagnostic,
        crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure::Invariant(
            error,
        ) => panic!("expected a type mismatch diagnostic, got invariant {error:?}"),
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), Some(environment));
    let diagnostic = compact.first().expect("the type mismatch should compact");
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("the compact diagnostic should retain type mismatch facts");
    assert_eq!(facts.context, TypeMismatchContext::Declaration);

    let strings = crate::compiler_frontend::symbols::string_interning::StringTable::new();
    let paths = crate::compiler_frontend::symbols::path_interner::PathInternerBuilder::new();
    assert_eq!(
        diagnostic
            .types()
            .display_for_render(facts.expected, &strings, Some(paths.paths())),
        crate::compiler_frontend::datatypes::display::display_type(
            expected,
            environment,
            &strings,
            paths.paths(),
        )
    );
    assert_eq!(
        diagnostic
            .types()
            .display_for_render(facts.found, &strings, Some(paths.paths())),
        crate::compiler_frontend::datatypes::display::display_type(
            found,
            environment,
            &strings,
            paths.paths(),
        )
    );
}

#[test]
fn same_type_integer_declaration_is_unchanged() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = fixed_expression(
        &mut builder,
        &mut constants,
        signed_value(FixedScalar::I32, 42),
    );

    let result = coerce_expression_to_declared_type(
        expression,
        fixed_scalar_id(FixedScalar::I32),
        &environment,
        &mut builder,
    )
    .expect("exact type needs no coercion");

    assert_eq!(result, expression);
    let view = builder.view_owned(result).expect("test node is committed");
    assert_eq!(
        view.value_type_id().expect("one success slot"),
        fixed_scalar_id(FixedScalar::I32)
    );
}

#[test]
fn same_type_binary_float_declaration_is_unchanged() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = fixed_expression(
        &mut builder,
        &mut constants,
        float_value(FixedScalar::F64, 1.5),
    );

    let result = coerce_expression_to_declared_type(
        expression,
        fixed_scalar_id(FixedScalar::F64),
        &environment,
        &mut builder,
    )
    .expect("exact type needs no coercion");

    assert_eq!(result, expression);
}

#[test]
fn numeric_declarations_never_convert_implicitly() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let integer = fixed_expression(
        &mut builder,
        &mut constants,
        signed_value(FixedScalar::I32, 16_777_217),
    );
    let result = coerce_expression_to_declared_type(
        integer,
        fixed_scalar_id(FixedScalar::F64),
        &environment,
        &mut builder,
    )
    .expect("incompatible receiver leaves its operand unchanged");
    assert_eq!(result, integer);

    let float = fixed_expression(
        &mut builder,
        &mut constants,
        float_value(FixedScalar::F64, 16_777_217.0),
    );
    let result = coerce_expression_to_declared_type(
        float,
        fixed_scalar_id(FixedScalar::F32),
        &environment,
        &mut builder,
    )
    .expect("incompatible receiver leaves its operand unchanged");
    assert_eq!(result, float);

    let unsigned = fixed_expression(
        &mut builder,
        &mut constants,
        unsigned_value(FixedScalar::U64, 9_007_199_791_611_905),
    );
    let result = coerce_expression_to_declared_type(
        unsigned,
        fixed_scalar_id(FixedScalar::I32),
        &environment,
        &mut builder,
    )
    .expect("incompatible receiver leaves its operand unchanged");
    assert_eq!(result, unsigned);
}

#[test]
fn bool_into_binary_float_declaration_is_unchanged() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = bool_expression(&mut builder, &mut constants, true);

    let result = coerce_expression_to_declared_type(
        expression,
        fixed_scalar_id(FixedScalar::F64),
        &environment,
        &mut builder,
    )
    .expect("bool is not coercible to a float receiver");

    assert_eq!(result, expression);
}

#[test]
fn numeric_explicit_type_boundary_rejects_implicit_conversion() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let i32_id = fixed_scalar_id(FixedScalar::I32);
    let u32_id = fixed_scalar_id(FixedScalar::U32);
    let u64_id = fixed_scalar_id(FixedScalar::U64);
    let f32_id = fixed_scalar_id(FixedScalar::F32);
    let f64_id = fixed_scalar_id(FixedScalar::F64);

    let cases = [
        (
            fixed_expression(
                &mut builder,
                &mut constants,
                signed_value(FixedScalar::I32, 1),
            ),
            f64_id,
            i32_id,
        ),
        (
            fixed_expression(
                &mut builder,
                &mut constants,
                unsigned_value(FixedScalar::U32, 1),
            ),
            f64_id,
            u32_id,
        ),
        (
            fixed_expression(
                &mut builder,
                &mut constants,
                unsigned_value(FixedScalar::U64, 1),
            ),
            f64_id,
            u64_id,
        ),
        (
            fixed_expression(
                &mut builder,
                &mut constants,
                unsigned_value(FixedScalar::U64, 1),
            ),
            i32_id,
            u64_id,
        ),
        (
            fixed_expression(
                &mut builder,
                &mut constants,
                float_value(FixedScalar::F64, 16_777_217.0),
            ),
            f32_id,
            f64_id,
        ),
    ];

    for (expression, expected, found) in cases {
        let failure = coerce_expression_to_explicit_type_boundary(
            expression,
            expected,
            &environment,
            TypeMismatchContext::Declaration,
            &mut builder,
        )
        .expect_err("a numeric value must not cross to another exact type implicitly");
        assert_type_mismatch(failure, expected, found, &environment);
    }
}

#[test]
fn option_declaration_from_inner_expression_becomes_coerced() {
    let mut environment = TypeEnvironment::new();
    let option_string = environment.intern_option(environment.builtins().string);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut strings = crate::compiler_frontend::symbols::string_interning::StringTable::new();
    let mut paths = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let name = crate::compiler_frontend::tests::ast_fixture_support::symbol(
        "value",
        &mut paths,
        &mut strings,
    );
    let expression =
        crate::compiler_frontend::tests::ast_fixture_support::reference_expr_with_type_id(
            &mut builder,
            name,
            environment.builtins().string,
            None,
            ValueMode::ImmutableOwned,
        );

    let result = coerce_expression_to_declared_type(
        builder.own(expression),
        option_string,
        &environment,
        &mut builder,
    )
    .expect("inner string coerces into its option receiver");

    let view = builder
        .view_owned(result)
        .expect("coerced node is committed");
    assert_eq!(
        view.value_type_id().expect("one success slot"),
        option_string
    );
    assert!(
        matches!(
            view.operation(),
            crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Coerce {
                operation: CoercionOperation::ToType(target),
                ..
            } if target == option_string
        ),
        "inner value should become an explicit option coercion"
    );
}

#[test]
fn option_declaration_from_option_expression_is_unchanged() {
    let mut environment = TypeEnvironment::new();
    let string_type = environment.builtins().string;
    let option_string = environment.intern_option(string_type);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let expression = builder
        .option_none_with_type_id(&mut constants, string_type, &mut environment, None)
        .expect("option none content construction cannot fail");

    let result =
        coerce_expression_to_declared_type(expression, option_string, &environment, &mut builder)
            .expect("option none already has its option type");

    assert_eq!(result, expression);
}

#[test]
fn explicit_boundary_checks_success_arity_before_scalar_type_access() {
    let environment = TypeEnvironment::new();
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    for (shape, produced_count) in [
        (SuccessShape::Zero, 0),
        (
            builder
                .success_from_slots(vec![builtin_type_ids::BOOL, builtin_type_ids::STRING], None)
                .expect("two slots are a valid multiple shape"),
            2,
        ),
    ] {
        let operand = bool_expression(&mut builder, &mut constants, true);
        let mut input = builder
            .completed_input(
                ExpressionOp::PropagateOption { value: operand },
                shape,
                None,
                ValueMode::ImmutableOwned,
            )
            .expect("test shape input is constructible");
        input.success = shape;
        let expression = builder
            .append_completed(input)
            .expect("test producer is constructible");
        let failure = coerce_expression_to_explicit_type_boundary(
            expression,
            builtin_type_ids::BOOL,
            &environment,
            TypeMismatchContext::Declaration,
            &mut builder,
        )
        .expect_err(
            "a scalar receiver must reject zero and multiple successes before reading a value type",
        );
        let diagnostic = match failure {
            crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure::Diagnostic(
                diagnostic,
            ) => diagnostic,
            crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure::Invariant(
                error,
            ) => panic!("expected an arity diagnostic, got invariant {error:?}"),
        };
        let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
        let diagnostic = compact
            .first()
            .expect("the invalid value receiver should compact");
        let facts = diagnostic
            .family_view::<InvalidValueReceiverView>()
            .expect("the compact diagnostic should retain value receiver facts");
        assert_eq!(facts.receiver, ValueReceiverContext::Declaration);
        assert_eq!(facts.target_count, 1);
        assert_eq!(facts.produced_count, produced_count);
    }
}
