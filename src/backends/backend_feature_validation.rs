//! Pre-lowering validation for backend-specific HIR feature support.
//!
//! WHAT: rejects reachable HIR operations that are valid language semantics but unsupported by
//! a selected backend target.
//! WHY: backend lowerers should receive only features they can lower, and users should see a
//! structured source diagnostic instead of a backend-internal lowering error.

use crate::backends::external_package_validation::BackendTarget;
use crate::backends::js::JsNumericCarrier;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, TypeConstructor, TypeId,
};
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalJsLowering, ExternalPackageRegistry, ExternalSignatureType,
};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reachability::{
    HirBackendSelection, HirReachability, ReachableAssertionMessageUse, ReachableExternalCall,
    ReachableFloatStatementKind, ReachableFloatStatementUse, ReachableMapUse, ReachableMapUseKind,
    ReachableNumericOpUse, ReachableReactiveSinkKind, ReachableReactiveSinkUse,
    ReachableReactiveTemplateUse, ReachableRuntimeCastUse,
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
///       operations, and generic runtime values are legal HIR, but only the JS backend lowers them
///       for Alpha. HTML-Wasm must reject reachable unsupported operations; unused functions stay
///       type checked but do not block the experimental Wasm build path. Fixed-width numeric and
///       `Byte` values lower on JS, while Wasm rejects reachable values of those types before its
///       target-specific operation checks.
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
            // Wasm does not yet lower hashmaps, reactive runtime features, runtime casts, checked
            // numeric operations, or generic runtime values.
            validate_wasm_maps(&reachability.reachable_map_uses, input.target, string_table)?;
            validate_wasm_reactive_features(
                &reachability.reachable_reactive_templates,
                input.target,
                string_table,
            )?;
            validate_wasm_runtime_casts(
                &reachability.reachable_runtime_casts,
                input.target,
                string_table,
            )?;
            validate_wasm_checked_numeric_ops(
                &reachability.reachable_numeric_ops,
                input.target,
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
    let mut backend_type_facts = BackendTypeFacts::new(type_environment);

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

/// Reports the first reachable fixed-width numeric or `Byte` value for the Wasm target.
///
/// WHAT: `I8`..`U64`, `F16`..`F64` and `Byte` are canonical frontend identities supported by JS
///       lowering, but not by HTML-Wasm. A reachable local, expression or function signature whose
///       type carries one is rejected.
/// WHY: reject early with a structured unsupported-backend diagnostic instead of an internal Wasm
///      lowering error. Unreachable private helpers keep their signatures and stay valid typed HIR.
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
    let mut backend_type_facts = BackendTypeFacts::new(type_environment);

    // Reachable blocks come first so the diagnostic lands on the authored local or expression. The
    // signature pass then covers generated or synthetic values that carry a fixed scalar with no
    // reachable expression to place a span on.
    let occurrence = first_unsupported_module_occurrence(hir, selection.blocks(), &mut |type_id| {
        backend_type_facts.contains_fixed_scalar(type_id)
    })
    .or_else(|| {
        first_unsupported_function_signature_occurrence(hir, selection, &mut |type_id| {
            backend_type_facts.contains_fixed_scalar(type_id)
        })
    });

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

/// Reports the first reachable runtime cast for the Wasm target.
///
/// WHAT: compiler-owned builtin runtime casts are valid HIR, but HTML-Wasm does not yet lower
///       them.
/// WHY: reject early with a structured diagnostic carrying the source span instead of a
///      backend-internal lowering failure.
fn validate_wasm_runtime_casts(
    runtime_casts: &[ReachableRuntimeCastUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(runtime_cast) = runtime_casts.first() else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::RuntimeCasts,
        runtime_cast.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
}

/// Reports the first reachable checked numeric operation for the Wasm target.
///
/// WHAT: checked arithmetic is valid HIR, but HTML-Wasm does not yet implement the helper and
///       trap/recoverability contract for `HirStatementKind::NumericOp`.
/// WHY: reject early with a structured unsupported-backend diagnostic instead of letting Wasm LIR
///      lowering report an infrastructure failure.
fn validate_wasm_checked_numeric_ops(
    numeric_ops: &[ReachableNumericOpUse],
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(numeric_op) = numeric_ops.first() else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::CheckedNumericOperations,
        numeric_op.span,
    );

    Err(BackendFeatureValidationError::Diagnostic(diagnostic))
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

/// A reachable unsupported value, with optional source provenance.
///
/// WHAT: keeps semantic detection separate from diagnostic placement so a spanless value still
///       produces the unsupported-feature diagnostic.
/// WHY: generated or synthetic HIR may legitimately omit source spans.
#[derive(Clone, Copy, Debug)]
struct ReachableTypeOccurrence {
    span: Option<SourceSpan>,
}

/// Finds the first reachable local, expression or terminator operand that a type-shape gate rejects.
///
/// WHAT: one traversal shared by every type-shape gate; a gate supplies only its type predicate, so
///       the reachable HIR walk cannot drift between checks.
/// WHY: type-shape gates need the module `TypeEnvironment`, which is not available in
///      backend-neutral HIR reachability collection.
fn first_unsupported_module_occurrence(
    module: &HirModule,
    reachable_blocks: &FxHashSet<BlockId>,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    for block in &module.blocks {
        if !reachable_blocks.contains(&block.id) {
            continue;
        }

        // A declared local carries its own authored span, so an unsupported local type is reported
        // on the binding rather than on a later use.
        for local in &block.locals {
            if is_unsupported(local.ty) {
                return Some(ReachableTypeOccurrence { span: local.span });
            }
        }

        for statement in &block.statements {
            if let Some(occurrence) =
                first_unsupported_statement_occurrence(statement, is_unsupported)
            {
                return Some(occurrence);
            }
        }

        if let Some(occurrence) =
            first_unsupported_terminator_occurrence(&block.terminator, is_unsupported)
        {
            return Some(occurrence);
        }
    }

    None
}

/// Finds the first reachable function whose return type a type-shape gate rejects.
///
/// WHAT: scans reachable functions in module order after the block pass.
/// WHY: parameters are entry-block locals, so the block pass already reports them on their
///      authored bindings. A return type has no local or expression of its own, so generated or
///      synthetic HIR can carry an unsupported type there alone; that occurrence is spanless.
fn first_unsupported_function_signature_occurrence(
    module: &HirModule,
    selection: &HirBackendSelection,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    for function in &module.functions {
        if selection.contains_function(function.id) && is_unsupported(function.return_type) {
            return Some(ReachableTypeOccurrence { span: None });
        }
    }

    None
}

fn first_unsupported_statement_occurrence(
    statement: &HirStatement,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    match &statement.kind {
        HirStatementKind::Assign { target, value } => {
            first_unsupported_place_occurrence(target, is_unsupported)
                .or_else(|| first_unsupported_expression_occurrence(value, is_unsupported))
        }
        HirStatementKind::Call { args, .. } => args
            .iter()
            .find_map(|arg| first_unsupported_expression_occurrence(arg, is_unsupported)),
        HirStatementKind::Expr(value) | HirStatementKind::PushRuntimeFragment { value, .. } => {
            first_unsupported_expression_occurrence(value, is_unsupported)
        }
        HirStatementKind::CastOp { source, .. }
        | HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            first_unsupported_expression_occurrence(source, is_unsupported)
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            first_unsupported_expression_occurrence(receiver, is_unsupported).or_else(|| {
                args.iter()
                    .find_map(|arg| first_unsupported_expression_occurrence(arg, is_unsupported))
            })
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                first_unsupported_expression_occurrence(operand, is_unsupported)
            }
            HirNumericOperands::Binary { left, right } => {
                first_unsupported_expression_occurrence(left, is_unsupported)
                    .or_else(|| first_unsupported_expression_occurrence(right, is_unsupported))
            }
        },
        HirStatementKind::Drop(_) => None,
    }
}

fn first_unsupported_terminator_occurrence(
    terminator: &HirTerminator,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    match terminator {
        HirTerminator::If { condition, .. } => {
            first_unsupported_expression_occurrence(condition, is_unsupported)
        }
        HirTerminator::FallibleBranch { result, .. }
        | HirTerminator::Return(result)
        | HirTerminator::ReturnSuccess(result)
        | HirTerminator::ReturnError(result) => {
            first_unsupported_expression_occurrence(result, is_unsupported)
        }
        HirTerminator::Match { scrutinee, arms } => {
            first_unsupported_expression_occurrence(scrutinee, is_unsupported).or_else(|| {
                arms.iter().find_map(|arm| {
                    first_unsupported_pattern_occurrence(&arm.pattern, is_unsupported).or_else(
                        || {
                            arm.guard.as_ref().and_then(|guard| {
                                first_unsupported_expression_occurrence(guard, is_unsupported)
                            })
                        },
                    )
                })
            })
        }
        HirTerminator::AssertFailure { message, .. } => {
            first_unsupported_expression_occurrence(message, is_unsupported)
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
fn first_unsupported_pattern_occurrence(
    pattern: &HirPattern,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    match pattern {
        HirPattern::Literal(value)
        | HirPattern::OptionValue { value }
        | HirPattern::OptionRelational { value, .. }
        | HirPattern::Relational { value, .. } => {
            first_unsupported_expression_occurrence(value, is_unsupported)
        }
        HirPattern::OptionNone
        | HirPattern::OptionPresent
        | HirPattern::Wildcard
        | HirPattern::ChoiceVariant { .. } => None,
    }
}

/// Finds the first unsupported index expression inside a place projection.
///
/// WHY: a projected place is not a value, but its index operand is an expression with a semantic
///      type, and an unsupported index type is as unlowed as an unsupported value.
fn first_unsupported_place_occurrence(
    place: &HirPlace,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    match place {
        HirPlace::Local(_) => None,
        HirPlace::Field { base, .. } => first_unsupported_place_occurrence(base, is_unsupported),
        HirPlace::Index { base, index } => first_unsupported_place_occurrence(base, is_unsupported)
            .or_else(|| first_unsupported_expression_occurrence(index, is_unsupported)),
    }
}

fn first_unsupported_expression_occurrence(
    expression: &HirExpression,
    is_unsupported: &mut impl FnMut(TypeId) -> bool,
) -> Option<ReachableTypeOccurrence> {
    if is_unsupported(expression.ty) {
        return Some(ReachableTypeOccurrence {
            span: expression.span,
        });
    }

    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            first_unsupported_expression_occurrence(left, is_unsupported)
                .or_else(|| first_unsupported_expression_occurrence(right, is_unsupported))
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
        } => first_unsupported_expression_occurrence(operand, is_unsupported),
        HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
            first_unsupported_place_occurrence(place, is_unsupported)
        }
        HirExpressionKind::StructConstruct { fields, .. } => fields
            .iter()
            .find_map(|(_, value)| first_unsupported_expression_occurrence(value, is_unsupported)),
        HirExpressionKind::Collection(items)
        | HirExpressionKind::TupleConstruct { elements: items } => items
            .iter()
            .find_map(|item| first_unsupported_expression_occurrence(item, is_unsupported)),
        HirExpressionKind::MapLiteral(entries) => entries.iter().find_map(|entry| {
            first_unsupported_expression_occurrence(&entry.key, is_unsupported)
                .or_else(|| first_unsupported_expression_occurrence(&entry.value, is_unsupported))
        }),
        HirExpressionKind::Range { start, end } => {
            first_unsupported_expression_occurrence(start, is_unsupported)
                .or_else(|| first_unsupported_expression_occurrence(end, is_unsupported))
        }
        HirExpressionKind::VariantConstruct { fields, .. } => fields.iter().find_map(|field| {
            first_unsupported_expression_occurrence(&field.value, is_unsupported)
        }),
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
/// WHAT: records fixed-width scalars, Moth `Int`, and Moth `Float` through constructed and nominal
///       type structure.
/// WHY: backend gates classify overlapping reachable types, so one cycle-safe walk owns the
///      structural traversal and memoises facts per `TypeId`.
struct BackendTypeFacts<'environment> {
    type_environment: &'environment TypeEnvironment,
    memo: FxHashMap<TypeId, BackendTypeVisit>,
    visiting: FxHashSet<TypeId>,
}

impl<'environment> BackendTypeFacts<'environment> {
    fn new(type_environment: &'environment TypeEnvironment) -> Self {
        Self {
            type_environment,
            memo: FxHashMap::default(),
            visiting: FxHashSet::default(),
        }
    }

    fn contains_fixed_scalar(&mut self, type_id: TypeId) -> bool {
        self.visit(type_id).contains_fixed_scalar
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

        let mut visit = BackendTypeVisit::EMPTY;
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
    contains_fixed_scalar: bool,
    contains_int: bool,
    contains_float: bool,
    open_cycle: bool,
}

impl BackendTypeVisit {
    const EMPTY: Self = Self {
        contains_fixed_scalar: false,
        contains_int: false,
        contains_float: false,
        open_cycle: false,
    };

    fn merge(&mut self, other: Self) {
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
