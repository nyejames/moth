//! Pre-lowering validation for backend-specific HIR feature support.
//!
//! WHAT: rejects reachable HIR operations that are valid language semantics but unsupported by
//! a selected backend target.
//! WHY: backend lowerers should receive only features they can lower, and users should see a
//! structured source diagnostic instead of a backend-internal lowering error.

use crate::backends::external_package_validation::BackendTarget;
use crate::backends::js::JsNumericCarrier;
use crate::compiler_frontend::builtins::casts::evidence::numeric_conversion_fallibility;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, TypeConstructor, TypeId,
};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalJsLowering, ExternalPackageRegistry, ExternalSignatureType,
};
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{HirNumericOperands, NumericFailureMode};
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reachability::{
    HirBackendSelection, HirReachability, ReachableAssertionMessageUse, ReachableExternalCall,
    ReachableFloatStatementKind, ReachableFloatStatementUse, ReachableMapUse, ReachableMapUseKind,
    ReachableNumericOpUse, ReachableReactiveSinkKind, ReachableReactiveSinkUse,
    ReachableReactiveTemplateUse, ReachableRuntimeCastForm, ReachableRuntimeCastUse,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::{FxHashMap, FxHashSet};

/// Failure mode for backend feature validation.
///
/// WHAT: either a user-facing diagnostic for an unsupported selected operation, or an
/// infrastructure error if supplied compiler metadata is inconsistent.
pub enum BackendFeatureValidationError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

/// Explicit build-owned selection consumed by backend feature validation.
///
/// WHAT: backend-neutral validation receives the exact reachable union, active target, selected
///       numeric profile, optional module type environment and optional external-package registry.
#[derive(Clone, Debug)]
pub struct BackendFeatureValidationInput<'a> {
    pub hir: &'a HirModule,
    pub reachability: &'a HirReachability,
    pub target: BackendTarget,
    pub type_environment: Option<&'a TypeEnvironment>,
    pub numeric_profile: NumericProfile,
    pub external_package_registry: Option<&'a ExternalPackageRegistry>,
}

/// Validates HIR runtime features that are target-specific after frontend semantics are complete.
///
/// WHAT: hashmap construction/use, reactive runtime features, runtime casts, checked numeric
///       operations, generic runtime values, Moth `Error` values and fallible control flow are
///       legal HIR, but this validator reports Wasm features that the target cannot lower. Its
///       bounded scalar step accepts direct integer/Byte and F32/F64 values while retaining gates
///       for F16 and aggregates.
/// WHY: fail early with a structured Rule error carrying the source span instead of a vague
///      backend-internal lowering failure.
pub fn validate_hir_backend_feature_support(
    input: BackendFeatureValidationInput<'_>,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let reachability = input.reachability;

    match input.target {
        BackendTarget::Wasm => {
            // Keep the fixed-value type-shape check first so its diagnostic retains priority over
            // later Wasm operation gates.
            validate_fixed_width_scalar_values(
                input.hir,
                input.type_environment,
                reachability.backend_selection(),
                input.target,
                string_table,
            )?;

            // Wasm has no failure-edge message presentation yet. Default and fully folded
            // optional values remain target-neutral and are accepted.
            validate_runtime_assertion_messages(
                &reachability.reachable_assertion_messages,
                input.target,
                string_table,
            )?;
            validate_wasm_cross_module_calls(input.hir, reachability, input.target, string_table)?;
            // Wasm still gates hashmaps, reactive runtime features, recoverable numeric failures,
            // statement casts, formatting/boundary validation, generic values and the later
            // Error-value/fallible-control-flow checks.
            validate_wasm_maps(&reachability.reachable_map_uses, input.target, string_table)?;
            validate_wasm_reactive_features(
                &reachability.reachable_reactive_templates,
                input.target,
                string_table,
            )?;
            validate_wasm_runtime_casts(
                &reachability.reachable_runtime_casts,
                input.target,
                input.numeric_profile,
                string_table,
            )?;
            validate_wasm_checked_numeric_ops(
                &reachability.reachable_numeric_ops,
                input.target,
                input.numeric_profile,
                string_table,
            )?;
            validate_wasm_float_statements(
                &reachability.reachable_float_statements,
                input.target,
                string_table,
            )?;
            validate_wasm_generic_runtime_values(
                input.hir,
                input.type_environment,
                reachability.backend_selection().blocks(),
                input.target,
                string_table,
            )?;
            // Keep the existing type, cast and numeric gates ahead of these broader deferred
            // features so their more specific diagnostics retain precedence.
            validate_wasm_error_values(
                input.hir,
                input.type_environment,
                reachability.backend_selection(),
                input.target,
                string_table,
            )?;
            validate_wasm_fallible_control_flow(
                input.hir,
                reachability.backend_selection().blocks(),
                input.target,
                string_table,
            )?;
        }
        BackendTarget::Js => {
            validate_js_external_numeric_profile_boundaries(
                &reachability.reachable_external_calls,
                input.external_package_registry,
                input.type_environment,
                input.numeric_profile,
                input.target,
                string_table,
            )?;

            // JS supports V1 top-level runtime fragment sinks, but not reactive template values
            // flowing into external/host calls such as `io.line(...)`.
            validate_js_reactive_sinks(
                input.hir,
                &reachability.reachable_reactive_sinks,
                input.target,
                string_table,
            )?;
        }
    }

    Ok(())
}

