//! Backend feature validation tests.
//!
//! WHAT: exercises backend-owned unsupported-feature diagnostics over synthetic HIR modules.
//! WHY: backend rejection policy should stay separate from frontend reachability tests while still
//! proving that unreachable HIR helper bodies do not block a backend build.

use crate::backends::backend_feature_validation::{
    BackendFeatureValidationError, BackendFeatureValidationInput,
    validate_hir_backend_feature_support,
};
use crate::backends::external_package_validation::BackendTarget;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, RuleDiagnosticKind, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarValue};
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode,
};
use crate::compiler_frontend::hir::reachability::{
    ReachableFloatStatementKind, collect_module_function_link_facts,
    collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn wasm_feature_validation_matches_explicit_numeric_cast_cases() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let int32_float32 = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits32,
    };
    let int32_float64 = NumericProfile {
        int_width: IntWidth::Bits32,
        float_precision: FloatPrecision::Bits64,
    };
    let int64_float32 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };
    let int64_float64 = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits64,
    };
    let int32_scalar = NumericScalar::Int;
    let float_scalar = NumericScalar::Float;
    let i32_scalar = NumericScalar::Fixed(FixedScalar::I32);
    let u32_scalar = NumericScalar::Fixed(FixedScalar::U32);
    let i64_scalar = NumericScalar::Fixed(FixedScalar::I64);
    let u64_scalar = NumericScalar::Fixed(FixedScalar::U64);
    let f16_scalar = NumericScalar::Fixed(FixedScalar::F16);
    let f32_scalar = NumericScalar::Fixed(FixedScalar::F32);
    let f64_scalar = NumericScalar::Fixed(FixedScalar::F64);
    let runtime_cast_rejection = Some(UnsupportedBackendFeatureReason::RuntimeCasts);

    let cases = [
        // Signed and unsigned integer carriers convert directly to F32 and F64.
        (int32_float32, i32_scalar, f32_scalar, None),
        (int32_float32, u32_scalar, f32_scalar, None),
        (int64_float32, i64_scalar, f32_scalar, None),
        (int64_float32, u64_scalar, f32_scalar, None),
        (int32_float64, i32_scalar, f64_scalar, None),
        (int32_float64, u32_scalar, f64_scalar, None),
        (int64_float64, i64_scalar, f64_scalar, None),
        (int64_float64, u64_scalar, f64_scalar, None),
        (int32_float32, int32_scalar, float_scalar, None),
        (int64_float64, int32_scalar, float_scalar, None),
        (int32_float32, f32_scalar, f64_scalar, None),
        // Nominal Float casts are infallible at equal precision.
        (int32_float32, f32_scalar, float_scalar, None),
        (int32_float32, float_scalar, f32_scalar, None),
        (int64_float64, f64_scalar, float_scalar, None),
        (int64_float64, float_scalar, f64_scalar, None),
        // Profile-selected Float may widen to fixed precision, never narrow.
        (int32_float32, float_scalar, f64_scalar, None),
        (int64_float64, f32_scalar, float_scalar, None),
        (
            int32_float32,
            f64_scalar,
            float_scalar,
            runtime_cast_rejection,
        ),
        (
            int64_float64,
            float_scalar,
            f32_scalar,
            runtime_cast_rejection,
        ),
        // Int-to-I32 depends on the selected Int width.
        (int32_float64, int32_scalar, i32_scalar, None),
        (
            int64_float64,
            int32_scalar,
            i32_scalar,
            runtime_cast_rejection,
        ),
        // Narrowing, float-to-int and same-type casts remain rejected.
        (
            int64_float64,
            i64_scalar,
            i32_scalar,
            runtime_cast_rejection,
        ),
        (
            int64_float64,
            u64_scalar,
            u32_scalar,
            runtime_cast_rejection,
        ),
        (
            int64_float64,
            f64_scalar,
            f32_scalar,
            runtime_cast_rejection,
        ),
        (
            int32_float32,
            f32_scalar,
            i32_scalar,
            runtime_cast_rejection,
        ),
        (
            int32_float32,
            i32_scalar,
            i32_scalar,
            runtime_cast_rejection,
        ),
        // F16 carriers widen directly, while fallible narrowing and wide integer pairs stay gated.
        (int32_float32, f16_scalar, f32_scalar, None),
        (int64_float64, f16_scalar, f64_scalar, None),
        (
            int32_float32,
            NumericScalar::Fixed(FixedScalar::I16),
            f16_scalar,
            None,
        ),
        (
            int32_float32,
            NumericScalar::Fixed(FixedScalar::I32),
            f16_scalar,
            runtime_cast_rejection,
        ),
        (
            int32_float32,
            f32_scalar,
            f16_scalar,
            runtime_cast_rejection,
        ),
        (
            int32_float32,
            f64_scalar,
            f16_scalar,
            runtime_cast_rejection,
        ),
        (
            int32_float32,
            f16_scalar,
            NumericScalar::Fixed(FixedScalar::I32),
            runtime_cast_rejection,
        ),
    ];

    for (profile, source, target, expected_rejection) in cases {
        let module = module_returning_cast_expression(numeric_cast_expression(0, source, target));
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: profile,
            },
            &mut string_table,
        );

        match (expected_rejection, result) {
            (None, Ok(())) => {}
            (None, Err(BackendFeatureValidationError::Diagnostic(_))) => {
                panic!("Wasm unexpectedly rejected {source:?} -> {target:?} in {profile:?}")
            }
            (None, Err(BackendFeatureValidationError::Infrastructure(error))) => {
                panic!("cast gate returned an infrastructure error: {error:?}")
            }
            (Some(reason), Err(BackendFeatureValidationError::Diagnostic(diagnostic))) => {
                assert_unsupported_feature(&diagnostic, &mut string_table, reason);
            }
            (Some(reason), Ok(())) => {
                panic!(
                    "Wasm unexpectedly allowed {source:?} -> {target:?} in {profile:?}; expected {reason:?}"
                )
            }
            (Some(_), Err(BackendFeatureValidationError::Infrastructure(error))) => {
                panic!("cast gate returned an infrastructure error: {error:?}")
            }
        }
    }
}

