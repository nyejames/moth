//! Projection tests for host package constants.
//!
//! WHAT: covers exact scalar projection and source diagnostics for raw host constant payloads.
//! WHY: registrations retain host bits until a source use can diagnose non-finite `F64` and
//!      out-of-range `U32` values, while every materialised expression is valid at its own width.

use super::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::external_namespace_members::project_external_constant;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantRawValue, ExternalConstantValue,
    ExternalPackageRegistry,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

#[test]
fn external_float_constant_projects_exact_f64_value() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_PI");

    let definition = ExternalConstantDef {
        name: "HOST_PI".to_owned(),
        data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
        value: ExternalConstantValue::Registered {
            scalar: FixedScalar::F64,
            raw: ExternalConstantRawValue::BinaryFloat(std::f64::consts::PI),
        },
    };
    let projected = project_external_constant(
        &definition,
        constant_name,
        None,
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("an in-range host float should project");
    assert_eq!(
        projected.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    let ExpressionKind::FixedScalar(value) = projected.kind else {
        panic!("a projected host float should stay a fixed scalar literal");
    };
    assert_eq!(
        value.as_f64(),
        Some(std::f64::consts::PI),
        "external float projection is F64-only and must not round to a narrower width"
    );
}

#[test]
fn external_float_constant_rejects_non_finite_host_value() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_HUGE");

    for host_value in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let mut registry = ExternalPackageRegistry::new();
        let package_id = registry
            .register_package(
                "@test/non_finite",
                crate::builder_surface::PackageOrigin::Builder,
            )
            .expect("the host package should register");
        registry
            .register_external_constant(
                package_id,
                ExternalConstantDef {
                    name: "HOST_HUGE".to_owned(),
                    data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
                    value: ExternalConstantValue::Registered {
                        scalar: FixedScalar::F64,
                        raw: ExternalConstantRawValue::BinaryFloat(host_value),
                    },
                },
            )
            .expect("host bits remain available until source-level finiteness checking");
        let (_, definition) = registry
            .resolve_package_constant("@test/non_finite", "HOST_HUGE")
            .expect("the registered host constant should resolve");
        let error = project_external_constant(
            definition,
            constant_name,
            None,
            ValueMode::ImmutableOwned,
            &mut string_table,
        )
        .expect_err("a non-finite host value must not project");

        let DiagnosticPayload::CompileTimeEvaluationError {
            reason, operation, ..
        } = error.payload
        else {
            panic!("a non-finite projection should report the compile-time evaluation lane");
        };
        assert_eq!(reason, CompileTimeEvaluationErrorReason::FloatOverflow);
        assert_eq!(
            operation,
            Some(constant_name),
            "the diagnostic should name the host constant that was non-finite"
        );
    }
}

