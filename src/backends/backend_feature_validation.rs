//! Pre-lowering validation for backend-specific HIR feature support.
//!
//! WHAT: rejects reachable HIR operations that are valid language semantics but unsupported by
//! a selected backend target.
//! WHY: backend lowerers should receive only features they can lower, and users should see a
//! structured source diagnostic instead of a backend-internal lowering error.

use crate::backends::external_package_validation::BackendTarget;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::reachability::{
    HirReachability, ReachableFloatStatementKind, ReachableFloatStatementUse, ReachableMapUse,
    ReachableMapUseKind, ReachableNumericOpUse, ReachableReactiveSinkKind,
    ReachableReactiveSinkUse, ReachableReactiveTemplateUse, ReachableRuntimeCastUse,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashSet;

/// Failure mode for backend feature validation.
///
/// WHAT: either a user-facing diagnostic for an unsupported selected operation, or an
/// infrastructure error if supplied compiler metadata is inconsistent.
pub enum BackendFeatureValidationError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

/// Explicit build-owned selection consumed by backend feature validation.
///
/// WHAT: backend-neutral validation receives the exact reachable union, active target and optional
/// module type environment needed for typed feature checks.
#[derive(Clone, Debug)]
pub struct BackendFeatureValidationInput<'a> {
    pub hir: &'a HirModule,
    pub reachability: &'a HirReachability,
    pub target: BackendTarget,
    pub type_environment: Option<&'a TypeEnvironment>,
}

/// Validates HIR runtime features that are target-specific after frontend semantics are complete.
///
/// WHAT: hashmap construction/use, reactive runtime features, runtime casts, checked numeric
///       operations, and generic runtime values are legal HIR, but only the JS backend lowers them
///       for Alpha. HTML-Wasm must reject reachable unsupported operations; unused functions stay
///       type checked but do not block the experimental Wasm build path.
/// WHY: fail early with a structured Rule error at the source location instead of a vague
///      backend-internal lowering failure.
pub fn validate_hir_backend_feature_support(
    input: BackendFeatureValidationInput<'_>,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let reachability = input.reachability;

    match input.target {
        BackendTarget::Wasm => {
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
    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        CompilerDiagnostic::unsupported_backend_feature(
            backend_name,
            UnsupportedBackendFeatureReason::CrossModuleCalls,
            statement.location.clone(),
        ),
    )))
}

/// Reports the first reachable unsupported hashmap operation for the Wasm target.
///
/// WHAT: hashmap literals and operations are valid HIR, but Wasm lowering does not yet
/// support them.
/// WHY: reject early with a structured diagnostic at the source location instead of a
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
        map_use.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
}

/// Reports the first reachable reactive runtime feature for the Wasm target.
///
/// WHAT: reactive template values with runtime dependencies are valid HIR, but HTML-Wasm does
///       not yet have a reactive runtime design.
/// WHY: reject early with a structured diagnostic at the source location instead of a
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
        reactive_template.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
}

/// Reports the first reachable runtime cast for the Wasm target.
///
/// WHAT: compiler-owned builtin runtime casts are valid HIR, but HTML-Wasm does not yet lower
///       them.
/// WHY: reject early with a structured diagnostic at the cast source location instead of a
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
        runtime_cast.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
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
        numeric_op.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
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
        float_statement.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
}