/// Rejects numeric Moth carriers that the current external-module glue cannot adapt safely.
///
/// WHAT: checks reachable external ES-module exports against their fixed I32/F64 signatures and
///       the concrete types crossing each selected call boundary.
/// WHY: generated glue currently forwards host values without adapting numeric carriers, so
///      BigInt/Number mixing and unrounded Float32 values must be rejected before JS lowering.
fn validate_js_external_numeric_profile_boundaries(
    calls: &[ReachableExternalCall],
    external_package_registry: Option<&ExternalPackageRegistry>,
    type_environment: Option<&TypeEnvironment>,
    numeric_profile: NumericProfile,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let int_uses_bigint = matches!(
        JsNumericCarrier::for_scalar(NumericScalar::Int, numeric_profile),
        Some(JsNumericCarrier::BigInteger { .. })
    );
    let float_uses_binary32 = matches!(
        JsNumericCarrier::for_scalar(NumericScalar::Float, numeric_profile),
        Some(JsNumericCarrier::BinaryFloat {
            precision: BinaryFloatPrecision::Binary32,
        })
    );

    if (!int_uses_bigint && !float_uses_binary32) || calls.is_empty() {
        return Ok(());
    }

    let Some(registry) = external_package_registry else {
        return Err(BackendFeatureValidationError::Infrastructure(Box::new(
            CompilerError::compiler_error(
                "JavaScript numeric-profile boundary validation requires the external-package registry",
            ),
        )));
    };
    let type_environment = require_type_environment(
        type_environment,
        target,
        "external numeric-profile boundaries",
    )?;
    let mut backend_type_facts = BackendTypeFacts::new(type_environment, None);

    for call in calls {
        let Some(function) = registry.get_function_by_id(call.function_id) else {
            return Err(BackendFeatureValidationError::Infrastructure(Box::new(
                CompilerError::compiler_error(format!(
                    "Reachable external call {:?} is missing its registered function definition",
                    call.function_id
                )),
            )));
        };
        if !matches!(
            function.lowerings.js.as_ref(),
            Some(ExternalJsLowering::ExternalModuleExport { .. })
        ) {
            continue;
        }

        let signature_crosses_unsupported_numeric = function.parameters.iter().any(|parameter| {
            signature_type_uses_unsupported_numeric_profile(
                &parameter.language_type,
                int_uses_bigint,
                float_uses_binary32,
            )
        }) || function.returns.iter().any(|returned| {
            signature_type_uses_unsupported_numeric_profile(
                &returned.value_type,
                int_uses_bigint,
                float_uses_binary32,
            )
        });

        let argument_crosses_unsupported_numeric = call.argument_types.iter().any(|type_id| {
            backend_type_facts.contains_external_numeric(
                *type_id,
                int_uses_bigint,
                float_uses_binary32,
            )
        });
        let result_crosses_unsupported_numeric = match call.result_type {
            Some(result_type) => {
                let success_type = if function.is_fallible() {
                    external_fallible_success_type(result_type, type_environment)?
                } else {
                    Some(result_type)
                };
                success_type.is_some_and(|type_id| {
                    backend_type_facts.contains_external_numeric(
                        type_id,
                        int_uses_bigint,
                        float_uses_binary32,
                    )
                })
            }
            None => false,
        };

        if !signature_crosses_unsupported_numeric
            && !argument_crosses_unsupported_numeric
            && !result_crosses_unsupported_numeric
        {
            continue;
        }

        let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
            string_table.intern(target.as_str()),
            UnsupportedBackendFeatureReason::ExternalNumericProfileBoundary,
            call.span,
        );
        return Err(BackendFeatureValidationError::Diagnostic(diagnostic));
    }

    Ok(())
}

/// Error-code values on the fallible channel are compiler-owned Moth `Error` values, not raw
/// numeric results from an external signature. Only inspect the success payload here.
fn external_fallible_success_type(
    result_type: TypeId,
    type_environment: &TypeEnvironment,
) -> Result<Option<TypeId>, BackendFeatureValidationError> {
    let Some(TypeDefinition::Constructed(carrier)) = type_environment.get(result_type) else {
        return Err(BackendFeatureValidationError::Infrastructure(Box::new(
            CompilerError::compiler_error(
                "Fallible external call result is missing its constructed carrier type",
            ),
        )));
    };
    if carrier.constructor != TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) {
        return Err(BackendFeatureValidationError::Infrastructure(Box::new(
            CompilerError::compiler_error(
                "Fallible external call result does not use the fallible carrier constructor",
            ),
        )));
    }

    let [success_type, _error_type] = carrier.arguments.as_ref() else {
        return Err(BackendFeatureValidationError::Infrastructure(Box::new(
            CompilerError::compiler_error(
                "Fallible external call result carrier does not contain success and error types",
            ),
        )));
    };

    Ok(Some(*success_type))
}

fn signature_type_uses_unsupported_numeric_profile(
    signature_type: &ExternalSignatureType,
    int_uses_bigint: bool,
    float_uses_binary32: bool,
) -> bool {
    match signature_type {
        ExternalSignatureType::Abi(ExternalAbiType::I32) => int_uses_bigint,
        ExternalSignatureType::Abi(ExternalAbiType::F64) => float_uses_binary32,
        ExternalSignatureType::Optional(inner) => signature_type_uses_unsupported_numeric_profile(
            inner,
            int_uses_bigint,
            float_uses_binary32,
        ),
        // Error is converted by the compiler-owned fallible glue. Inferred signatures are checked
        // against concrete HIR argument and result types below.
        ExternalSignatureType::Abi(_)
        | ExternalSignatureType::BuiltinError
        | ExternalSignatureType::External(_)
        | ExternalSignatureType::StringContent => false,
    }
}