#[test]
fn wasm_feature_validation_rejects_reachable_mutable_parameters_for_both_int_widths() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let span = test_source_span(43);
    let module = mutable_parameter_module(builtin_type_ids::INT, true, Some(span), true, false);
    let reachability = test_reachability(&module);

    for int_width in [IntWidth::Bits32, IntWidth::Bits64] {
        let error = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile {
                    int_width,
                    float_precision: FloatPrecision::Bits64,
                },
            },
            &mut string_table,
        )
        .expect_err("Wasm must reject selected mutable function parameters");

        let BackendFeatureValidationError::Diagnostic(diagnostic) = error else {
            panic!("mutable-parameter gate returned an infrastructure error");
        };
        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::MutableFunctionParameters,
        );
        assert_eq!(diagnostic.primary_span, Some(span));
    }
}

#[test]
fn wasm_mutable_parameter_gate_ignores_unselected_helpers_and_mutable_locals() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let span = test_source_span(51);
    let cases = [
        (
            "unselected mutable helper",
            mutable_parameter_module(builtin_type_ids::INT, true, Some(span), false, false),
            BackendTarget::Wasm,
        ),
        (
            "immutable parameter and mutable local",
            mutable_parameter_module(builtin_type_ids::INT, false, Some(span), true, true),
            BackendTarget::Wasm,
        ),
        (
            "JavaScript mutable parameter",
            mutable_parameter_module(builtin_type_ids::INT, true, Some(span), true, false),
            BackendTarget::Js,
        ),
    ];

    for (description, module, target) in cases {
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile::STANDARD,
            },
            &mut string_table,
        );
        assert!(result.is_ok(), "{description} should remain supported");
    }
}

#[test]
fn wasm_mutable_parameter_gate_preserves_existing_type_reason_precedence() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let error_type = register_test_builtin_error_type(&mut type_environment);
    let aggregate_type =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U32), None);
    let span = test_source_span(59);

    for (parameter_type, expected_reason) in [
        (
            aggregate_type,
            UnsupportedBackendFeatureReason::FixedWidthScalarValues,
        ),
        (error_type, UnsupportedBackendFeatureReason::ErrorValues),
    ] {
        let module = mutable_parameter_module(parameter_type, true, Some(span), true, false);
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "pre-existing unsupported type reasons should precede mutable-parameter rejection",
        );
        assert_unsupported_feature(&diagnostic, &mut string_table, expected_reason);
        assert_eq!(diagnostic.primary_span, Some(span));
    }
}

#[test]
fn wasm_feature_validation_allows_numeric_text_casts_and_keeps_other_text_casts_gated() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    for domain in [
        NumericScalar::Int,
        NumericScalar::Fixed(FixedScalar::I8),
        NumericScalar::Fixed(FixedScalar::U64),
        NumericScalar::Fixed(FixedScalar::F16),
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
    ] {
        let module = module_returning_cast_expression(HirExpression {
            id: HirValueId(0),
            kind: HirExpressionKind::Cast {
                source: Box::new(numeric_scalar_expression(1, domain)),
                policy: BuiltinCastPolicyId::NumericToString(domain),
            },
            ty: builtin_type_ids::STRING,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
            span: None,
        });
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile::STANDARD,
            },
            &mut string_table,
        );
        assert!(
            result.is_ok(),
            "Wasm should allow NumericToString casts for {domain:?}"
        );
    }
    for (policy, source, target) in [
        (
            BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
            numeric_scalar_expression(1, NumericScalar::Float),
            builtin_type_ids::STRING,
        ),
        (
            BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int),
            typed_expression(1, builtin_type_ids::STRING, None),
            builtin_type_ids::INT,
        ),
    ] {
        let module = module_returning_cast_expression(HirExpression {
            id: HirValueId(0),
            kind: HirExpressionKind::Cast {
                source: Box::new(source),
                policy,
            },
            ty: target,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
            span: None,
        });
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "Wasm should keep profile Float expression casts and StringToNumeric casts gated",
        );
        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::RuntimeCasts,
        );
    }
}

#[test]
fn wasm_feature_validation_allows_byte_u8_expression_casts() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    for (source_scalar, target_scalar, policy) in [
        (
            FixedScalar::Byte,
            FixedScalar::U8,
            BuiltinCastPolicyId::ByteToU8,
        ),
        (
            FixedScalar::U8,
            FixedScalar::Byte,
            BuiltinCastPolicyId::U8ToByte,
        ),
    ] {
        let module = module_returning_cast_expression(HirExpression {
            id: HirValueId(0),
            kind: HirExpressionKind::Cast {
                source: Box::new(fixed_scalar_expression(1, source_scalar)),
                policy,
            },
            ty: builtin_type_ids::fixed_scalar(target_scalar),
            value_kind: ValueKind::RValue,
            region: RegionId(0),
            span: None,
        });
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile::STANDARD,
            },
            &mut string_table,
        );

        assert!(
            result.is_ok(),
            "Wasm should allow {source_scalar:?} -> {target_scalar:?}"
        );
    }
}

#[test]
fn wasm_feature_validation_rejects_statement_casts() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let statement = HirStatement {
        id: HirNodeId(10),
        kind: HirStatementKind::CastOp {
            policy: BuiltinCastPolicyId::NumericConversion {
                source: NumericScalar::Int,
                target: NumericScalar::Float,
            },
            source: HirExpression {
                id: HirValueId(11),
                kind: HirExpressionKind::Int(1),
                ty: builtin_type_ids::INT,
                value_kind: ValueKind::Const,
                region: RegionId(0),
                span: None,
            },
            result: None,
        },
        span: None,
    };
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![statement],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm should reject statement-shaped casts, including Int-to-Float",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::RuntimeCasts,
    );
    let statement = HirStatement {
        id: HirNodeId(12),
        kind: HirStatementKind::CastOp {
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::I8)),
            source: numeric_scalar_expression(13, NumericScalar::Fixed(FixedScalar::I8)),
            result: None,
        },
        span: None,
    };
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![statement],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm should reject statement-shaped NumericToString casts",
    );
    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::RuntimeCasts,
    );
}

