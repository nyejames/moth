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
use moth_lexical::numeric::fixed_scalar::FixedScalar;
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

#[test]
fn abi_numeric_constants_keep_fixed_i32_and_f64_identities() {
    let mut string_table = StringTable::new();
    let int_name = string_table.intern("ABI_INT");
    let int_definition = ExternalConstantDef {
        name: "ABI_INT".to_owned(),
        data_type: ExternalAbiType::I32.into(),
        value: ExternalConstantValue::Int(i32::MIN),
    };
    let int_expression = project_external_constant(
        &int_definition,
        int_name,
        None,
        numeric_profile(FloatPrecision::Bits32),
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("I32 ABI constant should project");
    assert_eq!(
        int_expression.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    );
    let ExpressionKind::FixedScalar(int_value) = int_expression.kind else {
        panic!("ABI I32 constant should remain a fixed I32 expression");
    };
    assert_eq!(int_value.as_i64(), Some(i64::from(i32::MIN)));

    let float_name = string_table.intern("ABI_FLOAT");
    let float_definition = ExternalConstantDef {
        name: "ABI_FLOAT".to_owned(),
        data_type: ExternalAbiType::F64.into(),
        value: ExternalConstantValue::Float(std::f64::consts::PI),
    };
    let float_expression = project_external_constant(
        &float_definition,
        float_name,
        None,
        numeric_profile(FloatPrecision::Bits32),
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect("F64 ABI constant should project");
    assert_eq!(
        float_expression.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
    );
    let ExpressionKind::FixedScalar(float_value) = float_expression.kind else {
        panic!("ABI F64 constant should remain a fixed F64 expression");
    };
    assert_eq!(float_value.as_f64(), Some(std::f64::consts::PI));
}

#[test]
fn abi_f64_constant_rejects_nonfinite_values() {
    let mut string_table = StringTable::new();
    let constant_name = string_table.intern("ABI_NONFINITE");
    let definition = ExternalConstantDef {
        name: "ABI_NONFINITE".to_owned(),
        data_type: ExternalAbiType::F64.into(),
        value: ExternalConstantValue::Float(f64::INFINITY),
    };

    let error = project_external_constant(
        &definition,
        constant_name,
        None,
        numeric_profile(FloatPrecision::Bits64),
        ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect_err("non-finite ABI F64 constants are not representable");
    let DiagnosticPayload::CompileTimeEvaluationError {
        reason,
        operation,
        numeric_profile: selected_profile,
    } = error.payload
    else {
        panic!("non-finite ABI F64 should report the compile-time evaluation lane");
    };
    assert_eq!(reason, CompileTimeEvaluationErrorReason::FloatOverflow);
    assert_eq!(operation, Some(constant_name));
    assert_eq!(
        selected_profile,
        Some(numeric_profile(FloatPrecision::Bits64))
    );
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