/// Reports the first reachable generic runtime value for the Wasm target.
///
/// WHAT: generic nominal instances such as `Box of String` are valid HIR, but HTML-Wasm does not
///       yet have a generic runtime representation.
/// WHY: reject early with a structured diagnostic at the source location instead of a
///      backend-internal lowering failure.
fn validate_wasm_generic_runtime_values(
    hir: &HirModule,
    type_environment: Option<&TypeEnvironment>,
    reachable_blocks: &FxHashSet<BlockId>,
    target: BackendTarget,
    string_table: &mut StringTable,
) -> Result<(), BackendFeatureValidationError> {
    let Some(type_environment) = type_environment else {
        // Generic runtime value detection requires semantic type information. Without it we
        // cannot safely classify expressions, so we treat the absence as an internal invariant
        // failure rather than silently allowing unsupported values through.
        return Err(BackendFeatureValidationError::Infrastructure(Box::new(
            CompilerError::compiler_error(
                "Backend feature validation for Wasm requires a TypeEnvironment to detect generic runtime values",
            ),
        )));
    };

    let Some(location) =
        first_generic_runtime_module_location(hir, type_environment, reachable_blocks)
    else {
        return Ok(());
    };

    let diagnostic = CompilerDiagnostic::unsupported_backend_feature(
        string_table.intern(target.as_str()),
        UnsupportedBackendFeatureReason::GenericRuntimeValues,
        location,
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
}

/// Finds the first reachable expression whose type is a generic instance.
///
/// WHAT: scans only reachable blocks so dead helper bodies do not fail backend validation.
/// WHY: generic runtime value detection needs the module TypeEnvironment, which is not available
///      in backend-neutral HIR reachability collection.
fn first_generic_runtime_module_location(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    reachable_blocks: &FxHashSet<BlockId>,
) -> Option<SourceLocation> {
    for block in &module.blocks {
        if !reachable_blocks.contains(&block.id) {
            continue;
        }

        for statement in &block.statements {
            if let Some(location) =
                first_generic_runtime_statement_location(statement, module, type_environment)
            {
                return Some(location);
            }
        }

        if let Some(location) =
            first_generic_runtime_terminator_location(&block.terminator, module, type_environment)
        {
            return Some(location);
        }
    }

    None
}

fn first_generic_runtime_statement_location(
    statement: &HirStatement,
    module: &HirModule,
    type_environment: &TypeEnvironment,
) -> Option<SourceLocation> {
    match &statement.kind {
        HirStatementKind::Assign { value, .. }
        | HirStatementKind::Expr(value)
        | HirStatementKind::PushRuntimeFragment { value, .. } => {
            first_generic_runtime_expression_location(value, module, type_environment)
        }
        HirStatementKind::Call { args, .. } => args.iter().find_map(|arg| {
            first_generic_runtime_expression_location(arg, module, type_environment)
        }),
        HirStatementKind::CastOp { source, .. } => {
            first_generic_runtime_expression_location(source, module, type_environment)
        }
        HirStatementKind::FormatFloat { source, .. }
        | HirStatementKind::ValidateFloat { source, .. } => {
            first_generic_runtime_expression_location(source, module, type_environment)
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            first_generic_runtime_expression_location(receiver, module, type_environment).or_else(
                || {
                    args.iter().find_map(|arg| {
                        first_generic_runtime_expression_location(arg, module, type_environment)
                    })
                },
            )
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => {
                first_generic_runtime_expression_location(operand, module, type_environment)
            }
            HirNumericOperands::Binary { left, right } => {
                first_generic_runtime_expression_location(left, module, type_environment).or_else(
                    || first_generic_runtime_expression_location(right, module, type_environment),
                )
            }
        },
        HirStatementKind::Drop(_) => None,
    }
}

fn first_generic_runtime_terminator_location(
    terminator: &HirTerminator,
    module: &HirModule,
    type_environment: &TypeEnvironment,
) -> Option<SourceLocation> {
    match terminator {
        HirTerminator::If { condition, .. } => {
            first_generic_runtime_expression_location(condition, module, type_environment)
        }
        HirTerminator::FallibleBranch { result, .. }
        | HirTerminator::Return(result)
        | HirTerminator::ReturnSuccess(result)
        | HirTerminator::ReturnError(result) => {
            first_generic_runtime_expression_location(result, module, type_environment)
        }
        HirTerminator::Match { scrutinee, arms } => first_generic_runtime_expression_location(
            scrutinee,
            module,
            type_environment,
        )
        .or_else(|| {
            arms.iter().find_map(|arm| {
                arm.guard.as_ref().and_then(|guard| {
                    first_generic_runtime_expression_location(guard, module, type_environment)
                })
            })
        }),
        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::AssertFailure { .. } => None,
    }
}

fn first_generic_runtime_expression_location(
    expression: &HirExpression,
    module: &HirModule,
    type_environment: &TypeEnvironment,
) -> Option<SourceLocation> {
    if matches!(
        type_environment.get(expression.ty),
        Some(TypeDefinition::GenericInstance(_))
    ) {
        return Some(
            module
                .side_table
                .value_source_location(expression.id)
                .cloned()
                .unwrap_or_default(),
        );
    }

    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            first_generic_runtime_expression_location(left, module, type_environment).or_else(
                || first_generic_runtime_expression_location(right, module, type_environment),
            )
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
        } => first_generic_runtime_expression_location(operand, module, type_environment),
        HirExpressionKind::StructConstruct { fields, .. } => {
            fields.iter().find_map(|(_, value)| {
                first_generic_runtime_expression_location(value, module, type_environment)
            })
        }
        HirExpressionKind::Collection(items)
        | HirExpressionKind::TupleConstruct { elements: items } => items.iter().find_map(|item| {
            first_generic_runtime_expression_location(item, module, type_environment)
        }),
        HirExpressionKind::MapLiteral(entries) => entries.iter().find_map(|entry| {
            first_generic_runtime_expression_location(&entry.key, module, type_environment).or_else(
                || {
                    first_generic_runtime_expression_location(
                        &entry.value,
                        module,
                        type_environment,
                    )
                },
            )
        }),
        HirExpressionKind::Range { start, end } => {
            first_generic_runtime_expression_location(start, module, type_environment).or_else(
                || first_generic_runtime_expression_location(end, module, type_environment),
            )
        }
        HirExpressionKind::VariantConstruct { fields, .. } => fields.iter().find_map(|field| {
            first_generic_runtime_expression_location(&field.value, module, type_environment)
        }),
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::Load(_)
        | HirExpressionKind::Copy(_) => None,
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
        rejected_sink.location.clone(),
    );

    Err(BackendFeatureValidationError::Diagnostic(Box::new(
        diagnostic,
    )))
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