#[test]
fn wasm_feature_validation_allows_reachable_trap_format_float_under_each_profile() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    for float_precision in [FloatPrecision::Bits32, FloatPrecision::Bits64] {
        let module = hir_module(
            FunctionId(0),
            vec![function(FunctionId(0), BlockId(0))],
            vec![block(
                BlockId(0),
                vec![float_statement(
                    10,
                    ReachableFloatStatementKind::FormatFloat,
                    NumericFailureMode::Trap,
                    None,
                )],
                HirTerminator::Return(unit_expression(0)),
            )],
        );
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile {
                    int_width: IntWidth::Bits64,
                    float_precision,
                },
            },
            &mut string_table,
        );

        assert!(
            result.is_ok(),
            "Wasm should allow trap-mode FormatFloat with {float_precision:?}"
        );
    }
}

#[test]
fn wasm_feature_validation_rejects_reachable_return_error_format_float() {
    let span = test_source_span(13);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![float_statement(
                10,
                ReachableFloatStatementKind::FormatFloat,
                NumericFailureMode::ReturnError,
                Some(span),
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm validation should reject ReturnError FormatFloat",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FloatFormatting,
    );
    assert_eq!(diagnostic.primary_span, Some(span));
}

#[test]
fn wasm_feature_validation_allows_reachable_trap_validate_float() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![float_statement(
                10,
                ReachableFloatStatementKind::ValidateFloat,
                NumericFailureMode::Trap,
                None,
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm should allow reachable trap-mode ValidateFloat statements"
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_return_error_validate_float() {
    let span = test_source_span(13);
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![float_statement(
                10,
                ReachableFloatStatementKind::ValidateFloat,
                NumericFailureMode::ReturnError,
                Some(span),
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm validation should reject reachable ReturnError ValidateFloat",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FloatBoundaryValidation,
    );
    assert_eq!(diagnostic.primary_span, Some(span));
}

#[test]
fn wasm_feature_validation_allows_reachable_trap_integer_numeric_op() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![numeric_op_statement(10, int_add_op(), None)],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm should allow reachable trap-mode integer numeric operations"
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_return_error_numeric_ops() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let domains = [
        NumericScalar::Int,
        NumericScalar::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
    ];

    for (index, domain) in domains.into_iter().enumerate() {
        let module = hir_module(
            FunctionId(0),
            vec![function(FunctionId(0), BlockId(0))],
            vec![block(
                BlockId(0),
                vec![numeric_op_statement_with_failure(
                    10,
                    HirNumericOp {
                        operator: NumericOperator::Add,
                        domain,
                    },
                    None,
                    NumericFailureMode::ReturnError,
                )],
                HirTerminator::Return(unit_expression(index as u32)),
            )],
        );

        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "Wasm validation should reject recoverable checked numeric operations",
        );
        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::CheckedNumericOperations,
        );
    }
}

#[test]
fn wasm_feature_validation_allows_trap_float_operations_without_f16() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let domains = [
        NumericScalar::Float,
        NumericScalar::Fixed(FixedScalar::F32),
        NumericScalar::Fixed(FixedScalar::F64),
    ];
    let operators = [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::Divide,
        NumericOperator::Remainder,
        NumericOperator::Power,
        NumericOperator::Negate,
    ];
    let profiles = [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile::STANDARD,
    ];

    for profile in profiles {
        for domain in domains {
            for operator in operators {
                let module = hir_module(
                    FunctionId(0),
                    vec![function(FunctionId(0), BlockId(0))],
                    vec![block(
                        BlockId(0),
                        vec![numeric_op_statement(
                            10,
                            HirNumericOp { operator, domain },
                            None,
                        )],
                        HirTerminator::Return(unit_expression(0)),
                    )],
                );
                let reachability = test_reachability(&module);
                let result = validate_hir_backend_feature_support(
                    BackendFeatureValidationInput {
                        hir: &module,
                        reachability: &reachability,
                        target: BackendTarget::Wasm,
                        type_environment: Some(&type_environment),
                        numeric_profile: profile,
                    },
                    &mut string_table,
                );

                assert!(
                    result.is_ok(),
                    "Wasm should allow trap-mode {operator:?} in {domain:?} under {profile:?}"
                );
            }
        }
    }
}

#[test]
fn wasm_feature_validation_keeps_f16_numeric_operations_gated() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![numeric_op_statement(
                10,
                HirNumericOp {
                    operator: NumericOperator::Add,
                    domain: NumericScalar::Fixed(FixedScalar::F16),
                },
                None,
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm should gate a synthetic F16 operation domain rather than a promoted F32 operation",
    );
    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
    );
}

#[test]
fn wasm_feature_validation_rejects_integer_division_on_float_domains() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![numeric_op_statement(
                10,
                HirNumericOp {
                    operator: NumericOperator::IntegerDivide,
                    domain: NumericScalar::Fixed(FixedScalar::F32),
                },
                None,
            )],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "Wasm should reject integer division in a float domain",
    );
    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
    );
}

#[test]
fn wasm_feature_validation_ignores_unreachable_checked_numeric_ops() {
    let span = None;
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![numeric_op_statement(10, int_mul_op(), span)],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore unreachable checked numeric operations"
    );
}

#[test]
fn wasm_feature_validation_ignores_unreachable_float_statements() {
    let span = None;
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![float_statement(
                    10,
                    ReachableFloatStatementKind::FormatFloat,
                    NumericFailureMode::Trap,
                    span,
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore unreachable float statements"
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_generic_values_with_or_without_span() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let generic_type = type_environment.intern_generic_instance(
        NominalTypeId(0),
        vec![builtin_type_ids::STRING].into_boxed_slice(),
    );
    let spanful = Some(SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::source_start(),
    ));

    for span in [spanful, None] {
        let module = hir_module(
            FunctionId(0),
            vec![function(FunctionId(0), BlockId(0))],
            vec![block(
                BlockId(0),
                vec![],
                HirTerminator::Return(typed_expression(0, generic_type, span)),
            )],
        );
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "Wasm validation should reject reachable generic runtime values",
        );

        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::GenericRuntimeValues,
        );
        assert_eq!(
            diagnostic.primary_span, span,
            "generic runtime rejection should preserve optional source provenance"
        );
    }
}

#[test]
fn wasm_feature_validation_ignores_unreachable_generic_runtime_values() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let generic_type = type_environment.intern_generic_instance(
        NominalTypeId(0),
        vec![builtin_type_ids::STRING].into_boxed_slice(),
    );
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Return(typed_expression(1, generic_type, None)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore generic runtime values in unreachable helpers"
    );
}