/// Reports a reachable fixed-width scalar form that is outside the bounded Wasm scalar step.
///
/// WHAT: direct integer/Byte and F32/F64 scalar values are accepted, but F16 and aggregate types
///       containing any fixed scalar remain unsupported.
/// WHY: reject early with a structured target diagnostic rather than an internal lowering error.
fn validate_fixed_width_scalar_values(
    hir: &HirModule,
    type_environment: Option<&TypeEnvironment>,
    selection: &HirBackendSelection,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let type_environment = require_type_environment(
        type_environment,
        target,
        "fixed-width numeric and Byte values",
    )?;
    let mut backend_type_facts = BackendTypeFacts::new(type_environment, None);

    // Reachable block occurrences take source precedence over return signatures. Within each pass,
    // a source-mapped occurrence wins over earlier generated HIR that has no source provenance.
    let module_occurrences =
        first_unsupported_module_occurrence(hir, selection.blocks(), &mut |type_id| {
            match type_environment.fixed_scalar(type_id) {
                Some(FixedScalar::F16) => true,
                Some(_) => false,
                None => backend_type_facts.contains_fixed_scalar(type_id),
            }
        });
    let occurrence = if let Some(authored) = module_occurrences.authored {
        Some(authored)
    } else {
        let signature_occurrences =
            first_unsupported_function_signature_occurrence(hir, selection, &mut |type_id| {
                match type_environment.fixed_scalar(type_id) {
                    Some(FixedScalar::F16) => true,
                    Some(_) => false,
                    None => backend_type_facts.contains_fixed_scalar(type_id),
                }
            });
        signature_occurrences
            .authored
            .or(module_occurrences.spanless_fallback)
            .or(signature_occurrences.spanless_fallback)
    };

    let Some(occurrence) = occurrence else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::FixedWidthScalarValues,
        occurrence.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Resolves the semantic type environment every type-shape gate classifies reachable types with.
///
/// WHY: a missing environment means the gate cannot classify anything, so it is an internal
///      invariant failure rather than a reason to silently accept unsupported values.
fn require_type_environment<'environment>(
    type_environment: Option<&'environment TypeEnvironment>,
    target: BackendTarget,
    feature: &str,
) -> Result<&'environment TypeEnvironment, BackendFeatureValidationError> {
    type_environment.ok_or_else(|| {
        BackendFeatureValidationError::Infrastructure(Box::new(CompilerError::compiler_error(
            format!(
                "Backend feature validation for {} requires a TypeEnvironment to detect {feature}",
                target.as_str()
            ),
        )))
    })
}

