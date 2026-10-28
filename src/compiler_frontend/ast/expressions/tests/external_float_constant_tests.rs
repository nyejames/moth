//! Host constant projections preserve exact content and diagnose invalid registered bits.

use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionConstructionFailure, ExpressionOperationView, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::external_namespace_members::project_external_constant;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::CompileTimeEvaluationErrorView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    InvalidNumberLiteralView, NumberLiteralCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantRawValue, ExternalConstantValue,
    ExternalPackageRegistry, ExternalSignatureType,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn scalar_content(
    expressions: &AstExpressionBuilder,
    values: &ConstValueStore,
    root: OwnedExpr,
) -> FixedScalarValue {
    let view = expressions
        .view_owned(root)
        .expect("projection belongs to owner");
    let ExpressionOperationView::Constant(value) = view.operation() else {
        panic!("host projection must use constant content")
    };
    let ConstValuePayload::FixedScalar(value) = values.payload(value).expect("constant content")
    else {
        panic!("fixed host content")
    };
    *value
}

#[test]
fn external_float_constant_projects_exact_f64_value() {
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let name = strings.intern("HOST_PI");
    let definition = ExternalConstantDef {
        name: "HOST_PI".to_owned(),
        data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
        value: ExternalConstantValue::Registered {
            scalar: FixedScalar::F64,
            raw: ExternalConstantRawValue::BinaryFloat(std::f64::consts::PI),
        },
    };
    let root = project_external_constant(
        &definition,
        name,
        None,
        ValueMode::ImmutableOwned,
        &mut strings,
        &mut expressions,
        &mut values,
    )
    .expect("finite F64 projection");
    assert_eq!(
        expressions
            .view_owned(root)
            .expect("bound projection")
            .value_type_id()
            .expect("single scalar"),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert_eq!(
        scalar_content(&expressions, &values, root).as_f64(),
        Some(std::f64::consts::PI)
    );
}

#[test]
fn external_float_constant_rejects_non_finite_host_value() {
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let name = strings.intern("HOST_HUGE");
    for host_value in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut registry = ExternalPackageRegistry::new();
        let package = registry
            .register_package(
                "@test/non_finite",
                crate::builder_surface::PackageOrigin::Builder,
            )
            .expect("registered package");
        registry
            .register_external_constant(
                package,
                ExternalConstantDef {
                    name: "HOST_HUGE".to_owned(),
                    data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
                    value: ExternalConstantValue::Registered {
                        scalar: FixedScalar::F64,
                        raw: ExternalConstantRawValue::BinaryFloat(host_value),
                    },
                },
            )
            .expect("raw host bits remain available until source validation");
        let (_, definition) = registry
            .resolve_package_constant("@test/non_finite", "HOST_HUGE")
            .expect("registered constant");
        let error = project_external_constant(
            definition,
            name,
            None,
            ValueMode::ImmutableOwned,
            &mut strings,
            &mut expressions,
            &mut values,
        )
        .expect_err("non-finite host value is invalid");
        let ExpressionConstructionFailure::Diagnostic(error) = error else {
            panic!("host invalidity is a source diagnostic")
        };
        let compact = compact_bag_for_test(DiagnosticBag::from(error), None);
        let error = compact.first().expect("the diagnostic is retained");
        let facts = error
            .family_view::<CompileTimeEvaluationErrorView>()
            .expect("the diagnostic is a compile-time evaluation failure");
        assert_eq!(
            facts.reason,
            CompileTimeEvaluationErrorReason::FloatOverflow
        );
        assert_eq!(facts.operation, Some(name));
    }
    assert_eq!(
        expressions.len(),
        0,
        "invalid content publishes no expression row"
    );
}