#[test]
fn core_math_constant_projects_exact_f64() {
    let mut registry = ExternalPackageRegistry::new();
    crate::builder_surface::core_packages::register_core_math_package(&mut registry);
    let (_, pi) = registry
        .resolve_package_constant("@core/math", "PI")
        .expect("core math PI should be registered");
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("PI");

    let projected = project_external_constant(
        pi,
        constant_name,
        None,
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("finite Core Math constants should project as exact F64");

    assert_eq!(
        projected.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    let ExpressionKind::FixedScalar(value) = projected.kind else {
        panic!("Core Math PI should project as an F64 fixed scalar");
    };
    assert_eq!(value.as_f64(), Some(std::f64::consts::PI));
}

/// Fixed constants project their exact materialised value and keep the declared scalar. `U32`
/// above `i32::MAX` proves the value never passes through a signed or native carrier.
#[test]
fn fixed_scalar_constants_keep_declared_identity_and_exact_value() {
    let mut string_table = StringTable::new();
    let fixed_values = [
        FixedScalarValue::signed(FixedScalar::I32, i64::from(i32::MIN))
            .expect("I32 minimum is in range"),
        FixedScalarValue::unsigned(FixedScalar::U32, u64::from(u32::MAX))
            .expect("U32 maximum is in range"),
        FixedScalarValue::binary_float(FixedScalar::F32, -0.0).expect("F32 negative zero is exact"),
        FixedScalarValue::binary_float(FixedScalar::F64, std::f64::consts::PI)
            .expect("PI is an exact binary64 value"),
    ];

    for value in fixed_values {
        let scalar = value.scalar();
        let constant_name = string_table.intern(scalar.name());
        let definition = ExternalConstantDef {
            name: scalar.name().to_owned(),
            data_type: ExternalAbiType::Fixed(scalar).into(),
            value: ExternalConstantValue::Fixed(value),
        };
        let projected = project_external_constant(
            &definition,
            constant_name,
            None,
            ValueMode::ImmutableOwned,
            &mut string_table,
        )
        .expect("a fixed constant always projects");

        assert_eq!(projected.type_id, builtin_type_ids::fixed_scalar(scalar));
        let ExpressionKind::FixedScalar(projected_value) = projected.kind else {
            panic!(
                "{} constant should remain a fixed scalar expression",
                scalar.name()
            );
        };
        assert_eq!(projected_value, value);
    }
}

/// A finite binary64 constant whose value overflows every narrower binary-float precision keeps
/// its exact scalar, so consumer precision and canonical formatting stay binary64 instead of
/// collapsing to a narrower width.
#[test]
fn max_finite_f64_constant_keeps_binary64_consumer_precision() {
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
    use moth_lexical::numeric::format::format_finite_float;
    use moth_lexical::numeric::precision::BinaryFloatPrecision;

    let mut string_table = StringTable::new();
    let value = FixedScalarValue::binary_float(FixedScalar::F64, f64::MAX)
        .expect("f64::MAX is a finite, exactly representable binary64 value");
    let definition = ExternalConstantDef {
        name: "MAX_F64".to_owned(),
        data_type: ExternalAbiType::Fixed(FixedScalar::F64).into(),
        value: ExternalConstantValue::Fixed(value),
    };

    let projected = project_external_constant(
        &definition,
        string_table.intern("MAX_F64"),
        None,
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("a finite binary64 constant projects as exact F64");

    assert_eq!(
        projected.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    let ExpressionKind::FixedScalar(projected_value) = projected.kind else {
        panic!("an F64 constant must remain a fixed scalar expression");
    };

    // Consumer fact: the projected identity resolves to binary64 precision, so the value remains
    // exactly representable at its own precision and round-trips through canonical formatting.
    let environment = TypeEnvironment::new();
    let precision = NumericScalar::from_type_id(projected.type_id, &environment)
        .and_then(|scalar| scalar.binary_float_precision())
        .expect("a fixed F64 constant resolves to a binary-float scalar");
    assert_eq!(precision, BinaryFloatPrecision::Binary64);

    let projected_f64 = projected_value
        .as_f64()
        .expect("an F64 fixed value exposes its binary64 payload");
    let text = format_finite_float(projected_f64, precision)
        .expect("the maximum finite binary64 value must format at its own precision");
    assert_eq!(
        text.parse::<f64>().expect("formatted text parses"),
        f64::MAX
    );

    // A narrower precision cannot carry the constant, so the identity's own precision is
    // load-bearing rather than an accidental wider width.
    assert!(
        format_finite_float(projected_f64, BinaryFloatPrecision::Binary32).is_err(),
        "the maximum finite binary64 value must not collapse through binary32"
    );
}

#[test]
fn raw_u32_constant_projects_exact_contract() {
    use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
    use crate::compiler_frontend::external_packages::ExternalSignatureType;
    use moth_lexical::numeric::parse::NumberLiteralErrorReason;

    fn project(
        value: u64,
        string_table: &mut StringTable,
    ) -> Result<Expression, crate::compiler_frontend::compiler_messages::CompilerDiagnostic> {
        let constant_name = string_table.intern("HOST_UINT");
        let definition = ExternalConstantDef {
            name: "HOST_UINT".to_owned(),
            data_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::U32)),
            value: ExternalConstantValue::Registered {
                scalar: FixedScalar::U32,
                raw: ExternalConstantRawValue::Unsigned(value),
            },
        };
        project_external_constant(
            &definition,
            constant_name,
            None,
            ValueMode::ImmutableOwned,
            string_table,
        )
    }

    let mut string_table = StringTable::new();

    // The projected type is the exact `U32` identity, and the payload keeps its exact value.
    let projected = project(u64::from(u32::MAX), &mut string_table)
        .expect("the u32 maximum should project exactly");
    assert_eq!(
        projected.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    let ExpressionKind::FixedScalar(value) = projected.kind else {
        panic!("an exact U32 constant must project as a fixed scalar expression");
    };
    assert_eq!(
        value,
        FixedScalarValue::unsigned(FixedScalar::U32, u64::from(u32::MAX))
            .expect("the u32 maximum is in range")
    );

    // The successor of `u32::MAX` and `u64::MAX` have no `U32` representation, so they report
    // the source-level range diagnostic.
    for rejected in [u64::from(u32::MAX) + 1, u64::MAX] {
        let Err(error) = project(rejected, &mut string_table) else {
            panic!("an out-of-U32-range host payload must not project");
        };
        let DiagnosticPayload::InvalidNumberLiteral {
            literal_text,
            reason,
        } = error.payload
        else {
            panic!("an out-of-range U32 constant should report the numeric range lane");
        };
        assert_eq!(string_table.resolve(literal_text), rejected.to_string());
        assert_eq!(
            reason,
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32)
        );
    }
}