#[test]
fn wasm_feature_validation_rejects_reachable_canonical_error_values() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let error_type = register_test_builtin_error_type(&mut type_environment);
    let nested_error_type = type_environment.intern_tuple(vec![error_type]);
    let (_, error_member_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Box::new([FieldDefinition {
            name: PathId::ROOT,
            type_id: error_type,
            span: None,
        }]),
        generic_parameters: None,
        const_record: false,
    });
    let authored_span = test_source_span(7);

    let cases = [
        (
            module_returning_expression(error_type, Some(authored_span)),
            Some(authored_span),
        ),
        (module_returning_expression(error_type, None), None),
        (
            module_returning_expression(nested_error_type, Some(authored_span)),
            Some(authored_span),
        ),
        (
            module_returning_expression(error_member_type, Some(authored_span)),
            Some(authored_span),
        ),
    ];

    for (module, expected_span) in cases {
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "Wasm should reject reachable values containing the canonical builtin Error type",
        );
        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::ErrorValues,
        );
        assert_eq!(
            diagnostic.primary_span, expected_span,
            "Error-value diagnostics should use the authored reachable occurrence"
        );
    }
}

#[test]
fn wasm_feature_validation_rejects_fallible_control_flow_with_authored_span_and_allows_js() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let _builtin_error_type = register_test_builtin_error_type(&mut type_environment);
    let (_, custom_error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let carrier_type =
        type_environment.intern_fallible_carrier(builtin_type_ids::STRING, custom_error_type);
    let authored_span = test_source_span(13);
    let mut branch_module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::FallibleBranch {
                    result: typed_expression(0, carrier_type, None),
                    success_block: BlockId(1),
                    error_block: BlockId(2),
                },
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::ReturnSuccess(typed_expression(1, builtin_type_ids::STRING, None)),
            ),
            block(
                BlockId(2),
                vec![],
                HirTerminator::ReturnError(typed_expression(2, custom_error_type, None)),
            ),
        ],
    );
    branch_module
        .side_table
        .map_terminator_span(BlockId(0), authored_span);

    let diagnostic = wasm_feature_validation_diagnostic(
        &branch_module,
        &type_environment,
        &mut string_table,
        "Wasm should reject reachable fallible flow over a custom error channel",
    );
    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FallibleControlFlow,
    );
    assert_eq!(diagnostic.primary_span, Some(authored_span));

    let reachability = test_reachability(&branch_module);
    let js_result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &branch_module,
            reachability: &reachability,
            target: BackendTarget::Js,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );
    assert!(
        js_result.is_ok(),
        "JS validation should continue to allow reachable fallible control flow"
    );

    for terminator in [
        HirTerminator::ReturnSuccess(unit_expression(3)),
        HirTerminator::ReturnError(typed_expression(4, custom_error_type, None)),
    ] {
        let mut module = hir_module(
            FunctionId(0),
            vec![function(FunctionId(0), BlockId(0))],
            vec![block(BlockId(0), vec![], terminator)],
        );
        module
            .side_table
            .map_terminator_span(BlockId(0), authored_span);
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "Wasm should reject reachable fallible return terminators",
        );
        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::FallibleControlFlow,
        );
        assert_eq!(diagnostic.primary_span, Some(authored_span));
    }
}

#[test]
fn wasm_feature_validation_keeps_spanless_fallible_sites_and_ignores_unreachable_helpers() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let error_type = register_test_builtin_error_type(&mut type_environment);
    let carrier_type =
        type_environment.intern_fallible_carrier(builtin_type_ids::STRING, error_type);
    let unreachable_span = test_source_span(21);
    let mut module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::ReturnSuccess(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::FallibleBranch {
                    result: typed_expression(1, carrier_type, None),
                    success_block: BlockId(2),
                    error_block: BlockId(3),
                },
            ),
            block(
                BlockId(2),
                vec![],
                HirTerminator::ReturnSuccess(typed_expression(2, builtin_type_ids::STRING, None)),
            ),
            block(
                BlockId(3),
                vec![],
                HirTerminator::ReturnError(typed_expression(3, error_type, None)),
            ),
        ],
    );
    module
        .side_table
        .map_terminator_span(BlockId(1), unreachable_span);

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "synthetic reachable fallible control flow should still be rejected",
    );
    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FallibleControlFlow,
    );
    assert_eq!(
        diagnostic.primary_span, None,
        "an unreachable authored terminator must not supply provenance for synthetic HIR"
    );

    let mut unreachable_only_module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function_with_signature(FunctionId(1), BlockId(1), vec![], error_type),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::ReturnError(typed_expression(1, error_type, Some(unreachable_span))),
            ),
        ],
    );
    unreachable_only_module
        .side_table
        .map_terminator_span(BlockId(1), unreachable_span);
    let reachability = test_reachability(&unreachable_only_module);
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &unreachable_only_module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );
    assert!(
        result.is_ok(),
        "unreachable Error values and fallible helpers must not block Wasm validation"
    );
}

#[test]
fn wasm_feature_validation_preserves_numeric_and_cast_precedence_over_fallible_flow() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let error_type = register_test_builtin_error_type(&mut type_environment);
    let numeric_result_type =
        type_environment.intern_fallible_carrier(builtin_type_ids::INT, error_type);
    let numeric_module = hir_module(
        FunctionId(0),
        vec![function_with_signature(
            FunctionId(0),
            BlockId(0),
            vec![],
            numeric_result_type,
        )],
        vec![block_with_locals(
            BlockId(0),
            vec![HirLocal {
                id: LocalId(9000),
                ty: numeric_result_type,
                mutable: true,
                region: RegionId(0),
                span: None,
            }],
            vec![numeric_op_statement_with_failure(
                10,
                int_add_op(),
                None,
                NumericFailureMode::ReturnError,
            )],
            HirTerminator::ReturnError(typed_expression(0, error_type, None)),
        )],
    );
    let numeric_diagnostic = wasm_feature_validation_diagnostic(
        &numeric_module,
        &type_environment,
        &mut string_table,
        "checked numeric failures should retain precedence over Error values and fallible terminators",
    );
    assert_unsupported_feature(
        &numeric_diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
    );

    let cast_result_type =
        type_environment.intern_fallible_carrier(builtin_type_ids::INT, error_type);
    let fallible_cast_module = hir_module(
        FunctionId(0),
        vec![function_with_signature(
            FunctionId(0),
            BlockId(0),
            vec![],
            cast_result_type,
        )],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::ReturnSuccess(numeric_cast_expression(
                0,
                NumericScalar::Float,
                NumericScalar::Int,
            )),
        )],
    );
    let cast_diagnostic = wasm_feature_validation_diagnostic(
        &fallible_cast_module,
        &type_environment,
        &mut string_table,
        "fallible casts should retain precedence over Error values and fallible terminators",
    );
    assert_unsupported_feature(
        &cast_diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::RuntimeCasts,
    );
}