#[test]
fn core_math_constant_projects_exact_f64() {
    let mut registry = ExternalPackageRegistry::new();
    crate::builder_surface::core_packages::register_core_math_package(&mut registry);
    let (_, pi) = registry
        .resolve_package_constant("@core/math", "PI")
        .expect("Core PI");
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let name = strings.intern("PI");
    let root = project_external_constant(
        pi,
        name,
        None,
        ValueMode::ImmutableOwned,
        &mut strings,
        &mut expressions,
        &mut values,
    )
    .expect("finite Core Math constant");
    assert_eq!(
        expressions
            .view_owned(root)
            .expect("bound projection")
            .value_type_id()
            .expect("single scalar"),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert_eq!(
        scalar_content(&expressions, &values, root).as_f64(),
        Some(std::f64::consts::PI)
    );
}

#[test]
fn fixed_scalar_constants_keep_declared_identity_and_exact_value() {
    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    for value in [
        FixedScalarValue::signed(FixedScalar::I32, i64::from(i32::MIN)).expect("I32 minimum"),
        FixedScalarValue::unsigned(FixedScalar::U32, u64::from(u32::MAX)).expect("U32 maximum"),
        FixedScalarValue::binary_float(FixedScalar::F32, -0.0).expect("F32 negative zero"),
        FixedScalarValue::binary_float(FixedScalar::F64, std::f64::consts::PI).expect("F64 PI"),
    ] {
        let scalar = value.scalar();
        let name = strings.intern(scalar.name());
        let definition = ExternalConstantDef {
            name: scalar.name().to_owned(),
            data_type: ExternalAbiType::Fixed(scalar).into(),
            value: ExternalConstantValue::Fixed(value),
        };
        let root = project_external_constant(
            &definition,
            name,
            None,
            ValueMode::ImmutableOwned,
            &mut strings,
            &mut expressions,
            &mut values,
        )
        .expect("materialised fixed constant");
        assert_eq!(
            expressions
                .view_owned(root)
                .expect("bound projection")
                .value_type_id()
                .expect("single scalar"),
            builtin_type_ids::fixed_scalar(scalar)
        );
        assert_eq!(scalar_content(&expressions, &values, root), value);
    }
}

#[test]
fn max_finite_f64_constant_keeps_binary64_consumer_precision() {
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
    use moth_lexical::numeric::format::format_finite_float;
    use moth_lexical::numeric::precision::BinaryFloatPrecision;

    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let value = FixedScalarValue::binary_float(FixedScalar::F64, f64::MAX)
        .expect("finite binary64 maximum");
    let definition = ExternalConstantDef {
        name: "MAX_F64".to_owned(),
        data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
        value: ExternalConstantValue::Fixed(value),
    };
    let name = strings.intern("MAX_F64");
    let root = project_external_constant(
        &definition,
        name,
        None,
        ValueMode::ImmutableOwned,
        &mut strings,
        &mut expressions,
        &mut values,
    )
    .expect("finite F64 projection");
    let type_id = expressions
        .view_owned(root)
        .expect("bound projection")
        .value_type_id()
        .expect("single scalar");
    assert_eq!(type_id, builtin_type_ids::fixed_scalar(FixedScalar::F64));
    let precision = NumericScalar::from_type_id(type_id, &TypeEnvironment::new())
        .and_then(|scalar| scalar.binary_float_precision())
        .expect("binary-float precision");
    assert_eq!(precision, BinaryFloatPrecision::Binary64);
    let payload = scalar_content(&expressions, &values, root)
        .as_f64()
        .expect("F64 bits");
    let text = format_finite_float(payload, precision).expect("format at resolved precision");
    assert_eq!(text.parse::<f64>().expect("roundtrip"), f64::MAX);
    assert!(
        format_finite_float(payload, BinaryFloatPrecision::Binary32).is_err(),
        "constant cannot collapse through F32"
    );
}

#[test]
fn raw_u32_constant_projects_exact_contract() {
    use moth_lexical::numeric::parse::NumberLiteralErrorReason;

    let mut strings = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut values = ConstValueStore::default();
    let name = strings.intern("HOST_UINT");
    for raw in [u64::from(u32::MAX), u64::from(u32::MAX) + 1, u64::MAX] {
        let definition = ExternalConstantDef {
            name: "HOST_UINT".to_owned(),
            data_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::U32)),
            value: ExternalConstantValue::Registered {
                scalar: FixedScalar::U32,
                raw: ExternalConstantRawValue::Unsigned(raw),
            },
        };
        let result = project_external_constant(
            &definition,
            name,
            None,
            ValueMode::ImmutableOwned,
            &mut strings,
            &mut expressions,
            &mut values,
        );
        if raw == u64::from(u32::MAX) {
            let root = result.expect("U32 maximum projects exactly");
            assert_eq!(
                expressions
                    .view_owned(root)
                    .expect("bound projection")
                    .value_type_id()
                    .expect("single scalar"),
                builtin_type_ids::fixed_scalar(FixedScalar::U32)
            );
            assert_eq!(
                scalar_content(&expressions, &values, root),
                FixedScalarValue::unsigned(FixedScalar::U32, raw).expect("U32 maximum")
            );
        } else {
            let ExpressionConstructionFailure::Diagnostic(error) =
                result.expect_err("out-of-U32 host content rejected")
            else {
                panic!("range is a source diagnostic")
            };
            let compact = compact_bag_for_test(DiagnosticBag::from(error), None);
            let error = compact.first().expect("the diagnostic is retained");
            let facts = error
                .family_view::<InvalidNumberLiteralView>()
                .expect("the diagnostic is an invalid number literal");
            assert_eq!(strings.resolve(facts.literal_text), raw.to_string());
            let expected_reason =
                NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32);
            let (expected_code, expected_scalar, expected_scale) =
                NumberLiteralCode::split(&expected_reason);
            assert_eq!(facts.code, expected_code);
            assert_eq!(facts.scalar, expected_scalar);
            assert_eq!(facts.scale, expected_scale);
        }
    }
}