fn validate_runtime_assertion_messages(
    messages: &[ReachableAssertionMessageUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(message) = messages.iter().find(|message| {
        matches!(
            message.evaluation,
            crate::compiler_frontend::hir::terminators::HirAssertionMessageEvaluation::Runtime
        )
    }) else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::RuntimeAssertionMessages,
        message.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

fn validate_wasm_cross_module_calls(
    hir: &HirModule,
    reachability: &HirReachability,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    if reachability.reachable_cross_module_functions.is_empty() {
        return Ok(());
    }

    let selected_blocks = reachability.backend_selection().blocks();
    let statement = hir
        .blocks
        .iter()
        .filter(|block| selected_blocks.contains(&block.id))
        .flat_map(|block| &block.statements)
        .find(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Call {
                    target: crate::compiler_frontend::external_packages::CallTarget::CrossModule(_),
                    ..
                }
            )
        })
        .ok_or_else(|| {
            BackendFeatureValidationError::Infrastructure(Box::new(
                CompilerError::compiler_error(
                    "Wasm validation received cross-module reachability without a selected cross-module call statement",
                ),
            ))
        })?;
    let backend_name = string_table.intern(match target {
        BackendTarget::Wasm => "Wasm",
        BackendTarget::Js => "JavaScript",
    });
    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        backend_name,
        UnsupportedBackendFeatureReason::CrossModuleCalls,
        statement.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports the first reachable unsupported hashmap operation for the Wasm target.
///
/// WHAT: hashmap literals and operations are valid HIR, but Wasm lowering does not yet
/// support them.
/// WHY: reject early with a structured diagnostic carrying the source span instead of a
/// backend-internal lowering failure.
fn validate_wasm_maps(
    map_uses: &[ReachableMapUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(map_use) = map_uses.first() else {
        return Ok(());
    };

    let reason = match &map_use.kind {
        ReachableMapUseKind::Literal => UnsupportedBackendFeatureReason::HashmapConstruction,
        ReachableMapUseKind::Operation(_) => UnsupportedBackendFeatureReason::HashmapOperation,
    };

    // Only the first reachable unsupported operation is reported. Unreachable helpers remain
    // valid typed HIR and do not block the build.
    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        reason,
        map_use.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports the first reachable reactive runtime feature for the Wasm target.
///
/// WHAT: reactive template values with runtime dependencies are valid HIR, but HTML-Wasm does
///       not yet have a reactive runtime design.
/// WHY: reject early with a structured diagnostic carrying the source span instead of a
///      backend-internal lowering failure. Unreachable helper functions containing reactive
///      templates remain valid typed HIR and do not block the build.
fn validate_wasm_reactive_features(
    reactive_templates: &[ReachableReactiveTemplateUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(reactive_template) = reactive_templates.first() else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::ReactiveTemplateRuntime,
        reactive_template.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports the first reachable runtime cast outside Wasm's infallible expression conversions.
///
/// WHAT: accepts evidence-approved integer and F32/F64 expression conversions plus Byte/U8;
///       F16, fallible pairs and statement casts remain gated.
/// WHY: target validation consumes retained cast evidence before lowering, preserving authored
///      spans and keeping fallible conversion semantics out of the trap-only Wasm path.
fn validate_wasm_runtime_casts(
    runtime_casts: &[ReachableRuntimeCastUse],
    target: BackendTarget,
    numeric_profile: NumericProfile,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(runtime_cast) = runtime_casts
        .iter()
        .find(|runtime_cast| !wasm_supports_runtime_cast(runtime_cast, numeric_profile))
    else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::RuntimeCasts,
        runtime_cast.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

fn wasm_supports_runtime_cast(
    runtime_cast: &ReachableRuntimeCastUse,
    numeric_profile: NumericProfile,
) -> bool {
    if runtime_cast.form != ReachableRuntimeCastForm::Expression {
        return false;
    }

    match runtime_cast.policy {
        BuiltinCastPolicyId::NumericConversion { source, target } => {
            wasm_supports_numeric_conversion(source, target, numeric_profile)
        }
        BuiltinCastPolicyId::ByteToU8 | BuiltinCastPolicyId::U8ToByte => true,
        _ => false,
    }
}

fn wasm_supports_numeric_conversion(
    source: NumericScalar,
    target: NumericScalar,
    numeric_profile: NumericProfile,
) -> bool {
    if source == target
        || numeric_conversion_fallibility(source, target, numeric_profile)
            != BuiltinCastFallibility::Infallible
    {
        return false;
    }

    if source.is_integer() && target.is_integer() {
        return true;
    }

    if source.is_integer() && wasm_has_float_carrier(target, numeric_profile) {
        return true;
    }

    match (
        source.binary_float_precision(numeric_profile),
        target.binary_float_precision(numeric_profile),
    ) {
        (Some(source_precision), Some(target_precision)) => {
            matches!(
                (source_precision, target_precision),
                (
                    BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64,
                    BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64
                )
            ) && target_precision >= source_precision
        }
        _ => false,
    }
}

fn wasm_has_float_carrier(domain: NumericScalar, numeric_profile: NumericProfile) -> bool {
    matches!(
        domain.binary_float_precision(numeric_profile),
        Some(BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64)
    )
}

/// Reports the first reachable checked numeric operation outside Wasm's trap-mode integer and
/// binary32/binary64 paths.
///
/// WHAT: trap-mode integer and supported float operations are admitted. ReturnError, F16 and
///       integer-division-on-float operations remain target-gated.
/// WHY: lowerers receive only operations whose exact failure mode and semantic precision they
///      implement, using the operation/domain facts retained by HIR reachability.
fn validate_wasm_checked_numeric_ops(
    numeric_ops: &[ReachableNumericOpUse],
    target: BackendTarget,
    numeric_profile: NumericProfile,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(numeric_op) = numeric_ops
        .iter()
        .find(|numeric_op| !wasm_supports_checked_numeric_op(numeric_op, numeric_profile))
    else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
        numeric_op.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

fn wasm_supports_checked_numeric_op(
    numeric_op: &ReachableNumericOpUse,
    numeric_profile: NumericProfile,
) -> bool {
    if numeric_op.failure_mode != NumericFailureMode::Trap {
        return false;
    }

    if numeric_op.op.domain.is_integer() {
        return true;
    }

    matches!(
        numeric_op.op.domain.binary_float_precision(numeric_profile),
        Some(BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64)
    ) && numeric_op.op.operator != NumericOperator::IntegerDivide
}

/// Reports the first reachable Float formatting or validation statement for the Wasm target.
///
/// WHAT: Moth Float formatting and external-Float boundary validation are valid HIR, but
///       HTML-Wasm does not yet implement the helper and trap/recoverability contract for
///       `HirStatementKind::FormatFloat` or `HirStatementKind::ValidateFloat`.
/// WHY: reject early with a structured unsupported-backend diagnostic instead of letting Wasm LIR
///      lowering report an infrastructure failure.
fn validate_wasm_float_statements(
    float_statements: &[ReachableFloatStatementUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(float_statement) = float_statements.first() else {
        return Ok(());
    };

    let reason = match float_statement.kind {
        ReachableFloatStatementKind::FormatFloat => {
            UnsupportedBackendFeatureReason::FloatFormatting
        }
        ReachableFloatStatementKind::ValidateFloat => {
            UnsupportedBackendFeatureReason::FloatBoundaryValidation
        }
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        reason,
        float_statement.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports the first reachable generic runtime value for the Wasm target.
///
/// WHAT: generic nominal instances such as `Box of String` are valid HIR, but HTML-Wasm does not
///       yet have a generic runtime representation.
/// WHY: reject early with a structured diagnostic carrying the source span instead of a
///      backend-internal lowering failure.
fn validate_wasm_generic_runtime_values(
    hir: &HirModule,
    type_environment: Option<&TypeEnvironment>,
    reachable_blocks: &FxHashSet<BlockId>,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let type_environment =
        require_type_environment(type_environment, target, "generic runtime values")?;

    let Some(occurrence) =
        first_unsupported_module_occurrence(hir, reachable_blocks, &mut |type_id| {
            matches!(
                type_environment.get(type_id),
                Some(TypeDefinition::GenericInstance(_))
            )
        })
        .preferred()
    else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::GenericRuntimeValues,
        occurrence.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports a reachable value whose type directly or structurally contains the builtin `Error`.
///
/// WHAT: Wasm does not yet have a runtime representation for compiler-owned Error values.
/// WHY: canonical identity distinguishes the builtin type from user nominals with similar
///      spelling or shape, while the shared occurrence traversal retains reachable source sites.
fn validate_wasm_error_values(
    hir: &HirModule,
    type_environment: Option<&TypeEnvironment>,
    selection: &HirBackendSelection,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let type_environment = require_type_environment(type_environment, target, "Error values")?;
    let error_type_id = type_environment.type_id_for_canonical_identity(
        &CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
    );
    let Some(error_type_id) = error_type_id else {
        return Ok(());
    };
    let mut backend_type_facts = BackendTypeFacts::new(type_environment, Some(error_type_id));

    let module_occurrences =
        first_unsupported_module_occurrence(hir, selection.blocks(), &mut |type_id| {
            backend_type_facts.contains_error(type_id)
        });
    let occurrence = if let Some(authored) = module_occurrences.authored {
        Some(authored)
    } else {
        let signature_occurrences =
            first_unsupported_function_signature_occurrence(hir, selection, &mut |type_id| {
                backend_type_facts.contains_error(type_id)
            });
        signature_occurrences
            .authored
            .or(module_occurrences.spanless_fallback)
            .or(signature_occurrences.spanless_fallback)
    };

    let Some(occurrence) = occurrence else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::ErrorValues,
        occurrence.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports reachable fallible terminators, which Wasm lowering cannot yet represent.
///
/// WHAT: rejects `FallibleBranch`, `ReturnSuccess` and `ReturnError` only in selected blocks.
/// WHY: exact terminator spans are the authored control-flow site; synthetic HIR remains spanless
///      instead of inheriting provenance from a neighboring construct.
fn validate_wasm_fallible_control_flow(
    hir: &HirModule,
    reachable_blocks: &FxHashSet<BlockId>,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let mut spanless_fallback = None;
    let mut authored_occurrence = None;

    for block in &hir.blocks {
        if !reachable_blocks.contains(&block.id)
            || !matches!(
                &block.terminator,
                HirTerminator::FallibleBranch { .. }
                    | HirTerminator::ReturnSuccess(_)
                    | HirTerminator::ReturnError(_)
            )
        {
            continue;
        }

        let occurrence = ReachableTypeOccurrence {
            span: hir.side_table.terminator_span(block.id).copied(),
        };
        if occurrence.span.is_some() {
            authored_occurrence = Some(occurrence);
            break;
        }
        spanless_fallback.get_or_insert(occurrence);
    }

    let Some(occurrence) = authored_occurrence.or(spanless_fallback) else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::FallibleControlFlow,
        occurrence.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// A reachable unsupported value, with optional source provenance.
///
/// WHAT: keeps semantic detection separate from diagnostic placement so a spanless value still
///       produces the unsupported-feature diagnostic.
/// WHY: generated or synthetic HIR may legitimately omit source spans.
#[derive(Clone, Copy, Debug)]
struct ReachableTypeOccurrence {
    span: Option<SourceSpan>,
}

/// The preferred and fallback reachable occurrences found by a type-shape traversal.
///
/// WHAT: retains the first spanless occurrence for synthetic-only rejection while preferring the
///       first source-mapped occurrence found later in the same traversal.
#[derive(Clone, Copy, Debug, Default)]
struct ReachableTypeOccurrences {
    authored: Option<ReachableTypeOccurrence>,
    spanless_fallback: Option<ReachableTypeOccurrence>,
}

impl ReachableTypeOccurrences {
    fn preferred(self) -> Option<ReachableTypeOccurrence> {
        self.authored.or(self.spanless_fallback)
    }
}

struct TypeOccurrenceSearch<'predicate, IsUnsupported> {
    is_unsupported: &'predicate mut IsUnsupported,
    spanless_fallback: Option<ReachableTypeOccurrence>,
    unsupported_count: usize,
}

impl<'predicate, IsUnsupported> TypeOccurrenceSearch<'predicate, IsUnsupported>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    fn new(is_unsupported: &'predicate mut IsUnsupported) -> Self {
        TypeOccurrenceSearch {
            is_unsupported,
            spanless_fallback: None,
            unsupported_count: 0,
        }
    }

    fn check(
        &mut self,
        type_id: TypeId,
        span: Option<SourceSpan>,
    ) -> Option<ReachableTypeOccurrence> {
        if !(self.is_unsupported)(type_id) {
            return None;
        }

        self.record(span)
    }

    fn record(&mut self, span: Option<SourceSpan>) -> Option<ReachableTypeOccurrence> {
        self.unsupported_count += 1;
        let occurrence = ReachableTypeOccurrence { span };
        if span.is_some() {
            Some(occurrence)
        } else {
            if self.spanless_fallback.is_none() {
                self.spanless_fallback = Some(occurrence);
            }
            None
        }
    }

    fn finish(self, authored: Option<ReachableTypeOccurrence>) -> ReachableTypeOccurrences {
        ReachableTypeOccurrences {
            authored,
            spanless_fallback: self.spanless_fallback,
        }
    }
}

/// Finds reachable unsupported locals, expressions or terminator operands.
///
/// WHAT: one traversal is shared by every type-shape gate; a gate supplies only its type
///       predicate, so reachable-HIR coverage cannot drift between checks.
/// WHY: type-shape gates need the module `TypeEnvironment`, which is not available to
///      backend-neutral HIR reachability collection.
fn first_unsupported_module_occurrence<IsUnsupported>(
    module: &HirModule,
    reachable_blocks: &FxHashSet<BlockId>,
    is_unsupported: &mut IsUnsupported,
) -> ReachableTypeOccurrences
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    let mut search = TypeOccurrenceSearch::new(is_unsupported);
    let mut authored = None;

    'blocks: for block in &module.blocks {
        if !reachable_blocks.contains(&block.id) {
            continue;
        }

        // A declared local carries its own authored span, so an unsupported local type is reported
        // on the binding rather than on a later use.
        for local in &block.locals {
            if let Some(occurrence) = search.check(local.ty, local.span) {
                authored = Some(occurrence);
                break 'blocks;
            }
        }

        for statement in &block.statements {
            if let Some(occurrence) =
                first_unsupported_statement_occurrence(statement, &block.locals, &mut search)
            {
                authored = Some(occurrence);
                break 'blocks;
            }
        }

        if let Some(occurrence) =
            first_unsupported_terminator_occurrence(&block.terminator, &mut search)
        {
            authored = Some(occurrence);
            break 'blocks;
        }
    }

    search.finish(authored)
}

/// Finds a reachable function whose return type a type-shape gate rejects.
///
/// WHAT: scans reachable functions in module order after block occurrences.
/// WHY: parameters are entry-block locals, so the block pass already reports them on their
///      bindings. A return type has no local or expression of its own, so use its recorded
///      declaration span when available; generated or synthetic HIR can still be spanless.
fn first_unsupported_function_signature_occurrence<IsUnsupported>(
    module: &HirModule,
    selection: &HirBackendSelection,
    is_unsupported: &mut IsUnsupported,
) -> ReachableTypeOccurrences
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    let mut search = TypeOccurrenceSearch::new(is_unsupported);
    let mut authored = None;

    for function in &module.functions {
        if !selection.contains_function(function.id)
            || !(search.is_unsupported)(function.return_type)
        {
            continue;
        }

        let span = module
            .side_table
            .hir_source_span_for_hir(HirLocation::Function(function.id));
        if let Some(occurrence) = search.record(span) {
            authored = Some(occurrence);
            break;
        }
    }

    search.finish(authored)
}

fn first_unsupported_statement_occurrence<IsUnsupported>(
    statement: &HirStatement,
    locals: &[HirLocal],
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    match &statement.kind {
        HirStatementKind::Assign { target, value } => {
            first_unsupported_place_occurrence(target, search)
                .or_else(|| first_unsupported_expression_occurrence(value, search))
        }
        HirStatementKind::Call { args, .. } => args
            .iter()
            .find_map(|arg| first_unsupported_expression_occurrence(arg, search)),
        HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
            first_unsupported_expression_occurrence(value, search)
        }
        HirStatementKind::CastOp { source, result, .. } => result
            .as_ref()
            .and_then(|result_local| {
                first_unsupported_result_local_occurrence(
                    locals,
                    *result_local,
                    statement.span,
                    search,
                )
            })
            .or_else(|| first_unsupported_expression_occurrence(source, search)),
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            first_unsupported_expression_occurrence(source, search)
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            first_unsupported_expression_occurrence(receiver, search).or_else(|| {
                args.iter()
                    .find_map(|arg| first_unsupported_expression_occurrence(arg, search))
            })
        }
        HirStatementKind::NumericOp {
            operands, result, ..
        } => {
            if let Some(occurrence) =
                first_unsupported_result_local_occurrence(locals, *result, statement.span, search)
            {
                return Some(occurrence);
            }

            let unsupported_before = search.unsupported_count;
            let occurrence = match operands {
                HirNumericOperands::Unary { operand } => {
                    first_unsupported_expression_occurrence(operand, search)
                }
                HirNumericOperands::Binary { left, right } => {
                    first_unsupported_expression_occurrence(left, search)
                        .or_else(|| first_unsupported_expression_occurrence(right, search))
                }
            };

            if let Some(occurrence) = occurrence {
                // NumericOp's source span is the lowering's source anchor for this authored
                // operation, and is more precise than either source operand.
                return Some(ReachableTypeOccurrence {
                    span: statement.span.or(occurrence.span),
                });
            }

            if search.unsupported_count > unsupported_before {
                // Even when an operand is synthetic and spanless, an authored NumericOp span
                // identifies the operation that consumes that unsupported value.
                return statement
                    .span
                    .map(|span| ReachableTypeOccurrence { span: Some(span) });
            }

            None
        }
        HirStatementKind::Drop(_) => None,
    }
}

fn first_unsupported_result_local_occurrence<IsUnsupported>(
    locals: &[HirLocal],
    result_local: LocalId,
    statement_span: Option<SourceSpan>,
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    // Result types were already checked with the block locals. Only a rejected, spanless
    // value needs this producer lookup; successful validation must not rescan locals per operation.
    search.spanless_fallback?;
    let local = locals.iter().find(|local| local.id == result_local)?;
    // The statement span is the source of this local's produced value; never borrow a span from
    // an unrelated local declaration or neighboring HIR node.
    search.check(local.ty, statement_span)
}

fn first_unsupported_terminator_occurrence<IsUnsupported>(
    terminator: &HirTerminator,
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    match terminator {
        HirTerminator::If { condition, .. } => {
            first_unsupported_expression_occurrence(condition, search)
        }
        HirTerminator::FallibleBranch { result, .. }
        | HirTerminator::Return(result)
        | HirTerminator::ReturnSuccess(result)
        | HirTerminator::ReturnError(result) => {
            first_unsupported_expression_occurrence(result, search)
        }
        HirTerminator::Match { scrutinee, arms } => {
            first_unsupported_expression_occurrence(scrutinee, search).or_else(|| {
                arms.iter().find_map(|arm| {
                    first_unsupported_pattern_occurrence(&arm.pattern, search).or_else(|| {
                        arm.guard.as_ref().and_then(|guard| {
                            first_unsupported_expression_occurrence(guard, search)
                        })
                    })
                })
            })
        }
        HirTerminator::AssertFailure { message, .. } => {
            first_unsupported_expression_occurrence(message, search)
        }
        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::RuntimeFailure { .. } => None,
    }
}

/// Finds the first unsupported pattern value in a match arm.
///
/// WHY: a pattern's literal, option payload or relational value is a lowered expression with its
///      own semantic type, so it is scanned with the same predicate as an arm guard.
fn first_unsupported_pattern_occurrence<IsUnsupported>(
    pattern: &HirPattern,
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    match pattern {
        HirPattern::Literal(value)
        | HirPattern::OptionValue { value }
        | HirPattern::OptionRelational { value, .. }
        | HirPattern::Relational { value, .. } => {
            first_unsupported_expression_occurrence(value, search)
        }
        HirPattern::OptionNone
        | HirPattern::OptionPresent
        | HirPattern::Wildcard
        | HirPattern::ChoiceVariant { .. } => None,
    }
}

/// Finds an unsupported index expression inside a place projection.
///
/// WHY: a projected place is not a value, but its index operand has a semantic type and is scanned
///      like every other reachable expression.
fn first_unsupported_place_occurrence<IsUnsupported>(
    place: &HirPlace,
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    match place {
        HirPlace::Local(_) => None,
        HirPlace::Field { base, .. } => first_unsupported_place_occurrence(base, search),
        HirPlace::Index { base, index } => first_unsupported_place_occurrence(base, search)
            .or_else(|| first_unsupported_expression_occurrence(index, search)),
    }
}

fn first_unsupported_expression_occurrence<IsUnsupported>(
    expression: &HirExpression,
    search: &mut TypeOccurrenceSearch<'_, IsUnsupported>,
) -> Option<ReachableTypeOccurrence>
where
    IsUnsupported: FnMut(TypeId) -> bool,
{
    if let Some(occurrence) = search.check(expression.ty, expression.span) {
        return Some(occurrence);
    }

    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            first_unsupported_expression_occurrence(left, search)
                .or_else(|| first_unsupported_expression_occurrence(right, search))
        }
        HirExpressionKind::UnaryOp { operand, .. }
        | HirExpressionKind::TupleGet { tuple: operand, .. }
        | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
        | HirExpressionKind::FallibleUnwrapError { result: operand }
        | HirExpressionKind::Cast {
            source: operand, ..
        }
        | HirExpressionKind::VariantPayloadGet {
            source: operand, ..
        } => first_unsupported_expression_occurrence(operand, search),
        HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
            first_unsupported_place_occurrence(place, search)
        }
        HirExpressionKind::StructConstruct { fields, .. } => fields
            .iter()
            .find_map(|(_, value)| first_unsupported_expression_occurrence(value, search)),
        HirExpressionKind::Collection(items)
        | HirExpressionKind::TupleConstruct { elements: items } => items
            .iter()
            .find_map(|item| first_unsupported_expression_occurrence(item, search)),
        HirExpressionKind::MapLiteral(entries) => entries.iter().find_map(|entry| {
            first_unsupported_expression_occurrence(&entry.key, search)
                .or_else(|| first_unsupported_expression_occurrence(&entry.value, search))
        }),
        HirExpressionKind::Range { start, end } => {
            first_unsupported_expression_occurrence(start, search)
                .or_else(|| first_unsupported_expression_occurrence(end, search))
        }
        HirExpressionKind::VariantConstruct { fields, .. } => fields
            .iter()
            .find_map(|field| first_unsupported_expression_occurrence(&field.value, search)),
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::FixedScalar(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => None,
    }
}

// -----------------------------------------------------------------------------
//  Backend type structure
// -----------------------------------------------------------------------------

/// Memoised semantic type facts used by backend feature gates.
///
/// WHAT: records Error values, fixed-width scalars, Moth `Int` and Moth `Float` through constructed
///       and nominal type structure.
/// WHY: backend gates classify overlapping reachable types, so one cycle-safe walk owns the
///      structural traversal and memoises facts per `TypeId`.
struct BackendTypeFacts<'environment> {
    type_environment: &'environment TypeEnvironment,
    error_type_id: Option<TypeId>,
    memo: FxHashMap<TypeId, BackendTypeVisit>,
    visiting: FxHashSet<TypeId>,
}

impl<'environment> BackendTypeFacts<'environment> {
    fn new(type_environment: &'environment TypeEnvironment, error_type_id: Option<TypeId>) -> Self {
        Self {
            type_environment,
            error_type_id,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    fn contains_fixed_scalar(&mut self, type_id: TypeId) -> bool {
        self.visit(type_id).contains_fixed_scalar
    }

    fn contains_error(&mut self, type_id: TypeId) -> bool {
        self.visit(type_id).contains_error
    }

    fn contains_external_numeric(
        &mut self,
        type_id: TypeId,
        int_uses_bigint: bool,
        float_uses_binary32: bool,
    ) -> bool {
        let visit = self.visit(type_id);
        (int_uses_bigint && visit.contains_int) || (float_uses_binary32 && visit.contains_float)
    }

    /// A recursive nominal cycle is provisional until its outer walk completes.
    fn visit(&mut self, type_id: TypeId) -> BackendTypeVisit {
        if let Some(visit) = self.memo.get(&type_id) {
            return *visit;
        }

        if !self.visiting.insert(type_id) {
            return BackendTypeVisit {
                open_cycle: true,
                ..BackendTypeVisit::EMPTY
            };
        }

        let visit = self.walk(type_id);
        self.visiting.remove(&type_id);

        if !visit.open_cycle {
            self.memo.insert(type_id, visit);
        }

        visit
    }

    fn walk(&mut self, type_id: TypeId) -> BackendTypeVisit {
        let type_environment = self.type_environment;
        let Some(definition) = type_environment.get(type_id) else {
            return BackendTypeVisit::EMPTY;
        };

        let mut visit = BackendTypeVisit {
            contains_error: self.error_type_id == Some(type_id),
            ..BackendTypeVisit::EMPTY
        };
        match definition {
            TypeDefinition::Builtin(builtin) => match builtin.key {
                BuiltinTypeKey::FixedScalar(_) => visit.contains_fixed_scalar = true,
                BuiltinTypeKey::Int => visit.contains_int = true,
                BuiltinTypeKey::Float => visit.contains_float = true,
                _ => {}
            },
            TypeDefinition::Struct(struct_definition) => {
                for field in struct_definition.fields.iter() {
                    visit.merge(self.visit(field.type_id));
                }
            }
            TypeDefinition::Choice(choice_definition) => {
                for variant in choice_definition.variants.iter() {
                    let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload else {
                        continue;
                    };
                    for field in fields.iter() {
                        visit.merge(self.visit(field.type_id));
                    }
                }
            }
            TypeDefinition::Constructed(constructed) => {
                for argument in constructed.arguments.iter() {
                    visit.merge(self.visit(*argument));
                }
            }
            TypeDefinition::Function(function) => {
                for parameter in function.parameters.iter() {
                    visit.merge(self.visit(parameter.type_id));
                }
                for returned in function.returns.iter() {
                    visit.merge(self.visit(*returned));
                }
                if let Some(error_return) = function.error_return {
                    visit.merge(self.visit(error_return));
                }
            }
            TypeDefinition::GenericInstance(instance) => {
                for argument in instance.arguments.iter() {
                    visit.merge(self.visit(*argument));
                }
                if let Some(base_type_id) = type_environment.type_id_for_nominal_id(instance.base) {
                    visit.merge(self.visit(base_type_id));
                }
            }
            TypeDefinition::External(_)
            | TypeDefinition::GenericParameter(_)
            | TypeDefinition::AnonymousConstRecordMarker => {}
        }

        visit
    }
}

#[derive(Clone, Copy)]
struct BackendTypeVisit {
    contains_error: bool,
    contains_fixed_scalar: bool,
    contains_int: bool,
    contains_float: bool,
    open_cycle: bool,
}

impl BackendTypeVisit {
    const EMPTY: Self = Self {
        contains_error: false,
        contains_fixed_scalar: false,
        contains_int: false,
        contains_float: false,
        open_cycle: false,
    };

    fn merge(&mut self, other: Self) {
        self.contains_error |= other.contains_error;
        self.contains_fixed_scalar |= other.contains_fixed_scalar;
        self.contains_int |= other.contains_int;
        self.contains_float |= other.contains_float;
        self.open_cycle |= other.open_cycle;
    }
}

/// Reports the first reachable unsupported reactive sink for the JS target.
///
/// WHAT: JS supports V1 top-level runtime fragment sinks, but reactive template values with
///       runtime subscriptions passed to external/host calls such as `io.line(...)` are deferred.
///       Plain String parameters that merely *could* carry a reactive template are still allowed
///       at unsupported sinks until an actual reactive value flows there.
/// WHY: fail early with a structured diagnostic instead of silently snapshotting a reactive
///      template at an unsupported sink, while avoiding false positives from ordinary String
///      parameters.
fn validate_js_reactive_sinks(
    hir: &HirModule,
    reactive_sinks: &[ReachableReactiveSinkUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(rejected_sink) = reactive_sinks
        .iter()
        .filter(|sink| !matches!(sink.kind, ReachableReactiveSinkKind::RuntimeFragment))
        .find(|sink| sink_template_has_runtime_subscription(hir, sink))
    else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::ReactiveExternalCallSink,
        rejected_sink.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Returns true when the template consumed by `sink` has at least one runtime subscription.
///
/// WHAT: a template with only template-value-parameter placeholders is not yet a live reactive
///       value; it needs an actual `$(source)` subscription to trigger the unsupported-sink rule.
fn sink_template_has_runtime_subscription(
    hir: &HirModule,
    sink: &ReachableReactiveSinkUse,
) -> bool {
    hir.side_table
        .reactive_templates()
        .find(|template| template.id == sink.template_id)
        .is_some_and(|template| !template.dependencies.is_empty())
}