#[test]
fn backend_feature_validation_allows_direct_fixed_scalar_values() {
    let mut string_table = StringTable::new();
    let type_environment = TypeEnvironment::new();

    for scalar in FixedScalar::ALL {
        let module = module_returning_expression(builtin_type_ids::fixed_scalar(scalar), None);
        let reachability = test_reachability(&module);
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target: BackendTarget::Wasm,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile::STANDARD,
            },
            &mut string_table,
        );

        assert!(
            result.is_ok(),
            "direct {scalar:?} values should pass the Wasm shape gate"
        );
    }
}

#[test]
fn backend_feature_validation_rejects_fixed_width_scalars_only_for_wasm() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    // `{F16}?` reaches a fixed-width scalar through option and collection structure.
    let entries =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::F16), None);
    let optional_entries = type_environment.intern_option(entries);
    let spanful = Some(SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::source_start(),
    ));

    for span in [spanful, None] {
        let module = module_returning_expression(optional_entries, span);
        let reachability = test_reachability(&module);

        for target in [BackendTarget::Wasm, BackendTarget::Js] {
            let result = validate_hir_backend_feature_support(
                BackendFeatureValidationInput {
                    hir: &module,
                    reachability: &reachability,
                    target,
                    type_environment: Some(&type_environment),
                    numeric_profile: NumericProfile::STANDARD,
                },
                &mut string_table,
            );

            match target {
                BackendTarget::Wasm => {
                    let error = result.expect_err(
                        "Wasm should reject a reachable optional collection of fixed-width values",
                    );
                    let diagnostic = match error {
                        BackendFeatureValidationError::Diagnostic(diagnostic) => diagnostic,
                        BackendFeatureValidationError::Infrastructure(_) => {
                            panic!("expected a user-facing Wasm Rule diagnostic")
                        }
                    };
                    assert_unsupported_feature_for_target(
                        &diagnostic,
                        &mut string_table,
                        target,
                        UnsupportedBackendFeatureReason::FixedWidthScalarValues,
                    );
                    assert_eq!(
                        diagnostic.primary_span, span,
                        "fixed-width rejection should preserve optional source provenance"
                    );
                }
                BackendTarget::Js => assert!(
                    result.is_ok(),
                    "JS validation should accept fixed-width values after lowering support"
                ),
            }
        }
    }
}

#[test]
fn backend_feature_validation_rejects_fixed_width_scalars_in_nominal_members() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();

    // A struct field and a choice payload each reach a fixed-width scalar through their member type.
    let ledger_entries =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::I64), None);
    let (_, ledger_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Box::new([FieldDefinition {
            name: PathId::ROOT,
            type_id: ledger_entries,
            span: None,
        }]),
        generic_parameters: None,
        const_record: false,
    });

    let sample_value =
        type_environment.intern_option(builtin_type_ids::fixed_scalar(FixedScalar::F32));
    let (_, reading_type) = type_environment.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        variants: Box::new([ChoiceVariantDefinition {
            name: string_table.intern("Sample"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: Box::new([FieldDefinition {
                    name: PathId::ROOT,
                    type_id: sample_value,
                    span: None,
                }]),
            },
            span: None,
        }]),
        generic_parameters: None,
    });

    for member_type in [ledger_type, reading_type] {
        let module = module_returning_expression(member_type, None);
        let diagnostic = wasm_feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            "a reachable nominal value whose member type carries a fixed-width scalar must be rejected",
        );

        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::FixedWidthScalarValues,
        );
    }
}

#[test]
fn backend_feature_validation_rejects_fixed_width_return_type_without_an_expression() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let values =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U32), None);

    // Generated or synthetic HIR can declare a fixed-width return type with no reachable
    // expression of that type and no recorded source provenance.
    let module = hir_module(
        FunctionId(0),
        vec![function_with_signature(
            FunctionId(0),
            BlockId(0),
            vec![],
            values,
        )],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let diagnostic = feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        BackendTarget::Wasm,
        "a reachable return type carrying a fixed-width scalar must be rejected",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FixedWidthScalarValues,
    );
    assert_eq!(
        diagnostic.primary_span, None,
        "synthetic signatures without recorded source provenance stay spanless"
    );
}

#[test]
fn backend_feature_validation_prefers_authored_occurrence_over_synthetic_local_fallback() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let unsupported_type =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U32), None);
    let carrier_type =
        type_environment.intern_fallible_carrier(unsupported_type, builtin_type_ids::INT);
    let authored_span = SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start());

    for expected_span in [Some(authored_span), None] {
        let success_value = match expected_span {
            Some(span) => HirExpression {
                id: HirValueId(1),
                kind: HirExpressionKind::FallibleUnwrapSuccess {
                    result: Box::new(typed_expression(2, carrier_type, Some(span))),
                },
                ty: unsupported_type,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            },
            None => unit_expression(1),
        };
        let module = hir_module(
            FunctionId(0),
            vec![function(FunctionId(0), BlockId(0))],
            vec![
                block_with_locals(
                    BlockId(0),
                    vec![HirLocal {
                        id: LocalId(0),
                        ty: unsupported_type,
                        mutable: true,
                        region: RegionId(0),
                        span: None,
                    }],
                    vec![],
                    HirTerminator::FallibleBranch {
                        result: typed_expression(0, carrier_type, None),
                        success_block: BlockId(1),
                        error_block: BlockId(2),
                    },
                ),
                block(BlockId(1), vec![], HirTerminator::Return(success_value)),
                block(
                    BlockId(2),
                    vec![],
                    HirTerminator::Return(unit_expression(3)),
                ),
            ],
        );

        let diagnostic = feature_validation_diagnostic(
            &module,
            &type_environment,
            &mut string_table,
            BackendTarget::Wasm,
            "generated unsupported values remain rejected",
        );

        assert_unsupported_feature(
            &diagnostic,
            &mut string_table,
            UnsupportedBackendFeatureReason::FixedWidthScalarValues,
        );
        assert_eq!(
            diagnostic.primary_span, expected_span,
            "prefer an authored nested span, but keep synthetic-only rejection spanless"
        );
    }
}

