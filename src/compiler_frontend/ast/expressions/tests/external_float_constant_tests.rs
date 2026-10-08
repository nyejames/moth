//! Boundary-precision projection tests for host package `Float` constants.
//!
//! WHAT: covers the rounding and overflow rules `Expression::float_from_external_constant` applies
//!       when a host `f64` becomes a Moth `Float`.
//! WHY: `ExpressionKind::Float` is finite by contract and must hold a value exactly representable
//!      at the selected boundary precision, so a projection that skipped the rounding or admitted a
//!      non-finite result would leak an unrepresentable value into every later stage.

use super::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::external_namespace_members::project_external_constant;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticPayload,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantValue, ExternalPackageRegistry,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

fn numeric_profile(float_precision: FloatPrecision) -> NumericProfile {
    NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision,
    }
}

#[test]
fn external_float_constant_rounds_to_boundary_precision() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_PI");

    let narrow = Expression::float_from_external_constant(
        std::f64::consts::PI,
        numeric_profile(FloatPrecision::Bits32),
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect("an in-range host float should project at either precision");
    let ExpressionKind::Float(narrow_value) = narrow.kind else {
        panic!("a projected host float should stay a float literal");
    };
    assert_eq!(
        narrow_value,
        f64::from(std::f32::consts::PI),
        "a 32-bit boundary should store the host value rounded to its precision"
    );

    let wide = Expression::float_from_external_constant(
        std::f64::consts::PI,
        numeric_profile(FloatPrecision::Bits64),
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect("an in-range host float should project at either precision");
    let ExpressionKind::Float(wide_value) = wide.kind else {
        panic!("a projected host float should stay a float literal");
    };
    assert_eq!(
        wide_value,
        std::f64::consts::PI,
        "a 64-bit boundary should keep the host value unrounded"
    );
}

#[test]
fn external_float_constant_rejects_projection_overflowing_precision() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("HOST_HUGE");

    let error = Expression::float_from_external_constant(
        f64::MAX,
        numeric_profile(FloatPrecision::Bits32),
        constant_name,
        None,
        ValueMode::ImmutableOwned,
    )
    .expect_err("a host value beyond the boundary precision is not representable");

    let DiagnosticPayload::CompileTimeEvaluationError {
        reason,
        operation,
        numeric_profile: selected_profile,
    } = error.payload
    else {
        panic!("a precision overflow should report the compile-time evaluation lane");
    };
    assert_eq!(reason, CompileTimeEvaluationErrorReason::FloatOverflow);
    assert_eq!(
        operation,
        Some(constant_name),
        "the diagnostic should name the host constant that overflowed"
    );
    assert_eq!(
        selected_profile,
        Some(numeric_profile(FloatPrecision::Bits32))
    );
}

#[test]
fn core_math_constant_projects_native_float_at_profile_precision() {
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
        numeric_profile(FloatPrecision::Bits32),
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("finite Core Math constants should project at the selected Float precision");

    assert_eq!(projected.type_id, builtin_type_ids::FLOAT);
    let ExpressionKind::Float(value) = projected.kind else {
        panic!("Core Math PI should remain a native Float expression");
    };
    assert_eq!(value, f64::from(std::f32::consts::PI));
}

/// Fixed constants project their exact materialised value and keep the declared scalar under any
/// profile. `U32` above `i32::MAX` proves the value never passes through a signed or native carrier.
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
            numeric_profile(FloatPrecision::Bits32),
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

#[test]
fn native_uint_constant_projects_exact_payload() {
    use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
    use crate::compiler_frontend::external_packages::ExternalSignatureType;
    use moth_lexical::numeric::parse::NumberLiteralErrorReason;

    fn uint32_profile() -> NumericProfile {
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        }
    }

    fn uint64_profile() -> NumericProfile {
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        }
    }

    fn project(
        value: u64,
        profile: NumericProfile,
        string_table: &mut StringTable,
    ) -> Result<Expression, crate::compiler_frontend::compiler_messages::CompilerDiagnostic> {
        let constant_name = string_table.intern("HOST_UINT");
        let definition = ExternalConstantDef {
            name: "HOST_UINT".to_owned(),
            data_type: ExternalSignatureType::NativeUint,
            value: ExternalConstantValue::Uint(value),
        };
        project_external_constant(
            &definition,
            constant_name,
            None,
            profile,
            ValueMode::ImmutableOwned,
            string_table,
        )
    }

    let mut string_table = StringTable::new();

    // Uint32 accepts its own maximum exactly.
    let projected = project(u64::from(u32::MAX), uint32_profile(), &mut string_table)
        .expect("the Uint32 maximum should project under Bits32");
    assert_eq!(projected.type_id, builtin_type_ids::UINT);
    let ExpressionKind::Uint(value) = projected.kind else {
        panic!("a Uint constant must project as a Uint expression, not a signed reinterpretation");
    };
    assert_eq!(value, u64::from(u32::MAX));

    // Uint32 rejects the successor and u64::MAX with the source-level range diagnostic.
    for rejected in [u64::from(u32::MAX) + 1, u64::MAX] {
        let error = project(rejected, uint32_profile(), &mut string_table)
            .expect_err("an out-of-profile Uint payload must not project under Bits32");
        let DiagnosticPayload::InvalidNumberLiteral { reason, .. } = error.payload else {
            panic!("an out-of-profile Uint constant should report the numeric range lane");
        };
        assert_eq!(
            reason,
            NumberLiteralErrorReason::OutsideUintRange(IntWidth::Bits32)
        );
    }

    // Uint64 keeps the full range exact, including u64::MAX.
    let projected = project(u64::MAX, uint64_profile(), &mut string_table)
        .expect("u64::MAX should project under Bits64");
    let ExpressionKind::Uint(value) = projected.kind else {
        panic!("a Uint64 constant must project as a Uint expression");
    };
    assert_eq!(value, u64::MAX);
}