#[test]
fn backend_feature_validation_prefers_authored_function_span_over_synthetic_local_fallback() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let values =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U32), None);
    let function = function_with_signature(FunctionId(0), BlockId(0), vec![], values);
    let span = SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start());
    let mut module = hir_module(
        FunctionId(0),
        vec![function.clone()],
        vec![block_with_locals(
            BlockId(0),
            vec![HirLocal {
                id: LocalId(0),
                ty: values,
                mutable: false,
                region: RegionId(0),
                span: None,
            }],
            vec![],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    module.side_table.map_function(Some(span), &function);

    let diagnostic = feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        BackendTarget::Wasm,
        "a reachable authored return type carrying a fixed-width scalar must be rejected",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FixedWidthScalarValues,
    );
    assert_eq!(
        diagnostic.primary_span,
        Some(span),
        "an authored signature span should beat an earlier spanless unsupported local fallback"
    );
}

#[test]
fn backend_feature_validation_reports_fixed_width_before_generic_runtime_values() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    // `Box of U8` is both a generic runtime value and a value carrying a fixed-width scalar.
    let box_of_u8 = type_environment.intern_generic_instance(
        NominalTypeId(0),
        vec![builtin_type_ids::fixed_scalar(FixedScalar::U8)].into_boxed_slice(),
    );
    let module = module_returning_expression(box_of_u8, None);

    let diagnostic = wasm_feature_validation_diagnostic(
        &module,
        &type_environment,
        &mut string_table,
        "a Box of U8 must report the fixed-width scalar reason before the generic runtime reason",
    );

    assert_unsupported_feature(
        &diagnostic,
        &mut string_table,
        UnsupportedBackendFeatureReason::FixedWidthScalarValues,
    );
}

#[test]
fn backend_feature_validation_ignores_unreachable_fixed_width_scalars() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let values =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U8), None);

    // The private helper mentions fixed-width values in its parameter, its local and its return
    // type, but nothing reaches it from the start function.
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function_with_signature(FunctionId(1), BlockId(1), vec![LocalId(0)], values),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block_with_locals(
                BlockId(1),
                vec![HirLocal {
                    id: LocalId(0),
                    ty: builtin_type_ids::fixed_scalar(FixedScalar::Byte),
                    mutable: false,
                    region: RegionId(0),
                    span: None,
                }],
                vec![],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = test_reachability(&module);
    assert_eq!(
        reachability.backend_selection().functions().len(),
        1,
        "the helper must stay outside the reachable selection"
    );

    for target in [BackendTarget::Wasm, BackendTarget::Js] {
        let result = validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module,
                reachability: &reachability,
                target,
                type_environment: Some(&type_environment),
                numeric_profile: NumericProfile::STANDARD,
            },
            &mut string_table,
        );

        assert!(
            result.is_ok(),
            "{target:?} should ignore fixed-width values in unreachable helpers"
        );
    }
}

#[test]
fn backend_gate_accepts_default_and_folded_assertion_messages() {
    for target in [BackendTarget::Js, BackendTarget::Wasm] {
        let mut string_table = StringTable::new();
        let mut type_environment = TypeEnvironment::new();
        let option_string = type_environment.intern_option(builtin_type_ids::STRING);

        for evaluation in [
            HirAssertionMessageEvaluation::Default,
            HirAssertionMessageEvaluation::Folded,
        ] {
            let module = assertion_message_module(option_string, evaluation);
            let reachability = test_reachability(&module);
            let result = validate_hir_backend_feature_support(
                BackendFeatureValidationInput {
                    hir: &module,
                    reachability: &reachability,
                    target,
                    type_environment: Some(&type_environment),
                    numeric_profile: NumericProfile::STANDARD,
                },
                &mut string_table,
            );

            assert!(
                result.is_ok(),
                "{target:?} should accept {evaluation:?} assertion messages"
            );
        }
    }
}

#[test]
fn wasm_gate_rejects_reachable_runtime_assertion_messages() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let option_string = type_environment.intern_option(builtin_type_ids::STRING);
    let module = assertion_message_module(option_string, HirAssertionMessageEvaluation::Runtime);
    let diagnostic = match validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &test_reachability(&module),
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    ) {
        Err(BackendFeatureValidationError::Diagnostic(diagnostic)) => diagnostic,
        Err(BackendFeatureValidationError::Infrastructure(error)) => {
            panic!("expected a target diagnostic, got infrastructure error: {error:?}")
        }
        Ok(()) => panic!("Wasm should reject a reachable runtime message"),
    };

    assert_unsupported_feature_for_target(
        &diagnostic,
        &mut string_table,
        BackendTarget::Wasm,
        UnsupportedBackendFeatureReason::RuntimeAssertionMessages,
    );
}

#[test]
fn js_gate_accepts_reachable_runtime_assertion_messages() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let option_string = type_environment.intern_option(builtin_type_ids::STRING);
    let module = assertion_message_module(option_string, HirAssertionMessageEvaluation::Runtime);

    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &test_reachability(&module),
            target: BackendTarget::Js,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "JavaScript should accept a reachable runtime message"
    );
}

#[test]
fn wasm_feature_validation_ignores_unreachable_runtime_assertion_messages() {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let option_string = type_environment.intern_option(builtin_type_ids::STRING);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::AssertFailure {
                    message: assertion_message_expression(
                        option_string,
                        HirAssertionMessageEvaluation::Runtime,
                    ),
                    message_evaluation: HirAssertionMessageEvaluation::Runtime,
                },
            ),
        ],
    );

    let reachability = test_reachability(&module);
    assert!(
        reachability.reachable_assertion_messages.is_empty(),
        "the helper assertion must not enter the start-function reachability facts"
    );
    let result = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: &module,
            reachability: &reachability,
            target: BackendTarget::Wasm,
            type_environment: Some(&type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "Wasm validation should ignore runtime assertion messages in unreachable helpers"
    );
}

fn wasm_feature_validation_diagnostic(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    expectation: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    feature_validation_diagnostic(
        module,
        type_environment,
        string_table,
        BackendTarget::Wasm,
        expectation,
    )
}

fn feature_validation_diagnostic(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    target: BackendTarget,
    expectation: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let reachability = test_reachability(module);
    let error = validate_hir_backend_feature_support(
        BackendFeatureValidationInput {
            hir: module,
            reachability: &reachability,
            target,
            type_environment: Some(type_environment),
            numeric_profile: NumericProfile::STANDARD,
        },
        string_table,
    )
    .expect_err(expectation);

    match error {
        BackendFeatureValidationError::Diagnostic(diagnostic) => diagnostic,
        BackendFeatureValidationError::Infrastructure(_) => {
            panic!("expected a user-facing Rule diagnostic, not an infrastructure error")
        }
    }
}

fn test_reachability(
    module: &HirModule,
) -> crate::compiler_frontend::hir::reachability::HirReachability {
    let facts = collect_module_function_link_facts(module)
        .expect("test HIR should produce function link facts");
    collect_reachability_from_function_link_facts(
        &facts,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("test HIR should produce entry reachability")
}

fn assert_unsupported_feature(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    string_table: &mut StringTable,
    expected_reason: UnsupportedBackendFeatureReason,
) {
    assert_unsupported_feature_for_target(
        diagnostic,
        string_table,
        BackendTarget::Wasm,
        expected_reason,
    );
}

fn assert_unsupported_feature_for_target(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    string_table: &mut StringTable,
    target: BackendTarget,
    expected_reason: UnsupportedBackendFeatureReason,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBackendFeature)
    );

    let DiagnosticPayload::UnsupportedBackendFeature {
        backend_name,
        reason,
    } = &diagnostic.payload
    else {
        panic!("expected UnsupportedBackendFeature payload");
    };

    assert_eq!(*backend_name, string_table.intern(target.as_str()));
    assert_eq!(*reason, expected_reason);
}

fn assertion_message_module(
    option_string: crate::compiler_frontend::datatypes::ids::TypeId,
    evaluation: HirAssertionMessageEvaluation,
) -> HirModule {
    hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::AssertFailure {
                message: assertion_message_expression(option_string, evaluation),
                message_evaluation: evaluation,
            },
        )],
    )
}

fn assertion_message_expression(
    option_string: crate::compiler_frontend::datatypes::ids::TypeId,
    evaluation: HirAssertionMessageEvaluation,
) -> HirExpression {
    match evaluation {
        HirAssertionMessageEvaluation::Default => HirExpression {
            id: HirValueId(10),
            kind: HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: 0,
                fields: vec![],
            },
            ty: option_string,
            value_kind: ValueKind::Const,
            region: RegionId(0),
            span: None,
        },
        HirAssertionMessageEvaluation::Folded | HirAssertionMessageEvaluation::Runtime => {
            let value = HirExpression {
                id: HirValueId(11),
                kind: HirExpressionKind::StringLiteral("folded".to_owned()),
                ty: builtin_type_ids::STRING,
                value_kind: if matches!(evaluation, HirAssertionMessageEvaluation::Folded) {
                    ValueKind::Const
                } else {
                    ValueKind::RValue
                },
                region: RegionId(0),
                span: None,
            };
            HirExpression {
                id: HirValueId(12),
                kind: HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 1,
                    fields: vec![HirVariantField { name: None, value }],
                },
                ty: option_string,
                value_kind: ValueKind::RValue,
                region: RegionId(0),
                span: None,
            }
        }
    }
}

fn hir_module(
    start_function: FunctionId,
    functions: Vec<HirFunction>,
    blocks: Vec<HirBlock>,
) -> HirModule {
    let mut module = HirModule::new();
    module.start_function = Some(start_function);
    module.functions = functions;
    module.blocks = blocks;
    for function in &module.functions {
        module
            .function_provenance
            .insert(function.id, Default::default());
    }
    module
}

fn function(id: FunctionId, entry: BlockId) -> HirFunction {
    function_with_signature(id, entry, vec![], builtin_type_ids::NONE)
}

fn function_with_signature(
    id: FunctionId,
    entry: BlockId,
    params: Vec<LocalId>,
    return_type: TypeId,
) -> HirFunction {
    HirFunction {
        id,
        entry,
        params,
        return_type,
    }
}

fn mutable_parameter_module(
    parameter_type: TypeId,
    parameter_mutable: bool,
    parameter_span: Option<SourceSpan>,
    helper_reachable: bool,
    include_mutable_local: bool,
) -> HirModule {
    let mut start_statements = Vec::new();
    if helper_reachable {
        start_statements.push(HirStatement {
            id: HirNodeId(10),
            kind: HirStatementKind::Call {
                target: CallTarget::Local(FunctionId(1)),
                args: vec![typed_expression(11, parameter_type, None)],
                result: None,
            },
            span: None,
        });
    }

    let mut helper_locals = vec![HirLocal {
        id: LocalId(1),
        ty: parameter_type,
        mutable: parameter_mutable,
        region: RegionId(0),
        span: parameter_span,
    }];
    if include_mutable_local {
        helper_locals.push(HirLocal {
            id: LocalId(2),
            ty: builtin_type_ids::INT,
            mutable: true,
            region: RegionId(0),
            span: Some(test_source_span(67)),
        });
    }

    hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function_with_signature(
                FunctionId(1),
                BlockId(1),
                vec![LocalId(1)],
                builtin_type_ids::NONE,
            ),
        ],
        vec![
            block(
                BlockId(0),
                start_statements,
                HirTerminator::Return(unit_expression(0)),
            ),
            block_with_locals(
                BlockId(1),
                helper_locals,
                vec![],
                HirTerminator::Return(unit_expression(2)),
            ),
        ],
    )
}

fn block(id: BlockId, statements: Vec<HirStatement>, terminator: HirTerminator) -> HirBlock {
    block_with_locals(id, vec![], statements, terminator)
}

fn block_with_locals(
    id: BlockId,
    locals: Vec<HirLocal>,
    statements: Vec<HirStatement>,
    terminator: HirTerminator,
) -> HirBlock {
    HirBlock {
        id,
        region: RegionId(0),
        locals,
        statements,
        terminator,
    }
}

fn numeric_op_statement(id: u32, op: HirNumericOp, span: Option<SourceSpan>) -> HirStatement {
    numeric_op_statement_with_failure(id, op, span, NumericFailureMode::Trap)
}

fn numeric_op_statement_with_failure(
    id: u32,
    op: HirNumericOp,
    span: Option<SourceSpan>,
    failure_mode: NumericFailureMode,
) -> HirStatement {
    let left = numeric_scalar_expression(id + 100, op.domain);
    let operands = if op.operator.is_unary() {
        HirNumericOperands::Unary { operand: left }
    } else {
        HirNumericOperands::Binary {
            left,
            right: numeric_scalar_expression(id + 101, op.domain),
        }
    };

    HirStatement {
        id: HirNodeId(id),
        kind: HirStatementKind::NumericOp {
            op,
            failure_mode,
            operands,
            result: LocalId(9000),
        },
        span,
    }
}

fn numeric_scalar_expression(id: u32, domain: NumericScalar) -> HirExpression {
    let (kind, ty) = match domain {
        NumericScalar::Int => (HirExpressionKind::Int(1), builtin_type_ids::INT),
        NumericScalar::Float => (HirExpressionKind::Float(1.5), builtin_type_ids::FLOAT),
        NumericScalar::Fixed(scalar) => (
            HirExpressionKind::FixedScalar(fixed_scalar_value(scalar)),
            builtin_type_ids::fixed_scalar(scalar),
        ),
    };

    HirExpression {
        id: HirValueId(id),
        kind,
        ty,
        value_kind: ValueKind::Const,
        region: RegionId(0),
        span: None,
    }
}

fn numeric_cast_expression(
    id: u32,
    source_domain: NumericScalar,
    target_domain: NumericScalar,
) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Cast {
            source: Box::new(numeric_scalar_expression(id + 1, source_domain)),
            policy: BuiltinCastPolicyId::NumericConversion {
                source: source_domain,
                target: target_domain,
            },
        },
        ty: numeric_scalar_type_id(target_domain),
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span: None,
    }
}

fn numeric_scalar_type_id(domain: NumericScalar) -> TypeId {
    match domain {
        NumericScalar::Int => builtin_type_ids::INT,
        NumericScalar::Float => builtin_type_ids::FLOAT,
        NumericScalar::Fixed(scalar) => builtin_type_ids::fixed_scalar(scalar),
    }
}

fn fixed_scalar_expression(id: u32, scalar: FixedScalar) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::FixedScalar(fixed_scalar_value(scalar)),
        ty: builtin_type_ids::fixed_scalar(scalar),
        value_kind: ValueKind::Const,
        region: RegionId(0),
        span: None,
    }
}

fn fixed_scalar_value(scalar: FixedScalar) -> FixedScalarValue {
    match scalar {
        FixedScalar::I8 | FixedScalar::I16 | FixedScalar::I32 | FixedScalar::I64 => {
            FixedScalarValue::signed(scalar, 1).expect("one fits every signed fixed scalar")
        }
        FixedScalar::U8
        | FixedScalar::U16
        | FixedScalar::U32
        | FixedScalar::U64
        | FixedScalar::Byte => {
            FixedScalarValue::unsigned(scalar, 1).expect("one fits every unsigned fixed scalar")
        }
        FixedScalar::F16 | FixedScalar::F32 | FixedScalar::F64 => {
            FixedScalarValue::binary_float(scalar, 1.5)
                .expect("1.5 is a finite fixed binary-float value")
        }
    }
}

fn int_add_op() -> HirNumericOp {
    HirNumericOp {
        operator: NumericOperator::Add,
        domain: NumericScalar::Int,
    }
}

fn int_mul_op() -> HirNumericOp {
    HirNumericOp {
        operator: NumericOperator::Multiply,
        domain: NumericScalar::Int,
    }
}

fn float_statement(
    id: u32,
    kind: ReachableFloatStatementKind,
    failure_mode: NumericFailureMode,
    span: Option<SourceSpan>,
) -> HirStatement {
    let source = HirExpression {
        id: HirValueId(id + 100),
        kind: HirExpressionKind::Float(1.5),
        ty: builtin_type_ids::FLOAT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
        span: None,
    };
    let result = LocalId(9000);

    HirStatement {
        id: HirNodeId(id),
        kind: match kind {
            ReachableFloatStatementKind::FormatFloat => HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            },
            ReachableFloatStatementKind::ValidateFloat => HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            },
        },
        span,
    }
}

/// Builds a value expression carrying the semantic type a gate has to classify.
fn typed_expression(id: u32, ty: TypeId, span: Option<SourceSpan>) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::TupleConstruct { elements: vec![] },
        ty,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span,
    }
}

/// Builds a module whose reachable start function returns one expression of `ty`.
fn module_returning_expression(ty: TypeId, span: Option<SourceSpan>) -> HirModule {
    hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::Return(typed_expression(0, ty, span)),
        )],
    )
}

fn register_test_builtin_error_type(type_environment: &mut TypeEnvironment) -> TypeId {
    let (_, error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: PathId::ROOT,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    type_environment
        .register_canonical_identity(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
            error_type,
        )
        .expect("synthetic Error type should receive the builtin canonical identity");
    error_type
}

fn test_source_span(start: u32) -> SourceSpan {
    let mut extended_spans = ExtendedSpanBuilder::default();
    let local_span = LocalSpan::exact(start, 1, &mut extended_spans)
        .expect("small test source ranges should fit inline");
    SourceSpan::new(SourceId::COMPILATION_ROOT, local_span)
}

fn module_returning_cast_expression(expression: HirExpression) -> HirModule {
    let return_type = expression.ty;
    hir_module(
        FunctionId(0),
        vec![function_with_signature(
            FunctionId(0),
            BlockId(0),
            vec![],
            return_type,
        )],
        vec![block(BlockId(0), vec![], HirTerminator::Return(expression))],
    )
}

fn unit_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::TupleConstruct { elements: vec![] },
        ty: builtin_type_ids::NONE,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span: None,
    }
}
